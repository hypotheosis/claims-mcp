# Next-steps TL;DR Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a TL;DR mode that ranks a catalog's open leaf claims into priority tiers, each with one exact next step, exposed as the `next_steps` MCP tool and the `claims-mcp tldr` subcommand.

**Architecture:** A new submodule `src/report/next_steps.rs` holds the pure analysis (`next_steps(&Catalog, limit) -> NextSteps`). It reuses `report.rs`'s private child index plus two helpers extracted from `classify_gaps` (`is_stale`, `is_high_integrity`). `server.rs` exposes it as a read-only tool returning JSON. `main.rs` adds the `tldr` subcommand and a Markdown renderer next to the existing `verify` renderer.

**Tech Stack:** Rust 2021, `rmcp` 3.7 tool macros, `serde`/`serde_json`, `clap` derive, `schemars`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-23-next-steps-tldr-design.md`

**Verified:** Every code block in this plan was applied to a scratch copy of the tree at `372976e`, then formatted (`cargo fmt --check` clean), linted (`cargo clippy --all-targets -- -D warnings` clean) and tested (100 tests passing). It was also run against `bdc.json`: 1049 active claims, 32 wrong, 266 unproven, 109 other.

**Working directory:** `/home/pgrace/repos/claims-mcp/.worktrees/claims-mcp-claims-tldr` (branch `claims-mcp/claims-tldr`). `bdc.json` there is untracked. **Never `git add` it**: always add files by explicit path.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `src/report.rs` | Modify | Declare `pub mod next_steps;`; extract `is_stale` and `is_high_integrity` from `classify_gaps` so both analyses share one definition |
| `src/report/next_steps.rs` | Create | Tier ranking, `Fix:` extraction, area counts, and their unit tests |
| `src/server.rs` | Modify | `NextStepsParams`, the `next_steps` tool, and its tests; update the two tool-list tests |
| `src/main.rs` | Modify | `tldr` subcommand, Markdown renderer (`write_tldr`, `write_tier`, `area_list`, `truncated`), and renderer tests |
| `skills/claims-audit/SKILL.md` | Modify | Require `Fix: <step>` at the end of `refuted` / `not_enough_info` rationales |
| `README.md` | Modify | Subcommand row, tool row, and a TL;DR section |
| `docs/superpowers/specs/2026-10-23-next-steps-tldr-design.md` | Modify | Correct the function path to `report::next_steps::next_steps` |
| `changelog/<UTC timestamp>.md` | Create | Changelog entry (project rule: one dated file per iteration) |

Rust note for the reader: a file `src/report.rs` plus a directory `src/report/` is the 2018+ module layout. `pub mod next_steps;` inside `report.rs` loads `src/report/next_steps.rs`, and that child module can use the parent's private functions through `super::`.

---

### Task 1: Extract shared predicates in `report.rs`

Pure refactor. The existing `stale_flags_evidence_after_latest_verdict` and `rigor_flags_high_integrity_inspection` tests already cover both predicates.

**Files:**
- Modify: `src/report.rs` (module doc at line 1; `classify_gaps` at about lines 200-241)

- [ ] **Step 1: Replace the inline stale check in `classify_gaps`**

In `src/report.rs`, replace:

```rust
    if let Some(latest) = claim.verdicts.last() {
        if claim
            .evidence
            .iter()
            .any(|e| e.recorded_at > latest.recorded_at)
        {
            gaps.stale.push(claim.id);
        }
    }
    let high_integrity = claim.integrity_level.is_some_and(|level| level.get() >= 3);
    if high_integrity && claim.verification_method == VerificationMethod::Inspection {
```

with:

```rust
    if is_stale(claim) {
        gaps.stale.push(claim.id);
    }
    if is_high_integrity(claim) && claim.verification_method == VerificationMethod::Inspection {
```

- [ ] **Step 2: Add the two helpers**

Directly above `/// Walks the hierarchy from \`id\` breadth-first, ...` (the doc comment of `pub fn trace`), insert:

```rust
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

```

- [ ] **Step 3: Run the tests**

Run: `cargo test report::`
Expected: PASS. All existing `report::tests::*` pass (Typst template tests print a skip note if `typst` is not installed).

- [ ] **Step 4: Commit**

```bash
git add src/report.rs
git commit -m "refactor: extract is_stale and is_high_integrity from classify_gaps

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `next_steps` analysis module

**Files:**
- Modify: `src/report.rs:1` (module doc and `pub mod` declaration)
- Create: `src/report/next_steps.rs`

- [ ] **Step 1: Declare the module**

In `src/report.rs`, replace line 1:

```rust
//! Read-only analyses over a catalog: the audit report and trace queries.
```

with:

```rust
//! Read-only analyses over a catalog: the audit report, trace queries, and the next-steps
//! TL;DR.

pub mod next_steps;
```

- [ ] **Step 2: Create the module with its types and failing tests**

Create `src/report/next_steps.rs` with exactly this content. The functions the tests call are added in Step 4.

```rust
//! The TL;DR view of a catalog: open leaf claims ranked into priority tiers, each with one
//! next step (spec `docs/superpowers/specs/2026-10-23-next-steps-tldr-design.md`).

use std::cmp::Reverse;
use std::collections::BTreeMap;

use serde::Serialize;

use super::{children_index, is_high_integrity, is_stale};
use crate::catalog::Catalog;
use crate::model::{Claim, ClaimId, CurrentVerdict, IntegrityLevel};
use crate::seal::{self, IntegrityReport};

/// Items listed per tier when the caller does not choose a limit.
pub const DEFAULT_LIMIT: usize = 10;

/// Marker that starts the next step inside a verdict rationale.
const FIX_MARKER: &str = "Fix:";
/// Area of a claim with no `area:` tag.
const NO_AREA: &str = "(none)";
/// Prepended to the step of a claim whose evidence is newer than its verdict.
const STALE_PREFIX: &str = "Re-verify (new evidence since the last verdict): ";
/// Step for a claim with neither evidence nor a verdict.
const NO_EVIDENCE_STEP: &str = "No evidence attached; gather evidence, then record a verdict.";
/// Step for a claim with evidence but no verdict.
const NO_VERDICT_STEP: &str = "Evidence attached but no verdict; record one.";

/// Result of the `next_steps` tool and of `claims-mcp tldr --json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NextSteps {
    /// Integrity seal verification. When `verified` is false, act on nothing below: restore
    /// the catalog from git first (tier 0).
    pub integrity: IntegrityReport,
    /// Claim counts.
    pub summary: NextStepsSummary,
    /// Tier 1: refuted leaves.
    pub wrong: Tier,
    /// Tier 2: other open leaves at integrity level 3 or 4.
    pub unproven: Tier,
    /// Tier 3: the remaining open leaves, counted per area.
    pub other: Vec<AreaCount>,
}

/// Counts over active claims.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct NextStepsSummary {
    /// Number of active claims.
    pub active_claims: usize,
    /// Active claims with at least one active child; their verdicts roll up from children.
    pub rollup_claims: usize,
    /// Leaves that still need work.
    pub open_leaves: usize,
    /// Leaves that are supported, not stale, and have evidence.
    pub done_leaves: usize,
}

/// One priority tier.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Tier {
    /// Number of open leaves in the tier, listed or not.
    pub total: usize,
    /// The highest-priority items, at most the requested limit.
    pub items: Vec<NextStep>,
    /// Per-area counts of the items not listed.
    pub omitted: Vec<AreaCount>,
}

/// One open leaf claim and what to do about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NextStep {
    /// Claim id.
    pub id: ClaimId,
    /// Integrity level, if assigned.
    pub integrity_level: Option<IntegrityLevel>,
    /// Latest verdict or `unverified`.
    pub verdict: CurrentVerdict,
    /// `true` when evidence was recorded after the latest verdict.
    pub stale: bool,
    /// Suffix of the claim's first `area:` tag, or `(none)`.
    pub area: String,
    /// Claim statement, in full.
    pub statement: String,
    /// The one next step (spec section 5).
    pub next_step: String,
    /// Suffixes of the claim's `issue:` tags, in tag order.
    pub issues: Vec<String>,
}

/// Number of claims in one area.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AreaCount {
    /// Area name (`area:` tag suffix) or `(none)`.
    pub area: String,
    /// Number of claims.
    pub count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::{at, evidence, new_claim, verdict};
    use crate::catalog::NewClaim;
    use crate::model::{ClaimRef, Stance, Verdict};

    /// Adds an active, unjudged claim at time 0.
    fn leaf(catalog: &mut Catalog, statement: &str, level: Option<u8>, tags: &[&str]) -> ClaimId {
        let mut input = new_claim(statement);
        input.integrity_level =
            level.map(|value| IntegrityLevel::try_from(value).expect("level 1-4"));
        input.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        catalog.add_claim(input, at(0)).expect("add claim").id
    }

    /// Cites one matching evidence item (time 1) and records `outcome` with `rationale`
    /// (time 2).
    fn judge(catalog: &mut Catalog, id: ClaimId, outcome: Verdict, rationale: &str) {
        let stance = match outcome {
            Verdict::Supported => Stance::Supports,
            Verdict::Refuted => Stance::Refutes,
            Verdict::NotEnoughInfo => Stance::Neutral,
        };
        let cited = catalog
            .add_evidence(id, evidence(stance), at(1))
            .expect("evidence")
            .id;
        let mut input = verdict(outcome, vec![cited]);
        input.rationale = rationale.to_owned();
        catalog.record_verdict(id, input, at(2)).expect("verdict");
    }

    fn ids(tier: &Tier) -> Vec<ClaimId> {
        tier.items.iter().map(|item| item.id).collect()
    }

    fn area(name: &str, count: usize) -> AreaCount {
        AreaCount {
            area: name.to_owned(),
            count,
        }
    }

    #[test]
    fn fix_text_takes_the_text_after_the_last_marker() {
        assert_eq!(fix_text("Bad. Fix: one. Fix:  two. "), Some("two."));
        assert_eq!(fix_text("Bad. Fix:   "), None);
        assert_eq!(fix_text("No marker here."), None);
    }

    #[test]
    fn first_sentence_stops_at_the_first_period_and_space() {
        assert_eq!(first_sentence("v1.2 is out. More text."), "v1.2 is out.");
        assert_eq!(first_sentence("  Single sentence "), "Single sentence");
    }

    #[test]
    fn next_step_prefers_fix_then_first_sentence() {
        let mut catalog = Catalog::default();
        let with_fix = leaf(&mut catalog, "a", Some(4), &[]);
        judge(
            &mut catalog,
            with_fix,
            Verdict::NotEnoughInfo,
            "Untested. Fix: add a test.",
        );
        let empty_fix = leaf(&mut catalog, "b", Some(4), &[]);
        judge(
            &mut catalog,
            empty_fix,
            Verdict::NotEnoughInfo,
            "Untested here. Fix:",
        );

        let steps = next_steps(&catalog, 0);

        let texts: Vec<&str> = steps
            .unproven
            .items
            .iter()
            .map(|item| item.next_step.as_str())
            .collect();
        assert_eq!(texts, vec!["add a test.", "Untested here."]);
    }

    #[test]
    fn unjudged_claims_get_a_fixed_step() {
        let mut catalog = Catalog::default();
        leaf(&mut catalog, "bare", Some(3), &[]);
        let with_evidence = leaf(&mut catalog, "evidenced", Some(3), &[]);
        catalog
            .add_evidence(with_evidence, evidence(Stance::Supports), at(1))
            .expect("evidence");

        let items = next_steps(&catalog, 0).unproven.items;

        assert_eq!(items[0].next_step, NO_EVIDENCE_STEP);
        assert_eq!(items[1].next_step, NO_VERDICT_STEP);
        assert_eq!(items[1].verdict, CurrentVerdict::Unverified);
    }

    #[test]
    fn stale_supported_claim_is_open_with_a_re_verify_step() {
        let mut catalog = Catalog::default();
        let claim = leaf(&mut catalog, "s", Some(3), &[]);
        judge(&mut catalog, claim, Verdict::Supported, "Covered by tests.");
        catalog
            .add_evidence(claim, evidence(Stance::Refutes), at(10))
            .expect("late evidence");

        let steps = next_steps(&catalog, 0);

        assert_eq!(steps.summary.open_leaves, 1);
        let item = &steps.unproven.items[0];
        assert!(item.stale);
        assert_eq!(
            item.next_step,
            "Re-verify (new evidence since the last verdict): Covered by tests."
        );
    }

    #[test]
    fn tiers_follow_verdict_and_integrity_level() {
        let mut catalog = Catalog::default();
        let refuted = leaf(&mut catalog, "r", Some(1), &["area:AUTH"]);
        judge(
            &mut catalog,
            refuted,
            Verdict::Refuted,
            "Wrong. Fix: fix it.",
        );
        let unproven = leaf(&mut catalog, "u", Some(4), &[]);
        judge(&mut catalog, unproven, Verdict::NotEnoughInfo, "No test.");
        let low = leaf(&mut catalog, "l", Some(2), &["area:OPS"]);
        judge(&mut catalog, low, Verdict::NotEnoughInfo, "No test.");
        let done = leaf(&mut catalog, "d", Some(4), &[]);
        judge(&mut catalog, done, Verdict::Supported, "Tested.");
        leaf(&mut catalog, "unassigned", None, &[]);

        let steps = next_steps(&catalog, DEFAULT_LIMIT);

        assert_eq!(ids(&steps.wrong), vec![refuted]);
        assert_eq!(ids(&steps.unproven), vec![unproven]);
        assert_eq!(steps.other, vec![area("(none)", 1), area("OPS", 1)]);
        assert_eq!(
            steps.summary,
            NextStepsSummary {
                active_claims: 5,
                rollup_claims: 0,
                open_leaves: 4,
                done_leaves: 1,
            }
        );
    }

    #[test]
    fn parents_with_active_children_are_roll_ups() {
        let mut catalog = Catalog::default();
        let parent = leaf(&mut catalog, "parent", Some(4), &[]);
        let child = catalog
            .add_claim(
                NewClaim {
                    derived_from: vec![ClaimRef::Existing(parent)],
                    ..new_claim("child")
                },
                at(0),
            )
            .expect("child")
            .id;
        let orphan_parent = leaf(&mut catalog, "orphan parent", Some(4), &[]);
        let retired_child = catalog
            .add_claim(
                NewClaim {
                    derived_from: vec![ClaimRef::Existing(orphan_parent)],
                    ..new_claim("retired child")
                },
                at(0),
            )
            .expect("retired child")
            .id;
        catalog
            .retire_claim(retired_child, "gone", at(1))
            .expect("retire");

        let steps = next_steps(&catalog, 0);

        assert_eq!(steps.summary.rollup_claims, 1);
        assert_eq!(steps.summary.active_claims, 3);
        assert_eq!(ids(&steps.unproven), vec![orphan_parent]);
        assert_eq!(steps.other, vec![area("(none)", 1)]);
        assert!(!ids(&steps.unproven).contains(&child));
    }

    #[test]
    fn tiers_sort_by_level_then_id_and_count_the_overflow() {
        let mut catalog = Catalog::default();
        let c1 = leaf(&mut catalog, "c1", Some(3), &["area:B"]);
        let c2 = leaf(&mut catalog, "c2", Some(4), &["area:A"]);
        let c3 = leaf(&mut catalog, "c3", Some(4), &["area:C"]);
        let c4 = leaf(&mut catalog, "c4", Some(3), &["area:A"]);
        let c5 = leaf(&mut catalog, "c5", Some(3), &["area:C"]);

        let limited = next_steps(&catalog, 1).unproven;
        let everything = next_steps(&catalog, 0).unproven;

        assert_eq!(limited.total, 5);
        assert_eq!(ids(&limited), vec![c2]);
        assert_eq!(
            limited.omitted,
            vec![area("C", 2), area("A", 1), area("B", 1)]
        );
        assert_eq!(ids(&everything), vec![c2, c3, c1, c4, c5]);
        assert!(everything.omitted.is_empty());
    }

    #[test]
    fn unassigned_integrity_sorts_last_in_tier_one() {
        let mut catalog = Catalog::default();
        let unassigned = leaf(&mut catalog, "none", None, &[]);
        judge(&mut catalog, unassigned, Verdict::Refuted, "Wrong.");
        let low = leaf(&mut catalog, "low", Some(1), &[]);
        judge(&mut catalog, low, Verdict::Refuted, "Wrong.");

        assert_eq!(ids(&next_steps(&catalog, 0).wrong), vec![low, unassigned]);
    }

    #[test]
    fn first_area_tag_wins_and_issues_keep_tag_order() {
        let mut catalog = Catalog::default();
        let tags = ["issue:7", "area:X", "area:Y", "issue:9"];
        let claim = leaf(&mut catalog, "tagged", Some(4), &tags);
        judge(&mut catalog, claim, Verdict::Refuted, "Wrong.");

        let item = &next_steps(&catalog, 0).wrong.items[0];

        assert_eq!(item.area, "X");
        assert_eq!(item.issues, vec!["7".to_owned(), "9".to_owned()]);
    }

    #[test]
    fn integrity_is_the_seal_check() {
        let mut catalog = Catalog::default();
        leaf(&mut catalog, "x", None, &[]);
        assert_eq!(next_steps(&catalog, 0).integrity, seal::verify(&catalog));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test next_steps`
Expected: FAIL to compile with `error[E0425]: cannot find function \`fix_text\`` (also `first_sentence`, `next_steps`).

- [ ] **Step 4: Add the implementation**

In `src/report/next_steps.rs`, insert the following between the end of `pub struct AreaCount { ... }` and `#[cfg(test)]`:

```rust
/// Ranks the open leaf claims of `catalog` into tiers. Tiers 1 and 2 list at most `limit`
/// items each (`0` lists everything); the rest are counted per area.
///
/// One child-index build plus one pass over the claims, then a sort of each listed tier.
///
/// # Examples
///
/// ```ignore
/// let steps = next_steps(&catalog, DEFAULT_LIMIT);
/// for item in &steps.wrong.items {
///     println!("{}: {}", item.id, item.next_step);
/// }
/// ```
pub fn next_steps(catalog: &Catalog, limit: usize) -> NextSteps {
    let children = children_index(catalog);
    let mut summary = NextStepsSummary::default();
    // Rust note: these hold `&Claim` borrows into the catalog, so ranking copies no claim data.
    let mut wrong = Vec::new();
    let mut unproven = Vec::new();
    let mut other_areas = Vec::new();

    for claim in catalog.claims().iter().filter(|claim| claim.is_active()) {
        summary.active_claims += 1;
        if children.contains_key(&claim.id) {
            summary.rollup_claims += 1;
            continue;
        }
        let stale = is_stale(claim);
        if is_done(claim, stale) {
            summary.done_leaves += 1;
            continue;
        }
        summary.open_leaves += 1;
        if claim.current_verdict() == CurrentVerdict::Refuted {
            wrong.push((claim, stale));
        } else if is_high_integrity(claim) {
            unproven.push((claim, stale));
        } else {
            other_areas.push(area_of(claim));
        }
    }

    NextSteps {
        integrity: seal::verify(catalog),
        summary,
        wrong: build_tier(wrong, limit),
        unproven: build_tier(unproven, limit),
        other: area_counts(other_areas),
    }
}

/// A leaf needs no work when it is supported, not stale, and has evidence.
fn is_done(claim: &Claim, stale: bool) -> bool {
    claim.current_verdict() == CurrentVerdict::Supported && !stale && !claim.evidence.is_empty()
}

/// Sorts a tier by integrity level (highest first, unassigned last) then id, lists the first
/// `limit` items, and counts the rest per area.
fn build_tier(mut open: Vec<(&Claim, bool)>, limit: usize) -> Tier {
    // Rust note: `Reverse` flips the ordering. `Option` sorts `None` before `Some`, so
    // reversed it puts unassigned levels last. Ids are unique, so an unstable sort is exact.
    open.sort_unstable_by_key(|(claim, _)| (Reverse(claim.integrity_level), claim.id));
    let shown = if limit == 0 {
        open.len()
    } else {
        limit.min(open.len())
    };
    let (listed, rest) = open.split_at(shown);
    Tier {
        total: open.len(),
        items: listed
            .iter()
            .map(|&(claim, stale)| next_step(claim, stale))
            .collect(),
        omitted: area_counts(rest.iter().map(|(claim, _)| area_of(claim)).collect()),
    }
}

fn next_step(claim: &Claim, stale: bool) -> NextStep {
    NextStep {
        id: claim.id,
        integrity_level: claim.integrity_level,
        verdict: claim.current_verdict(),
        stale,
        area: area_of(claim).to_owned(),
        statement: claim.statement.clone(),
        next_step: next_step_text(claim, stale),
        issues: claim
            .tags
            .iter()
            .filter_map(|tag| tag.strip_prefix("issue:"))
            .map(str::to_owned)
            .collect(),
    }
}

/// Spec section 5: the text after the last `Fix:`, else the rationale's first sentence, else
/// a fixed phrase for a claim without a verdict.
fn next_step_text(claim: &Claim, stale: bool) -> String {
    let Some(latest) = claim.verdicts.last() else {
        let step = if claim.evidence.is_empty() {
            NO_EVIDENCE_STEP
        } else {
            NO_VERDICT_STEP
        };
        return step.to_owned();
    };
    let step = fix_text(&latest.rationale).unwrap_or_else(|| first_sentence(&latest.rationale));
    if stale {
        format!("{STALE_PREFIX}{step}")
    } else {
        step.to_owned()
    }
}

/// The trimmed text after the last `Fix:`, or `None` when there is no marker or nothing
/// follows it.
fn fix_text(rationale: &str) -> Option<&str> {
    let start = rationale.rfind(FIX_MARKER)? + FIX_MARKER.len();
    Some(rationale[start..].trim()).filter(|text| !text.is_empty())
}

/// Text up to and including the first `. `, or the whole rationale, trimmed.
fn first_sentence(rationale: &str) -> &str {
    // `find` returns a byte offset; `. ` is ASCII, so `index + 1` is a char boundary.
    let end = rationale
        .find(". ")
        .map_or(rationale.len(), |index| index + 1);
    rationale[..end].trim()
}

/// Suffix of the claim's first `area:` tag, or `(none)`.
fn area_of(claim: &Claim) -> &str {
    claim
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix("area:"))
        .unwrap_or(NO_AREA)
}

/// Counts per area, sorted by count descending, then by area name.
fn area_counts(areas: Vec<&str>) -> Vec<AreaCount> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for area in areas {
        *counts.entry(area).or_default() += 1;
    }
    let mut sorted: Vec<AreaCount> = counts
        .into_iter()
        .map(|(area, count)| AreaCount {
            area: area.to_owned(),
            count,
        })
        .collect();
    // The map yields areas by name and `sort_by_key` is stable, so equal counts stay sorted
    // by name.
    sorted.sort_by_key(|entry| Reverse(entry.count));
    sorted
}

```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test next_steps`
Expected: PASS, 11 tests in `report::next_steps::tests`. (`cargo build` may warn that `next_steps` and the types are never used outside tests; Task 3 wires them up and removes those warnings.)

- [ ] **Step 6: Commit**

```bash
git add src/report.rs src/report/next_steps.rs
git commit -m "feat: next_steps analysis ranking open leaf claims into tiers

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `next_steps` MCP tool

**Files:**
- Modify: `src/server.rs` (import at line 15; params after `AuditReportParams` at about line 84; tool after `audit_report` at about line 260; tests at about lines 363-410)

- [ ] **Step 1: Write the failing tests**

In the `#[cfg(test)] mod tests` of `src/server.rs`:

(a) In `read_only_tools_are_annotated`, replace:

```rust
            vec!["audit_report", "get_claim", "list_claims", "trace"]
```

with:

```rust
            vec![
                "audit_report",
                "get_claim",
                "list_claims",
                "next_steps",
                "trace"
            ]
```

(b) Rename `fn registers_all_ten_tools()` to `fn registers_all_eleven_tools()`, and in its expected list replace:

```rust
                "list_claims",
                "record_verdict",
```

with:

```rust
                "list_claims",
                "next_steps",
                "record_verdict",
```

(c) Directly above `#[tokio::test]\n    async fn full_audit_flow_round_trips_json()`, insert:

```rust
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

```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test server::`
Expected: FAIL to compile: `cannot find struct, variant or union type \`NextStepsParams\`` and `no method named \`next_steps\``.

- [ ] **Step 3: Add the import**

Replace:

```rust
use crate::report::{self, TraceDirection};
```

with:

```rust
use crate::report::{self, next_steps, TraceDirection};
```

- [ ] **Step 4: Add the parameter type**

Directly above `/// Serde default for boolean parameters that are on unless the caller says otherwise.`, insert:

```rust
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

```

- [ ] **Step 5: Add the tool**

Inside `#[tool_router] impl ClaimsServer`, directly after the closing `}` of `async fn audit_report(...)`, insert:

```rust

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
```

Rust note: inside the method, `next_steps::next_steps` names the module function (path syntax). The method itself could only be reached as `self.next_steps`, so the two do not clash.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test server::`
Expected: PASS, including `next_steps_lists_open_leaves_as_json`, `next_steps_limit_defaults_to_ten`, `read_only_tools_are_annotated`, and `registers_all_eleven_tools`.

- [ ] **Step 7: Commit**

```bash
git add src/server.rs
git commit -m "feat: next_steps MCP tool

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: `claims-mcp tldr` subcommand and Markdown renderer

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing tests**

In `src/main.rs`, in `mod tests`, replace:

```rust
    use super::*;
    use crate::seal::IntegrityProblem;
```

with:

```rust
    use super::*;
    use crate::model::{ClaimId, CurrentVerdict, IntegrityLevel};
    use crate::report::next_steps::{NextStep, NextStepsSummary};
    use crate::seal::IntegrityProblem;
```

Then add these items at the end of `mod tests`, just before its closing `}`:

```rust

    fn integrity(problems: usize) -> IntegrityReport {
        IntegrityReport {
            verified: problems == 0,
            records: 9,
            problems: (0..problems)
                .map(|index| IntegrityProblem {
                    record: format!("CLM-000{index}"),
                    problem: "seal does not match its content".to_owned(),
                })
                .collect(),
            adopted: Vec::new(),
        }
    }

    /// One refuted item listed with one more omitted, an empty tier 2, and two tier 3 claims.
    fn sample_steps(problems: usize) -> NextSteps {
        NextSteps {
            integrity: integrity(problems),
            summary: NextStepsSummary {
                active_claims: 6,
                rollup_claims: 1,
                open_leaves: 4,
                done_leaves: 1,
            },
            wrong: Tier {
                total: 2,
                items: vec![NextStep {
                    id: ClaimId::new(12),
                    integrity_level: Some(IntegrityLevel::try_from(4).expect("level")),
                    verdict: CurrentVerdict::Refuted,
                    stale: false,
                    area: "AUTH".to_owned(),
                    statement: "Tokens expire.".to_owned(),
                    next_step: "Add an expiry test.".to_owned(),
                    issues: vec!["360".to_owned(), "361".to_owned()],
                }],
                omitted: vec![AreaCount {
                    area: "OPS".to_owned(),
                    count: 1,
                }],
            },
            unproven: Tier::default(),
            other: vec![AreaCount {
                area: "(none)".to_owned(),
                count: 2,
            }],
        }
    }

    fn rendered_tldr(steps: &NextSteps) -> String {
        let mut out = Vec::new();
        write_tldr(&mut out, "claims.json", steps).expect("write to memory");
        String::from_utf8(out).expect("UTF-8")
    }

    #[test]
    fn tldr_renders_every_tier() {
        assert_eq!(
            rendered_tldr(&sample_steps(0)),
            "# TL;DR: claims.json\n\n\
             6 active claims: 5 leaves (1 done, 4 open), 1 roll-ups.\n\n\
             ## 1. Wrong (2)\n\n\
             - **CLM-0012** IL4 [#360, #361] Tokens expire.\n  \
             - Next: Add an expiry test.\n\n\
             ...and 1 more: OPS 1\n\n\
             ## 2. Unproven, integrity 3-4 (0)\n\n\
             None.\n\n\
             ## 3. Everything else (2)\n\n\
             (none) 2\n"
        );
    }

    #[test]
    fn tldr_opens_with_stop_when_integrity_fails() {
        let text = rendered_tldr(&sample_steps(2));
        let stop = text
            .find("## 0. STOP: integrity check failed\n\n2 records failed")
            .expect("STOP section");
        let wrong = text.find("## 1. Wrong").expect("tier 1");
        assert!(stop < wrong);
        assert!(!rendered_tldr(&sample_steps(0)).contains("STOP"));
    }

    #[test]
    fn tldr_omits_missing_level_and_issues() {
        let mut steps = sample_steps(0);
        steps.wrong.items[0].integrity_level = None;
        steps.wrong.items[0].issues.clear();
        assert!(rendered_tldr(&steps).contains("- **CLM-0012** Tokens expire.\n"));
    }

    #[test]
    fn truncated_cuts_on_char_boundaries() {
        let long = "\u{e9}".repeat(STATEMENT_CHARS + 1);
        let exact = "\u{e9}".repeat(STATEMENT_CHARS);

        assert_eq!(truncated(&long, STATEMENT_CHARS), format!("{exact}..."));
        assert!(matches!(
            truncated(&exact, STATEMENT_CHARS),
            Cow::Borrowed(_)
        ));
    }
```

(`\u{e9}` is the two-byte `e`-acute: the multibyte-truncation test the project rules allow.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test tldr && cargo test truncated` (`cargo test` takes one name filter per run)
Expected: FAIL to compile: `cannot find type \`NextSteps\``, `cannot find function \`write_tldr\``, `cannot find value \`STATEMENT_CHARS\``.

- [ ] **Step 3: Add imports and the truncation constant**

Replace:

```rust
use std::io::Write;
```

with:

```rust
use std::borrow::Cow;
use std::io::Write;
```

Replace:

```rust
use crate::seal::IntegrityReport;
```

with:

```rust
use crate::report::next_steps::{self, AreaCount, NextSteps, Tier};
use crate::seal::IntegrityReport;
```

Directly above `/// Command-line options.`, insert:

```rust
/// Statements longer than this many `char`s are cut in the TL;DR.
const STATEMENT_CHARS: usize = 100;

```

- [ ] **Step 4: Add the subcommand**

In `enum Command`, directly above `/// Seal records that have no seal, for maintainers migrating a catalog.`, insert:

```rust
    /// Print a TL;DR: open leaf claims ranked into priority tiers, each with its next step.
    Tldr {
        /// Items listed in each of tiers 1 and 2; the rest are counted per area. 0 lists
        /// everything.
        #[arg(long, default_value_t = next_steps::DEFAULT_LIMIT)]
        limit: usize,
        /// Print JSON (the `next_steps` tool's output) instead of Markdown.
        #[arg(long)]
        json: bool,
    },
```

In `main`, replace:

```rust
        Some(Command::Verify { json }) => verify(&store, json).await,
```

with:

```rust
        Some(Command::Verify { json }) => verify(&store, json).await,
        Some(Command::Tldr { limit, json }) => {
            tldr(&store, limit, json).await.map(|()| ExitCode::SUCCESS)
        }
```

- [ ] **Step 5: Add the command and renderer functions**

Directly above `/// Seals unsealed records as adopted and lists them.`, insert:

```rust
/// Prints the next-steps TL;DR as Markdown, or as JSON with `json`. Gaps are the expected
/// content, so the exit code is 0 whenever the catalog could be read.
async fn tldr(store: &CatalogStore, limit: usize, json: bool) -> anyhow::Result<()> {
    let steps = store
        .read(move |catalog| Ok(next_steps::next_steps(catalog, limit)))
        .await
        .context("failed to read the catalog")?;
    let mut out = std::io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, &steps)?;
        writeln!(out)?;
    } else {
        write_tldr(&mut out, &store.path().display().to_string(), &steps)?;
    }
    Ok(())
}

/// Markdown form of the TL;DR (spec section 6).
fn write_tldr(out: &mut impl Write, path: &str, steps: &NextSteps) -> std::io::Result<()> {
    let summary = &steps.summary;
    writeln!(out, "# TL;DR: {path}\n")?;
    writeln!(
        out,
        "{} active claims: {} leaves ({} done, {} open), {} roll-ups.\n",
        summary.active_claims,
        summary.done_leaves + summary.open_leaves,
        summary.done_leaves,
        summary.open_leaves,
        summary.rollup_claims
    )?;
    if !steps.integrity.verified {
        writeln!(out, "## 0. STOP: integrity check failed\n")?;
        writeln!(
            out,
            "{} records failed their seal check (`claims-mcp verify` lists them). The catalog \
             was changed outside the server, so nothing below can be trusted. Restore it from \
             git before acting on this list.\n",
            steps.integrity.problems.len()
        )?;
    }
    write_tier(out, "1. Wrong", &steps.wrong)?;
    write_tier(out, "2. Unproven, integrity 3-4", &steps.unproven)?;
    let other_total: usize = steps.other.iter().map(|entry| entry.count).sum();
    writeln!(out, "## 3. Everything else ({other_total})\n")?;
    if steps.other.is_empty() {
        writeln!(out, "None.")
    } else {
        writeln!(out, "{}", area_list(&steps.other))
    }
}

/// One tier: a bullet per listed item, then the per-area count of the rest.
fn write_tier(out: &mut impl Write, heading: &str, tier: &Tier) -> std::io::Result<()> {
    writeln!(out, "## {heading} ({})\n", tier.total)?;
    if tier.items.is_empty() {
        return writeln!(out, "None.\n");
    }
    for item in &tier.items {
        write!(out, "- **{}**", item.id)?;
        if let Some(level) = item.integrity_level {
            write!(out, " IL{}", level.get())?;
        }
        if !item.issues.is_empty() {
            write!(out, " [#{}]", item.issues.join(", #"))?;
        }
        writeln!(
            out,
            " {}\n  - Next: {}",
            truncated(&item.statement, STATEMENT_CHARS),
            item.next_step
        )?;
    }
    if !tier.omitted.is_empty() {
        let hidden = tier.total - tier.items.len();
        writeln!(out, "\n...and {hidden} more: {}", area_list(&tier.omitted))?;
    }
    writeln!(out)
}

/// `OPS 30, POL 21, ...`
fn area_list(counts: &[AreaCount]) -> String {
    counts
        .iter()
        .map(|entry| format!("{} {}", entry.area, entry.count))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `text` cut to `max_chars` `char`s with `...` appended, or unchanged when it fits.
///
/// Rust note: `Cow` (clone-on-write) borrows the input when it fits and only allocates a new
/// `String` when it has to cut. `char_indices` walks whole Unicode scalar values, so the cut
/// never splits a multibyte character.
fn truncated(text: &str, max_chars: usize) -> Cow<'_, str> {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => Cow::Owned(format!("{}...", &text[..cut])),
        None => Cow::Borrowed(text),
    }
}

```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test tldr && cargo test truncated`
Expected: PASS (3 `tldr_*` tests, then `truncated_cuts_on_char_boundaries`).

- [ ] **Step 7: Smoke-test against the real catalog**

Run: `cargo run -q -- --catalog bdc.json tldr --limit 5 | head -12`
Expected (first lines):

```
# TL;DR: bdc.json

1049 active claims: 1038 leaves (631 done, 407 open), 11 roll-ups.

## 1. Wrong (32)

- **CLM-0612** IL4 [#360] No secret value survives redaction, across chunk boundaries.
  - Next: Add the test or e2e step the blind finding names as missing, or correct the source if it overstates.
```

Run: `cargo run -q -- --catalog bdc.json tldr --json | python3 -I -c "import json,sys; d=json.load(sys.stdin); print(d['wrong']['total'], d['unproven']['total'], sum(a['count'] for a in d['other']))"`
Expected: `32 266 109`

- [ ] **Step 8: Commit**

```bash
git add src/main.rs
git commit -m "feat: claims-mcp tldr subcommand with Markdown output

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Skill, README, spec correction and changelog

**Files:**
- Modify: `skills/claims-audit/SKILL.md` (verdict section, about line 124)
- Modify: `README.md` (Run subcommand table at about line 143; Tools table at about line 212)
- Modify: `docs/superpowers/specs/2026-10-23-next-steps-tldr-design.md` (section 2)
- Create: `changelog/<UTC timestamp>.md`

- [ ] **Step 1: Make `Fix:` part of the audit skill**

In `skills/claims-audit/SKILL.md`, replace:

```markdown
evidence. Your `rationale` must explain *why* the cited evidence decides the claim.
```

with:

```markdown
evidence. Your `rationale` must explain *why* the cited evidence decides the claim.

A `refuted` or `not_enough_info` rationale must end with `Fix: <one concrete next step>`: what
to change, or which test or evidence would decide the claim. `next_steps` and
`claims-mcp tldr` show the text after the last `Fix:` as the claim's next step, so make it
specific enough to act on without rereading the rationale ("Fix: add a test that sends a
1024-bit RSA JWK and asserts it is rejected", not "Fix: add tests").
```

- [ ] **Step 2: Document the subcommand and tool in the README**

In `README.md`, replace:

```markdown
| `verify [--json]` | Check every integrity seal; exit 1 if any check fails |
```

with:

```markdown
| `verify [--json]` | Check every integrity seal; exit 1 if any check fails |
| `tldr [--limit N] [--json]` | Print the ranked next steps; see [TL;DR of next steps](#tldr-of-next-steps) |
```

Replace:

```markdown
| `audit_report` | Totals, gap lists, and the traceability matrix |
```

with:

~~~~markdown
| `audit_report` | Totals, gap lists, and the traceability matrix |
| `next_steps` | Ranked TL;DR: open leaf claims in priority tiers, each with one next step |

## TL;DR of next steps

On a large catalog, `audit_report` is too long to act on. `next_steps` (MCP) and
`claims-mcp tldr` (CLI) list only what to do next:

```bash
claims-mcp --catalog claims.json tldr             # Markdown, 10 items per tier
claims-mcp --catalog claims.json tldr --limit 0   # every item
claims-mcp --catalog claims.json tldr --json      # same data as the next_steps tool
```

Only active **leaf** claims (no active children) are listed: a parent's verdict rolls up from
its children. Open leaves are ranked into tiers:

| Tier | Contents |
|---|---|
| 0. STOP | Shown only when the integrity check fails: restore the catalog from git first |
| 1. Wrong | Refuted claims |
| 2. Unproven | Other open claims at integrity level 3-4 (not_enough_info, unverified, stale, no evidence) |
| 3. Everything else | Counts per `area:` tag |

Tiers 1 and 2 are sorted by integrity level, then id. Each item shows its `issue:` tags and its
next step: the text after the last `Fix:` in the latest verdict rationale (the `claims-audit`
skill requires one on every `refuted` and `not_enough_info` verdict), or else the rationale's
first sentence. Items beyond `--limit` are counted per area. The command exits 0 whenever the
catalog can be read; use `verify` to gate CI.
~~~~

- [ ] **Step 3: Correct the function path in the spec**

In `docs/superpowers/specs/2026-10-23-next-steps-tldr-design.md`, replace:

```markdown
- `report::next_steps(catalog: &Catalog, limit: usize) -> NextSteps` in `src/report.rs`.
```

with:

```markdown
- `report::next_steps::next_steps(catalog: &Catalog, limit: usize) -> NextSteps` in
  `src/report/next_steps.rs`, a submodule of `src/report.rs` that reuses its child index and
  its `is_stale` / `is_high_integrity` helpers.
```

- [ ] **Step 4: Write the changelog entry**

Run: `date -u +%Y-%m-%dT%H-%M-%SZ`
Create `changelog/<that output>.md` (for example `changelog/2026-10-23T19-30-00Z.md`) with:

```markdown
# Next-steps TL;DR

- New `next_steps` MCP tool and `claims-mcp tldr [--limit N] [--json]` subcommand: open
  active leaf claims ranked into tiers (0 integrity failure, 1 refuted, 2 open at integrity 3-4,
  3 the rest counted per area), each with one next step taken from the last `Fix:` in its
  latest verdict rationale, falling back to the rationale's first sentence.
- Roll-up parents (claims with active children) are counted, not listed.
- `claims-audit` skill: `refuted` and `not_enough_info` rationales must end with
  `Fix: <one concrete next step>`.
- Refactor: `is_stale` and `is_high_integrity` extracted from `classify_gaps` and shared with
  the new module (`src/report/next_steps.rs`).
- On `bdc.json` (bolt_deployment_controller, 1049 active claims): 32 wrong, 266 unproven at
  integrity 3-4, 109 other. The default output is 57 lines.
- Spec: `docs/superpowers/specs/2026-10-23-next-steps-tldr-design.md`. Plan:
  `docs/superpowers/plans/2026-10-23-next-steps-tldr.md`.
```

- [ ] **Step 5: Commit**

```bash
git add skills/claims-audit/SKILL.md README.md docs/superpowers/specs/2026-10-23-next-steps-tldr-design.md changelog/
git status --short   # must not list bdc.json as staged
git commit -m "docs: document next_steps / tldr and require Fix: in audit rationales

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Full verification

**Files:** none changed unless a check fails.

- [ ] **Step 1: Format check**

Run: `cargo fmt --check`
Expected: no output, exit 0. If it fails, run `cargo fmt`, review the diff, and amend the last commit that touched the file with a new `style: cargo fmt` commit.

- [ ] **Step 2: Lint**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: `Finished` with no warnings.

- [ ] **Step 3: Tests**

Run: `cargo test`
Expected: `test result: ok. 100 passed; 0 failed` (83 before this change, plus 11 `next_steps`, 2 server, and 4 renderer tests).

- [ ] **Step 4: Docs build without warnings**

Run: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items`
Expected: `Finished` with no warnings.

- [ ] **Step 5: Release build and real-catalog check**

Run: `cargo build --release && ./target/release/claims-mcp --catalog bdc.json tldr | wc -l && ./target/release/claims-mcp --catalog bdc.json tldr --limit 0 | grep -c '^- \*\*CLM-'`
Expected: `57`, then `298` (32 + 266 listed items).

- [ ] **Step 6: Confirm the tree is clean apart from `bdc.json`**

Run: `git status --short`
Expected: exactly `?? bdc.json`.
