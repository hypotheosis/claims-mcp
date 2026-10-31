//! In-memory catalog operations and invariant checks. No I/O happens here.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{ClaimsError, Result};
use crate::model::{
    Anchor, Claim, ClaimId, ClaimKind, ClaimLevel, ClaimRef, ClaimStatus, CurrentVerdict, Evidence,
    EvidenceId, EvidenceKind, FileAnchor, IntegrityLevel, Seal, Stance, Verdict, VerdictRecord,
    VerificationMethod,
};

/// The catalog `schema_version` this build writes: records carry integrity seals.
pub const SCHEMA_VERSION: u32 = 2;

/// The older, unsealed `schema_version`. It is still read, so `claims-mcp seal --adopt` can
/// migrate it, but it never passes verification, so the server refuses to write to it.
pub const UNSEALED_SCHEMA_VERSION: u32 = 1;

/// Input for creating a claim (the `add_claim` tool parameters and `add_claims` entries).
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
pub struct NewClaim {
    /// One atomic, falsifiable assertion.
    pub statement: String,
    /// What sort of property the claim asserts.
    pub kind: ClaimKind,
    /// Position in the requirements hierarchy.
    pub level: ClaimLevel,
    /// How the claim is meant to be verified.
    pub verification_method: VerificationMethod,
    /// IEEE 1012 integrity level (1-4).
    #[serde(default)]
    pub integrity_level: Option<IntegrityLevel>,
    /// Where the claim came from (prompt, spec section, ticket).
    #[serde(default)]
    pub source: Option<String>,
    /// Pinned location of the claim's source text. Never read from tool input: the server
    /// pins the caller's anchor and sets it.
    #[serde(skip)]
    pub source_anchor: Option<FileAnchor>,
    /// Parent claims: existing ids, or `@N` for an earlier entry of the same batch.
    #[serde(default)]
    pub derived_from: Vec<ClaimRef>,
    /// Artifacts the claim is about.
    #[serde(default)]
    pub artifacts: Vec<String>,
    /// Free-form labels.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Metadata changes for `update_claim`. Omitted fields are left unchanged; list fields
/// replace the whole list.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, JsonSchema)]
pub struct ClaimUpdate {
    /// New statement text.
    #[serde(default)]
    pub statement: Option<String>,
    /// New kind.
    #[serde(default)]
    pub kind: Option<ClaimKind>,
    /// New hierarchy level.
    #[serde(default)]
    pub level: Option<ClaimLevel>,
    /// New verification method.
    #[serde(default)]
    pub verification_method: Option<VerificationMethod>,
    /// New integrity level.
    #[serde(default)]
    pub integrity_level: Option<IntegrityLevel>,
    /// New source.
    #[serde(default)]
    pub source: Option<String>,
    /// New pinned source location, set by the server from the caller's anchor.
    #[serde(skip)]
    pub source_anchor: Option<FileAnchor>,
    /// Replacement parent list (existing claim ids only).
    #[serde(default)]
    pub derived_from: Option<Vec<ClaimId>>,
    /// Replacement artifact list.
    #[serde(default)]
    pub artifacts: Option<Vec<String>>,
    /// Replacement tag list.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Input for `add_evidence`.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
pub struct NewEvidence {
    /// Form of the evidence.
    pub kind: EvidenceKind,
    /// Where to find it: `path:line`, test name, command, or URL.
    pub locator: String,
    /// What the evidence shows.
    pub summary: String,
    /// Whether it supports, refutes, or is neutral toward the claim.
    pub stance: Stance,
    /// Pinned location, set by the server from the caller's anchor.
    #[serde(skip)]
    pub anchor: Option<Anchor>,
}

/// Input for `record_verdict`.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
pub struct NewVerdict {
    /// FEVER verdict.
    pub verdict: Verdict,
    /// Why the evidence leads to this verdict.
    pub rationale: String,
    /// Who is recording the verdict (person or model id).
    pub auditor: String,
    /// Evidence on this claim the verdict relies on.
    #[serde(default)]
    pub evidence_ids: Vec<EvidenceId>,
}

/// Filters for `list_claims`. All set filters must match.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, JsonSchema)]
pub struct ClaimFilter {
    /// Current verdict, including `unverified`.
    #[serde(default)]
    pub verdict: Option<CurrentVerdict>,
    /// Claim kind.
    #[serde(default)]
    pub kind: Option<ClaimKind>,
    /// Hierarchy level.
    #[serde(default)]
    pub level: Option<ClaimLevel>,
    /// Exact tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Substring of any artifact entry.
    #[serde(default)]
    pub artifact: Option<String>,
    /// Case-insensitive substring of the statement.
    #[serde(default)]
    pub text: Option<String>,
    /// Include retired claims (default false).
    #[serde(default)]
    pub include_retired: bool,
}

/// Compact row returned by `list_claims`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClaimSummary {
    /// Claim id.
    pub id: ClaimId,
    /// Claim statement.
    pub statement: String,
    /// Claim kind.
    pub kind: ClaimKind,
    /// Hierarchy level.
    pub level: ClaimLevel,
    /// Integrity level, if assigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity_level: Option<IntegrityLevel>,
    /// Latest verdict or `unverified`.
    pub current_verdict: CurrentVerdict,
    /// Number of evidence items.
    pub evidence_count: usize,
    /// `true` when the claim is retired.
    pub retired: bool,
}

/// Full claim plus derived fields, returned by `get_claim`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClaimDetail {
    /// The stored claim.
    #[serde(flatten)]
    pub claim: Claim,
    /// Latest verdict or `unverified`.
    pub current_verdict: CurrentVerdict,
    /// Active claims that list this one in `derived_from`.
    pub children: Vec<ClaimId>,
}

/// The whole catalog document.
///
/// Claims are kept sorted by id; because ids only grow, appending preserves the order and
/// lookups are a binary search (O(log n)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Catalog {
    /// Catalog format version described by this schema.
    #[schemars(range(equal = SCHEMA_VERSION))]
    schema_version: u32,
    /// Next never-used claim sequence number.
    next_claim_seq: u32,
    /// Integrity seal over `schema_version` and `next_claim_seq` (see `src/seal.rs`). Each
    /// counter has its own seal, next to it, so branches that advanced different counters
    /// merge without touching each other's seal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_claim_seq_seal: Option<Seal>,
    /// Next never-used, catalog-wide evidence sequence number.
    next_evidence_seq: u32,
    /// Integrity seal over `schema_version` and `next_evidence_seq`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_evidence_seq_seal: Option<Seal>,
    claims: Vec<Claim>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            next_claim_seq: 1,
            next_claim_seq_seal: None,
            next_evidence_seq: 1,
            next_evidence_seq_seal: None,
            claims: Vec::new(),
        }
    }
}

impl Catalog {
    /// Returns the schema version declared by the loaded document.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns every claim, sorted by id.
    pub fn claims(&self) -> &[Claim] {
        &self.claims
    }

    /// Mutable access to every claim, for (re)sealing records in place. Ids and order must not
    /// change.
    pub(crate) fn claims_mut(&mut self) -> &mut [Claim] {
        &mut self.claims
    }

    /// The next claim sequence number to be assigned.
    pub fn next_claim_seq(&self) -> u32 {
        self.next_claim_seq
    }

    /// Each id counter with its name, value, and integrity seal.
    pub fn counter_seals(&self) -> [(&'static str, u32, Option<&Seal>); 2] {
        [
            (
                "next_claim_seq",
                self.next_claim_seq,
                self.next_claim_seq_seal.as_ref(),
            ),
            (
                "next_evidence_seq",
                self.next_evidence_seq,
                self.next_evidence_seq_seal.as_ref(),
            ),
        ]
    }

    /// Like [`Catalog::counter_seals`], with mutable seals for (re)sealing in place.
    ///
    /// Rust note: borrowing two different fields mutably at once is allowed, because the
    /// compiler can see they never overlap.
    pub(crate) fn counter_seals_mut(&mut self) -> [(&'static str, u32, &mut Option<Seal>); 2] {
        [
            (
                "next_claim_seq",
                self.next_claim_seq,
                &mut self.next_claim_seq_seal,
            ),
            (
                "next_evidence_seq",
                self.next_evidence_seq,
                &mut self.next_evidence_seq_seal,
            ),
        ]
    }

    /// Marks the catalog as the current (sealed) schema version, for `seal --adopt`.
    pub(crate) fn upgrade_schema(&mut self) {
        self.schema_version = SCHEMA_VERSION;
    }

    /// `true` for a catalog nothing was ever added to: no claims and both counters at 1. A
    /// missing catalog file loads as one, and it needs no seal.
    pub fn is_pristine(&self) -> bool {
        self.claims.is_empty() && self.next_claim_seq == 1 && self.next_evidence_seq == 1
    }

    /// Checks invariants that a hand-edited or merged file could break: ids strictly
    /// increasing (lookups binary-search), every id below its `next_*_seq` counter (ids are
    /// never reused), evidence ids unique, and every `derived_from` link resolving.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Validation`] describing the first problem found.
    pub fn validate(&self) -> Result<()> {
        let inconsistent = |problem: String| {
            Err(ClaimsError::Validation(format!(
                "catalog is inconsistent: {problem}"
            )))
        };
        if let Some(pair) = self.claims.windows(2).find(|pair| pair[0].id >= pair[1].id) {
            return inconsistent(format!("{} is listed after {}", pair[1].id, pair[0].id));
        }
        let mut evidence_ids = HashSet::new();
        for claim in &self.claims {
            if claim.id >= ClaimId::new(self.next_claim_seq) {
                return inconsistent(format!(
                    "{} is not below next_claim_seq {}",
                    claim.id, self.next_claim_seq
                ));
            }
            if let Some(parent) = claim
                .derived_from
                .iter()
                .find(|p| self.index_of(**p).is_none())
            {
                return inconsistent(format!("{} derives from missing {parent}", claim.id));
            }
            for evidence in &claim.evidence {
                if evidence.id >= EvidenceId::new(self.next_evidence_seq) {
                    return inconsistent(format!(
                        "{} is not below next_evidence_seq {}",
                        evidence.id, self.next_evidence_seq
                    ));
                }
                if !evidence_ids.insert(evidence.id) {
                    return inconsistent(format!("{} appears more than once", evidence.id));
                }
            }
        }
        Ok(())
    }

    /// Looks up a claim by id.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::ClaimNotFound`] if no claim has this id.
    pub fn get(&self, id: ClaimId) -> Result<&Claim> {
        self.index_of(id)
            .map(|index| &self.claims[index])
            .ok_or(ClaimsError::ClaimNotFound(id))
    }

    /// Returns a claim with its current verdict and active children.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::ClaimNotFound`] if no claim has this id.
    pub fn detail(&self, id: ClaimId) -> Result<ClaimDetail> {
        let claim = self.get(id)?;
        let children = self
            .claims
            .iter()
            .filter(|candidate| candidate.is_active() && candidate.derived_from.contains(&id))
            .map(|candidate| candidate.id)
            .collect();
        Ok(ClaimDetail {
            current_verdict: claim.current_verdict(),
            claim: claim.clone(),
            children,
        })
    }

    /// Returns summaries of the claims matching `filter`, in id order.
    pub fn list(&self, filter: &ClaimFilter) -> Vec<ClaimSummary> {
        // Lower-case the needle once instead of once per claim.
        let text_needle = filter.text.as_deref().map(str::to_lowercase);
        self.claims
            .iter()
            .filter(|claim| filter.include_retired || claim.is_active())
            .filter(|claim| filter.verdict.is_none_or(|v| claim.current_verdict() == v))
            .filter(|claim| filter.kind.is_none_or(|kind| claim.kind == kind))
            .filter(|claim| filter.level.is_none_or(|level| claim.level == level))
            .filter(|claim| {
                filter
                    .tag
                    .as_deref()
                    .is_none_or(|tag| claim.tags.iter().any(|t| t == tag))
            })
            .filter(|claim| {
                filter
                    .artifact
                    .as_deref()
                    .is_none_or(|needle| claim.artifacts.iter().any(|a| a.contains(needle)))
            })
            .filter(|claim| {
                text_needle
                    .as_deref()
                    .is_none_or(|needle| claim.statement.to_lowercase().contains(needle))
            })
            .map(|claim| ClaimSummary {
                id: claim.id,
                statement: claim.statement.clone(),
                kind: claim.kind,
                level: claim.level,
                integrity_level: claim.integrity_level,
                current_verdict: claim.current_verdict(),
                evidence_count: claim.evidence.len(),
                retired: !claim.is_active(),
            })
            .collect()
    }

    /// Creates one claim. `@N` batch references are rejected here.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Validation`] for empty statements or batch references, and
    /// [`ClaimsError::ClaimNotFound`] / [`ClaimsError::RetiredParent`] for bad parents.
    pub fn add_claim(&mut self, new_claim: NewClaim, now: DateTime<Utc>) -> Result<Claim> {
        if new_claim
            .derived_from
            .iter()
            .any(|parent| matches!(parent, ClaimRef::Batch(_)))
        {
            return Err(ClaimsError::Validation(
                "@N references are only valid in add_claims".to_owned(),
            ));
        }
        let mut created = self.add_claims(vec![new_claim], now)?;
        Ok(created
            .pop()
            .expect("add_claims returns exactly one claim per input"))
    }

    /// Creates several claims atomically. `@N` in `derived_from` refers to the earlier
    /// batch entry at index `N`, so a whole hierarchy can be added in one call.
    ///
    /// Every entry is validated before anything is inserted, so on error the catalog is
    /// unchanged. New claims have no children yet and may only point at existing claims or
    /// earlier batch entries, so no cycle can be formed.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Validation`] for empty input, empty statements, or forward/self `@N`
    /// references; [`ClaimsError::ClaimNotFound`] / [`ClaimsError::RetiredParent`] for bad
    /// parents. When the batch has more than one entry, errors are wrapped as
    /// `Validation("claims[i]: ...")` so the caller knows which entry failed.
    pub fn add_claims(
        &mut self,
        new_claims: Vec<NewClaim>,
        now: DateTime<Utc>,
    ) -> Result<Vec<Claim>> {
        if new_claims.is_empty() {
            return Err(ClaimsError::Validation(
                "claims must not be empty".to_owned(),
            ));
        }
        let batch_size = new_claims.len();
        let first_seq = self.next_claim_seq;
        // Reserve the whole id range up front with checked arithmetic, so a huge batch or a
        // hand-edited `next_claim_seq` near u32::MAX is an error instead of a wrapped id.
        let next_seq = u32::try_from(batch_size)
            .ok()
            .and_then(|count| first_seq.checked_add(count))
            .ok_or_else(|| ClaimsError::Validation("claim id space exhausted".to_owned()))?;
        let mut created = Vec::with_capacity(batch_size);
        for (index, new_claim) in new_claims.into_iter().enumerate() {
            let claim = self
                .build_claim(new_claim, index, first_seq, now)
                .map_err(|error| batch_error(batch_size, index, error))?;
            created.push(claim);
        }
        self.next_claim_seq = next_seq;
        self.claims.extend(created.iter().cloned());
        Ok(created)
    }

    /// Validates one batch entry and builds the claim it describes (nothing is inserted).
    fn build_claim(
        &self,
        new_claim: NewClaim,
        index: usize,
        first_seq: u32,
        now: DateTime<Utc>,
    ) -> Result<Claim> {
        let derived_from = new_claim
            .derived_from
            .iter()
            .map(|parent| self.resolve_ref(*parent, index, first_seq))
            .collect::<Result<Vec<_>>>()?;
        Ok(Claim {
            // Cannot overflow: `add_claims` checked `first_seq + batch size` already.
            id: ClaimId::new(first_seq + index as u32),
            statement: required_text("statement", &new_claim.statement)?,
            kind: new_claim.kind,
            level: new_claim.level,
            integrity_level: new_claim.integrity_level,
            verification_method: new_claim.verification_method,
            source: optional_text(new_claim.source),
            source_anchor: new_claim.source_anchor,
            derived_from: dedup_ids(derived_from),
            artifacts: clean_list(new_claim.artifacts),
            tags: clean_list(new_claim.tags),
            status: ClaimStatus::Active,
            evidence: Vec::new(),
            verdicts: Vec::new(),
            created_at: now,
            updated_at: now,
            seal: None,
        })
    }

    /// Applies metadata changes to an active claim.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::ClaimNotFound`], [`ClaimsError::Retired`], [`ClaimsError::RetiredParent`],
    /// [`ClaimsError::Validation`] for an empty statement, and [`ClaimsError::Cycle`] if a new
    /// parent is a descendant.
    pub fn update_claim(
        &mut self,
        id: ClaimId,
        update: ClaimUpdate,
        now: DateTime<Utc>,
    ) -> Result<Claim> {
        let index = self.active_index(id)?;
        let statement = update
            .statement
            .as_deref()
            .map(|text| required_text("statement", text))
            .transpose()?;
        let derived_from = match update.derived_from {
            Some(parents) => {
                let parents = dedup_ids(parents);
                for parent in &parents {
                    // Links that already exist may stay even if that parent was retired
                    // later (the audit report lists them as dangling_links); new links must
                    // point at active claims.
                    if !self.claims[index].derived_from.contains(parent) {
                        self.check_parent(*parent)?;
                    }
                    if self.reaches(*parent, id) {
                        return Err(ClaimsError::Cycle {
                            child: id,
                            parent: *parent,
                        });
                    }
                }
                Some(parents)
            }
            None => None,
        };

        let claim = &mut self.claims[index];
        if let Some(statement) = statement {
            claim.statement = statement;
        }
        if let Some(kind) = update.kind {
            claim.kind = kind;
        }
        if let Some(level) = update.level {
            claim.level = level;
        }
        if let Some(method) = update.verification_method {
            claim.verification_method = method;
        }
        if let Some(level) = update.integrity_level {
            claim.integrity_level = Some(level);
        }
        if let Some(source) = update.source {
            claim.source = optional_text(Some(source));
        }
        if let Some(anchor) = update.source_anchor {
            claim.source_anchor = Some(anchor);
        }
        if let Some(parents) = derived_from {
            claim.derived_from = parents;
        }
        if let Some(artifacts) = update.artifacts {
            claim.artifacts = clean_list(artifacts);
        }
        if let Some(tags) = update.tags {
            claim.tags = clean_list(tags);
        }
        claim.updated_at = now;
        Ok(claim.clone())
    }

    /// Retires an active claim. It stays in the catalog so trace links remain valid.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::ClaimNotFound`], [`ClaimsError::Retired`], or
    /// [`ClaimsError::Validation`] for an empty reason.
    pub fn retire_claim(&mut self, id: ClaimId, reason: &str, now: DateTime<Utc>) -> Result<Claim> {
        let index = self.active_index(id)?;
        let reason = required_text("reason", reason)?;
        let claim = &mut self.claims[index];
        claim.status = ClaimStatus::Retired {
            reason,
            retired_at: now,
        };
        claim.updated_at = now;
        Ok(claim.clone())
    }

    /// Attaches evidence to an active claim and returns it with its new id.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::ClaimNotFound`], [`ClaimsError::Retired`], or
    /// [`ClaimsError::Validation`] for an empty locator or summary.
    pub fn add_evidence(
        &mut self,
        claim_id: ClaimId,
        new_evidence: NewEvidence,
        now: DateTime<Utc>,
    ) -> Result<Evidence> {
        let index = self.active_index(claim_id)?;
        let next_seq = self
            .next_evidence_seq
            .checked_add(1)
            .ok_or_else(|| ClaimsError::Validation("evidence id space exhausted".to_owned()))?;
        let evidence = Evidence {
            id: EvidenceId::new(self.next_evidence_seq),
            kind: new_evidence.kind,
            locator: required_text("locator", &new_evidence.locator)?,
            summary: required_text("summary", &new_evidence.summary)?,
            stance: new_evidence.stance,
            anchor: new_evidence.anchor,
            recorded_at: now,
            seal: None,
        };
        self.next_evidence_seq = next_seq;
        let claim = &mut self.claims[index];
        claim.evidence.push(evidence.clone());
        claim.updated_at = now;
        Ok(evidence)
    }

    /// Appends a verdict to an active claim's history after checking it against the cited
    /// evidence (spec section 3.4, rule 4).
    ///
    /// # Errors
    ///
    /// [`ClaimsError::ClaimNotFound`], [`ClaimsError::Retired`],
    /// [`ClaimsError::ForeignEvidence`] for evidence not on this claim, and
    /// [`ClaimsError::Validation`] for empty text or a verdict its evidence does not back.
    pub fn record_verdict(
        &mut self,
        claim_id: ClaimId,
        new_verdict: NewVerdict,
        now: DateTime<Utc>,
    ) -> Result<Claim> {
        let index = self.active_index(claim_id)?;
        let rationale = required_text("rationale", &new_verdict.rationale)?;
        let auditor = required_text("auditor", &new_verdict.auditor)?;
        let evidence_ids = dedup_ids(new_verdict.evidence_ids);

        let claim = &mut self.claims[index];
        let cited = evidence_ids
            .iter()
            .map(|evidence_id| {
                claim
                    .evidence
                    .iter()
                    .find(|item| item.id == *evidence_id)
                    .ok_or(ClaimsError::ForeignEvidence {
                        claim: claim_id,
                        evidence: *evidence_id,
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let required_stance = match new_verdict.verdict {
            Verdict::Supported => Some(Stance::Supports),
            Verdict::Refuted => Some(Stance::Refutes),
            Verdict::NotEnoughInfo => None,
        };
        if let Some(stance) = required_stance {
            if !cited.iter().any(|item| item.stance == stance) {
                return Err(ClaimsError::Validation(format!(
                    "verdict '{}' requires at least one cited evidence item with stance '{}'",
                    snake_name(&new_verdict.verdict),
                    snake_name(&stance)
                )));
            }
        }

        claim.verdicts.push(VerdictRecord {
            verdict: new_verdict.verdict,
            rationale,
            auditor,
            evidence_ids,
            recorded_at: now,
            seal: None,
        });
        claim.updated_at = now;
        Ok(claim.clone())
    }

    fn index_of(&self, id: ClaimId) -> Option<usize> {
        self.claims.binary_search_by_key(&id, |claim| claim.id).ok()
    }

    /// Checks that `id` exists and is active, so callers can fail fast before slow work.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::ClaimNotFound`] or [`ClaimsError::Retired`].
    pub fn ensure_active(&self, id: ClaimId) -> Result<()> {
        self.active_index(id).map(|_| ())
    }

    /// Index of `id`, failing if it is missing or retired.
    fn active_index(&self, id: ClaimId) -> Result<usize> {
        let index = self.index_of(id).ok_or(ClaimsError::ClaimNotFound(id))?;
        if self.claims[index].is_active() {
            Ok(index)
        } else {
            Err(ClaimsError::Retired(id))
        }
    }

    /// A new `derived_from` entry must name an existing, active claim.
    fn check_parent(&self, parent: ClaimId) -> Result<()> {
        let index = self
            .index_of(parent)
            .ok_or(ClaimsError::ClaimNotFound(parent))?;
        if self.claims[index].is_active() {
            Ok(())
        } else {
            Err(ClaimsError::RetiredParent(parent))
        }
    }

    /// Resolves a batch entry's parent reference to a concrete id.
    fn resolve_ref(&self, parent: ClaimRef, entry_index: usize, first_seq: u32) -> Result<ClaimId> {
        match parent {
            ClaimRef::Existing(id) => self.check_parent(id).map(|()| id),
            ClaimRef::Batch(target) if target < entry_index => {
                Ok(ClaimId::new(first_seq + target as u32))
            }
            ClaimRef::Batch(target) => Err(ClaimsError::Validation(format!(
                "entry {entry_index} references @{target}; @N must point at an earlier entry"
            ))),
        }
    }

    /// Returns `true` if walking `derived_from` links upward from `start` reaches `target`
    /// (or `start == target`). Depth-first with a visited set: O(n + e).
    fn reaches(&self, start: ClaimId, target: ClaimId) -> bool {
        let mut stack = vec![start];
        let mut visited = HashSet::new();
        while let Some(current) = stack.pop() {
            if current == target {
                return true;
            }
            if !visited.insert(current) {
                continue;
            }
            if let Ok(claim) = self.get(current) {
                stack.extend(claim.derived_from.iter().copied());
            }
        }
        false
    }
}

/// Prefixes an error with the failing entry's index when a batch has several entries.
fn batch_error(batch_size: usize, index: usize, error: ClaimsError) -> ClaimsError {
    if batch_size > 1 {
        ClaimsError::Validation(format!("claims[{index}]: {error}"))
    } else {
        error
    }
}

/// Serializes a unit enum variant to its snake_case name for error messages.
fn snake_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Trims `value` and rejects it if nothing is left.
fn required_text(field: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(ClaimsError::Validation(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(trimmed.to_owned())
    }
}

/// Trims an optional string, turning blank input into `None`.
fn optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// Trims entries, drops blanks, and removes duplicates keeping the first occurrence.
fn clean_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::with_capacity(values.len());
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

/// Removes duplicate ids keeping the first occurrence.
fn dedup_ids<T: Copy + Eq + std::hash::Hash>(ids: Vec<T>) -> Vec<T> {
    let mut seen = HashSet::with_capacity(ids.len());
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Fixed timestamp so tests are deterministic.
    pub(crate) fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).expect("valid test timestamp")
    }

    pub(crate) fn new_claim(statement: &str) -> NewClaim {
        NewClaim {
            statement: statement.to_owned(),
            kind: ClaimKind::Functional,
            level: ClaimLevel::Software,
            verification_method: VerificationMethod::Test,
            integrity_level: None,
            source: None,
            source_anchor: None,
            derived_from: Vec::new(),
            artifacts: Vec::new(),
            tags: Vec::new(),
        }
    }

    pub(crate) fn evidence(stance: Stance) -> NewEvidence {
        NewEvidence {
            kind: EvidenceKind::Test,
            locator: "tests/a.rs:1".to_owned(),
            summary: "observed".to_owned(),
            stance,
            anchor: None,
        }
    }

    pub(crate) fn verdict(verdict: Verdict, evidence_ids: Vec<EvidenceId>) -> NewVerdict {
        NewVerdict {
            verdict,
            rationale: "because".to_owned(),
            auditor: "tester".to_owned(),
            evidence_ids,
        }
    }

    fn child_of(statement: &str, parent: ClaimRef) -> NewClaim {
        NewClaim {
            derived_from: vec![parent],
            ..new_claim(statement)
        }
    }

    #[test]
    fn add_claim_assigns_sequential_ids_and_cleans_input() {
        let mut catalog = Catalog::default();
        let mut input = new_claim("  first  ");
        input.tags = vec!["a".into(), " a ".into(), "".into(), "b".into()];
        let first = catalog.add_claim(input, at(0)).expect("add first");
        let second = catalog
            .add_claim(new_claim("second"), at(1))
            .expect("add second");

        assert_eq!(first.id, ClaimId::new(1));
        assert_eq!(first.statement, "first");
        assert_eq!(first.tags, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(second.id, ClaimId::new(2));
        assert_eq!(catalog.get(second.id).expect("get").statement, "second");
    }

    #[test]
    fn add_claim_rejects_empty_statement_and_batch_refs() {
        let mut catalog = Catalog::default();
        assert!(matches!(
            catalog.add_claim(new_claim("   "), at(0)),
            Err(ClaimsError::Validation(_))
        ));
        assert!(matches!(
            catalog.add_claim(child_of("x", ClaimRef::Batch(0)), at(0)),
            Err(ClaimsError::Validation(_))
        ));
    }

    #[test]
    fn add_claim_rejects_missing_parent() {
        let mut catalog = Catalog::default();
        let result = catalog.add_claim(child_of("x", ClaimRef::Existing(ClaimId::new(9))), at(0));
        assert!(matches!(result, Err(ClaimsError::ClaimNotFound(id)) if id == ClaimId::new(9)));
    }

    #[test]
    fn add_claims_resolves_batch_refs() {
        let mut catalog = Catalog::default();
        catalog
            .add_claim(new_claim("existing"), at(0))
            .expect("seed");
        let created = catalog
            .add_claims(
                vec![
                    child_of("root", ClaimRef::Existing(ClaimId::new(1))),
                    child_of("leaf", ClaimRef::Batch(0)),
                ],
                at(1),
            )
            .expect("batch");

        assert_eq!(created[0].id, ClaimId::new(2));
        assert_eq!(created[1].derived_from, vec![ClaimId::new(2)]);
    }

    #[test]
    fn add_claims_is_all_or_nothing() {
        let mut catalog = Catalog::default();
        let before = catalog.clone();
        let result = catalog.add_claims(
            vec![new_claim("ok"), child_of("forward", ClaimRef::Batch(1))],
            at(0),
        );
        assert!(matches!(result, Err(ClaimsError::Validation(_))));
        assert_eq!(catalog, before);
    }

    #[test]
    fn retired_claims_reject_changes_and_ids_are_not_reused() {
        let mut catalog = Catalog::default();
        let claim = catalog.add_claim(new_claim("x"), at(0)).expect("add");
        catalog
            .retire_claim(claim.id, "obsolete", at(1))
            .expect("retire");

        assert!(matches!(
            catalog.add_evidence(claim.id, evidence(Stance::Supports), at(2)),
            Err(ClaimsError::Retired(_))
        ));
        assert!(matches!(
            catalog.update_claim(claim.id, ClaimUpdate::default(), at(2)),
            Err(ClaimsError::Retired(_))
        ));
        assert!(matches!(
            catalog.add_claim(child_of("y", ClaimRef::Existing(claim.id)), at(2)),
            Err(ClaimsError::RetiredParent(_))
        ));
        let next = catalog
            .add_claim(new_claim("z"), at(3))
            .expect("add after retire");
        assert_eq!(next.id, ClaimId::new(2));
    }

    #[test]
    fn update_claim_rejects_cycles_and_self_links() {
        let mut catalog = Catalog::default();
        let root = catalog.add_claim(new_claim("root"), at(0)).expect("root");
        let leaf = catalog
            .add_claim(child_of("leaf", ClaimRef::Existing(root.id)), at(0))
            .expect("leaf");

        let cycle = ClaimUpdate {
            derived_from: Some(vec![leaf.id]),
            ..ClaimUpdate::default()
        };
        assert!(matches!(
            catalog.update_claim(root.id, cycle, at(1)),
            Err(ClaimsError::Cycle { .. })
        ));
        let self_link = ClaimUpdate {
            derived_from: Some(vec![root.id]),
            ..ClaimUpdate::default()
        };
        assert!(matches!(
            catalog.update_claim(root.id, self_link, at(1)),
            Err(ClaimsError::Cycle { .. })
        ));
    }

    #[test]
    fn update_claim_changes_only_given_fields() {
        let mut catalog = Catalog::default();
        let claim = catalog.add_claim(new_claim("old"), at(0)).expect("add");
        let update = ClaimUpdate {
            statement: Some("new".into()),
            tags: Some(vec!["t".into()]),
            ..ClaimUpdate::default()
        };
        let updated = catalog
            .update_claim(claim.id, update, at(5))
            .expect("update");

        assert_eq!(updated.statement, "new");
        assert_eq!(updated.tags, vec!["t".to_owned()]);
        assert_eq!(updated.kind, claim.kind);
        assert_eq!(updated.updated_at, at(5));
    }

    #[test]
    fn evidence_ids_are_catalog_wide() {
        let mut catalog = Catalog::default();
        let a = catalog.add_claim(new_claim("a"), at(0)).expect("a");
        let b = catalog.add_claim(new_claim("b"), at(0)).expect("b");
        let first = catalog
            .add_evidence(a.id, evidence(Stance::Supports), at(1))
            .expect("e1");
        let second = catalog
            .add_evidence(b.id, evidence(Stance::Refutes), at(1))
            .expect("e2");
        assert_eq!(first.id, EvidenceId::new(1));
        assert_eq!(second.id, EvidenceId::new(2));
    }

    #[test]
    fn add_evidence_rejects_blank_locator() {
        let mut catalog = Catalog::default();
        let claim = catalog.add_claim(new_claim("a"), at(0)).expect("add");
        let mut input = evidence(Stance::Neutral);
        input.locator = " ".into();
        assert!(matches!(
            catalog.add_evidence(claim.id, input, at(1)),
            Err(ClaimsError::Validation(_))
        ));
    }

    #[test]
    fn supported_verdict_requires_supporting_evidence() {
        let mut catalog = Catalog::default();
        let claim = catalog.add_claim(new_claim("a"), at(0)).expect("add");
        let neutral = catalog
            .add_evidence(claim.id, evidence(Stance::Neutral), at(1))
            .expect("e");

        let result = catalog.record_verdict(
            claim.id,
            verdict(Verdict::Supported, vec![neutral.id]),
            at(2),
        );
        assert!(matches!(result, Err(ClaimsError::Validation(_))));

        let support = catalog
            .add_evidence(claim.id, evidence(Stance::Supports), at(3))
            .expect("e");
        let judged = catalog
            .record_verdict(
                claim.id,
                verdict(Verdict::Supported, vec![support.id]),
                at(4),
            )
            .expect("verdict");
        assert_eq!(judged.current_verdict(), CurrentVerdict::Supported);
    }

    #[test]
    fn refuted_verdict_requires_refuting_evidence() {
        let mut catalog = Catalog::default();
        let claim = catalog.add_claim(new_claim("a"), at(0)).expect("add");
        let support = catalog
            .add_evidence(claim.id, evidence(Stance::Supports), at(1))
            .expect("e");
        let result =
            catalog.record_verdict(claim.id, verdict(Verdict::Refuted, vec![support.id]), at(2));
        assert!(matches!(result, Err(ClaimsError::Validation(_))));
    }

    #[test]
    fn verdict_rejects_evidence_from_another_claim() {
        let mut catalog = Catalog::default();
        let a = catalog.add_claim(new_claim("a"), at(0)).expect("a");
        let b = catalog.add_claim(new_claim("b"), at(0)).expect("b");
        let foreign = catalog
            .add_evidence(b.id, evidence(Stance::Supports), at(1))
            .expect("e");
        let result =
            catalog.record_verdict(a.id, verdict(Verdict::Supported, vec![foreign.id]), at(2));
        assert!(matches!(result, Err(ClaimsError::ForeignEvidence { .. })));
    }

    #[test]
    fn not_enough_info_needs_rationale_but_no_evidence() {
        let mut catalog = Catalog::default();
        let claim = catalog.add_claim(new_claim("a"), at(0)).expect("add");
        let mut blank = verdict(Verdict::NotEnoughInfo, Vec::new());
        blank.rationale = "  ".into();
        assert!(matches!(
            catalog.record_verdict(claim.id, blank, at(1)),
            Err(ClaimsError::Validation(_))
        ));
        let judged = catalog
            .record_verdict(claim.id, verdict(Verdict::NotEnoughInfo, Vec::new()), at(1))
            .expect("nei");
        assert_eq!(judged.current_verdict(), CurrentVerdict::NotEnoughInfo);
    }

    #[test]
    fn verdict_history_is_append_only() {
        let mut catalog = Catalog::default();
        let claim = catalog.add_claim(new_claim("a"), at(0)).expect("add");
        let refute = catalog
            .add_evidence(claim.id, evidence(Stance::Refutes), at(1))
            .expect("e");
        catalog
            .record_verdict(claim.id, verdict(Verdict::Refuted, vec![refute.id]), at(2))
            .expect("refuted");
        let judged = catalog
            .record_verdict(claim.id, verdict(Verdict::NotEnoughInfo, Vec::new()), at(3))
            .expect("nei");
        assert_eq!(judged.verdicts.len(), 2);
        assert_eq!(judged.verdicts[0].verdict, Verdict::Refuted);
    }

    #[test]
    fn list_applies_filters() {
        let mut catalog = Catalog::default();
        let mut tagged = new_claim("Parser rejects BIG files");
        tagged.tags = vec!["parser".into()];
        tagged.artifacts = vec!["src/parse.rs".into()];
        catalog.add_claim(tagged, at(0)).expect("tagged");
        let other = catalog.add_claim(new_claim("other"), at(0)).expect("other");
        catalog
            .retire_claim(other.id, "gone", at(1))
            .expect("retire");

        let by_text = ClaimFilter {
            text: Some("big".into()),
            ..ClaimFilter::default()
        };
        assert_eq!(catalog.list(&by_text).len(), 1);
        let by_artifact = ClaimFilter {
            artifact: Some("parse".into()),
            ..ClaimFilter::default()
        };
        assert_eq!(catalog.list(&by_artifact).len(), 1);
        let unverified = ClaimFilter {
            verdict: Some(CurrentVerdict::Unverified),
            ..ClaimFilter::default()
        };
        assert_eq!(catalog.list(&unverified).len(), 1);
        let all = ClaimFilter {
            include_retired: true,
            ..ClaimFilter::default()
        };
        assert_eq!(catalog.list(&all).len(), 2);
    }

    #[test]
    fn detail_lists_active_children() {
        let mut catalog = Catalog::default();
        let root = catalog.add_claim(new_claim("root"), at(0)).expect("root");
        let kept = catalog
            .add_claim(child_of("kept", ClaimRef::Existing(root.id)), at(0))
            .expect("kept");
        let dropped = catalog
            .add_claim(child_of("dropped", ClaimRef::Existing(root.id)), at(0))
            .expect("dropped");
        catalog
            .retire_claim(dropped.id, "gone", at(1))
            .expect("retire");

        let detail = catalog.detail(root.id).expect("detail");
        assert_eq!(detail.children, vec![kept.id]);
        assert_eq!(detail.current_verdict, CurrentVerdict::Unverified);
    }

    #[test]
    fn validate_detects_inconsistent_files() {
        let mut catalog = Catalog::default();
        let a = catalog.add_claim(new_claim("a"), at(0)).expect("a");
        catalog.add_claim(new_claim("b"), at(0)).expect("b");
        catalog
            .add_evidence(a.id, evidence(Stance::Supports), at(1))
            .expect("e");
        assert!(catalog.validate().is_ok());

        let mut stale_counter = catalog.clone();
        stale_counter.next_claim_seq = 2;
        assert!(stale_counter.validate().is_err());

        let mut unsorted = catalog.clone();
        unsorted.claims.swap(0, 1);
        assert!(unsorted.validate().is_err());

        let mut duplicate_evidence = catalog.clone();
        let copied = duplicate_evidence.claims[0].evidence[0].clone();
        duplicate_evidence.claims[1].evidence.push(copied);
        assert!(duplicate_evidence.validate().is_err());

        let mut missing_parent = catalog;
        missing_parent.claims[1].derived_from.push(ClaimId::new(99));
        assert!(missing_parent.validate().is_err());
    }

    #[test]
    fn exhausted_id_space_is_an_error_not_a_wrap() {
        let mut catalog = Catalog::default();
        let claim = catalog.add_claim(new_claim("a"), at(0)).expect("a");
        catalog.next_claim_seq = u32::MAX;
        assert!(matches!(
            catalog.add_claim(new_claim("b"), at(1)),
            Err(ClaimsError::Validation(_))
        ));
        catalog.next_evidence_seq = u32::MAX;
        assert!(matches!(
            catalog.add_evidence(claim.id, evidence(Stance::Neutral), at(1)),
            Err(ClaimsError::Validation(_))
        ));
    }

    #[test]
    fn batch_errors_name_the_failing_entry() {
        let mut catalog = Catalog::default();
        let error = catalog
            .add_claims(vec![new_claim("ok"), new_claim("  ")], at(0))
            .expect_err("blank statement in entry 1");
        assert!(error.to_string().contains("claims[1]"), "{error}");
    }

    #[test]
    fn update_keeps_existing_retired_parent_but_rejects_new_one() {
        let mut catalog = Catalog::default();
        let old_parent = catalog.add_claim(new_claim("old"), at(0)).expect("old");
        let other = catalog.add_claim(new_claim("other"), at(0)).expect("other");
        let leaf = catalog
            .add_claim(child_of("leaf", ClaimRef::Existing(old_parent.id)), at(0))
            .expect("leaf");
        catalog
            .retire_claim(old_parent.id, "superseded", at(1))
            .expect("retire old");
        catalog
            .retire_claim(other.id, "gone", at(1))
            .expect("retire other");

        let keep = ClaimUpdate {
            derived_from: Some(vec![old_parent.id]),
            ..ClaimUpdate::default()
        };
        assert!(catalog.update_claim(leaf.id, keep, at(2)).is_ok());
        let add_retired = ClaimUpdate {
            derived_from: Some(vec![old_parent.id, other.id]),
            ..ClaimUpdate::default()
        };
        assert!(matches!(
            catalog.update_claim(leaf.id, add_retired, at(2)),
            Err(ClaimsError::RetiredParent(id)) if id == other.id
        ));
    }
}
