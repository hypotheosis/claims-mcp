//! Data model persisted in the catalog file.
//!
//! Every type here serializes to the exact JSON shape documented in
//! `docs/superpowers/specs/2026-10-23-claims-mcp-design.md` section 3.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::ClaimsError;

/// Parses a non-empty run of ASCII digits.
///
/// Rust note: `str::parse::<u32>` also accepts a leading `+`, which the published JSON schema
/// patterns reject, so the digits are checked explicitly first.
fn parse_digits<T: FromStr>(digits: &str) -> Option<T> {
    let all_digits = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    all_digits.then(|| digits.parse().ok()).flatten()
}

/// Deserializes any `FromStr` type from a JSON string, reporting parse errors through serde.
fn deserialize_parsed<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let text = String::deserialize(deserializer)?;
    text.parse().map_err(serde::de::Error::custom)
}

/// Generates a numeric id newtype that is written as `PREFIX-0007`.
///
/// Rust note: `macro_rules!` is compile-time code generation (closer to a template than a
/// Python decorator). It keeps `ClaimId` and `EvidenceId` identical without copy-paste.
macro_rules! prefixed_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal, $pattern:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            /// Wraps a raw sequence number.
            pub fn new(seq: u32) -> Self {
                Self(seq)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                // `{:04}` pads to at least four digits and grows naturally past 9999.
                write!(formatter, "{}-{:04}", $prefix, self.0)
            }
        }

        impl FromStr for $name {
            type Err = ClaimsError;

            /// Parses `PREFIX-n` with the prefix in any letter case.
            ///
            /// # Errors
            ///
            /// [`ClaimsError::Validation`] if the prefix is wrong or `n` is not plain digits
            /// that fit in a `u32`.
            fn from_str(text: &str) -> Result<Self, Self::Err> {
                let invalid = || {
                    ClaimsError::Validation(format!(
                        "'{text}' is not a valid id; expected {}-<number>",
                        $prefix
                    ))
                };
                let (prefix, digits) = text.trim().split_once('-').ok_or_else(invalid)?;
                if !prefix.eq_ignore_ascii_case($prefix) {
                    return Err(invalid());
                }
                parse_digits(digits).map(Self).ok_or_else(invalid)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                deserialize_parsed(deserializer)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn inline_schema() -> bool {
                true
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", "pattern": $pattern })
            }
        }
    };
}

prefixed_id!(
    /// Identifier of a claim, written as `CLM-0007`. Ordering is numeric.
    ClaimId,
    "CLM",
    "^[Cc][Ll][Mm]-[0-9]+$"
);

prefixed_id!(
    /// Identifier of an evidence item, written as `EVD-0014`. Unique across the catalog.
    EvidenceId,
    "EVD",
    "^[Ee][Vv][Dd]-[0-9]+$"
);

/// What sort of property a claim asserts.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    /// Observable behavior or output.
    Functional,
    /// Quality attribute such as usability or maintainability.
    NonFunctional,
    /// Contract with another component, API, or user.
    Interface,
    /// Latency, throughput, or resource bound.
    Performance,
    /// Protection against harm to people or property.
    Safety,
    /// Confidentiality, integrity, or availability property.
    Security,
    /// Statement about documentation content.
    Documentation,
    /// Imposed limitation such as a language, license, or dependency rule.
    Constraint,
}

/// Position of a claim in the requirements hierarchy, from broadest to narrowest.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ClaimLevel {
    /// Whole-system requirement or top-level task instruction.
    System,
    /// Software-level requirement.
    Software,
    /// Requirement on one component or module.
    Component,
    /// Requirement on one function or type.
    Unit,
    /// Assertion about a concrete produced artifact.
    Artifact,
}

/// How a claim is meant to be verified (IEEE 1012 / INCOSE methods plus formal proof).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMethod {
    /// Reading code, documents, or output.
    Inspection,
    /// Reasoning, calculation, or static analysis.
    Analysis,
    /// Running the system and observing behavior without instrumentation.
    Demonstration,
    /// Executing a defined test with pass/fail criteria.
    Test,
    /// Machine-checked or rigorous mathematical proof.
    FormalProof,
}

/// The form a piece of evidence takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// Result of an executed test.
    Test,
    /// Observation from reading an artifact.
    Inspection,
    /// Result of reasoning or static analysis.
    Analysis,
    /// Observation from running the system.
    Demonstration,
    /// Proof artifact or proof-checker output.
    Proof,
    /// Captured log or command output.
    Log,
    /// Anything else.
    Other,
}

/// Whether a piece of evidence supports or refutes its claim (FEVER terminology).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    /// The evidence shows the claim holds.
    Supports,
    /// The evidence shows the claim does not hold.
    Refutes,
    /// The evidence is relevant but decides nothing.
    Neutral,
}

/// A FEVER verdict an auditor can record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Cited evidence shows the claim holds.
    Supported,
    /// Cited evidence shows the claim does not hold.
    Refuted,
    /// The available evidence cannot decide the claim.
    NotEnoughInfo,
}

/// The verdict a claim currently carries: its latest recorded verdict, or `Unverified`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CurrentVerdict {
    /// No verdict has been recorded yet.
    Unverified,
    /// Latest verdict is `supported`.
    Supported,
    /// Latest verdict is `refuted`.
    Refuted,
    /// Latest verdict is `not_enough_info`.
    NotEnoughInfo,
}

impl From<Verdict> for CurrentVerdict {
    fn from(verdict: Verdict) -> Self {
        match verdict {
            Verdict::Supported => Self::Supported,
            Verdict::Refuted => Self::Refuted,
            Verdict::NotEnoughInfo => Self::NotEnoughInfo,
        }
    }
}

/// IEEE 1012 software integrity level, 1 (lowest consequence) to 4 (highest).
///
/// Rust note: `try_from = "u8"` makes serde reject out-of-range numbers during parsing, so an
/// invalid level can never exist in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct IntegrityLevel(u8);

impl IntegrityLevel {
    /// Lowest valid level.
    pub const MIN: u8 = 1;
    /// Highest valid level.
    pub const MAX: u8 = 4;

    /// Returns the numeric level.
    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for IntegrityLevel {
    type Error = ClaimsError;

    /// # Errors
    ///
    /// Returns [`ClaimsError::Validation`] when `level` is outside 1..=4.
    fn try_from(level: u8) -> Result<Self, Self::Error> {
        if (Self::MIN..=Self::MAX).contains(&level) {
            Ok(Self(level))
        } else {
            Err(ClaimsError::Validation(format!(
                "integrity_level must be between {} and {}, got {level}",
                Self::MIN,
                Self::MAX
            )))
        }
    }
}

impl From<IntegrityLevel> for u8 {
    fn from(level: IntegrityLevel) -> Self {
        level.0
    }
}

impl JsonSchema for IntegrityLevel {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("IntegrityLevel")
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": IntegrityLevel::MIN,
            "maximum": IntegrityLevel::MAX,
            "description": "IEEE 1012 integrity level: 4 safety/security/data loss, \
                            3 core function, 2 degraded function, 1 cosmetic or docs"
        })
    }
}

/// Lifecycle state of a claim. Claims are retired instead of deleted.
///
/// Rust note: `tag = "state"` writes the variant name into a `state` field, giving
/// `{"state":"active"}` or `{"state":"retired","reason":...,"retired_at":...}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClaimStatus {
    /// The claim is in force.
    Active,
    /// The claim was withdrawn; it stays in the file so trace links remain valid.
    Retired {
        /// Why the claim was retired.
        reason: String,
        /// When it was retired.
        retired_at: DateTime<Utc>,
    },
}

/// Who produced a [`Seal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealOrigin {
    /// The server sealed the record when a tool created or changed it.
    Server,
    /// `claims-mcp seal --adopt` sealed a record that had no seal (catalog migration).
    Adopt,
}

/// Integrity seal on a catalog record: an HMAC over the record's content and its position in
/// the catalog (see `src/seal.rs`). The origin is covered by the MAC, so relabeling an adopted
/// record as server-sealed invalidates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seal {
    /// Who sealed the record.
    pub by: SealOrigin,
    /// Lower-case hex HMAC-SHA256.
    pub mac: String,
}

/// An inclusive, 1-based range of lines in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LineRange {
    /// First line, counting from 1.
    pub start: u32,
    /// Last line, at least `start`.
    pub end: u32,
}

impl LineRange {
    /// Returns `true` if the two ranges share at least one line.
    pub fn overlaps(self, other: LineRange) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

impl fmt::Display for LineRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(formatter, "{}", self.start)
        } else {
            write!(formatter, "{}-{}", self.start, self.end)
        }
    }
}

/// A file, or lines in it, at the commit that was checked out when the anchor was recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAnchor {
    /// Path relative to the repository root.
    pub path: String,
    /// The anchored lines; the whole file when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<LineRange>,
    /// Full commit id of `HEAD` when the anchor was recorded.
    pub commit: String,
    /// `true` if the file had uncommitted changes then, so `commit` may not show what was seen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// A `git grep -E` search and how many lines it matched at the pinned commit. With zero hits
/// it shows that something does not exist, such as a feature a document promises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchAnchor {
    /// POSIX extended regular expression, as `git grep -E` reads it.
    pub pattern: String,
    /// Whether the search ignores case (`git grep -i`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub case_insensitive: bool,
    /// Paths searched, relative to the repository root (`.` is the whole repository).
    pub scope: Vec<String>,
    /// Matching lines at `commit`, counted by the server.
    pub hits: u32,
    /// Full commit id of `HEAD` when the search was recorded.
    pub commit: String,
    /// `true` if a scope path had uncommitted changes then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// Machine-checkable location behind an evidence item, pinned to a commit so it can be
/// rechecked later (`claims-mcp recheck`).
///
/// Rust note: serde's default enum form is "externally tagged": `{"file": {...}}` or
/// `{"search": {...}}`, like a one-key dict whose key names the variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// A file or line range.
    File(FileAnchor),
    /// A search; with zero hits, a demonstrated absence.
    Search(SearchAnchor),
}

impl Anchor {
    /// Whether the anchored content had uncommitted changes when it was recorded.
    pub fn dirty(&self) -> bool {
        match self {
            Self::File(anchor) => anchor.dirty,
            Self::Search(anchor) => anchor.dirty,
        }
    }
}

impl fmt::Display for FileAnchor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.path)?;
        if let Some(lines) = self.lines {
            write!(formatter, ":{lines}")?;
        }
        write!(formatter, " @ {}", short_commit(&self.commit))
    }
}

impl fmt::Display for SearchAnchor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let flags = if self.case_insensitive { "-iE" } else { "-E" };
        write!(
            formatter,
            "git grep {flags} '{}' -- {} @ {} ({} hits)",
            self.pattern,
            self.scope.join(" "),
            short_commit(&self.commit),
            self.hits
        )
    }
}

impl fmt::Display for Anchor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(anchor) => anchor.fmt(formatter),
            Self::Search(anchor) => anchor.fmt(formatter),
        }
    }
}

/// `1 line` or `N lines`.
pub fn line_count_text(count: u32) -> String {
    if count == 1 {
        "1 line".to_owned()
    } else {
        format!("{count} lines")
    }
}

/// The first 12 characters of a commit id, the usual abbreviated form.
pub fn short_commit(commit: &str) -> &str {
    // Commit ids are ASCII hex, so any byte index is a char boundary.
    commit.get(..12).unwrap_or(commit)
}

/// One piece of evidence attached to a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// Catalog-wide unique id.
    pub id: EvidenceId,
    /// Form of the evidence.
    pub kind: EvidenceKind,
    /// Where to find it: `path:line`, test name, command, or URL.
    pub locator: String,
    /// What the evidence shows.
    pub summary: String,
    /// Whether it supports or refutes the claim.
    pub stance: Stance,
    /// Commit-pinned location that `recheck` can verify; absent for free-text-only evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// When it was recorded.
    pub recorded_at: DateTime<Utc>,
    /// Integrity seal; `None` only in unsealed (schema v1) catalogs or after tampering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<Seal>,
}

/// One entry in a claim's append-only verdict history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictRecord {
    /// The verdict reached.
    pub verdict: Verdict,
    /// Why the auditor reached it.
    pub rationale: String,
    /// Who or what recorded it (a person or a model id).
    pub auditor: String,
    /// Evidence on the same claim that the verdict relies on.
    pub evidence_ids: Vec<EvidenceId>,
    /// When it was recorded.
    pub recorded_at: DateTime<Utc>,
    /// Integrity seal; also chains to the previous verdict's seal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<Seal>,
}

/// A single atomic, falsifiable assertion and everything recorded about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// Unique, never-reused id.
    pub id: ClaimId,
    /// The assertion itself.
    pub statement: String,
    /// What sort of property it asserts.
    pub kind: ClaimKind,
    /// Position in the requirements hierarchy.
    pub level: ClaimLevel,
    /// IEEE 1012 integrity level, if assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity_level: Option<IntegrityLevel>,
    /// Intended verification method.
    pub verification_method: VerificationMethod,
    /// Where the claim came from (prompt, spec section, ticket).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Commit-pinned location of the text that makes the claim, such as a README line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_anchor: Option<FileAnchor>,
    /// Parent claims this one refines (upstream trace links).
    #[serde(default)]
    pub derived_from: Vec<ClaimId>,
    /// Artifacts the claim is about (paths, modules, URLs).
    #[serde(default)]
    pub artifacts: Vec<String>,
    /// Free-form labels for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Active or retired.
    pub status: ClaimStatus,
    /// Evidence gathered so far.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// Append-only verdict history; the last entry is current.
    #[serde(default)]
    pub verdicts: Vec<VerdictRecord>,
    /// When the claim was created.
    pub created_at: DateTime<Utc>,
    /// When the claim last changed in any way.
    pub updated_at: DateTime<Utc>,
    /// Integrity seal over the claim's own fields, its evidence ids, and its latest verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<Seal>,
}

impl Claim {
    /// Returns the latest verdict, or [`CurrentVerdict::Unverified`] if none was recorded.
    pub fn current_verdict(&self) -> CurrentVerdict {
        self.verdicts
            .last()
            .map_or(CurrentVerdict::Unverified, |record| record.verdict.into())
    }

    /// Returns `true` unless the claim has been retired.
    pub fn is_active(&self) -> bool {
        matches!(self.status, ClaimStatus::Active)
    }
}

/// A `derived_from` entry in a new claim: an existing id or `@N`, a reference to entry `N`
/// of the same `add_claims` batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimRef {
    /// A claim already in the catalog.
    Existing(ClaimId),
    /// The 0-based index of an earlier entry in the same batch.
    Batch(usize),
}

impl FromStr for ClaimRef {
    type Err = ClaimsError;

    /// Parses `@N` as a batch reference or anything else as a [`ClaimId`].
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Validation`] if neither form parses.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().strip_prefix('@') {
            Some(index) => parse_digits(index).map(Self::Batch).ok_or_else(|| {
                ClaimsError::Validation(format!("'{text}' is not a valid batch reference"))
            }),
            None => text.parse().map(Self::Existing),
        }
    }
}

impl<'de> Deserialize<'de> for ClaimRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_parsed(deserializer)
    }
}

impl JsonSchema for ClaimRef {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ClaimRef")
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^([Cc][Ll][Mm]-[0-9]+|@[0-9]+)$",
            "description": "Existing claim id (CLM-0001) or, in add_claims only, @N for \
                            the earlier batch entry at 0-based index N"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_id_displays_with_padding_and_grows() {
        assert_eq!(ClaimId::new(7).to_string(), "CLM-0007");
        assert_eq!(ClaimId::new(12345).to_string(), "CLM-12345");
    }

    #[test]
    fn claim_id_parses_any_case_and_rejects_garbage() {
        assert_eq!("clm-7".parse::<ClaimId>().ok(), Some(ClaimId::new(7)));
        assert_eq!(" CLM-0042 ".parse::<ClaimId>().ok(), Some(ClaimId::new(42)));
        assert!("EVD-1".parse::<ClaimId>().is_err());
        assert!("CLM-".parse::<ClaimId>().is_err());
        assert!("CLM7".parse::<ClaimId>().is_err());
    }

    #[test]
    fn ids_order_numerically() {
        assert!(ClaimId::new(9999) < ClaimId::new(10000));
    }

    #[test]
    fn evidence_id_round_trips_through_json() {
        let id = EvidenceId::new(14);
        let json = serde_json::to_string(&id).expect("serialize id");
        assert_eq!(json, "\"EVD-0014\"");
        let back: EvidenceId = serde_json::from_str(&json).expect("deserialize id");
        assert_eq!(back, id);
    }

    #[test]
    fn integrity_level_rejects_out_of_range() {
        assert!(serde_json::from_str::<IntegrityLevel>("0").is_err());
        assert!(serde_json::from_str::<IntegrityLevel>("5").is_err());
        let level: IntegrityLevel = serde_json::from_str("4").expect("level 4 is valid");
        assert_eq!(level.get(), 4);
    }

    #[test]
    fn enums_use_snake_case_names() {
        let json = serde_json::to_string(&VerificationMethod::FormalProof).expect("serialize");
        assert_eq!(json, "\"formal_proof\"");
        let json = serde_json::to_string(&CurrentVerdict::NotEnoughInfo).expect("serialize");
        assert_eq!(json, "\"not_enough_info\"");
    }

    #[test]
    fn status_serializes_with_state_tag() {
        let json = serde_json::to_string(&ClaimStatus::Active).expect("serialize");
        assert_eq!(json, r#"{"state":"active"}"#);
    }

    #[test]
    fn claim_ref_parses_ids_and_batch_indexes() {
        assert_eq!(
            "CLM-3".parse::<ClaimRef>().ok(),
            Some(ClaimRef::Existing(ClaimId::new(3)))
        );
        assert_eq!("@2".parse::<ClaimRef>().ok(), Some(ClaimRef::Batch(2)));
        assert!("@x".parse::<ClaimRef>().is_err());
    }

    #[test]
    fn ids_reject_signs_and_overflow() {
        assert!("CLM-+7".parse::<ClaimId>().is_err());
        assert!("CLM--1".parse::<ClaimId>().is_err());
        assert!("CLM-99999999999".parse::<ClaimId>().is_err());
        assert!("@+2".parse::<ClaimRef>().is_err());
    }

    #[test]
    fn claim_round_trips_spec_example() {
        let example = serde_json::json!({
            "id": "CLM-0007",
            "statement": "parse_config rejects files larger than 1 MiB",
            "kind": "functional",
            "level": "software",
            "integrity_level": 3,
            "verification_method": "test",
            "source": "prompt: task-42 / spec.md#limits",
            "derived_from": ["CLM-0002"],
            "artifacts": ["src/config.rs"],
            "tags": ["config"],
            "status": { "state": "active" },
            "evidence": [{
                "id": "EVD-0014",
                "kind": "test",
                "locator": "tests/config.rs:88 oversized_file_rejected",
                "summary": "test passes; returns ConfigError::TooLarge",
                "stance": "supports",
                "recorded_at": "2026-10-23T14:02:11Z"
            }],
            "verdicts": [{
                "verdict": "supported",
                "rationale": "Direct test covers the boundary and passes.",
                "auditor": "claude-opus-5-5",
                "evidence_ids": ["EVD-0014"],
                "recorded_at": "2026-10-23T14:03:40Z"
            }],
            "created_at": "2026-10-23T13:58:00Z",
            "updated_at": "2026-10-23T14:03:40Z"
        });
        let claim: Claim = serde_json::from_value(example.clone()).expect("spec example parses");
        assert_eq!(claim.current_verdict(), CurrentVerdict::Supported);
        assert!(claim.is_active());
        assert_eq!(serde_json::to_value(&claim).expect("serialize"), example);
    }
}
