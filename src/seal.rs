//! Integrity seals: detect catalog edits that did not go through the server.
//!
//! Every claim, evidence item, and verdict, plus the catalog header (schema version and id
//! counters), carries an HMAC-SHA256 seal under a key built into this binary. The server
//! verifies every seal on every load and refuses all writes while any check fails.
//!
//! This is a deterrent, not cryptographic security: the key is in the public source, so anyone
//! who reads it can forge seals. What it stops is the casual edit, such as an agent opening
//! `claims.json` and flipping a verdict. Doing that now takes deliberately finding the key and
//! writing code to recompute seals.
//!
//! Each MAC covers a record's content *and* its place in the catalog, so records cannot be
//! moved, reordered, or silently deleted:
//!
//! - evidence: the owning claim id and the evidence fields
//! - verdict: the owning claim id, its index, and the previous verdict's MAC (a chain)
//! - claim: the claim's own fields, its evidence ids, and its latest verdict's MAC
//! - header: `schema_version`, `next_claim_seq`, `next_evidence_seq`
//!
//! Because claims are never deleted (only retired) and ids are never reused, any gap below
//! `next_claim_seq` means a claim was removed. Sealing record by record, rather than the whole
//! file, keeps seals valid across git merges of branches that changed different claims.

use hmac::{Hmac, KeyInit, Mac};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::Sha256;

use crate::catalog::{Catalog, UNSEALED_SCHEMA_VERSION};
use crate::error::{ClaimsError, Result};
use crate::model::{Claim, ClaimId, Seal, SealOrigin, VerdictRecord};

/// Key for every seal. Changing it invalidates every existing catalog, so treat it as
/// permanent; a future rotation would need a new domain tag and a migration.
const SEAL_KEY: [u8; 32] = [
    0xf1, 0xbb, 0xb1, 0xe5, 0x9c, 0xd8, 0x22, 0x08, 0xbd, 0x1d, 0x37, 0x08, 0x97, 0x1d, 0x63, 0x3c,
    0xae, 0x0f, 0x1a, 0x53, 0xff, 0x73, 0xc5, 0x86, 0xec, 0x9e, 0x19, 0x72, 0x47, 0x73, 0x0d, 0xbd,
];

/// Prefix of every MAC input, so these MACs cannot be confused with any other use of the key.
const DOMAIN: &[u8] = b"claims-mcp seal v1\0";

/// One failed integrity check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntegrityProblem {
    /// The record that failed: `catalog`, `CLM-0001`, `EVD-0014`, or `CLM-0001 verdict 2`.
    pub record: String,
    /// What is wrong with it.
    pub problem: String,
}

/// Result of verifying every seal in a catalog. Part of the `audit_report` output and of
/// `claims-mcp verify --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntegrityReport {
    /// `true` when every check passed; the server only writes when this holds.
    pub verified: bool,
    /// Number of sealed records checked (header, claims, evidence, verdicts).
    pub records: usize,
    /// Every failed check, in catalog order.
    pub problems: Vec<IntegrityProblem>,
    /// Records sealed by `seal --adopt` rather than by the server, in catalog order.
    pub adopted: Vec<String>,
}

/// Verifies every seal in `catalog` without changing it.
pub fn verify(catalog: &Catalog) -> IntegrityReport {
    let mut report = IntegrityReport {
        verified: true,
        records: 0,
        problems: Vec::new(),
        adopted: Vec::new(),
    };
    if catalog.schema_version() == UNSEALED_SCHEMA_VERSION {
        report.verified = false;
        report.problems.push(IntegrityProblem {
            record: "catalog".to_owned(),
            problem: format!(
                "schema_version {UNSEALED_SCHEMA_VERSION} catalog has no integrity seals; a \
                 maintainer must run `claims-mcp seal --adopt` once to migrate it"
            ),
        });
        return report;
    }

    // An untouched empty catalog (missing file or never written) has nothing to vouch for.
    if !catalog.is_pristine() {
        report.check(
            "catalog".to_owned(),
            catalog.seal(),
            &header_message(catalog),
        );
    }
    for id in missing_claim_ids(catalog) {
        report.fail(
            id.to_string(),
            "is missing; claims are retired, never deleted".to_owned(),
        );
    }
    for claim in catalog.claims() {
        for evidence in &claim.evidence {
            let message = evidence_message(claim.id, evidence);
            report.check(evidence.id.to_string(), evidence.seal.as_ref(), &message);
        }
        let mut previous = None;
        for (index, verdict) in claim.verdicts.iter().enumerate() {
            let message = verdict_message(claim.id, index, previous, verdict);
            report.check(
                verdict_name(claim.id, index),
                verdict.seal.as_ref(),
                &message,
            );
            previous = mac_of(verdict.seal.as_ref());
        }
        report.check(
            claim.id.to_string(),
            claim.seal.as_ref(),
            &claim_message(claim),
        );
    }
    report
}

/// Re-seals, as the server, every record whose seal no longer matches.
///
/// Called by the store after a successful tool operation on a catalog that verified before
/// the operation, so the only mismatched records are the ones the operation created or
/// changed. Records that still match keep their seal, including its origin.
pub fn reseal(catalog: &mut Catalog) {
    for claim in catalog.claims_mut() {
        let claim_id = claim.id;
        for evidence in &mut claim.evidence {
            let message = evidence_message(claim_id, evidence);
            refresh(&mut evidence.seal, &message);
        }
        let mut previous: Option<String> = None;
        for (index, verdict) in claim.verdicts.iter_mut().enumerate() {
            let message = verdict_message(claim_id, index, previous.as_deref(), verdict);
            refresh(&mut verdict.seal, &message);
            previous = mac_of(verdict.seal.as_ref()).map(str::to_owned);
        }
        let message = claim_message(claim);
        refresh(&mut claim.seal, &message);
    }
    let message = header_message(catalog);
    refresh(catalog.seal_mut(), &message);
}

/// Seals every unsealed record as adopted, for migrating a schema v1 catalog or recovering
/// from a merge conflict in the catalog header. Returns the names of the records it sealed.
///
/// It never re-seals a record whose seal is present but wrong, so it cannot launder an edit to
/// a sealed record. The one exception is the catalog header, which is re-sealed when every
/// other record verifies: two branches that each added claims or evidence conflict there, and
/// the id-gap check still catches deleted claims.
///
/// # Errors
///
/// [`ClaimsError::AdoptRefused`] listing every record with a wrong seal and every missing
/// claim id. Nothing is changed in that case.
pub fn adopt(catalog: &mut Catalog) -> Result<Vec<String>> {
    if catalog.schema_version() == UNSEALED_SCHEMA_VERSION {
        catalog.upgrade_schema();
        // A v1 file has no seals, but drop any that were added by hand so nothing is trusted.
        for claim in catalog.claims_mut() {
            claim.seal = None;
            claim
                .evidence
                .iter_mut()
                .for_each(|evidence| evidence.seal = None);
            claim
                .verdicts
                .iter_mut()
                .for_each(|verdict| verdict.seal = None);
        }
        *catalog.seal_mut() = None;
    }

    let report = verify(catalog);
    let wrong: Vec<IntegrityProblem> = report
        .problems
        .into_iter()
        .filter(|problem| problem.record != "catalog" && problem.problem != NO_SEAL)
        .collect();
    if !wrong.is_empty() {
        return Err(ClaimsError::AdoptRefused { problems: wrong });
    }

    let mut adopted = Vec::new();
    for claim in catalog.claims_mut() {
        let claim_id = claim.id;
        for evidence in &mut claim.evidence {
            if evidence.seal.is_none() {
                evidence.seal = Some(seal(
                    SealOrigin::Adopt,
                    &evidence_message(claim_id, evidence),
                ));
                adopted.push(evidence.id.to_string());
            }
        }
        let mut previous: Option<String> = None;
        for (index, verdict) in claim.verdicts.iter_mut().enumerate() {
            if verdict.seal.is_none() {
                let message = verdict_message(claim_id, index, previous.as_deref(), verdict);
                verdict.seal = Some(seal(SealOrigin::Adopt, &message));
                adopted.push(verdict_name(claim_id, index));
            }
            previous = mac_of(verdict.seal.as_ref()).map(str::to_owned);
        }
        if claim.seal.is_none() {
            claim.seal = Some(seal(SealOrigin::Adopt, &claim_message(claim)));
            adopted.push(claim_id.to_string());
        }
    }
    let message = header_message(catalog);
    if catalog
        .seal()
        .is_none_or(|existing| !matches(existing, &message))
    {
        *catalog.seal_mut() = Some(seal(SealOrigin::Adopt, &message));
        adopted.push("catalog".to_owned());
    }
    Ok(adopted)
}

/// Problem text for a record with no seal; [`adopt`] seals exactly these.
const NO_SEAL: &str = "has no seal";
/// Problem text for a record whose seal does not match its content.
const WRONG_SEAL: &str = "seal does not match its content";

impl IntegrityReport {
    /// Records one check of `seal` against `message`.
    fn check(&mut self, record: String, seal: Option<&Seal>, message: &Value) {
        self.records += 1;
        match seal {
            None => self.fail(record, NO_SEAL.to_owned()),
            Some(existing) if !matches(existing, message) => {
                self.fail(record, WRONG_SEAL.to_owned());
            }
            Some(existing) => {
                if existing.by == SealOrigin::Adopt {
                    self.adopted.push(record);
                }
            }
        }
    }

    fn fail(&mut self, record: String, problem: String) {
        self.verified = false;
        self.problems.push(IntegrityProblem { record, problem });
    }
}

/// Keeps `slot` if it matches `message`, otherwise replaces it with a fresh server seal.
fn refresh(slot: &mut Option<Seal>, message: &Value) {
    if !slot
        .as_ref()
        .is_some_and(|existing| matches(existing, message))
    {
        *slot = Some(seal(SealOrigin::Server, message));
    }
}

fn matches(existing: &Seal, message: &Value) -> bool {
    existing.mac == mac(existing.by, message)
}

fn seal(by: SealOrigin, message: &Value) -> Seal {
    Seal {
        by,
        mac: mac(by, message),
    }
}

/// HMAC-SHA256 over the domain tag, the origin, and the message, as lower-case hex.
///
/// The message is a `serde_json::Value`. Without serde_json's `preserve_order` feature its
/// objects are `BTreeMap`s, so keys serialize sorted: the bytes depend only on the record's
/// content, not on how the file was formatted or which order its keys were written in.
fn mac(by: SealOrigin, message: &Value) -> String {
    // Rust note: `<Hmac<Sha256> as KeyInit>::new_from_slice` only fails for key lengths HMAC
    // cannot use, and HMAC accepts any length, so the error branch is unreachable.
    let mut hmac = <Hmac<Sha256> as KeyInit>::new_from_slice(&SEAL_KEY)
        .expect("HMAC accepts keys of any length");
    hmac.update(DOMAIN);
    hmac.update(&to_bytes(&json!([by, message])));
    hmac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn header_message(catalog: &Catalog) -> Value {
    json!([
        "catalog",
        catalog.schema_version(),
        catalog.next_claim_seq(),
        catalog.next_evidence_seq(),
    ])
}

fn claim_message(claim: &Claim) -> Value {
    let mut fields = to_value(claim);
    if let Value::Object(map) = &mut fields {
        for key in ["seal", "evidence", "verdicts"] {
            map.remove(key);
        }
    }
    let evidence_ids: Vec<String> = claim.evidence.iter().map(|e| e.id.to_string()).collect();
    let latest_verdict = claim
        .verdicts
        .last()
        .and_then(|verdict| mac_of(verdict.seal.as_ref()));
    json!(["claim", fields, evidence_ids, latest_verdict])
}

fn evidence_message(claim_id: ClaimId, evidence: &impl Serialize) -> Value {
    json!(["evidence", claim_id.to_string(), without_seal(evidence)])
}

fn verdict_message(
    claim_id: ClaimId,
    index: usize,
    previous: Option<&str>,
    verdict: &VerdictRecord,
) -> Value {
    json!([
        "verdict",
        claim_id.to_string(),
        index,
        previous,
        without_seal(verdict)
    ])
}

fn without_seal(record: &impl Serialize) -> Value {
    let mut fields = to_value(record);
    if let Value::Object(map) = &mut fields {
        map.remove("seal");
    }
    fields
}

fn mac_of(seal: Option<&Seal>) -> Option<&str> {
    seal.map(|existing| existing.mac.as_str())
}

/// `CLM-0001 verdict 2`: verdicts have no ids, so they are named by claim and 1-based position.
fn verdict_name(claim_id: ClaimId, index: usize) -> String {
    format!("{claim_id} verdict {}", index + 1)
}

/// Ids below `next_claim_seq` with no claim. `Catalog::validate` already guarantees the ids
/// are strictly increasing and below the counter, so a single merge-style walk finds them.
fn missing_claim_ids(catalog: &Catalog) -> Vec<ClaimId> {
    let mut present = catalog.claims().iter().map(|claim| claim.id).peekable();
    (1..catalog.next_claim_seq())
        .map(ClaimId::new)
        .filter(|expected| {
            if present.peek() == Some(expected) {
                present.next();
                false
            } else {
                true
            }
        })
        .collect()
}

fn to_value(record: &impl Serialize) -> Value {
    serde_json::to_value(record).expect("catalog records always serialize to JSON")
}

fn to_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a JSON value always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::{at, evidence, new_claim, verdict};
    use crate::catalog::ClaimUpdate;
    use crate::model::{EvidenceId, Stance, Verdict};

    /// Two sealed claims: CLM-0001 with two evidence items and two verdicts, CLM-0002 bare.
    fn sealed() -> Catalog {
        let mut catalog = Catalog::default();
        let first = catalog
            .add_claim(new_claim("first"), at(0))
            .expect("add")
            .id;
        catalog.add_claim(new_claim("second"), at(0)).expect("add");
        let supports = catalog
            .add_evidence(first, evidence(Stance::Supports), at(1))
            .expect("evidence")
            .id;
        let refutes = catalog
            .add_evidence(first, evidence(Stance::Refutes), at(2))
            .expect("evidence")
            .id;
        catalog
            .record_verdict(first, verdict(Verdict::Supported, vec![supports]), at(3))
            .expect("verdict");
        catalog
            .record_verdict(first, verdict(Verdict::Refuted, vec![refutes]), at(4))
            .expect("verdict");
        reseal(&mut catalog);
        catalog
    }

    /// A hand edit of the catalog document.
    type Edit = fn(&mut Value);

    /// Applies `edit` to the catalog's JSON, the way a hand edit of the file would.
    fn hand_edit(catalog: &Catalog, edit: impl FnOnce(&mut Value)) -> Catalog {
        let mut document = serde_json::to_value(catalog).expect("serialize");
        edit(&mut document);
        serde_json::from_value(document).expect("still a well-formed catalog")
    }

    fn failed_records(catalog: &Catalog) -> Vec<String> {
        let report = verify(catalog);
        assert_eq!(report.verified, report.problems.is_empty());
        report
            .problems
            .into_iter()
            .map(|item| item.record)
            .collect()
    }

    #[test]
    fn sealed_catalog_verifies_every_record() {
        let report = verify(&sealed());
        assert!(report.verified, "{:?}", report.problems);
        // Header, 2 claims, 2 evidence items, 2 verdicts.
        assert_eq!(report.records, 7);
        assert!(report.adopted.is_empty());
    }

    #[test]
    fn pristine_catalog_needs_no_seal() {
        assert!(verify(&Catalog::default()).verified);
    }

    #[test]
    fn unsealed_v1_catalog_fails_with_one_problem() {
        let catalog = hand_edit(&sealed(), |document| document["schema_version"] = json!(1));
        let report = verify(&catalog);
        assert!(!report.verified);
        assert_eq!(report.problems.len(), 1);
        assert_eq!(report.problems[0].record, "catalog");
        assert!(report.problems[0].problem.contains("seal --adopt"));
    }

    #[test]
    fn reformatting_and_key_order_do_not_break_seals() {
        let catalog = sealed();
        // `Value` serializes keys alphabetically and compactly, unlike the struct's
        // pretty-printed field order.
        let reordered =
            serde_json::to_string(&serde_json::to_value(&catalog).expect("value")).expect("encode");
        let reloaded: Catalog = serde_json::from_str(&reordered).expect("decode");
        assert!(verify(&reloaded).verified);
    }

    #[test]
    fn edited_fields_are_detected_and_named() {
        let catalog = sealed();
        let cases: [(&str, Edit); 4] = [
            ("CLM-0001 verdict 2", |d| {
                d["claims"][0]["verdicts"][1]["verdict"] = json!("supported");
            }),
            ("EVD-0001", |d| {
                d["claims"][0]["evidence"][0]["summary"] = json!("rewritten");
            }),
            ("CLM-0002", |d| {
                d["claims"][1]["statement"] = json!("weakened")
            }),
            ("catalog", |d| d["next_evidence_seq"] = json!(9)),
        ];
        for (record, edit) in cases {
            assert_eq!(failed_records(&hand_edit(&catalog, edit)), [record]);
        }
    }

    #[test]
    fn deleted_and_reordered_records_are_detected() {
        let catalog = sealed();
        let cases: [Edit; 5] = [
            // Truncate the verdict history: the claim seal covers the latest verdict.
            |d| {
                drop(
                    d["claims"][0]["verdicts"]
                        .as_array_mut()
                        .expect("list")
                        .pop(),
                )
            },
            // Drop the first verdict: the second one chains to it.
            |d| {
                drop(
                    d["claims"][0]["verdicts"]
                        .as_array_mut()
                        .expect("list")
                        .remove(0),
                )
            },
            |d| {
                d["claims"][0]["verdicts"]
                    .as_array_mut()
                    .expect("list")
                    .swap(0, 1)
            },
            |d| {
                drop(
                    d["claims"][0]["evidence"]
                        .as_array_mut()
                        .expect("list")
                        .remove(1),
                )
            },
            // Delete a whole claim: ids are never reused, so the gap gives it away.
            |d| drop(d["claims"].as_array_mut().expect("list").remove(1)),
        ];
        for edit in cases {
            assert!(!verify(&hand_edit(&catalog, edit)).verified);
        }
        let without_second = hand_edit(&catalog, cases[4]);
        assert_eq!(failed_records(&without_second), ["CLM-0002"]);
    }

    #[test]
    fn moved_unsealed_and_relabeled_records_are_detected() {
        let catalog = sealed();
        let moved = hand_edit(&catalog, |d| {
            let item = d["claims"][0]["evidence"]
                .as_array_mut()
                .expect("list")
                .remove(1);
            d["claims"][1]["evidence"] = json!([item]);
        });
        assert!(failed_records(&moved).contains(&"EVD-0002".to_owned()));

        let unsealed = hand_edit(&catalog, |d| {
            d["claims"][1]["seal"].take();
        });
        let report = verify(&unsealed);
        assert_eq!(report.problems[0].problem, NO_SEAL);

        let relabeled = hand_edit(&catalog, |d| d["claims"][1]["seal"]["by"] = json!("adopt"));
        assert_eq!(failed_records(&relabeled), ["CLM-0002"]);
    }

    #[test]
    fn reseal_replaces_only_changed_seals_and_keeps_adopted_origin() {
        let mut catalog = sealed();
        catalog.claims_mut()[1].seal = None;
        adopt(&mut catalog).expect("adopt");
        let first_seal = catalog.claims()[0].seal.clone();

        catalog
            .update_claim(
                ClaimId::new(1),
                ClaimUpdate {
                    statement: Some("restated".to_owned()),
                    ..ClaimUpdate::default()
                },
                at(5),
            )
            .expect("update");
        reseal(&mut catalog);

        let report = verify(&catalog);
        assert!(report.verified);
        assert_ne!(catalog.claims()[0].seal, first_seal);
        assert_eq!(report.adopted, ["CLM-0002"]);
    }

    #[test]
    fn adopt_migrates_a_v1_catalog() {
        let v1 = hand_edit(&sealed(), |d| {
            d["schema_version"] = json!(1);
            d["seal"].take();
        });
        let mut catalog = v1.clone();
        let adopted = adopt(&mut catalog).expect("adopt");

        assert_eq!(catalog.schema_version(), crate::catalog::SCHEMA_VERSION);
        assert_eq!(adopted.len(), 7);
        let report = verify(&catalog);
        assert!(report.verified);
        assert_eq!(report.adopted.len(), 7);
        assert_eq!(
            adopt(&mut catalog).expect("adopt again"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn adopt_refuses_edited_records_and_changes_nothing() {
        let edited = hand_edit(&sealed(), |d| {
            d["claims"][0]["verdicts"][1]["verdict"] = json!("supported");
        });
        let mut catalog = edited.clone();
        let error = adopt(&mut catalog).expect_err("must refuse");
        assert!(matches!(error, ClaimsError::AdoptRefused { .. }));
        assert_eq!(catalog, edited);
    }

    #[test]
    fn adopt_labels_a_stripped_record_instead_of_trusting_it() {
        let mut catalog = hand_edit(&sealed(), |d| {
            d["claims"][1]["statement"] = json!("weakened");
            d["claims"][1]["seal"].take();
        });
        assert_eq!(adopt(&mut catalog).expect("adopt"), ["CLM-0002"]);
        assert_eq!(verify(&catalog).adopted, ["CLM-0002"]);
    }

    #[test]
    fn branches_touching_different_claims_merge_without_resealing() {
        let base = sealed();
        let mut ours = base.clone();
        ours.retire_claim(ClaimId::new(1), "obsolete", at(5))
            .expect("retire");
        reseal(&mut ours);
        let mut theirs = base.clone();
        let restate = ClaimUpdate {
            statement: Some("restated".to_owned()),
            ..ClaimUpdate::default()
        };
        theirs
            .update_claim(ClaimId::new(2), restate, at(6))
            .expect("update");
        reseal(&mut theirs);

        // What git produces when each side changed a different claim.
        let merged = hand_edit(&base, |d| {
            d["claims"][0] = serde_json::to_value(&ours.claims()[0]).expect("value");
            d["claims"][1] = serde_json::to_value(&theirs.claims()[1]).expect("value");
        });
        assert!(verify(&merged).verified);
    }

    #[test]
    fn adopt_reseals_only_the_header_after_a_counter_conflict() {
        let base = sealed();
        let mut ours = base.clone();
        ours.add_claim(new_claim("third"), at(5)).expect("add");
        reseal(&mut ours);
        let mut theirs = base.clone();
        theirs
            .add_evidence(ClaimId::new(2), evidence(Stance::Neutral), at(6))
            .expect("evidence");
        reseal(&mut theirs);

        // Both sides changed the header's counters; the conflict is resolved by hand.
        let mut merged = hand_edit(&ours, |d| {
            d["claims"][1] = serde_json::to_value(&theirs.claims()[1]).expect("value");
            d["next_evidence_seq"] = json!(theirs.next_evidence_seq());
        });
        assert_eq!(failed_records(&merged), ["catalog"]);
        assert_eq!(adopt(&mut merged).expect("adopt"), ["catalog"]);
        assert!(verify(&merged).verified);
        assert_eq!(merged.claims()[1].evidence[0].id, EvidenceId::new(3));
    }

    #[test]
    fn missing_claim_ids_lists_every_gap() {
        let catalog = hand_edit(&sealed(), |d| {
            drop(d["claims"].as_array_mut().expect("list").remove(0));
            d["next_claim_seq"] = json!(5);
        });
        let missing: Vec<String> = missing_claim_ids(&catalog)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(missing, ["CLM-0001", "CLM-0003", "CLM-0004"]);
    }
}
