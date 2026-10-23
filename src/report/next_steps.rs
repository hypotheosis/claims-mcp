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
/// Step for a verdict whose rationale names no next step.
const NO_STEP_IN_RATIONALE: &str = "Rationale names no next step; read the latest verdict.";

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
            // Rust note: `|&(claim, stale)|` destructures each `&(&Claim, bool)` in the closure
            // header, like `for claim, stale in pairs` in Python.
            .map(|&(claim, stale)| to_next_step(claim, stale))
            .collect(),
        omitted: area_counts(rest.iter().map(|(claim, _)| area_of(claim))),
    }
}

/// Builds the output row for one open leaf.
fn to_next_step(claim: &Claim, stale: bool) -> NextStep {
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

/// Spec section 5: the text after the last `Fix:`, else the first sentence before it, else a
/// fixed phrase (for a claim without a verdict, or a rationale with no usable text).
fn next_step_text(claim: &Claim, stale: bool) -> String {
    // Rust note: `let ... else` binds on a match or runs the `else` block, which must leave the
    // function (here by returning), so `latest` is a plain value afterwards.
    let Some(latest) = claim.verdicts.last() else {
        let step = if claim.evidence.is_empty() {
            NO_EVIDENCE_STEP
        } else {
            NO_VERDICT_STEP
        };
        return step.to_owned();
    };
    let step = fix_text(&latest.rationale).unwrap_or_else(|| fallback_step(&latest.rationale));
    if stale {
        format!("{STALE_PREFIX}{step}")
    } else {
        step.to_owned()
    }
}

/// The trimmed text after the last `Fix:`, or `None` when there is no marker or nothing
/// follows it.
fn fix_text(rationale: &str) -> Option<&str> {
    // Rust note: `?` on an `Option` returns `None` from this function when the value is `None`.
    // `rfind` gives a byte offset and `Fix:` is ASCII, so adding its length is a char boundary.
    let start = rationale.rfind(FIX_MARKER)? + FIX_MARKER.len();
    // Rust note: `Some(x).filter(pred)` keeps the value only when `pred` is true, else `None`.
    Some(rationale[start..].trim()).filter(|text| !text.is_empty())
}

/// First sentence of the text before the last `Fix:` (the whole rationale without a marker),
/// or `NO_STEP_IN_RATIONALE` when that text is empty.
fn fallback_step(rationale: &str) -> &str {
    // `rfind` returns a byte offset at the start of the ASCII marker, a char boundary.
    let before = rationale
        .rfind(FIX_MARKER)
        .map_or(rationale, |index| &rationale[..index]);
    Some(first_sentence(before))
        .filter(|sentence| !sentence.is_empty())
        .unwrap_or(NO_STEP_IN_RATIONALE)
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
fn area_counts<'a>(areas: impl IntoIterator<Item = &'a str>) -> Vec<AreaCount> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for area in areas {
        // Rust note: `entry(..).or_default()` returns a `&mut usize`, inserting 0 if the key is
        // new; `*` dereferences it so `+= 1` updates the map (like Python's `defaultdict(int)`).
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

    /// Records a verdict that cites no evidence, with `rationale`, at time 2.
    fn judge_without_evidence(catalog: &mut Catalog, id: ClaimId, rationale: &str) {
        let mut input = verdict(Verdict::NotEnoughInfo, Vec::new());
        input.rationale = rationale.to_owned();
        catalog.record_verdict(id, input, at(2)).expect("verdict");
    }

    fn step_of(tier: &Tier) -> &str {
        &tier.items[0].next_step
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
    fn fallback_uses_text_before_the_marker_or_a_fixed_phrase() {
        let mut catalog = Catalog::default();
        let named = leaf(&mut catalog, "a", Some(4), &[]);
        judge(&mut catalog, named, Verdict::NotEnoughInfo, "Untested Fix:");
        let bare = leaf(&mut catalog, "b", Some(3), &[]);
        judge(&mut catalog, bare, Verdict::NotEnoughInfo, "Fix:");

        let items = next_steps(&catalog, 0).unproven.items;

        assert_eq!(items[0].next_step, "Untested");
        assert_eq!(items[1].next_step, NO_STEP_IN_RATIONALE);
    }

    #[test]
    fn stale_claim_with_a_fix_prefixes_the_fix_text() {
        let mut catalog = Catalog::default();
        let claim = leaf(&mut catalog, "s", Some(3), &[]);
        judge(
            &mut catalog,
            claim,
            Verdict::NotEnoughInfo,
            "Untested. Fix: add a test.",
        );
        catalog
            .add_evidence(claim, evidence(Stance::Supports), at(10))
            .expect("late evidence");

        let steps = next_steps(&catalog, 0);

        assert_eq!(
            step_of(&steps.unproven),
            format!("{STALE_PREFIX}add a test.")
        );
    }

    #[test]
    fn stale_refuted_claim_stays_in_tier_one() {
        let mut catalog = Catalog::default();
        let claim = leaf(&mut catalog, "r", Some(3), &[]);
        judge(&mut catalog, claim, Verdict::Refuted, "Wrong.");
        catalog
            .add_evidence(claim, evidence(Stance::Supports), at(10))
            .expect("late evidence");

        let steps = next_steps(&catalog, 0);

        assert_eq!(ids(&steps.wrong), vec![claim]);
        assert!(steps.wrong.items[0].stale);
        assert_eq!(steps.unproven.total, 0);
    }

    #[test]
    fn wrong_tier_counts_omitted_items_per_area() {
        let mut catalog = Catalog::default();
        let first = leaf(&mut catalog, "a", Some(4), &["area:A"]);
        judge(&mut catalog, first, Verdict::Refuted, "Wrong.");
        let second = leaf(&mut catalog, "b", Some(3), &["area:B"]);
        judge(&mut catalog, second, Verdict::Refuted, "Wrong.");

        let wrong = next_steps(&catalog, 1).wrong;

        assert_eq!(ids(&wrong), vec![first]);
        assert_eq!(wrong.omitted, vec![area("B", 1)]);
    }

    #[test]
    fn verdict_without_evidence_is_open_and_uses_its_rationale() {
        let mut catalog = Catalog::default();
        let claim = leaf(&mut catalog, "n", Some(4), &[]);
        judge_without_evidence(&mut catalog, claim, "Nothing checked yet. More text.");

        let steps = next_steps(&catalog, 0);

        assert_eq!(steps.summary.open_leaves, 1);
        assert_eq!(step_of(&steps.unproven), "Nothing checked yet.");
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
