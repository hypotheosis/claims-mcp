//! MCP tool layer: parameter types, tool registration, and error mapping. Thin glue only;
//! all logic lives in `catalog`, `report`, and `store`.

use chrono::Utc;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anchor::{self, GitCli, NewAnchor, NewFileAnchor};
use crate::catalog::{ClaimFilter, ClaimUpdate, NewClaim, NewEvidence, NewVerdict};
use crate::error;
use crate::model::ClaimId;
use crate::recheck;
use crate::report::{self, files, next_steps, TraceDirection};
use crate::store::CatalogStore;

/// A new claim as the tools take it: the claim plus an optional source anchor to pin.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClaimInput {
    /// The claim.
    #[serde(flatten)]
    pub claim: NewClaim,
    /// Exact place the claim is made, such as the README lines that promise a feature. The
    /// server pins it to the current commit.
    #[serde(default)]
    pub source_anchor: Option<NewFileAnchor>,
}

/// Parameters for `add_claims`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddClaimsParams {
    /// Claims to create, in order. `derived_from` may use `@N` for an earlier entry.
    pub claims: Vec<ClaimInput>,
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
    /// New exact place the claim is made; the server pins it to the current commit.
    #[serde(default)]
    pub source_anchor: Option<NewFileAnchor>,
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
    /// Machine-checkable location: `{"file": {"path", "lines": {"start", "end"}}}` for code
    /// or text, or `{"search": {"pattern", "case_insensitive", "scope"}}` for a `git grep -E`
    /// search (zero hits shows something is absent). The server pins it to the current
    /// commit and counts search hits itself.
    #[serde(default)]
    pub anchor: Option<NewAnchor>,
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
        with its new CLM id. Give source_anchor ({path, lines}) when the claim comes from a \
        file in the repository, so a refutation points back at the text to fix."
    )]
    async fn add_claim(&self, Parameters(input): Parameters<ClaimInput>) -> ToolResult {
        let store = self.store.clone();
        respond(
            self.store
                .mutate(move |catalog| {
                    let mut repo = LazyRepo::new(&store);
                    catalog.add_claim(repo.pin_claim(input)?, Utc::now())
                })
                .await,
        )
    }

    #[tool(
        description = "Create several claims atomically (all or nothing). In derived_from, \
        '@N' refers to the earlier entry at 0-based index N, so a whole hierarchy can be \
        registered in one call."
    )]
    async fn add_claims(&self, Parameters(params): Parameters<AddClaimsParams>) -> ToolResult {
        let store = self.store.clone();
        respond(
            self.store
                .mutate(move |catalog| {
                    let mut repo = LazyRepo::new(&store);
                    let claims = params
                        .claims
                        .into_iter()
                        .enumerate()
                        .map(|(index, input)| {
                            repo.pin_claim(input).map_err(|error| {
                                error::ClaimsError::Validation(format!("claims[{index}]: {error}"))
                            })
                        })
                        .collect::<error::Result<Vec<_>>>()?;
                    catalog.add_claims(claims, Utc::now())
                })
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
        let store = self.store.clone();
        respond(
            self.store
                .mutate(move |catalog| {
                    let mut update = params.update;
                    if let Some(input) = params.source_anchor {
                        let repo = GitCli::for_store(&store)?;
                        update.source_anchor = Some(anchor::pin_file(&repo, input)?);
                    }
                    catalog.update_claim(params.id, update, Utc::now())
                })
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
        name, command) and an honest stance: supports, refutes, or neutral. Add an anchor \
        whenever the evidence is in the repository: a file anchor for code or text, or a \
        search anchor to show something is absent (e.g. a promised feature with no \
        implementation). Anchors are pinned to the current commit so recheck can tell when \
        they go stale. Returns the evidence with its EVD id."
    )]
    async fn add_evidence(&self, Parameters(params): Parameters<AddEvidenceParams>) -> ToolResult {
        let store = self.store.clone();
        respond(
            self.store
                .mutate(move |catalog| {
                    let mut evidence = params.evidence;
                    if let Some(input) = params.anchor {
                        let repo = GitCli::for_store(&store)?;
                        evidence.anchor = Some(anchor::pin(&repo, input)?);
                    }
                    catalog.add_evidence(params.claim_id, evidence, Utc::now())
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
        description = "TL;DR of the audit: open claims ranked into tiers, each with one next \
        step taken from the 'Fix:' in its latest verdict rationale. Open claims are leaves that \
        need work, plus parents whose active children are all done but which still need their \
        roll-up verdict. Tier 0 is an integrity failure (make `claims-mcp verify` pass first, \
        usually by restoring the catalog from git), tier 1 refuted claims, tier 2 other open \
        claims at integrity level 3-4, tier 3 the rest counted per area. Tiers 1 and 2 list at \
        most `limit` items (default 10, 0 for all). Use audit_report for the full gap lists."
    )]
    async fn next_steps(&self, Parameters(params): Parameters<NextStepsParams>) -> ToolResult {
        let limit = params.limit;
        respond(
            self.store
                .read(move |catalog| Ok(next_steps::next_steps(catalog, limit)))
                .await,
        )
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "Compare every source and evidence anchor on active claims with the \
        repository's HEAD. Lists each anchor that is not current: moved (same lines, new \
        line numbers), changed (anchored lines or search hit count changed), overturned (an \
        absence search now matches, or a matching search finds nothing), missing (file \
        gone), commit_missing (history rewritten), or unpinned (recorded on a dirty tree). \
        Re-audit the claims it lists."
    )]
    async fn recheck(&self) -> ToolResult {
        let store = self.store.clone();
        respond(
            self.store
                .read(move |catalog| recheck::recheck(catalog, &GitCli::for_store(&store)?))
                .await,
        )
    }
}

/// The catalog's git repository, found only when an anchor needs it, so claims and evidence
/// without anchors work outside git.
struct LazyRepo<'store> {
    store: &'store CatalogStore,
    repo: Option<GitCli>,
}

impl<'store> LazyRepo<'store> {
    fn new(store: &'store CatalogStore) -> Self {
        Self { store, repo: None }
    }

    /// The repository, discovered on first use.
    fn get(&mut self) -> error::Result<&GitCli> {
        if self.repo.is_none() {
            self.repo = Some(GitCli::for_store(self.store)?);
        }
        // Rust note: `get_or_insert_with` cannot be used because discovery can fail.
        Ok(self.repo.as_ref().expect("repository was just discovered"))
    }

    /// Pins the input's source anchor, if any, into the claim.
    fn pin_claim(&mut self, input: ClaimInput) -> error::Result<NewClaim> {
        let mut claim = input.claim;
        if let Some(source) = input.source_anchor {
            claim.source_anchor = Some(anchor::pin_file(self.get()?, source)?);
        }
        Ok(claim)
    }
}

/// Server instructions sent to the client at initialization.
const INSTRUCTIONS: &str = "Ledger of claims for requirements traceability, IEEE 1012 V&V, \
    and FEVER-style auditing. Register atomic claims, attach evidence you gathered yourself, \
    then record verdicts backed by that evidence. Use audit_report to find gaps, and next_steps \
    for a ranked to-do list. Never edit the catalog file directly: every record is sealed, and \
    the server refuses all writes after a hand edit until the file is restored from git. For a \
    PDF audit report, the resources under claims://report/ hold the Typst template and a \
    commented example of the notes file (claims://report/audit-notes.example.toml) that the \
    auditor writes for the audited project; the `claims-mcp report init` command writes them \
    into the project.";

/// Rust note: `#[tool_handler]` fills in the tool methods of `ServerHandler`. It would also
/// generate `get_info`, but only with the tools capability, so `get_info` is written here to
/// add resources.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for ClaimsServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("claims-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS.to_owned())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(report_resources()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        read_report_resource(&request.uri).map(ReadResourceResponse::from)
    }
}

/// One resource per embedded report file.
fn report_resources() -> Vec<Resource> {
    files::FILES
        .iter()
        .map(|file| {
            Resource::new(file.uri(), file.name)
                .with_description(file.description)
                .with_mime_type(file.mime_type)
                // Rust note: `as u64` widens `usize`, which never loses data on 32- or 64-bit.
                .with_size(file.contents.len() as u64)
        })
        .collect()
}

/// The embedded report file at `uri`, or a `resource_not_found` error naming it.
fn read_report_resource(uri: &str) -> Result<ReadResourceResult, ErrorData> {
    let file = files::by_uri(uri)
        .ok_or_else(|| ErrorData::resource_not_found(format!("no resource at {uri}"), None))?;
    Ok(ReadResourceResult::new(vec![ResourceContents::text(
        file.contents,
        uri,
    )
    .with_mime_type(file.mime_type)]))
}

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
    use std::path::Path;

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

    fn claim_input(statement: &str, derived_from: Vec<ClaimRef>) -> ClaimInput {
        ClaimInput {
            claim: new_claim(statement, derived_from),
            source_anchor: None,
        }
    }

    fn new_claim(statement: &str, derived_from: Vec<ClaimRef>) -> NewClaim {
        NewClaim {
            statement: statement.to_owned(),
            kind: ClaimKind::Functional,
            level: ClaimLevel::Software,
            verification_method: VerificationMethod::Test,
            integrity_level: None,
            source: Some("prompt".to_owned()),
            source_anchor: None,
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
                "recheck",
                "trace"
            ]
        );
    }

    /// A real MCP client connected over an in-memory pipe lists every report file as a
    /// resource and reads back exactly the embedded contents.
    #[tokio::test]
    async fn client_lists_and_reads_report_resources() {
        use rmcp::ServiceExt;

        let (_directory, server) = server();
        // Rust note: `duplex` returns two connected in-memory byte streams, one per side.
        let (server_side, client_side) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            server
                .serve(server_side)
                .await
                .expect("server starts")
                .waiting()
                .await
        });
        // `()` is rmcp's do-nothing client handler.
        let client = ().serve(client_side).await.expect("client connects");

        let info = client.peer_info().expect("server info");
        assert!(info.capabilities.resources.is_some());
        let instructions = info.instructions.as_deref().expect("instructions");
        assert!(instructions.contains(&files::NOTES_EXAMPLE.uri()));
        assert!(instructions.contains("claims-mcp report init"));

        let listed = client.list_all_resources().await.expect("list resources");
        let uris: Vec<_> = listed
            .iter()
            .map(|resource| resource.uri.as_str())
            .collect();
        let expected: Vec<_> = files::FILES.iter().map(files::ReportFile::uri).collect();
        assert_eq!(uris, expected);

        let read = client
            .read_resource(ReadResourceRequestParams::new(files::NOTES_EXAMPLE.uri()))
            .await
            .expect("read notes example");
        match read.contents.as_slice() {
            [ResourceContents::TextResourceContents { text, .. }] => {
                assert_eq!(text, files::NOTES_EXAMPLE.contents);
            }
            other => panic!("expected one text content, got {other:?}"),
        }
        assert!(client
            .read_resource(ReadResourceRequestParams::new("claims://report/missing"))
            .await
            .is_err());

        client.cancel().await.expect("client closes");
        server_task
            .await
            .expect("server task")
            .expect("server exits");
    }

    #[test]
    fn registers_all_twelve_tools() {
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
                "recheck",
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
                    anchor: None,
                },
                anchor: None,
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

    /// Runs `git` in `directory`, panicking with its stderr on failure.
    fn git(directory: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(directory)
            .args([
                "-c",
                "user.name=tester",
                "-c",
                "user.email=tester@example.com",
            ])
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A git repository with one commit holding a README and a source file, and a server
    /// whose catalog lives in it.
    fn repo_server() -> (tempfile::TempDir, ClaimsServer) {
        let directory = tempfile::tempdir().expect("create temp dir");
        let root = directory.path();
        git(root, &["init", "--quiet"]);
        std::fs::write(
            root.join("README.md"),
            "# Demo\n\nFeatures:\n- rate limiting on every endpoint\n",
        )
        .expect("write README");
        std::fs::create_dir(root.join("src")).expect("mkdir src");
        std::fs::write(root.join("src/lib.rs"), "fn serve() {}\n").expect("write lib.rs");
        git(root, &["add", "README.md", "src"]);
        git(root, &["commit", "--quiet", "-m", "initial"]);
        let store = CatalogStore::new(root.join("claims.json"));
        (directory, ClaimsServer::new(store))
    }

    /// The flow for a feature a document promises but the code lacks: the claim is anchored
    /// to the README line, refuted by a zero-hit search, and recheck overturns the refutation
    /// once an implementation is committed.
    #[tokio::test]
    async fn absence_refutation_is_pinned_and_rechecked() {
        let (directory, server) = repo_server();
        let root = directory.path();
        let input: ClaimInput = serde_json::from_value(serde_json::json!({
            "statement": "Every endpoint is rate limited",
            "kind": "functional",
            "level": "software",
            "verification_method": "inspection",
            "source": "README.md Features list",
            "source_anchor": {"path": "README.md", "lines": {"start": 4, "end": 4}}
        }))
        .expect("claim input parses");
        let claim = json_of(
            &server
                .add_claim(Parameters(input))
                .await
                .expect("add_claim"),
        );
        assert_eq!(claim["source_anchor"]["path"], "README.md");
        assert_eq!(
            claim["source_anchor"]["commit"].as_str().map(str::len),
            Some(40)
        );

        let params: AddEvidenceParams = serde_json::from_value(serde_json::json!({
            "claim_id": "CLM-0001",
            "kind": "inspection",
            "locator": "git grep -iE 'throttl|governor' -- .",
            "summary": "No throttling exists anywhere in the repository.",
            "stance": "refutes",
            "anchor": {"search": {"pattern": "throttl|governor", "case_insensitive": true,
                                  "scope": ["."], "hits": 0}}
        }))
        .expect("evidence params parse");
        let evidence = json_of(&server.add_evidence(Parameters(params)).await.expect("add"));
        assert_eq!(evidence["anchor"]["search"]["hits"], 0);
        // The uncommitted catalog and its untracked lock file do not make the anchor dirty.
        assert!(evidence["anchor"]["search"].get("dirty").is_none());

        let report = json_of(
            &server
                .audit_report(Parameters(AuditReportParams {
                    include_matrix: true,
                }))
                .await
                .expect("audit_report"),
        );
        assert_eq!(report["integrity"]["verified"], true);

        // The committed catalog records the pattern and summary, but searches skip it, so the
        // absence still holds.
        git(root, &["add", "claims.json"]);
        git(root, &["commit", "--quiet", "-m", "record the audit"]);
        let current = json_of(&server.recheck().await.expect("recheck"));
        assert_eq!(
            (current["checked"].as_u64(), current["current"].as_u64()),
            (Some(2), Some(2))
        );

        std::fs::write(root.join("src/lib.rs"), "fn serve() {}\nfn throttle() {}\n")
            .expect("implement the feature");
        git(root, &["commit", "--quiet", "-am", "add throttling"]);
        let after = json_of(&server.recheck().await.expect("recheck"));
        assert_eq!(after["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(after["items"][0]["status"], "overturned");
        assert_eq!(after["items"][0]["evidence"], "EVD-0001");
    }

    #[tokio::test]
    async fn anchors_are_validated_against_the_repository() {
        let (_directory, server) = repo_server();
        let mut input = claim_input("anchored", Vec::new());
        input.source_anchor = Some(NewFileAnchor {
            path: "README.md".to_owned(),
            lines: Some(crate::model::LineRange { start: 4, end: 9 }),
        });
        let refused = server
            .add_claim(Parameters(input))
            .await
            .expect("tool result");
        assert_eq!(refused.is_error, Some(true));
        assert!(text_of(&refused).contains("README.md, which has 4 lines"));

        let mut input = claim_input("anchored", Vec::new());
        input.source_anchor = Some(NewFileAnchor {
            path: "../outside.md".to_owned(),
            lines: None,
        });
        let refused = server
            .add_claims(Parameters(AddClaimsParams {
                claims: vec![claim_input("fine", Vec::new()), input],
            }))
            .await
            .expect("tool result");
        assert_eq!(refused.is_error, Some(true));
        assert!(
            text_of(&refused).contains("claims[1]"),
            "{}",
            text_of(&refused)
        );
    }

    #[tokio::test]
    async fn anchors_outside_git_are_explained_errors() {
        let (_directory, server) = server();
        let mut input = claim_input("anchored", Vec::new());
        input.source_anchor = Some(NewFileAnchor {
            path: "README.md".to_owned(),
            lines: None,
        });
        let refused = server
            .add_claim(Parameters(input))
            .await
            .expect("tool result");
        assert_eq!(refused.is_error, Some(true));
        assert!(
            text_of(&refused).starts_with("git rev-parse"),
            "{}",
            text_of(&refused)
        );
        // Claims without anchors still work outside git.
        let added = server
            .add_claim(Parameters(claim_input("plain", Vec::new())))
            .await
            .expect("tool result");
        assert_eq!(added.is_error, Some(false));
    }
}
