//! Read-only analyses over a catalog: the audit report, trace queries, and the next-steps
//! TL;DR.

pub mod next_steps;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::error::Result;
use crate::model::{
    Claim, ClaimId, ClaimKind, ClaimLevel, CurrentVerdict, IntegrityLevel, VerificationMethod,
};
use crate::seal::{self, IntegrityReport};

/// Counts over active claims.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Totals {
    /// Number of active claims.
    pub active_claims: usize,
    /// Number of retired claims.
    pub retired_claims: usize,
    /// Active claims per current verdict.
    pub by_verdict: BTreeMap<CurrentVerdict, usize>,
    /// Active claims per kind.
    pub by_kind: BTreeMap<ClaimKind, usize>,
    /// Active claims per hierarchy level.
    pub by_level: BTreeMap<ClaimLevel, usize>,
    /// Active claims per integrity level (`"1"`..`"4"` or `"unassigned"`).
    pub by_integrity_level: BTreeMap<&'static str, usize>,
}

/// Claim ids per gap category (spec section 4.1). Only active claims are considered.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Gaps {
    /// Claims with no evidence at all.
    pub no_evidence: Vec<ClaimId>,
    /// Claims with no verdict.
    pub unverified: Vec<ClaimId>,
    /// Claims whose current verdict is `refuted`.
    pub refuted: Vec<ClaimId>,
    /// Claims whose current verdict is `not_enough_info`.
    pub not_enough_info: Vec<ClaimId>,
    /// Claims with evidence newer than their latest verdict.
    pub stale: Vec<ClaimId>,
    /// Integrity level 3-4 claims verified only by inspection.
    pub rigor: Vec<ClaimId>,
    /// Claims with at least one active child that is not `supported`.
    pub decomposition: Vec<ClaimId>,
    /// Claims whose `derived_from` includes a retired claim.
    pub dangling_links: Vec<ClaimId>,
}

/// One row of the traceability matrix.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatrixRow {
    /// Claim id.
    pub id: ClaimId,
    /// Claim statement.
    pub statement: String,
    /// Upstream links.
    pub derived_from: Vec<ClaimId>,
    /// Active downstream claims.
    pub children: Vec<ClaimId>,
    /// Intended verification method.
    pub verification_method: VerificationMethod,
    /// Locators of every attached evidence item.
    pub evidence_locators: Vec<String>,
    /// Latest verdict or `unverified`.
    pub current_verdict: CurrentVerdict,
}

/// Result of the `audit_report` tool.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditReport {
    /// Counts.
    pub totals: Totals,
    /// Gap categories.
    pub gaps: Gaps,
    /// Integrity seal verification. When `verified` is false the catalog was changed outside
    /// the server, and none of the verdicts above can be trusted.
    pub integrity: IntegrityReport,
    /// Traceability matrix, when requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<Vec<MatrixRow>>,
}

/// Which way `trace` walks the hierarchy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceDirection {
    /// Ancestors only (`derived_from`).
    Up,
    /// Descendants only (active children).
    Down,
    /// Both directions.
    #[default]
    Both,
}

/// One claim reached by `trace`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceNode {
    /// Claim id.
    pub id: ClaimId,
    /// Claim statement.
    pub statement: String,
    /// Latest verdict or `unverified`.
    pub current_verdict: CurrentVerdict,
    /// Shortest link distance from the root claim.
    pub depth: usize,
    /// `true` when the claim is retired.
    pub retired: bool,
}

/// Result of the `trace` tool. Nodes are in breadth-first order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceResult {
    /// The claim the trace started from (depth 0).
    pub root: TraceNode,
    /// Upstream claims.
    pub ancestors: Vec<TraceNode>,
    /// Downstream claims.
    pub descendants: Vec<TraceNode>,
}

/// Maps every claim id to its active children. Built once per report: O(n + e).
///
/// Rust note: children are `&Claim` borrows into the catalog (lifetime `'c`), so no claim data
/// is copied.
fn children_index(catalog: &Catalog) -> HashMap<ClaimId, Vec<&Claim>> {
    let mut index: HashMap<ClaimId, Vec<&Claim>> = HashMap::new();
    for claim in catalog.claims().iter().filter(|claim| claim.is_active()) {
        for parent in &claim.derived_from {
            index.entry(*parent).or_default().push(claim);
        }
    }
    index
}

/// Builds the audit report in a single pass over the claims plus one child-index build.
pub fn audit_report(catalog: &Catalog, include_matrix: bool) -> AuditReport {
    let children = children_index(catalog);
    let retired: HashSet<ClaimId> = catalog
        .claims()
        .iter()
        .filter(|claim| !claim.is_active())
        .map(|claim| claim.id)
        .collect();

    let mut totals = Totals {
        retired_claims: retired.len(),
        ..Totals::default()
    };
    let mut gaps = Gaps::default();
    let mut matrix = include_matrix.then(Vec::new);
    let no_children: Vec<&Claim> = Vec::new();

    for claim in catalog.claims().iter().filter(|claim| claim.is_active()) {
        let verdict = claim.current_verdict();
        let claim_children = children.get(&claim.id).unwrap_or(&no_children);
        tally(&mut totals, claim, verdict);
        classify_gaps(&mut gaps, claim, verdict, claim_children, &retired);
        if let Some(rows) = matrix.as_mut() {
            rows.push(MatrixRow {
                id: claim.id,
                statement: claim.statement.clone(),
                derived_from: claim.derived_from.clone(),
                children: claim_children.iter().map(|child| child.id).collect(),
                verification_method: claim.verification_method,
                evidence_locators: claim.evidence.iter().map(|e| e.locator.clone()).collect(),
                current_verdict: verdict,
            });
        }
    }

    AuditReport {
        totals,
        gaps,
        integrity: seal::verify(catalog),
        matrix,
    }
}

fn tally(totals: &mut Totals, claim: &Claim, verdict: CurrentVerdict) {
    totals.active_claims += 1;
    *totals.by_verdict.entry(verdict).or_default() += 1;
    *totals.by_kind.entry(claim.kind).or_default() += 1;
    *totals.by_level.entry(claim.level).or_default() += 1;
    // `IntegrityLevel` guarantees 1..=4, so the fallback arm only covers `None`.
    let integrity = match claim.integrity_level.map(IntegrityLevel::get) {
        Some(1) => "1",
        Some(2) => "2",
        Some(3) => "3",
        Some(4) => "4",
        _ => "unassigned",
    };
    *totals.by_integrity_level.entry(integrity).or_default() += 1;
}

fn classify_gaps(
    gaps: &mut Gaps,
    claim: &Claim,
    verdict: CurrentVerdict,
    children: &[&Claim],
    retired: &HashSet<ClaimId>,
) {
    if claim.evidence.is_empty() {
        gaps.no_evidence.push(claim.id);
    }
    match verdict {
        CurrentVerdict::Unverified => gaps.unverified.push(claim.id),
        CurrentVerdict::Refuted => gaps.refuted.push(claim.id),
        CurrentVerdict::NotEnoughInfo => gaps.not_enough_info.push(claim.id),
        CurrentVerdict::Supported => {}
    }
    if is_stale(claim) {
        gaps.stale.push(claim.id);
    }
    if is_high_integrity(claim) && claim.verification_method == VerificationMethod::Inspection {
        gaps.rigor.push(claim.id);
    }
    if children
        .iter()
        .any(|child| child.current_verdict() != CurrentVerdict::Supported)
    {
        gaps.decomposition.push(claim.id);
    }
    if claim
        .derived_from
        .iter()
        .any(|parent| retired.contains(parent))
    {
        gaps.dangling_links.push(claim.id);
    }
}

/// `true` when the claim has evidence recorded after its latest verdict (spec section 4.1).
/// A claim without a verdict is never stale.
fn is_stale(claim: &Claim) -> bool {
    claim.verdicts.last().is_some_and(|latest| {
        claim
            .evidence
            .iter()
            .any(|e| e.recorded_at > latest.recorded_at)
    })
}

/// `true` for integrity level 3 or 4.
fn is_high_integrity(claim: &Claim) -> bool {
    claim.integrity_level.is_some_and(|level| level.get() >= 3)
}

/// Walks the hierarchy from `id` breadth-first, up to `max_depth` links (unlimited if `None`).
///
/// # Errors
///
/// [`crate::error::ClaimsError::ClaimNotFound`] if `id` does not exist.
pub fn trace(
    catalog: &Catalog,
    id: ClaimId,
    direction: TraceDirection,
    max_depth: Option<usize>,
) -> Result<TraceResult> {
    let root = node(catalog.get(id)?, 0);
    let limit = max_depth.unwrap_or(usize::MAX);
    let ancestors = match direction {
        TraceDirection::Up | TraceDirection::Both => breadth_first(catalog, id, limit, |claim| {
            claim.derived_from.iter().copied()
        }),
        TraceDirection::Down => Vec::new(),
    };
    let descendants = match direction {
        TraceDirection::Down | TraceDirection::Both => {
            let children = children_index(catalog);
            breadth_first(catalog, id, limit, |claim| {
                children
                    .get(&claim.id)
                    .into_iter()
                    .flatten()
                    .map(|child| child.id)
            })
        }
        TraceDirection::Up => Vec::new(),
    };
    Ok(TraceResult {
        root,
        ancestors,
        descendants,
    })
}

fn node(claim: &Claim, depth: usize) -> TraceNode {
    TraceNode {
        id: claim.id,
        statement: claim.statement.clone(),
        current_verdict: claim.current_verdict(),
        depth,
        retired: !claim.is_active(),
    }
}

/// Breadth-first search so each claim appears once, at its shortest distance.
///
/// Rust note: the `'c` lifetime says the neighbour iterators borrow from data (the catalog or a
/// child index) that outlives the search, so no per-node `Vec` copies are needed. `I` is any
/// iterator of ids that the `neighbours` closure returns.
fn breadth_first<'c, I: IntoIterator<Item = ClaimId>>(
    catalog: &'c Catalog,
    start: ClaimId,
    limit: usize,
    neighbours: impl Fn(&'c Claim) -> I,
) -> Vec<TraceNode> {
    let mut visited = HashSet::from([start]);
    let mut queue = VecDeque::from([(start, 0_usize)]);
    let mut nodes = Vec::new();
    while let Some((current, depth)) = queue.pop_front() {
        if depth == limit {
            continue;
        }
        let Ok(claim) = catalog.get(current) else {
            continue;
        };
        for next in neighbours(claim) {
            if visited.insert(next) {
                if let Ok(next_claim) = catalog.get(next) {
                    nodes.push(node(next_claim, depth + 1));
                    queue.push_back((next, depth + 1));
                }
            }
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::path::Path;
    use std::process::Command;

    use chrono::DateTime;

    use super::*;
    use crate::catalog::tests::{at, evidence, new_claim, verdict};
    use crate::catalog::NewClaim;
    use crate::model::{ClaimRef, Stance, Verdict};

    fn child_of(statement: &str, parent: ClaimId) -> NewClaim {
        NewClaim {
            derived_from: vec![ClaimRef::Existing(parent)],
            ..new_claim(statement)
        }
    }

    /// root <- mid <- leaf, with leaf supported and mid unverified.
    fn chain() -> (Catalog, ClaimId, ClaimId, ClaimId) {
        let mut catalog = Catalog::default();
        let root = catalog
            .add_claim(new_claim("root"), at(0))
            .expect("root")
            .id;
        let mid = catalog
            .add_claim(child_of("mid", root), at(0))
            .expect("mid")
            .id;
        let leaf = catalog
            .add_claim(child_of("leaf", mid), at(0))
            .expect("leaf")
            .id;
        let support = catalog
            .add_evidence(leaf, evidence(Stance::Supports), at(1))
            .expect("e");
        catalog
            .record_verdict(leaf, verdict(Verdict::Supported, vec![support.id]), at(2))
            .expect("verdict");
        (catalog, root, mid, leaf)
    }

    #[test]
    fn totals_count_active_claims() {
        let (mut catalog, _, _, leaf) = chain();
        let extra = catalog.add_claim(new_claim("extra"), at(3)).expect("extra");
        catalog
            .retire_claim(extra.id, "gone", at(4))
            .expect("retire");
        let report = audit_report(&catalog, false);

        assert_eq!(report.totals.active_claims, 3);
        assert_eq!(report.totals.retired_claims, 1);
        assert_eq!(
            report.totals.by_verdict.get(&CurrentVerdict::Supported),
            Some(&1)
        );
        assert_eq!(report.totals.by_integrity_level.get("unassigned"), Some(&3));
        assert!(report.matrix.is_none());
        assert!(!report.gaps.unverified.contains(&leaf));
    }

    #[test]
    fn gaps_cover_evidence_verdict_and_decomposition() {
        let (catalog, root, mid, leaf) = chain();
        let gaps = audit_report(&catalog, false).gaps;

        assert_eq!(gaps.no_evidence, vec![root, mid]);
        assert_eq!(gaps.unverified, vec![root, mid]);
        // root's child `mid` is unverified; mid's child `leaf` is supported.
        assert_eq!(gaps.decomposition, vec![root]);
        assert!(!gaps.no_evidence.contains(&leaf));
    }

    #[test]
    fn stale_flags_evidence_after_latest_verdict() {
        let (mut catalog, _, _, leaf) = chain();
        catalog
            .add_evidence(leaf, evidence(Stance::Refutes), at(10))
            .expect("late evidence");
        assert_eq!(audit_report(&catalog, false).gaps.stale, vec![leaf]);
    }

    #[test]
    fn rigor_flags_high_integrity_inspection() {
        let mut catalog = Catalog::default();
        let mut input = new_claim("critical");
        input.integrity_level = Some(IntegrityLevel::try_from(4).expect("level"));
        input.verification_method = VerificationMethod::Inspection;
        let claim = catalog.add_claim(input, at(0)).expect("add");
        assert_eq!(audit_report(&catalog, false).gaps.rigor, vec![claim.id]);
    }

    #[test]
    fn refuted_and_not_enough_info_are_listed() {
        let mut catalog = Catalog::default();
        let refuted = catalog.add_claim(new_claim("r"), at(0)).expect("r").id;
        let unknown = catalog.add_claim(new_claim("n"), at(0)).expect("n").id;
        let refute = catalog
            .add_evidence(refuted, evidence(Stance::Refutes), at(1))
            .expect("e");
        catalog
            .record_verdict(refuted, verdict(Verdict::Refuted, vec![refute.id]), at(2))
            .expect("refuted");
        catalog
            .record_verdict(unknown, verdict(Verdict::NotEnoughInfo, Vec::new()), at(2))
            .expect("nei");
        let gaps = audit_report(&catalog, false).gaps;
        assert_eq!(gaps.refuted, vec![refuted]);
        assert_eq!(gaps.not_enough_info, vec![unknown]);
    }

    #[test]
    fn dangling_links_flag_retired_parents() {
        let (mut catalog, root, mid, _) = chain();
        catalog
            .retire_claim(root, "superseded", at(5))
            .expect("retire");
        assert_eq!(audit_report(&catalog, false).gaps.dangling_links, vec![mid]);
    }

    #[test]
    fn matrix_has_one_row_per_active_claim() {
        let (catalog, root, mid, leaf) = chain();
        let matrix = audit_report(&catalog, true)
            .matrix
            .expect("matrix requested");
        assert_eq!(matrix.len(), 3);
        assert_eq!(matrix[0].children, vec![mid]);
        assert_eq!(matrix[2].id, leaf);
        assert_eq!(matrix[2].evidence_locators, vec!["tests/a.rs:1".to_owned()]);
        assert_eq!(matrix[1].derived_from, vec![root]);
    }

    #[test]
    fn trace_walks_both_directions_with_depth() {
        let (catalog, root, mid, leaf) = chain();
        let from_mid = trace(&catalog, mid, TraceDirection::Both, None).expect("trace");
        assert_eq!(from_mid.ancestors.len(), 1);
        assert_eq!(from_mid.ancestors[0].id, root);
        assert_eq!(from_mid.descendants[0].id, leaf);

        let shallow = trace(&catalog, root, TraceDirection::Down, Some(1)).expect("trace");
        assert_eq!(shallow.descendants.len(), 1);
        assert!(shallow.ancestors.is_empty());

        let deep = trace(&catalog, root, TraceDirection::Down, None).expect("trace");
        assert_eq!(
            deep.descendants.iter().map(|n| n.depth).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn report_serializes_to_json() {
        let (catalog, ..) = chain();
        let value = serde_json::to_value(audit_report(&catalog, true)).expect("serialize");
        assert_eq!(value["totals"]["by_verdict"]["supported"], 1);
    }

    /// Compiles `report/claims-report.typ` over `catalog` and checks that it produces a PDF
    /// with no warnings and that its `<audit-report>` metadata equals `audit_report`.
    ///
    /// The Typst CLI is an external tool, so the check is skipped (with a note on stderr) when
    /// `typst` is not on `PATH`, unless `REQUIRE_TYPST=1` is set (as CI should), which turns
    /// the skip into a failure.
    fn assert_template_matches(catalog: &Catalog) {
        let typst_installed = Command::new("typst")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if !typst_installed {
            // Rust note: `env::var` returns `Result`; `is_ok_and` is false for unset variables.
            let required = std::env::var("REQUIRE_TYPST").is_ok_and(|value| value == "1");
            assert!(!required, "REQUIRE_TYPST=1 but `typst` is not on PATH");
            eprintln!("skipping Typst template check: `typst` is not on PATH");
            return;
        }
        let dir = tempfile::tempdir().expect("temp dir");
        let catalog_path = dir.path().join("claims.json");
        std::fs::write(
            &catalog_path,
            serde_json::to_vec(catalog).expect("serialize catalog"),
        )
        .expect("write catalog");
        let template = Path::new(env!("CARGO_MANIFEST_DIR")).join("report/claims-report.typ");
        // Rust note: `OsString` is the platform's native string type; `Command` args take it
        // directly, so paths need no lossy conversion to `String`.
        let mut catalog_input = OsString::from("catalog=");
        catalog_input.push(&catalog_path);
        // The template cannot compute HMACs, so it takes the integrity result as an input, as
        // `just report` passes it from `claims-mcp verify --json`.
        let integrity_path = dir.path().join("integrity.json");
        std::fs::write(
            &integrity_path,
            serde_json::to_vec(&seal::verify(catalog)).expect("serialize integrity"),
        )
        .expect("write integrity");
        let mut integrity_input = OsString::from("integrity=");
        integrity_input.push(&integrity_path);
        // `--root /` lets the template read both the repo template and the temp catalog.
        let typst = |subcommand: &str, extra: &[&OsStr]| {
            let output = Command::new("typst")
                .args([
                    OsStr::new(subcommand),
                    OsStr::new("--root"),
                    OsStr::new("/"),
                ])
                .arg(&template)
                .args(extra)
                .args([OsStr::new("--input"), &catalog_input])
                .args([OsStr::new("--input"), &integrity_input])
                .output()
                .expect("run typst");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "typst {subcommand} failed: {stderr}"
            );
            assert!(
                stderr.trim().is_empty(),
                "typst {subcommand} warned: {stderr}"
            );
            output.stdout
        };

        typst("compile", &[dir.path().join("audit.pdf").as_os_str()]);
        let queried = typst(
            "query",
            &["<audit-report>", "--field", "value", "--one"].map(OsStr::new),
        );

        let from_template: serde_json::Value =
            serde_json::from_slice(&queried).expect("typst query emits JSON");
        let from_rust = serde_json::to_value(audit_report(catalog, true)).expect("serialize");
        assert_eq!(from_template, from_rust);
    }

    #[test]
    fn typst_template_matches_repo_catalog() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("claims.json");
        let catalog: Catalog =
            serde_json::from_slice(&std::fs::read(path).expect("read claims.json"))
                .expect("parse claims.json");
        assert_template_matches(&catalog);
    }

    #[test]
    fn typst_template_handles_empty_catalog() {
        assert_template_matches(&Catalog::default());
    }

    /// Every gap category is non-empty, a claim is retired, and one stale check depends on
    /// sub-second timestamp precision.
    #[test]
    fn typst_template_matches_every_gap_and_retired_claims() {
        let (mut catalog, root, mid, leaf) = chain();
        catalog
            .add_claim(child_of("sibling", mid), at(3))
            .expect("unverified child makes mid a decomposition gap");
        catalog
            .retire_claim(root, "superseded", at(4))
            .expect("retire root leaves mid dangling");
        let nanos_later = DateTime::from_timestamp(2, 5).expect("valid timestamp");
        catalog
            .add_evidence(leaf, evidence(Stance::Neutral), nanos_later)
            .expect("evidence 5ns after the verdict makes leaf stale");

        let mut critical = new_claim("critical");
        critical.integrity_level = Some(IntegrityLevel::try_from(4).expect("level"));
        critical.verification_method = VerificationMethod::Inspection;
        let refuted = catalog.add_claim(critical, at(5)).expect("rigor").id;
        let refute = catalog
            .add_evidence(refuted, evidence(Stance::Refutes), at(6))
            .expect("refuting evidence");
        catalog
            .record_verdict(refuted, verdict(Verdict::Refuted, vec![refute.id]), at(7))
            .expect("refuted");
        let undecided = catalog
            .add_claim(new_claim("undecided"), at(5))
            .expect("nei");
        catalog
            .record_verdict(
                undecided.id,
                verdict(Verdict::NotEnoughInfo, Vec::new()),
                at(7),
            )
            .expect("not enough info");

        let gaps = serde_json::to_value(audit_report(&catalog, false).gaps).expect("gaps");
        let all_gaps_present = gaps
            .as_object()
            .expect("gaps object")
            .values()
            .all(|members| members.as_array().is_some_and(|ids| !ids.is_empty()));
        assert!(all_gaps_present, "fixture must exercise every gap: {gaps}");
        assert_template_matches(&catalog);
    }

    #[test]
    fn trace_unknown_claim_errors() {
        let catalog = Catalog::default();
        assert!(trace(&catalog, ClaimId::new(1), TraceDirection::Both, None).is_err());
    }
}
