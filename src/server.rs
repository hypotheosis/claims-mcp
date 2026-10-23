//! MCP tool layer: parameter types, tool registration, and error mapping. Thin glue only;
//! all logic lives in `catalog`, `report`, and `store`.

use chrono::Utc;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{ClaimFilter, ClaimUpdate, NewClaim, NewEvidence, NewVerdict};
use crate::error;
use crate::model::ClaimId;
use crate::report::{self, next_steps, TraceDirection};
use crate::store::CatalogStore;

/// Parameters for `add_claims`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddClaimsParams {
    /// Claims to create, in order. `derived_from` may use `@N` for an earlier entry.
    pub claims: Vec<NewClaim>,
}

/// Parameters naming one claim.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClaimIdParams {
    /// Claim id, e.g. `CLM-0007`.
    pub id: ClaimId,
}

/// Parameters for `update_claim`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateClaimParams {
    /// Claim to update.
    pub id: ClaimId,
    /// Fields to change.
    #[serde(flatten)]
    pub update: ClaimUpdate,
}

/// Parameters for `retire_claim`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RetireClaimParams {
    /// Claim to retire.
    pub id: ClaimId,
    /// Why it is being retired.
    pub reason: String,
}

/// Parameters for `add_evidence`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddEvidenceParams {
    /// Claim the evidence is about.
    pub claim_id: ClaimId,
    /// The evidence.
    #[serde(flatten)]
    pub evidence: NewEvidence,
}

/// Parameters for `record_verdict`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordVerdictParams {
    /// Claim being judged.
    pub claim_id: ClaimId,
    /// The verdict.
    #[serde(flatten)]
    pub verdict: NewVerdict,
}

/// Parameters for `trace`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TraceParams {
    /// Claim to start from.
    pub id: ClaimId,
    /// `up` (ancestors), `down` (descendants), or `both` (default).
    #[serde(default)]
    pub direction: TraceDirection,
    /// Maximum number of links to follow; unlimited when omitted.
    #[serde(default)]
    pub max_depth: Option<usize>,
}

/// Parameters for `audit_report`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AuditReportParams {
    /// Include the traceability matrix (default true).
    #[serde(default = "default_true")]
    pub include_matrix: bool,
}

/// Parameters for `next_steps`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NextStepsParams {
    /// Items listed in each of tiers 1 and 2 (default 10); the rest are counted per area.
    /// 0 lists everything.
    #[serde(default = "default_next_steps_limit")]
    pub limit: usize,
}

/// Serde default for `NextStepsParams::limit`.
fn default_next_steps_limit() -> usize {
    next_steps::DEFAULT_LIMIT
}

/// Serde default for boolean parameters that are on unless the caller says otherwise.
fn default_true() -> bool {
    true
}

/// The claims MCP server. Cloning is cheap: the store is an `Arc` and the router is shared
/// per clone.
#[derive(Clone)]
pub struct ClaimsServer {
    store: CatalogStore,
    tool_router: ToolRouter<Self>,
}

/// Rust note: `#[tool_router]` reads every `#[tool]` method below and generates a
/// `tool_router()` constructor that registers them with JSON schemas derived from the
/// parameter types (via `schemars`). Doc comments on parameter fields become descriptions.
#[tool_router]
impl ClaimsServer {
    /// Creates a server backed by `store`.
    pub fn new(store: CatalogStore) -> Self {
        Self {
            store,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Create one claim: an atomic, falsifiable assertion. Returns the claim \
        with its new CLM id."
    )]
    async fn add_claim(&self, Parameters(new_claim): Parameters<NewClaim>) -> ToolResult {
        respond(
            self.store
                .mutate(move |catalog| catalog.add_claim(new_claim, Utc::now()))
                .await,
        )
    }

    #[tool(
        description = "Create several claims atomically (all or nothing). In derived_from, \
        '@N' refers to the earlier entry at 0-based index N, so a whole hierarchy can be \
        registered in one call."
    )]
    async fn add_claims(&self, Parameters(params): Parameters<AddClaimsParams>) -> ToolResult {
        respond(
            self.store
                .mutate(move |catalog| catalog.add_claims(params.claims, Utc::now()))
                .await,
        )
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "Get one claim with its evidence, verdict history, current verdict, \
        and active children."
    )]
    async fn get_claim(&self, Parameters(params): Parameters<ClaimIdParams>) -> ToolResult {
        respond(
            self.store
                .read(move |catalog| catalog.detail(params.id))
                .await,
        )
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "List claim summaries. Optional filters: verdict (unverified, \
        supported, refuted, not_enough_info), kind, level, tag, artifact substring, \
        case-insensitive text substring, include_retired."
    )]
    async fn list_claims(&self, Parameters(filter): Parameters<ClaimFilter>) -> ToolResult {
        respond(
            self.store
                .read(move |catalog| Ok(catalog.list(&filter)))
                .await,
        )
    }

    #[tool(
        description = "Update a claim's metadata. Omitted or null fields are unchanged; list \
        fields replace the whole list ([] empties it); source \"\" removes the source; \
        integrity_level cannot be removed. A new derived_from link must point at an active \
        claim; an existing link to a parent retired later may be kept. Evidence and verdicts \
        cannot be edited. To change what a judged claim means, retire it and add a new one."
    )]
    async fn update_claim(&self, Parameters(params): Parameters<UpdateClaimParams>) -> ToolResult {
        respond(
            self.store
                .mutate(move |catalog| catalog.update_claim(params.id, params.update, Utc::now()))
                .await,
        )
    }

    #[tool(
        description = "Retire a claim with a reason. Retired claims stay in the catalog so \
        trace links remain valid, but accept no further evidence or verdicts."
    )]
    async fn retire_claim(&self, Parameters(params): Parameters<RetireClaimParams>) -> ToolResult {
        respond(
            self.store
                .mutate(move |catalog| catalog.retire_claim(params.id, &params.reason, Utc::now()))
                .await,
        )
    }

    #[tool(
        description = "Attach evidence to a claim. Use a precise locator (path:line, test \
        name, command) and an honest stance: supports, refutes, or neutral. Returns the \
        evidence with its EVD id."
    )]
    async fn add_evidence(&self, Parameters(params): Parameters<AddEvidenceParams>) -> ToolResult {
        respond(
            self.store
                .mutate(move |catalog| {
                    catalog.add_evidence(params.claim_id, params.evidence, Utc::now())
                })
                .await,
        )
    }

    #[tool(
        description = "Append a FEVER verdict to a claim's history. 'supported' must cite \
        evidence with stance supports; 'refuted' must cite evidence with stance refutes; \
        'not_enough_info' may cite none. Cited evidence must belong to this claim."
    )]
    async fn record_verdict(
        &self,
        Parameters(params): Parameters<RecordVerdictParams>,
    ) -> ToolResult {
        respond(
            self.store
                .mutate(move |catalog| {
                    catalog.record_verdict(params.claim_id, params.verdict, Utc::now())
                })
                .await,
        )
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "Trace a claim through the requirements hierarchy: ancestors via \
        derived_from (up), active descendants (down), or both, optionally limited to \
        max_depth links."
    )]
    async fn trace(&self, Parameters(params): Parameters<TraceParams>) -> ToolResult {
        respond(
            self.store
                .read(move |catalog| {
                    report::trace(catalog, params.id, params.direction, params.max_depth)
                })
                .await,
        )
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "Audit report over active claims: totals by verdict, kind, level, and \
        integrity level; gaps (no_evidence, unverified, refuted, not_enough_info, stale, \
        rigor, decomposition, dangling_links); integrity seal verification (records changed \
        outside the server); and a traceability matrix unless include_matrix is false."
    )]
    async fn audit_report(&self, Parameters(params): Parameters<AuditReportParams>) -> ToolResult {
        let include_matrix = params.include_matrix;
        respond(
            self.store
                .read(move |catalog| Ok(report::audit_report(catalog, include_matrix)))
                .await,
        )
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "TL;DR of the audit: open leaf claims ranked into tiers, each with one \
        next step taken from the 'Fix:' in its latest verdict rationale. Tier 0 is an \
        integrity failure (restore the catalog from git first), tier 1 refuted claims, tier 2 \
        other open claims at integrity level 3-4, tier 3 the rest counted per area. Tiers 1 \
        and 2 list at most `limit` items (default 10, 0 for all). Use audit_report for the \
        full gap lists."
    )]
    async fn next_steps(&self, Parameters(params): Parameters<NextStepsParams>) -> ToolResult {
        let limit = params.limit;
        respond(
            self.store
                .read(move |catalog| Ok(next_steps::next_steps(catalog, limit)))
                .await,
        )
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "claims-mcp",
    instructions = "Ledger of claims for requirements traceability, IEEE 1012 V&V, and \
        FEVER-style auditing. Register atomic claims, attach evidence you gathered yourself, \
        then record verdicts backed by that evidence. Use audit_report to find gaps. Never edit \
        the catalog file directly: every record is sealed, and the server refuses all writes \
        after a hand edit until the file is restored from git."
)]
impl ServerHandler for ClaimsServer {}

/// Every tool returns this. Domain errors become `Ok` results with `is_error: true` so the
/// calling agent sees the message; only response encoding failures are protocol errors.
///
/// Rust note: the crate's own `Result` alias is not imported here because the rmcp macros
/// expand to code that uses the standard two-parameter `Result`.
type ToolResult = Result<CallToolResult, ErrorData>;

/// Converts a store/catalog result into a tool result carrying compact JSON text.
fn respond<T: Serialize>(result: error::Result<T>) -> ToolResult {
    match result {
        Ok(value) => serde_json::to_string(&value)
            .map(|json| CallToolResult::success(vec![ContentBlock::text(json)]))
            .map_err(|error| ErrorData::internal_error(error.to_string(), None)),
        Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(
            error.to_string(),
        )])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ClaimKind, ClaimLevel, ClaimRef, EvidenceKind, Stance, Verdict, VerificationMethod,
    };

    fn server() -> (tempfile::TempDir, ClaimsServer) {
        let directory = tempfile::tempdir().expect("create temp dir");
        let store = CatalogStore::new(directory.path().join("claims.json"));
        (directory, ClaimsServer::new(store))
    }

    fn text_of(result: &CallToolResult) -> &str {
        match result.content.first() {
            Some(ContentBlock::Text(text)) => &text.text,
            other => panic!("expected text content, got {other:?}"),
        }
    }

    fn json_of(result: &CallToolResult) -> serde_json::Value {
        serde_json::from_str(text_of(result)).expect("tool output is JSON")
    }

    fn claim_input(statement: &str, derived_from: Vec<ClaimRef>) -> NewClaim {
        NewClaim {
            statement: statement.to_owned(),
            kind: ClaimKind::Functional,
            level: ClaimLevel::Software,
            verification_method: VerificationMethod::Test,
            integrity_level: None,
            source: Some("prompt".to_owned()),
            derived_from,
            artifacts: Vec::new(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn read_only_tools_are_annotated() {
        let (_directory, server) = server();
        let mut read_only: Vec<String> = server
            .tool_router
            .list_all()
            .into_iter()
            .filter(|tool| {
                tool.annotations
                    .as_ref()
                    .is_some_and(|annotations| annotations.read_only_hint == Some(true))
            })
            .map(|tool| tool.name.into_owned())
            .collect();
        read_only.sort();
        assert_eq!(
            read_only,
            vec![
                "audit_report",
                "get_claim",
                "list_claims",
                "next_steps",
                "trace"
            ]
        );
    }

    #[test]
    fn registers_all_eleven_tools() {
        let (_directory, server) = server();
        let mut names: Vec<String> = server
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "add_claim",
                "add_claims",
                "add_evidence",
                "audit_report",
                "get_claim",
                "list_claims",
                "next_steps",
                "record_verdict",
                "retire_claim",
                "trace",
                "update_claim",
            ]
        );
    }

    #[tokio::test]
    async fn next_steps_lists_open_leaves_as_json() {
        let (_directory, server) = server();
        server
            .add_claim(Parameters(claim_input("unjudged", Vec::new())))
            .await
            .expect("add_claim");

        let result = server
            .next_steps(Parameters(NextStepsParams { limit: 10 }))
            .await
            .expect("next_steps");

        let steps = json_of(&result);
        assert_eq!(steps["summary"]["open_leaves"], 1);
        assert_eq!(steps["wrong"]["total"], 0);
        assert_eq!(steps["other"][0]["area"], "(none)");
        assert_eq!(steps["integrity"]["verified"], true);
    }

    #[test]
    fn next_steps_limit_defaults_to_ten() {
        let params: NextStepsParams =
            serde_json::from_value(serde_json::json!({})).expect("empty object parses");
        assert_eq!(params.limit, 10);
    }

    #[tokio::test]
    async fn full_audit_flow_round_trips_json() {
        let (_directory, server) = server();
        let added = server
            .add_claims(Parameters(AddClaimsParams {
                claims: vec![
                    claim_input("root", Vec::new()),
                    claim_input("leaf", vec![ClaimRef::Batch(0)]),
                ],
            }))
            .await
            .expect("add_claims");
        assert_eq!(json_of(&added)[1]["derived_from"][0], "CLM-0001");

        let leaf: ClaimId = "CLM-0002".parse().expect("id");
        let evidence = server
            .add_evidence(Parameters(AddEvidenceParams {
                claim_id: leaf,
                evidence: NewEvidence {
                    kind: EvidenceKind::Test,
                    locator: "tests/x.rs:1".to_owned(),
                    summary: "passes".to_owned(),
                    stance: Stance::Supports,
                },
            }))
            .await
            .expect("add_evidence");
        assert_eq!(json_of(&evidence)["id"], "EVD-0001");

        let judged = server
            .record_verdict(Parameters(RecordVerdictParams {
                claim_id: leaf,
                verdict: NewVerdict {
                    verdict: Verdict::Supported,
                    rationale: "test passes".to_owned(),
                    auditor: "tester".to_owned(),
                    evidence_ids: vec!["EVD-0001".parse().expect("id")],
                },
            }))
            .await
            .expect("record_verdict");
        assert_eq!(judged.is_error, Some(false));

        let report = server
            .audit_report(Parameters(AuditReportParams {
                include_matrix: true,
            }))
            .await
            .expect("audit_report");
        let report = json_of(&report);
        assert_eq!(report["totals"]["by_verdict"]["supported"], 1);
        assert_eq!(report["gaps"]["unverified"][0], "CLM-0001");
        assert_eq!(report["matrix"].as_array().map(Vec::len), Some(2));
        assert_eq!(report["integrity"]["verified"], true);
    }

    #[tokio::test]
    async fn hand_edited_catalog_turns_writes_into_explained_tool_errors() {
        let (directory, server) = server();
        server
            .add_claim(Parameters(claim_input("original", Vec::new())))
            .await
            .expect("add_claim");
        let path = directory.path().join("claims.json");
        let text = std::fs::read_to_string(&path).expect("read catalog");
        std::fs::write(&path, text.replace("original", "edited")).expect("write catalog");

        let refused = server
            .retire_claim(Parameters(RetireClaimParams {
                id: ClaimId::new(1),
                reason: "cover it up".to_owned(),
            }))
            .await
            .expect("tool errors are Ok results");
        assert_eq!(refused.is_error, Some(true));
        let message = text_of(&refused);
        for expected in [
            "changed outside claims-mcp",
            "CLM-0001",
            "git checkout",
            "Never edit",
        ] {
            assert!(
                message.contains(expected),
                "{expected:?} missing from: {message}"
            );
        }

        let report = server
            .audit_report(Parameters(AuditReportParams {
                include_matrix: false,
            }))
            .await
            .expect("audit_report");
        assert_eq!(json_of(&report)["integrity"]["verified"], false);
    }

    #[tokio::test]
    async fn domain_errors_are_tool_errors_not_protocol_errors() {
        let (_directory, server) = server();
        let result = server
            .get_claim(Parameters(ClaimIdParams {
                id: ClaimId::new(99),
            }))
            .await
            .expect("tool errors are Ok results");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(text_of(&result), "claim CLM-0099 not found");
    }

    #[test]
    fn flattened_params_deserialize_from_flat_json() {
        let params: RecordVerdictParams = serde_json::from_value(serde_json::json!({
            "claim_id": "clm-3",
            "verdict": "not_enough_info",
            "rationale": "no test exists",
            "auditor": "tester"
        }))
        .expect("flat JSON parses");
        assert_eq!(params.claim_id, ClaimId::new(3));
        assert!(params.verdict.evidence_ids.is_empty());
    }
}
