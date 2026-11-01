//! Replays the records a branch added to its ledger onto another ledger under new ids
//! (`claims-mcp rebase`, issue #32).
//!
//! Ids come from per-ledger counters, so two branches that both add claims, or both add
//! evidence, hand out the same ids. Git may merge two such ledgers without a conflict into one
//! that holds two records with the same id, which the server refuses to read. A rebase avoids
//! that: it takes three verified ledgers,
//!
//! - `base`: the branch's ledger at the merge base (what both sides started from),
//! - `branch`: the branch's ledger now,
//! - `onto`: the other side's ledger now (usually `main`'s),
//!
//! and builds `onto` plus everything the branch added since `base`:
//!
//! - claims with ids from `base`'s `next_claim_seq` up, appended under the next free ids;
//! - evidence with ids from `base`'s `next_evidence_seq` up, renumbered in id order;
//! - verdicts the branch appended to claims that existed in `base`;
//! - field changes (`update_claim`, `retire_claim`) the branch made to such claims, unless the
//!   other side changed the same claim's fields differently, which is a conflict.
//!
//! References among the replayed records (`derived_from` and the evidence ids a verdict cites)
//! are rewritten to the new ids, and the result is resealed. Nothing is replayed from a ledger
//! that fails its integrity check, so a rebase cannot launder a hand edit.

use std::collections::HashMap;

use serde::Serialize;

use crate::catalog::Catalog;
use crate::error::{ClaimsError, Result};
use crate::model::{
    Claim, ClaimId, ClaimKind, ClaimLevel, ClaimStatus, Evidence, EvidenceId, FileAnchor,
    IntegrityLevel, VerdictRecord, VerificationMethod,
};
use crate::seal;

/// The id a record had on the branch and the id the rebase gave it.
///
/// Rust note: `<T>` makes this one struct serve both id types, like a Python generic
/// `Renumbered[T]`, but checked at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Renumbered<T> {
    /// Id on the branch.
    pub from: T,
    /// Id in the rebased ledger.
    pub to: T,
}

/// Old-to-new ids of every claim and evidence item the branch added, each sorted by old id.
/// Records that existed at the merge base keep their ids and are not listed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IdMapping {
    /// Claims the branch added.
    pub claims: Vec<Renumbered<ClaimId>>,
    /// Evidence items the branch added, on new or existing claims.
    pub evidence: Vec<Renumbered<EvidenceId>>,
}

impl IdMapping {
    /// New id of a claim; claims the branch did not add keep theirs.
    fn claim(&self, id: ClaimId) -> ClaimId {
        lookup(&self.claims, id).unwrap_or(id)
    }

    /// New id of an evidence item; evidence the branch did not add keeps its id.
    fn evidence(&self, id: EvidenceId) -> EvidenceId {
        lookup(&self.evidence, id).unwrap_or(id)
    }
}

/// Binary search of a table sorted by `from`: O(log n).
fn lookup<T: Copy + Ord>(table: &[Renumbered<T>], id: T) -> Option<T> {
    table
        .binary_search_by_key(&id, |entry| entry.from)
        .ok()
        .map(|index| table[index].to)
}

/// Builds `onto` plus every record `branch` added since `base`, under new ids.
///
/// # Arguments
///
/// * `base` - the branch's ledger at the merge base; an empty catalog when the branch created
///   the ledger.
/// * `branch` - the branch's ledger now.
/// * `onto` - the ledger to replay onto, such as `main`'s.
///
/// # Returns
///
/// The rebased, resealed ledger and the old-to-new id mapping of the replayed records.
///
/// # Errors
///
/// [`ClaimsError::Rebase`] when an input fails its integrity check, `branch` or `onto` does not
/// descend from `base`, `onto` already holds the branch's records, a claim's fields were
/// changed differently on both sides, the branch added records to a claim `onto` retired, or
/// the merged links form a cycle. [`ClaimsError::Validation`] if an id space is exhausted.
///
/// # Examples
///
/// ```ignore
/// let (rebased, mapping) = rebase(&base, &branch, &main)?;
/// for claim in &mapping.claims {
///     println!("{} -> {}", claim.from, claim.to);
/// }
/// ```
pub fn rebase(base: &Catalog, branch: &Catalog, onto: &Catalog) -> Result<(Catalog, IdMapping)> {
    for (name, catalog) in [("--base", base), ("branch", branch), ("--onto", onto)] {
        check_integrity(name, catalog)?;
        check_descends(name, base, catalog)?;
    }
    let mapping = number_new_records(base, branch, onto)?;
    reject_already_applied(base, branch, onto)?;

    let first_new_claim = ClaimId::new(base.next_claim_seq());
    let mut rebased = onto.clone();
    let mut relinked = Vec::new();
    for claim in branch.claims() {
        if claim.id < first_new_claim {
            let base_claim = base.get(claim.id)?;
            if replay_onto_existing(base_claim, claim, &mut rebased, &mapping)? {
                relinked.push(claim.id);
            }
        } else {
            rebased.append_replayed_claim(renumbered_claim(claim, &mapping)?)?;
        }
    }
    reject_cycles(&rebased, &relinked)?;
    // The mapping's length already fit in a `u32` when its ids were numbered.
    let evidence_count = u32::try_from(mapping.evidence.len())
        .map_err(|_| ClaimsError::Validation("evidence id space exhausted".to_owned()))?;
    rebased.advance_evidence_seq(evidence_count)?;
    rebased.validate()?;
    seal::reseal(&mut rebased);
    Ok((rebased, mapping))
}

fn refused(message: String) -> ClaimsError {
    ClaimsError::Rebase(message)
}

/// Refuses a ledger with any failed seal, so only server-written content is replayed.
fn check_integrity(name: &str, catalog: &Catalog) -> Result<()> {
    let report = seal::verify(catalog);
    match report.problems.first() {
        Some(problem) => Err(refused(format!(
            "the {name} catalog fails its integrity check ({}: {}); restore it from git",
            problem.record, problem.problem
        ))),
        None => Ok(()),
    }
}

/// Counters never go down and claims are never deleted (a sealed catalog has no gaps), so a
/// ledger descending from `base` has counters at least as high, and so every `base` claim.
fn check_descends(name: &str, base: &Catalog, catalog: &Catalog) -> Result<()> {
    if catalog.next_claim_seq() < base.next_claim_seq()
        || catalog.next_evidence_seq() < base.next_evidence_seq()
    {
        return Err(refused(format!(
            "the {name} catalog's id counters are below --base's, so it does not descend from \
             --base; pass the ledger at the merge base (`git merge-base HEAD origin/main`)"
        )));
    }
    Ok(())
}

/// Numbers the branch's new claims from `onto`'s claim counter and its new evidence, in id
/// order, from `onto`'s evidence counter. When `onto` is `base`, every id maps to itself.
fn number_new_records(base: &Catalog, branch: &Catalog, onto: &Catalog) -> Result<IdMapping> {
    let first_new_claim = ClaimId::new(base.next_claim_seq());
    let first_new_evidence = EvidenceId::new(base.next_evidence_seq());
    let claims: Vec<ClaimId> = branch
        .claims()
        .iter()
        .map(|claim| claim.id)
        .filter(|id| *id >= first_new_claim)
        .collect();
    let mut evidence: Vec<EvidenceId> = branch
        .claims()
        .iter()
        .flat_map(|claim| &claim.evidence)
        .map(|item| item.id)
        .filter(|id| *id >= first_new_evidence)
        .collect();
    evidence.sort_unstable();
    Ok(IdMapping {
        claims: renumber(claims, onto.next_claim_seq(), ClaimId::new, "claim")?,
        evidence: renumber(
            evidence,
            onto.next_evidence_seq(),
            EvidenceId::new,
            "evidence",
        )?,
    })
}

/// Pairs each old id with `first`, `first + 1`, ... and checks the counter stays in range.
fn renumber<T: Copy>(
    old: Vec<T>,
    first: u32,
    make: fn(u32) -> T,
    kind: &str,
) -> Result<Vec<Renumbered<T>>> {
    let fits = u32::try_from(old.len())
        .ok()
        .and_then(|count| first.checked_add(count))
        .is_some();
    if !fits {
        return Err(ClaimsError::Validation(format!(
            "{kind} id space exhausted"
        )));
    }
    // Rust note: `first..` is an endless range; `zip` stops at the end of `old`, and the check
    // above guarantees it never steps past `u32::MAX`.
    Ok(old
        .into_iter()
        .zip(first..)
        .map(|(from, seq)| Renumbered {
            from,
            to: make(seq),
        })
        .collect())
}

/// Refuses to replay records that `onto` already holds unchanged under the same id: the branch
/// was already rebased onto it, or `--base` is older than the merge base. Replaying them again
/// would duplicate them.
fn reject_already_applied(base: &Catalog, branch: &Catalog, onto: &Catalog) -> Result<()> {
    let first_new_claim = ClaimId::new(base.next_claim_seq());
    let first_new_evidence = EvidenceId::new(base.next_evidence_seq());
    let onto_evidence: HashMap<EvidenceId, &Evidence> = onto
        .claims()
        .iter()
        .flat_map(|claim| &claim.evidence)
        .filter(|item| item.id >= first_new_evidence)
        .map(|item| (item.id, item))
        .collect();
    let already = |id: String| {
        Err(refused(format!(
            "--onto already holds {id} exactly as the branch does, so the branch was already \
             rebased onto it or --base is not the merge base"
        )))
    };
    for claim in branch.claims() {
        if claim.id >= first_new_claim && onto.get(claim.id).is_ok_and(|other| other == claim) {
            return already(claim.id.to_string());
        }
        for item in &claim.evidence {
            if onto_evidence.get(&item.id) == Some(&item) {
                return already(item.id.to_string());
            }
        }
    }
    Ok(())
}

/// The records a ledger appended to a claim since `base`: `records` minus the `base` prefix.
fn appended<'a, T: PartialEq>(
    records: &'a [T],
    base: &[T],
    what: &str,
    claim: ClaimId,
    name: &str,
) -> Result<&'a [T]> {
    records.strip_prefix(base).ok_or_else(|| {
        refused(format!(
            "{claim}'s {what} on the {name} catalog do not start with --base's, so it does not \
             descend from --base"
        ))
    })
}

/// Replays what the branch did to a claim that existed in `base` onto the same claim in
/// `rebased`. Returns `true` when it took the branch's `derived_from` links.
fn replay_onto_existing(
    base_claim: &Claim,
    branch_claim: &Claim,
    rebased: &mut Catalog,
    mapping: &IdMapping,
) -> Result<bool> {
    let id = branch_claim.id;
    let new_evidence = appended(
        &branch_claim.evidence,
        &base_claim.evidence,
        "evidence",
        id,
        "branch",
    )?;
    let new_verdicts = appended(
        &branch_claim.verdicts,
        &base_claim.verdicts,
        "verdicts",
        id,
        "branch",
    )?;
    let base_fields = OwnFields::of(base_claim);
    let branch_fields = OwnFields::of(branch_claim);
    let fields_changed = branch_fields != base_fields;
    if !fields_changed && new_evidence.is_empty() && new_verdicts.is_empty() {
        return Ok(false);
    }

    // `check_descends` guarantees every base claim is in `onto`, and so in `rebased`.
    let target = rebased
        .claim_mut(id)
        .ok_or_else(|| refused(format!("{id} is missing from --onto")))?;
    appended(
        &target.evidence,
        &base_claim.evidence,
        "evidence",
        id,
        "--onto",
    )?;
    appended(
        &target.verdicts,
        &base_claim.verdicts,
        "verdicts",
        id,
        "--onto",
    )?;
    let adds_records = !new_evidence.is_empty() || !new_verdicts.is_empty();
    if adds_records && base_claim.is_active() && !target.is_active() {
        return Err(refused(format!(
            "the branch added evidence or verdicts to {id}, which --onto retired; decide on \
             --onto whether to restore the claim, then rebase again"
        )));
    }

    let relinked = fields_changed && branch_claim.derived_from != base_claim.derived_from;
    if fields_changed {
        let derived_from: Vec<ClaimId> = branch_claim
            .derived_from
            .iter()
            .map(|parent| mapping.claim(*parent))
            .collect();
        let wanted = OwnFields {
            derived_from: &derived_from,
            ..branch_fields
        };
        let onto_fields = OwnFields::of(target);
        if onto_fields != base_fields && onto_fields != wanted {
            return Err(refused(format!(
                "{id} was changed differently on the branch and on --onto; make the two \
                 agree through the tools on one side, then rebase again"
            )));
        }
        wanted.write_to(target);
    }

    for item in new_evidence {
        target.evidence.push(renumbered_evidence(item, mapping)?);
    }
    target.verdicts.extend(
        new_verdicts
            .iter()
            .map(|verdict| renumbered_verdict(verdict, mapping)),
    );
    target.updated_at = target.updated_at.max(branch_claim.updated_at);
    Ok(relinked)
}

/// A branch-new claim under its new id, with its links, evidence, and verdicts renumbered.
fn renumbered_claim(claim: &Claim, mapping: &IdMapping) -> Result<Claim> {
    // Rust note: `..claim.clone()` fills the remaining fields from a clone, like
    // `dataclasses.replace` in Python.
    Ok(Claim {
        id: mapping.claim(claim.id),
        derived_from: claim
            .derived_from
            .iter()
            .map(|parent| mapping.claim(*parent))
            .collect(),
        evidence: claim
            .evidence
            .iter()
            .map(|item| renumbered_evidence(item, mapping))
            .collect::<Result<_>>()?,
        verdicts: claim
            .verdicts
            .iter()
            .map(|verdict| renumbered_verdict(verdict, mapping))
            .collect(),
        ..claim.clone()
    })
}

/// A branch-new evidence item under its new id. Its seal is kept; `seal::reseal` replaces it if
/// the id or the owning claim changed.
fn renumbered_evidence(item: &Evidence, mapping: &IdMapping) -> Result<Evidence> {
    let id = lookup(&mapping.evidence, item.id).ok_or_else(|| {
        refused(format!(
            "{} was added to an existing claim but has an id below --base's evidence counter, \
             so the branch does not descend from --base",
            item.id
        ))
    })?;
    Ok(Evidence { id, ..item.clone() })
}

/// A verdict with the evidence ids it cites renumbered.
fn renumbered_verdict(verdict: &VerdictRecord, mapping: &IdMapping) -> VerdictRecord {
    VerdictRecord {
        evidence_ids: verdict
            .evidence_ids
            .iter()
            .map(|id| mapping.evidence(*id))
            .collect(),
        ..verdict.clone()
    }
}

/// Each side's `derived_from` is acyclic, but links taken from the branch next to links the
/// other side added can close a loop. Only claims whose links came from the branch can be on
/// such a loop's branch half, so only they are checked.
fn reject_cycles(rebased: &Catalog, relinked: &[ClaimId]) -> Result<()> {
    for id in relinked {
        let claim = rebased.get(*id)?;
        if let Some(parent) = claim
            .derived_from
            .iter()
            .find(|parent| rebased.reaches(**parent, *id))
        {
            return Err(refused(format!(
                "the branch links {id} to {parent}, and --onto's links lead from {parent} back \
                 to {id}; remove one of the links, then rebase again"
            )));
        }
    }
    Ok(())
}

/// The fields of a claim that `update_claim` and `retire_claim` change, borrowed so two
/// versions of a claim compare without cloning.
#[derive(Debug, PartialEq)]
struct OwnFields<'a> {
    statement: &'a str,
    kind: ClaimKind,
    level: ClaimLevel,
    integrity_level: Option<IntegrityLevel>,
    verification_method: VerificationMethod,
    source: Option<&'a str>,
    source_anchor: Option<&'a FileAnchor>,
    derived_from: &'a [ClaimId],
    artifacts: &'a [String],
    tags: &'a [String],
    status: &'a ClaimStatus,
}

impl<'a> OwnFields<'a> {
    fn of(claim: &'a Claim) -> Self {
        // Rust note: destructuring without `..` names every field, so adding a field to
        // `Claim` fails to compile here until the rebase decides how to merge it.
        let Claim {
            id: _,
            statement,
            kind,
            level,
            integrity_level,
            verification_method,
            source,
            source_anchor,
            derived_from,
            artifacts,
            tags,
            status,
            evidence: _,
            verdicts: _,
            created_at: _,
            updated_at: _,
            seal: _,
        } = claim;
        Self {
            statement,
            kind: *kind,
            level: *level,
            integrity_level: *integrity_level,
            verification_method: *verification_method,
            source: source.as_deref(),
            source_anchor: source_anchor.as_ref(),
            derived_from,
            artifacts,
            tags,
            status,
        }
    }

    /// Copies these fields into `claim`.
    fn write_to(&self, claim: &mut Claim) {
        claim.statement = self.statement.to_owned();
        claim.kind = self.kind;
        claim.level = self.level;
        claim.integrity_level = self.integrity_level;
        claim.verification_method = self.verification_method;
        claim.source = self.source.map(str::to_owned);
        claim.source_anchor = self.source_anchor.cloned();
        claim.derived_from = self.derived_from.to_vec();
        claim.artifacts = self.artifacts.to_vec();
        claim.tags = self.tags.to_vec();
        claim.status = self.status.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::{at, child_of, evidence, new_claim, verdict};
    use crate::catalog::ClaimUpdate;
    use crate::model::{ClaimRef, Stance, Verdict};

    /// Applies `operation` to `catalog` the way the store does: change, then reseal.
    fn write(catalog: &mut Catalog, operation: impl FnOnce(&mut Catalog)) {
        operation(catalog);
        seal::reseal(catalog);
    }

    /// The issue's starting point: CLM-0001 and CLM-0002, sealed, no evidence.
    fn main_with_two_claims() -> Catalog {
        let mut catalog = Catalog::default();
        write(&mut catalog, |catalog| {
            catalog.add_claim(new_claim("one"), at(0)).expect("add");
            catalog.add_claim(new_claim("two"), at(0)).expect("add");
        });
        catalog
    }

    fn add_supported(catalog: &mut Catalog, claim: ClaimId, seconds: i64) -> EvidenceId {
        let mut id = None;
        write(catalog, |catalog| {
            let item = catalog
                .add_evidence(claim, evidence(Stance::Supports), at(seconds))
                .expect("evidence");
            catalog
                .record_verdict(
                    claim,
                    verdict(Verdict::Supported, vec![item.id]),
                    at(seconds),
                )
                .expect("verdict");
            id = Some(item.id);
        });
        id.expect("evidence added")
    }

    fn ids<T: std::fmt::Display>(mapping: &[Renumbered<T>]) -> Vec<(String, String)> {
        mapping
            .iter()
            .map(|entry| (entry.from.to_string(), entry.to.to_string()))
            .collect()
    }

    #[test]
    fn evidence_that_merged_without_a_conflict_gets_new_ids() {
        // Arrange: the issue's reproduction; both branches hand out EVD-0001.
        let base = main_with_two_claims();
        let mut main = base.clone();
        add_supported(&mut main, ClaimId::new(1), 10);
        let mut branch = base.clone();
        add_supported(&mut branch, ClaimId::new(2), 20);

        // Act
        let (rebased, mapping) = rebase(&base, &branch, &main).expect("rebase");

        // Assert
        assert!(seal::verify(&rebased).verified);
        assert_eq!(
            ids(&mapping.evidence),
            [("EVD-0001".into(), "EVD-0002".into())]
        );
        assert!(mapping.claims.is_empty());
        let second = rebased.get(ClaimId::new(2)).expect("claim");
        assert_eq!(second.evidence[0].id, EvidenceId::new(2));
        assert_eq!(second.verdicts[0].evidence_ids, [EvidenceId::new(2)]);
        assert_eq!(
            rebased.get(ClaimId::new(1)).expect("claim").evidence[0].id,
            EvidenceId::new(1)
        );
        assert_eq!(rebased.next_evidence_seq(), 3);
    }

    #[test]
    fn new_claims_on_both_sides_are_renumbered_with_their_links() {
        // Arrange: both sides add CLM-0003; the branch's CLM-0004 derives from its CLM-0003.
        let base = main_with_two_claims();
        let mut main = base.clone();
        write(&mut main, |catalog| {
            catalog
                .add_claim(new_claim("main three"), at(5))
                .expect("add");
        });
        add_supported(&mut main, ClaimId::new(3), 6);
        let mut branch = base.clone();
        write(&mut branch, |catalog| {
            catalog
                .add_claims(
                    vec![
                        new_claim("branch three"),
                        child_of("branch four", ClaimRef::Batch(0)),
                    ],
                    at(7),
                )
                .expect("add");
        });
        add_supported(&mut branch, ClaimId::new(4), 8);
        add_supported(&mut branch, ClaimId::new(1), 9);

        // Act
        let (rebased, mapping) = rebase(&base, &branch, &main).expect("rebase");

        // Assert
        assert!(seal::verify(&rebased).verified);
        assert_eq!(
            ids(&mapping.claims),
            [
                ("CLM-0003".into(), "CLM-0004".into()),
                ("CLM-0004".into(), "CLM-0005".into())
            ]
        );
        assert_eq!(
            ids(&mapping.evidence),
            [
                ("EVD-0001".into(), "EVD-0002".into()),
                ("EVD-0002".into(), "EVD-0003".into())
            ]
        );
        let statements: Vec<_> = rebased
            .claims()
            .iter()
            .map(|claim| claim.statement.as_str())
            .collect();
        assert_eq!(
            statements,
            ["one", "two", "main three", "branch three", "branch four"]
        );
        let four = rebased.get(ClaimId::new(5)).expect("claim");
        assert_eq!(four.derived_from, [ClaimId::new(4)]);
        assert_eq!(four.evidence[0].id, EvidenceId::new(2));
        assert_eq!(four.verdicts[0].evidence_ids, [EvidenceId::new(2)]);
        let one = rebased.get(ClaimId::new(1)).expect("claim");
        assert_eq!(one.verdicts[0].evidence_ids, [EvidenceId::new(3)]);
        assert_eq!(rebased.next_claim_seq(), 6);
        assert_eq!(rebased.next_evidence_seq(), 4);
    }

    #[test]
    fn rebasing_onto_the_base_changes_nothing() {
        let base = main_with_two_claims();
        let mut branch = base.clone();
        write(&mut branch, |catalog| {
            catalog.add_claim(new_claim("three"), at(5)).expect("add");
        });
        add_supported(&mut branch, ClaimId::new(3), 6);

        let (rebased, mapping) = rebase(&base, &branch, &base).expect("rebase");

        assert_eq!(rebased, branch);
        assert!(mapping.claims.iter().all(|entry| entry.from == entry.to));
        assert!(mapping.evidence.iter().all(|entry| entry.from == entry.to));
    }

    #[test]
    fn a_ledger_new_on_the_branch_rebases_from_an_empty_base() {
        let base = Catalog::default();
        let main = main_with_two_claims();
        let mut branch = Catalog::default();
        write(&mut branch, |catalog| {
            catalog.add_claim(new_claim("mine"), at(5)).expect("add");
        });

        let (rebased, mapping) = rebase(&base, &branch, &main).expect("rebase");

        assert!(seal::verify(&rebased).verified);
        assert_eq!(mapping.claims[0].to, ClaimId::new(3));
        assert_eq!(rebased.claims().len(), 3);
    }

    #[test]
    fn verdicts_from_both_sides_on_one_claim_are_kept_in_order() {
        let base = main_with_two_claims();
        let mut main = base.clone();
        add_supported(&mut main, ClaimId::new(1), 10);
        let mut branch = base.clone();
        add_supported(&mut branch, ClaimId::new(1), 20);

        let (rebased, _) = rebase(&base, &branch, &main).expect("rebase");

        assert!(seal::verify(&rebased).verified);
        let one = rebased.get(ClaimId::new(1)).expect("claim");
        let cited: Vec<_> = one
            .verdicts
            .iter()
            .map(|record| record.evidence_ids.clone())
            .collect();
        assert_eq!(cited, [vec![EvidenceId::new(1)], vec![EvidenceId::new(2)]]);
        assert_eq!(one.updated_at, at(20));
    }

    #[test]
    fn a_field_change_on_one_side_is_kept() {
        let base = main_with_two_claims();
        let mut main = base.clone();
        add_supported(&mut main, ClaimId::new(1), 10);
        let mut branch = base.clone();
        write(&mut branch, |catalog| {
            let update = ClaimUpdate {
                statement: Some("one, reworded".to_owned()),
                ..ClaimUpdate::default()
            };
            catalog
                .update_claim(ClaimId::new(1), update, at(20))
                .expect("update");
        });

        let (rebased, _) = rebase(&base, &branch, &main).expect("rebase");

        let one = rebased.get(ClaimId::new(1)).expect("claim");
        assert_eq!(one.statement, "one, reworded");
        assert_eq!(one.evidence.len(), 1);
        assert!(seal::verify(&rebased).verified);
    }

    #[test]
    fn different_field_changes_on_both_sides_conflict() {
        let base = main_with_two_claims();
        let reword = |text: &str| {
            let mut catalog = base.clone();
            write(&mut catalog, |catalog| {
                let update = ClaimUpdate {
                    statement: Some(text.to_owned()),
                    ..ClaimUpdate::default()
                };
                catalog
                    .update_claim(ClaimId::new(2), update, at(20))
                    .expect("update");
            });
            catalog
        };

        let error = rebase(&base, &reword("branch"), &reword("main")).expect_err("conflict");
        assert!(error
            .to_string()
            .contains("CLM-0002 was changed differently"));

        assert!(rebase(&base, &reword("same"), &reword("same")).is_ok());
    }

    #[test]
    fn evidence_on_a_claim_main_retired_conflicts() {
        let base = main_with_two_claims();
        let mut main = base.clone();
        write(&mut main, |catalog| {
            catalog
                .retire_claim(ClaimId::new(2), "obsolete", at(10))
                .expect("retire");
        });
        let mut branch = base.clone();
        add_supported(&mut branch, ClaimId::new(2), 20);

        let error = rebase(&base, &branch, &main).expect_err("conflict");
        assert!(error.to_string().contains("which --onto retired"));
    }

    #[test]
    fn links_that_close_a_loop_across_the_sides_are_refused() {
        let base = main_with_two_claims();
        let link = |child: u32, parent: u32| {
            let mut catalog = base.clone();
            write(&mut catalog, |catalog| {
                let update = ClaimUpdate {
                    derived_from: Some(vec![ClaimId::new(parent)]),
                    ..ClaimUpdate::default()
                };
                catalog
                    .update_claim(ClaimId::new(child), update, at(20))
                    .expect("update");
            });
            catalog
        };

        let error = rebase(&base, &link(1, 2), &link(2, 1)).expect_err("cycle");
        assert!(error.to_string().contains("lead from CLM-0002 back"));
    }

    #[test]
    fn rebasing_twice_is_refused() {
        let base = main_with_two_claims();
        let mut main = base.clone();
        add_supported(&mut main, ClaimId::new(1), 10);
        let mut branch = base.clone();
        add_supported(&mut branch, ClaimId::new(2), 20);
        let (rebased, _) = rebase(&base, &branch, &main).expect("rebase");

        let error = rebase(&base, &rebased, &main).expect_err("already applied");
        assert!(error.to_string().contains("already holds EVD-0001"));
    }

    #[test]
    fn a_tampered_input_is_refused() {
        let base = main_with_two_claims();
        let mut branch = base.clone();
        add_supported(&mut branch, ClaimId::new(2), 20);
        branch.claims_mut()[1].evidence[0].summary = "edited by hand".to_owned();

        let error = rebase(&base, &branch, &base).expect_err("tampered");
        assert!(error
            .to_string()
            .contains("branch catalog fails its integrity check"));
    }

    #[test]
    fn a_base_newer_than_the_branch_is_refused() {
        let base = main_with_two_claims();
        let error = rebase(&base, &Catalog::default(), &base).expect_err("not descended");
        assert!(error.to_string().contains("does not descend from --base"));
    }

    #[test]
    fn a_base_that_is_not_the_merge_base_is_refused() {
        // Arrange: `other` did not grow from `base`; its evidence ids mean other records.
        let mut base = main_with_two_claims();
        add_supported(&mut base, ClaimId::new(2), 5);
        let mut other = main_with_two_claims();
        add_supported(&mut other, ClaimId::new(1), 5);
        add_supported(&mut other, ClaimId::new(2), 6);

        let error = rebase(&base, &other, &base).expect_err("not descended");
        assert!(error.to_string().contains("does not descend from --base"));
    }
}
