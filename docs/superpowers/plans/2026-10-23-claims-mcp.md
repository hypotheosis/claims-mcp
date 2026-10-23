# claims-mcp Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a stdio MCP server on `rmcp` 3.7 that maintains a JSON claims catalog for requirements traceability, IEEE 1012 V&V, and FEVER-style auditing, plus a `claims-audit` skill that drives it.

**Architecture:** Pure in-memory `Catalog` logic (`catalog.rs`, `report.rs`) over serde model types (`model.rs`), wrapped by a `CatalogStore` (`store.rs`) that locks a sidecar file, re-reads the catalog, and saves atomically on every call. `server.rs` is a thin `rmcp` `#[tool_router]` layer mapping 10 tools onto store operations; `main.rs` serves it over stdio with logs on stderr.

**Tech Stack:** Rust 2021 (toolchain >= 1.89 for `File::lock`), `rmcp` 3.7 (`server`, `macros`, `transport-io`), `schemars` 1, `serde`/`serde_json`, `chrono`, `clap`, `thiserror`, `anyhow`, `tokio` (current-thread), `tracing`, `tempfile` (tests).

**Spec:** `docs/superpowers/specs/2026-10-23-claims-mcp-design.md`

**Verified:** Every code block in this plan was compiled, linted (`clippy -D warnings`), formatted, and tested (46 tests passing) in a scratch prototype before the plan was written, and the binary was smoke-tested over stdio.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `Cargo.toml` | Modify | Dependencies for the server; drop unused template crates |
| `.cargo/config.toml` | Delete | Only existed for `tokio_unstable` (console-subscriber) |
| `.gitignore` | Modify | Ignore catalog lock and temp files |
| `src/error.rs` | Create | `ClaimsError` (thiserror) and `Result` alias |
| `src/model.rs` | Create | Ids, enums, `Claim`, `Evidence`, `VerdictRecord`, `ClaimRef` |
| `src/catalog.rs` | Create | `Catalog` operations, input types, invariant checks (no I/O) |
| `src/report.rs` | Create | `audit_report` and `trace` (read-only analyses) |
| `src/store.rs` | Create | `CatalogStore`: lock, load, atomic save, async wrappers |
| `src/server.rs` | Create | `ClaimsServer`: rmcp tools, parameter structs, error mapping |
| `src/main.rs` | Rewrite | CLI, stderr tracing, stdio serve |
| `skills/claims-audit/SKILL.md` | Create | Auditing workflow skill |
| `.mcp.json` | Create | Registers the server for this repo |
| `README.md` | Create | Build, run, register, tools |
| `changelog/<UTC timestamp>.md` | Create | Required by CLAUDE.md |

Module dependencies (no cycles except the error/model id reference): `main` -> `server`, `store`; `server` -> `store`, `catalog`, `report`, `model`; `store` -> `catalog`; `catalog`, `report` -> `model`; `error` <-> `model` (error messages name ids; id parsing returns `ClaimsError`).

**Note on warnings during Tasks 2-5:** modules are added before `server.rs` uses them, so `cargo build` reports `dead_code` warnings until Task 6. That is expected; run the full `clippy -D warnings` gate only from Task 6 on.

---

### Task 1: Project scaffolding

**Files:**
- Modify: `Cargo.toml`
- Delete: `.cargo/config.toml`
- Modify: `.gitignore`
- Rewrite: `src/main.rs`

- [ ] **Step 1: Replace `Cargo.toml`**

```toml
[package]
name = "claims-mcp"
version = "0.1.0"
edition = "2021"
rust-version = "1.89"
authors = ["Peter Grace <pete.grace@gmail.com>"]
description = "Stdio MCP server maintaining a JSON claims catalog for traceability, V&V, and FEVER auditing"

[dependencies]
# Application-level error context in main.
anyhow = "1.0.104"
# RFC 3339 timestamps on claims, evidence, and verdicts.
chrono = { version = "0.4.45", default-features = false, features = ["serde", "clock", "std"] }
# Command-line parsing (--catalog / CLAIMS_CATALOG).
clap = { version = "4.6.7", features = ["derive", "env"] }
# Optional .env loading for CLAIMS_CATALOG / RUST_LOG.
dotenvy = "0.15.7"
# MCP server SDK: tool macros and stdio transport.
rmcp = { version = "3.7.0", features = ["server", "macros", "transport-io"] }
# JSON Schema generation for tool parameters (must match rmcp's schemars major version).
schemars = "1.2.3"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
# Domain error type.
thiserror = "2.0.6"
# Async runtime; current-thread flavor plus the blocking pool for file I/O.
tokio = { version = "1.54.1", features = ["rt", "macros"] }
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["fmt", "env-filter"] }

[dev-dependencies]
# Isolated temp directories for store and server tests.
tempfile = "3.27.0"
```

This removes `ctrlc`, `lazy_static`, `console-subscriber`, `tracing-log`, and `dotenv` (replaced by `dotenvy`).

- [ ] **Step 2: Delete the tokio_unstable config and update `.gitignore`**

```bash
git rm .cargo/config.toml
printf '%s\n' '/target' '.claims.json.lock' '.claims.json.tmp' > .gitignore
```

- [ ] **Step 3: Replace `src/main.rs` with a minimal stderr-logging stub**

```rust
//! `claims-mcp`: a stdio MCP server that maintains a JSON catalog of verifiable claims.

use tracing_subscriber::EnvFilter;

fn main() {
    let _ = dotenvy::dotenv();
    init_tracing();
}

/// Logs go to stderr because stdout carries the MCP protocol. Level comes from `RUST_LOG`
/// (default `warn`).
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}
```

- [ ] **Step 4: Build**

Run: `cargo build`
Expected: `Finished` with no errors. (If `Cargo.lock` resolves newer patch versions, that is fine.)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src/main.rs
git commit -m "chore: replace template deps with rmcp server stack

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Error type and data model

**Files:**
- Create: `src/error.rs`
- Create: `src/model.rs`
- Modify: `src/main.rs` (module declarations)
- Test: `src/model.rs` (`#[cfg(test)] mod tests`)

`error.rs` and `model.rs` reference each other (error messages name ids; id parsing returns `ClaimsError`), so they land together.

- [ ] **Step 1: Create `src/error.rs`**

`Encode` and `Join` variants are used by the store (Task 5); `Join` needs tokio's `JoinError`.

```rust
//! Error type shared by every layer of the claims catalog.

use std::path::PathBuf;

use crate::model::{ClaimId, EvidenceId};

/// Every failure the catalog, store, or report code can produce.
///
/// The server layer turns these into MCP tool errors (`is_error: true`) using the
/// `Display` text, so each message is written for an agent to read and act on.
#[derive(Debug, thiserror::Error)]
pub enum ClaimsError {
    /// The requested claim id does not exist in the catalog.
    #[error("claim {0} not found")]
    ClaimNotFound(ClaimId),

    /// A verdict cited evidence that is not attached to the claim being judged.
    #[error("evidence {evidence} is not attached to claim {claim}")]
    ForeignEvidence {
        /// Claim the verdict was recorded against.
        claim: ClaimId,
        /// Evidence id that was cited but not found on that claim.
        evidence: EvidenceId,
    },

    /// The operation is not allowed on a retired claim.
    #[error("claim {0} is retired")]
    Retired(ClaimId),

    /// Adding the requested `derived_from` link would make the hierarchy cyclic.
    #[error("linking {child} to parent {parent} would create a cycle")]
    Cycle {
        /// Claim whose `derived_from` list was being changed.
        child: ClaimId,
        /// Parent that is already a descendant of (or equal to) `child`.
        parent: ClaimId,
    },

    /// Input failed a validation rule; the message names the rule.
    #[error("invalid input: {0}")]
    Validation(String),

    /// Reading, writing, locking, or renaming a catalog file failed.
    #[error("catalog I/O error on {path}: {source}")]
    Io {
        /// File the operation was acting on.
        path: PathBuf,
        /// Underlying operating-system error.
        source: std::io::Error,
    },

    /// The catalog file exists but is not a valid catalog document.
    #[error("catalog {path} is not a valid claims catalog: {source}")]
    Parse {
        /// Catalog file that failed to parse.
        path: PathBuf,
        /// serde_json error, including line and column.
        source: serde_json::Error,
    },

    /// The catalog file declares a schema version this build does not understand.
    #[error("catalog {path} has schema_version {found}; this server supports {supported}")]
    UnsupportedSchema {
        /// Catalog file that was read.
        path: PathBuf,
        /// Version found in the file.
        found: u32,
        /// Version this build reads and writes.
        supported: u32,
    },

    /// Serializing the catalog or a tool response failed.
    #[error("failed to encode JSON: {0}")]
    Encode(#[from] serde_json::Error),

    /// The blocking worker thread running a store operation panicked or was cancelled.
    #[error("catalog worker task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, ClaimsError>;
```

- [ ] **Step 2: Write the failing model tests**

Add `mod error;` and `mod model;` to `src/main.rs` below the `//!` line, then create `src/model.rs` containing only:

```rust
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
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test model::`
Expected: compile errors (`cannot find type ClaimId`, `ClaimStatus`, ...).

- [ ] **Step 4: Write the model implementation**

Insert above the `#[cfg(test)]` line in `src/model.rs`:

```rust
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
                digits.parse::<u32>().map(Self).map_err(|_| invalid())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                // `Cow` borrows from the input when possible and only allocates if the JSON
                // string contained escapes.
                let text = Cow::<str>::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
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
    /// When it was recorded.
    pub recorded_at: DateTime<Utc>,
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

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().strip_prefix('@') {
            Some(index) => index.parse().map(Self::Batch).map_err(|_| {
                ClaimsError::Validation(format!("'{text}' is not a valid batch reference"))
            }),
            None => text.parse().map(Self::Existing),
        }
    }
}

impl<'de> Deserialize<'de> for ClaimRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test model::`
Expected: 8 tests PASS.

- [ ] **Step 6: Format and commit**

```bash
cargo fmt
git add src/
git commit -m "feat: add claims data model and error type

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Catalog operations and invariants

**Files:**
- Create: `src/catalog.rs`
- Modify: `src/main.rs` (add `mod catalog;`)
- Test: `src/catalog.rs` (`#[cfg(test)] mod tests`)

The test module is `pub(crate)` and exposes helpers (`at`, `new_claim`, `evidence`, `verdict`) reused by the report, store, and server tests.

Key rules implemented (spec 3.4): ids never reused; claims sorted by id with binary-search lookup; `add_claims` validates everything before inserting (atomic) and `@N` must point at an earlier entry, so adds can never form a cycle; `update_claim` checks cycles with a DFS; retired claims reject changes and cannot become new parents; `supported` / `refuted` must cite evidence with the matching stance.

- [ ] **Step 1: Write the failing tests**

Create `src/catalog.rs` containing only the test module below, and add `mod catalog;` to the top of `src/main.rs` (after the `//!` doc line):

```rust
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
            Err(ClaimsError::Retired(_))
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
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test catalog::`
Expected: compile errors (`cannot find type Catalog`, `NewClaim`, ...).

- [ ] **Step 3: Write the implementation**

Insert this above the `#[cfg(test)]` line in `src/catalog.rs`:

```rust
//! In-memory catalog operations and invariant checks. No I/O happens here.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{ClaimsError, Result};
use crate::model::{
    Claim, ClaimId, ClaimKind, ClaimLevel, ClaimRef, ClaimStatus, CurrentVerdict, Evidence,
    EvidenceId, EvidenceKind, IntegrityLevel, Stance, Verdict, VerdictRecord, VerificationMethod,
};

/// The only catalog `schema_version` this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    schema_version: u32,
    next_claim_seq: u32,
    next_evidence_seq: u32,
    claims: Vec<Claim>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            next_claim_seq: 1,
            next_evidence_seq: 1,
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
    /// [`ClaimsError::ClaimNotFound`] / [`ClaimsError::Retired`] for bad parents.
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
        // `add_claims` returns exactly one claim per input, so this pop always succeeds.
        created
            .pop()
            .ok_or_else(|| ClaimsError::Validation("no claim was created".to_owned()))
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
    /// references; [`ClaimsError::ClaimNotFound`] / [`ClaimsError::Retired`] for bad parents.
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
        let first_seq = self.next_claim_seq;
        let mut created = Vec::with_capacity(new_claims.len());
        for (index, new_claim) in new_claims.into_iter().enumerate() {
            let derived_from = new_claim
                .derived_from
                .iter()
                .map(|parent| self.resolve_ref(*parent, index, first_seq))
                .collect::<Result<Vec<_>>>()?;
            created.push(Claim {
                id: ClaimId::new(first_seq + index as u32),
                statement: required_text("statement", &new_claim.statement)?,
                kind: new_claim.kind,
                level: new_claim.level,
                integrity_level: new_claim.integrity_level,
                verification_method: new_claim.verification_method,
                source: optional_text(new_claim.source),
                derived_from: dedup_ids(derived_from),
                artifacts: clean_list(new_claim.artifacts),
                tags: clean_list(new_claim.tags),
                status: ClaimStatus::Active,
                evidence: Vec::new(),
                verdicts: Vec::new(),
                created_at: now,
                updated_at: now,
            });
        }
        self.next_claim_seq = first_seq + created.len() as u32;
        self.claims.extend(created.iter().cloned());
        Ok(created)
    }

    /// Applies metadata changes to an active claim.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::ClaimNotFound`], [`ClaimsError::Retired`], [`ClaimsError::Validation`]
    /// for an empty statement, and [`ClaimsError::Cycle`] if a new parent is a descendant.
    pub fn update_claim(
        &mut self,
        id: ClaimId,
        update: ClaimUpdate,
        now: DateTime<Utc>,
    ) -> Result<Claim> {
        self.active_index(id)?;
        let statement = update
            .statement
            .as_deref()
            .map(|text| required_text("statement", text))
            .transpose()?;
        let derived_from = match update.derived_from {
            Some(parents) => {
                let parents = dedup_ids(parents);
                for parent in &parents {
                    self.check_parent(*parent)?;
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

        let index = self.active_index(id)?;
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
        let evidence = Evidence {
            id: EvidenceId::new(self.next_evidence_seq),
            kind: new_evidence.kind,
            locator: required_text("locator", &new_evidence.locator)?,
            summary: required_text("summary", &new_evidence.summary)?,
            stance: new_evidence.stance,
            recorded_at: now,
        };
        self.next_evidence_seq += 1;
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
        });
        claim.updated_at = now;
        Ok(claim.clone())
    }

    fn index_of(&self, id: ClaimId) -> Option<usize> {
        self.claims.binary_search_by_key(&id, |claim| claim.id).ok()
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

    /// A new or changed `derived_from` entry must name an existing, active claim.
    fn check_parent(&self, parent: ClaimId) -> Result<()> {
        self.active_index(parent).map(|_| ())
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test catalog::`
Expected: all `catalog::` tests PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/
git commit -m "feat: add catalog operations with traceability and verdict invariants

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Audit report and trace

**Files:**
- Create: `src/report.rs`
- Modify: `src/main.rs` (add `mod report;`)
- Test: `src/report.rs` (`#[cfg(test)] mod tests`)

One pass over active claims plus one child-index build: O(n + e). `trace` is breadth-first so each claim appears once at its shortest depth, borrowing neighbour slices instead of cloning.

- [ ] **Step 1: Write the failing tests**

Create `src/report.rs` containing only the test module below, and add `mod report;` to the top of `src/main.rs` (after the `//!` doc line):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::{at, evidence, new_claim, verdict};
    use crate::catalog::NewClaim;
    use crate::model::{ClaimRef, IntegrityLevel, Stance, Verdict};

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
    fn trace_unknown_claim_errors() {
        let catalog = Catalog::default();
        assert!(trace(&catalog, ClaimId::new(1), TraceDirection::Both, None).is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test report::`
Expected: compile errors (`cannot find function audit_report`, `trace`, ...).

- [ ] **Step 3: Write the implementation**

Insert this above the `#[cfg(test)]` line in `src/report.rs`:

```rust
//! Read-only analyses over a catalog: the audit report and trace queries.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::error::Result;
use crate::model::{Claim, ClaimId, ClaimKind, ClaimLevel, CurrentVerdict, VerificationMethod};

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
    pub by_integrity_level: BTreeMap<String, usize>,
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
fn children_index(catalog: &Catalog) -> HashMap<ClaimId, Vec<ClaimId>> {
    let mut index: HashMap<ClaimId, Vec<ClaimId>> = HashMap::new();
    for claim in catalog.claims().iter().filter(|claim| claim.is_active()) {
        for parent in &claim.derived_from {
            index.entry(*parent).or_default().push(claim.id);
        }
    }
    index
}

/// Builds the audit report in a single pass over the claims plus one child-index build.
pub fn audit_report(catalog: &Catalog, include_matrix: bool) -> AuditReport {
    let children = children_index(catalog);
    let verdict_of: HashMap<ClaimId, CurrentVerdict> = catalog
        .claims()
        .iter()
        .map(|claim| (claim.id, claim.current_verdict()))
        .collect();
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
    let no_children = Vec::new();

    for claim in catalog.claims().iter().filter(|claim| claim.is_active()) {
        let verdict = claim.current_verdict();
        let claim_children = children.get(&claim.id).unwrap_or(&no_children);
        tally(&mut totals, claim, verdict);
        classify_gaps(
            &mut gaps,
            claim,
            verdict,
            claim_children,
            &verdict_of,
            &retired,
        );
        if let Some(rows) = matrix.as_mut() {
            rows.push(MatrixRow {
                id: claim.id,
                statement: claim.statement.clone(),
                derived_from: claim.derived_from.clone(),
                children: claim_children.clone(),
                verification_method: claim.verification_method,
                evidence_locators: claim.evidence.iter().map(|e| e.locator.clone()).collect(),
                current_verdict: verdict,
            });
        }
    }

    AuditReport {
        totals,
        gaps,
        matrix,
    }
}

fn tally(totals: &mut Totals, claim: &Claim, verdict: CurrentVerdict) {
    totals.active_claims += 1;
    *totals.by_verdict.entry(verdict).or_default() += 1;
    *totals.by_kind.entry(claim.kind).or_default() += 1;
    *totals.by_level.entry(claim.level).or_default() += 1;
    let integrity = claim
        .integrity_level
        .map_or_else(|| "unassigned".to_owned(), |level| level.get().to_string());
    *totals.by_integrity_level.entry(integrity).or_default() += 1;
}

fn classify_gaps(
    gaps: &mut Gaps,
    claim: &Claim,
    verdict: CurrentVerdict,
    children: &[ClaimId],
    verdict_of: &HashMap<ClaimId, CurrentVerdict>,
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
        gaps.rigor.push(claim.id);
    }
    if children
        .iter()
        .any(|child| verdict_of.get(child) != Some(&CurrentVerdict::Supported))
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
        TraceDirection::Up | TraceDirection::Both => {
            breadth_first(catalog, id, limit, |claim| &claim.derived_from)
        }
        TraceDirection::Down => Vec::new(),
    };
    let descendants = match direction {
        TraceDirection::Down | TraceDirection::Both => {
            let children = children_index(catalog);
            breadth_first(catalog, id, limit, |claim| {
                children.get(&claim.id).map_or(&[][..], Vec::as_slice)
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
/// Rust note: the `'c` lifetime says the neighbour slices borrow from data (the catalog or a
/// child index) that outlives the search, so no per-node `Vec` copies are needed.
fn breadth_first<'c>(
    catalog: &'c Catalog,
    start: ClaimId,
    limit: usize,
    neighbours: impl Fn(&'c Claim) -> &'c [ClaimId],
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
        for &next in neighbours(claim) {
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test report::`
Expected: all `report::` tests PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/
git commit -m "feat: add audit report gaps, traceability matrix, and trace

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Durable catalog store

**Files:**
- Create: `src/store.rs`
- Modify: `src/main.rs` (add `mod store;`)
- Test: `src/store.rs` (`#[cfg(test)] mod tests`)

Each call: exclusive `File::lock` on `.<name>.lock` (std, Rust >= 1.89) -> read (missing file = empty catalog) -> check `schema_version` -> run the closure -> on success write `.<name>.tmp`, `sync_all`, rename over the catalog, sync the directory (Unix). Async wrappers run the blocking work via `tokio::task::spawn_blocking`.

- [ ] **Step 1: Write the failing tests**

Create `src/store.rs` containing only the test module below, and add `mod store;` to the top of `src/main.rs` (after the `//!` doc line):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::{at, new_claim};

    fn temp_store() -> (tempfile::TempDir, CatalogStore) {
        let directory = tempfile::tempdir().expect("create temp dir");
        let store = CatalogStore::new(directory.path().join("claims.json"));
        (directory, store)
    }

    #[test]
    fn missing_file_reads_as_empty_catalog() {
        let (_directory, store) = temp_store();
        let count = store
            .read_blocking(|catalog| Ok(catalog.claims().len()))
            .expect("read");
        assert_eq!(count, 0);
        assert!(!store.path().exists());
    }

    #[test]
    fn mutate_persists_and_read_sees_it() {
        let (directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("persisted"), at(0)))
            .expect("mutate");

        let statement = store
            .read_blocking(|catalog| Ok(catalog.claims()[0].statement.clone()))
            .expect("read");
        assert_eq!(statement, "persisted");
        assert!(!directory.path().join(".claims.json.tmp").exists());
    }

    #[test]
    fn output_is_deterministic_and_ends_with_newline() {
        let (_directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("a"), at(0)))
            .expect("mutate");
        let first = fs::read(store.path()).expect("read file");
        store.mutate_blocking(|_| Ok(())).expect("rewrite");
        let second = fs::read(store.path()).expect("read file");

        assert_eq!(first, second);
        assert_eq!(first.last(), Some(&b'\n'));
    }

    #[test]
    fn failed_operation_leaves_file_untouched() {
        let (_directory, store) = temp_store();
        store
            .mutate_blocking(|catalog| catalog.add_claim(new_claim("a"), at(0)))
            .expect("mutate");
        let before = fs::read(store.path()).expect("read file");

        let result = store.mutate_blocking(|catalog| {
            catalog.add_claim(new_claim("b"), at(1))?;
            Err::<(), _>(ClaimsError::Validation("forced failure".into()))
        });

        assert!(result.is_err());
        assert_eq!(fs::read(store.path()).expect("read file"), before);
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let (_directory, store) = temp_store();
        let document =
            r#"{"schema_version":2,"next_claim_seq":1,"next_evidence_seq":1,"claims":[]}"#;
        fs::write(store.path(), document).expect("write file");
        let result = store.read_blocking(|_| Ok(()));
        assert!(matches!(
            result,
            Err(ClaimsError::UnsupportedSchema { found: 2, .. })
        ));
    }

    #[test]
    fn invalid_json_is_a_parse_error() {
        let (_directory, store) = temp_store();
        fs::write(store.path(), "{ not json").expect("write file");
        assert!(matches!(
            store.read_blocking(|_| Ok(())),
            Err(ClaimsError::Parse { .. })
        ));
    }

    #[test]
    fn relative_path_without_directory_uses_current_directory() {
        let store = CatalogStore::new("claims.json");
        assert_eq!(store.paths.lock, PathBuf::from("./.claims.json.lock"));
    }

    #[tokio::test]
    async fn async_wrappers_run_on_blocking_pool() {
        let (_directory, store) = temp_store();
        store
            .mutate(|catalog| catalog.add_claim(new_claim("async"), at(0)))
            .await
            .expect("mutate");
        let count = store
            .read(|catalog| Ok(catalog.claims().len()))
            .await
            .expect("read");
        assert_eq!(count, 1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test store::`
Expected: compile errors (`cannot find type CatalogStore`).

- [ ] **Step 3: Write the implementation**

Insert this above the `#[cfg(test)]` line in `src/store.rs`:

```rust
//! Durable storage of the catalog file: locking, loading, and atomic saving.

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::catalog::{Catalog, SCHEMA_VERSION};
use crate::error::{ClaimsError, Result};

/// Paths derived once from the catalog path.
#[derive(Debug)]
struct StorePaths {
    catalog: PathBuf,
    lock: PathBuf,
    temp: PathBuf,
    directory: PathBuf,
}

/// Handle to the catalog file. Cheap to clone (an `Arc` bump).
///
/// Every operation takes an exclusive lock on the sidecar `.<name>.lock` file, re-reads the
/// catalog from disk, and, for mutations, writes `.<name>.tmp` and renames it over the
/// catalog. The file is therefore always complete, and changes made outside the server
/// (git checkout, hand edits, another instance) are always seen.
#[derive(Debug, Clone)]
pub struct CatalogStore {
    paths: Arc<StorePaths>,
}

impl CatalogStore {
    /// Creates a store for the catalog at `catalog_path`. Nothing is touched on disk until the
    /// first operation; a missing file is treated as an empty catalog.
    pub fn new(catalog_path: impl Into<PathBuf>) -> Self {
        let catalog = catalog_path.into();
        let directory = match catalog.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let file_name = catalog
            .file_name()
            .map_or_else(|| "claims.json".into(), |name| name.to_string_lossy());
        let lock = directory.join(format!(".{file_name}.lock"));
        let temp = directory.join(format!(".{file_name}.tmp"));
        Self {
            paths: Arc::new(StorePaths {
                catalog,
                lock,
                temp,
                directory,
            }),
        }
    }

    /// Returns the catalog file path.
    pub fn path(&self) -> &Path {
        &self.paths.catalog
    }

    /// Runs `operation` against a fresh read of the catalog without writing.
    ///
    /// Blocking file I/O runs on tokio's blocking thread pool so the async reactor stays free.
    ///
    /// # Errors
    ///
    /// Lock, read, or parse failures, or whatever `operation` returns.
    pub async fn read<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Catalog) -> Result<T> + Send + 'static,
    {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.read_blocking(operation)).await?
    }

    /// Runs `operation` against a fresh read of the catalog and saves the result atomically.
    /// If `operation` fails, nothing is written.
    ///
    /// # Errors
    ///
    /// Lock, read, parse, or write failures, or whatever `operation` returns.
    pub async fn mutate<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Catalog) -> Result<T> + Send + 'static,
    {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.mutate_blocking(operation)).await?
    }

    /// Synchronous body of [`CatalogStore::read`].
    fn read_blocking<T>(&self, operation: impl FnOnce(&Catalog) -> Result<T>) -> Result<T> {
        let _lock = self.lock()?;
        operation(&self.load()?)
    }

    /// Synchronous body of [`CatalogStore::mutate`].
    fn mutate_blocking<T>(&self, operation: impl FnOnce(&mut Catalog) -> Result<T>) -> Result<T> {
        // Rust note: `_lock` keeps the file handle alive until the end of the function; the OS
        // lock is released automatically when it is dropped (RAII, like a `with` block).
        let _lock = self.lock()?;
        let mut catalog = self.load()?;
        let output = operation(&mut catalog)?;
        self.save(&catalog)?;
        Ok(output)
    }

    /// Opens (creating if needed) the sidecar lock file and takes an exclusive lock on it.
    fn lock(&self) -> Result<File> {
        let path = &self.paths.lock;
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|source| io_error(path, source))?;
        file.lock().map_err(|source| io_error(path, source))?;
        Ok(file)
    }

    /// Reads and parses the catalog; a missing file yields an empty catalog.
    fn load(&self) -> Result<Catalog> {
        let path = &self.paths.catalog;
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Catalog::default()),
            Err(source) => return Err(io_error(path, source)),
        };
        let catalog: Catalog =
            serde_json::from_slice(&bytes).map_err(|source| ClaimsError::Parse {
                path: path.clone(),
                source,
            })?;
        if catalog.schema_version() != SCHEMA_VERSION {
            return Err(ClaimsError::UnsupportedSchema {
                path: path.clone(),
                found: catalog.schema_version(),
                supported: SCHEMA_VERSION,
            });
        }
        Ok(catalog)
    }

    /// Writes the catalog to the temp file, flushes it to disk, and renames it into place.
    fn save(&self, catalog: &Catalog) -> Result<()> {
        let paths = &self.paths;
        let mut bytes = serde_json::to_vec_pretty(catalog)?;
        bytes.push(b'\n');

        let mut temp = File::create(&paths.temp).map_err(|source| io_error(&paths.temp, source))?;
        temp.write_all(&bytes)
            .and_then(|()| temp.sync_all())
            .map_err(|source| io_error(&paths.temp, source))?;
        fs::rename(&paths.temp, &paths.catalog)
            .map_err(|source| io_error(&paths.catalog, source))?;
        sync_directory(&paths.directory)
    }
}

/// Makes the rename itself durable by syncing the containing directory (Unix only; on other
/// platforms directories cannot be opened as files and the rename is already durable enough).
#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<()> {
    File::open(directory)
        .and_then(|handle| handle.sync_all())
        .map_err(|source| io_error(directory, source))
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<()> {
    Ok(())
}

fn io_error(path: &Path, source: std::io::Error) -> ClaimsError {
    ClaimsError::Io {
        path: path.to_path_buf(),
        source,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test store::`
Expected: all `store::` tests PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/
git commit -m "feat: add locked, atomic catalog store

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: rmcp tool server

**Files:**
- Create: `src/server.rs`
- Modify: `src/main.rs` (add `mod server;`)
- Test: `src/server.rs` (`#[cfg(test)] mod tests`)

Gotcha found while prototyping: `#[tool_handler]` expands to code using the bare two-parameter `Result`, so this module must **not** `use crate::error::Result`; it imports `crate::error` and writes `error::Result<T>` instead.

Domain errors are returned as `Ok(CallToolResult::error(..))` (`is_error: true`) so the agent sees the message; only JSON encoding failures become protocol errors. `#[serde(flatten)]` keeps `update_claim`, `add_evidence`, and `record_verdict` parameters as one flat JSON object (verified in the generated schemas).

- [ ] **Step 1: Write the failing tests**

Create `src/server.rs` containing only the test module below, and add `mod server;` to the top of `src/main.rs` (after the `//!` doc line):

```rust
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
    fn registers_all_ten_tools() {
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
                "record_verdict",
                "retire_claim",
                "trace",
                "update_claim",
            ]
        );
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
                include_matrix: None,
            }))
            .await
            .expect("audit_report");
        let report = json_of(&report);
        assert_eq!(report["totals"]["by_verdict"]["supported"], 1);
        assert_eq!(report["gaps"]["unverified"][0], "CLM-0001");
        assert_eq!(report["matrix"].as_array().map(Vec::len), Some(2));
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test server::`
Expected: compile errors (`cannot find type ClaimsServer`, `AddClaimsParams`, ...).

- [ ] **Step 3: Write the implementation**

Insert this above the `#[cfg(test)]` line in `src/server.rs`:

```rust
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
use crate::report::{self, TraceDirection};
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
    #[serde(default)]
    pub include_matrix: Option<bool>,
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
        description = "Update a claim's metadata. Omitted fields are unchanged; list fields \
        replace the whole list. Evidence and verdicts cannot be edited. To change what a \
        claim means after it has been judged, retire it and add a new one instead."
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
        description = "Audit report over active claims: totals by verdict, kind, level, and \
        integrity level; gaps (no_evidence, unverified, refuted, not_enough_info, stale, \
        rigor, decomposition, dangling_links); and a traceability matrix unless \
        include_matrix is false."
    )]
    async fn audit_report(&self, Parameters(params): Parameters<AuditReportParams>) -> ToolResult {
        let include_matrix = params.include_matrix.unwrap_or(true);
        respond(
            self.store
                .read(move |catalog| Ok(report::audit_report(catalog, include_matrix)))
                .await,
        )
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "claims-mcp",
    instructions = "Ledger of claims for requirements traceability, IEEE 1012 V&V, and \
        FEVER-style auditing. Register atomic claims, attach evidence you gathered yourself, \
        then record verdicts backed by that evidence. Use audit_report to find gaps."
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test server::`
Expected: all `server::` tests PASS.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt
git add src/
git commit -m "feat: expose catalog as ten rmcp MCP tools

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Wire `main.rs` and smoke-test over stdio

**Files:**
- Rewrite: `src/main.rs`

- [ ] **Step 1: Replace `src/main.rs`**

```rust
//! `claims-mcp`: a stdio MCP server that maintains a JSON catalog of verifiable claims.

mod catalog;
mod error;
mod model;
mod report;
mod server;
mod store;

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use crate::server::ClaimsServer;
use crate::store::CatalogStore;

/// Command-line options.
#[derive(Debug, Parser)]
#[command(
    version,
    about = "Stdio MCP server for a claims catalog (traceability, V&V, FEVER)"
)]
struct Cli {
    /// Path to the catalog JSON file; created on first write if missing.
    #[arg(long, env = "CLAIMS_CATALOG", default_value = "claims.json")]
    catalog: PathBuf,
}

/// Rust note: `current_thread` runs the async reactor on one thread. That is enough for a
/// stdio server; blocking file work is still sent to tokio's separate blocking pool.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    // A `.env` file is optional, so a missing one is not an error.
    let _ = dotenvy::dotenv();
    init_tracing();
    if let Err(error) = run(Cli::parse()).await {
        // `{:#}` prints the whole anyhow context chain on one line.
        tracing::error!("{error:#}");
        std::process::exit(1);
    }
}

/// Logs go to stderr because stdout carries the MCP protocol. Level comes from `RUST_LOG`
/// (default `warn`).
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}

/// Serves the claims tools over stdio until the client disconnects.
async fn run(cli: Cli) -> anyhow::Result<()> {
    let store = CatalogStore::new(cli.catalog);
    tracing::info!(catalog = %store.path().display(), "starting claims-mcp");
    let server = ClaimsServer::new(store);
    let service = server
        .serve(rmcp::transport::stdio_pipes())
        .await
        .context("failed to start the MCP stdio transport")?;
    service.waiting().await.context("MCP service task failed")?;
    Ok(())
}
```

- [ ] **Step 2: Full quality gate**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: no fmt diff, no clippy warnings, `test result: ok. 46 passed`.

- [ ] **Step 3: Stdio smoke test**

```bash
cargo build
SMOKE=$(mktemp -d)
{ printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"add_claim","arguments":{"statement":"x","kind":"functional","level":"unit","verification_method":"test"}}}'
  sleep 1; } | ./target/debug/claims-mcp --catalog "$SMOKE/claims.json" > "$SMOKE/out.jsonl"
python3 -c "
import json, sys
for line in open(sys.argv[1]):
    m = json.loads(line)
    if m.get('id') == 2: print(sorted(t['name'] for t in m['result']['tools']))
    if m.get('id') == 3: print(m['result']['content'][0]['text'][:40])
" "$SMOKE/out.jsonl"
head -5 "$SMOKE/claims.json"
rm -rf "$SMOKE"
```

Expected:
```text
['add_claim', 'add_claims', 'add_evidence', 'audit_report', 'get_claim', 'list_claims', 'record_verdict', 'retire_claim', 'trace', 'update_claim']
{"id":"CLM-0001","statement":"x","kind":"f
{
  "schema_version": 1,
  "next_claim_seq": 2,
  "next_evidence_seq": 1,
  "claims": [
```

Nothing but JSON-RPC lines may appear on stdout; any log line there means tracing is not on stderr.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: serve claims tools over stdio

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: claims-audit skill

**Files:**
- Create: `skills/claims-audit/SKILL.md`

- [ ] **Step 1: Write the skill**

````markdown
---
name: claims-audit
description: Audit whether an artifact does what it was supposed to do by recording atomic claims, independently gathered evidence, and FEVER verdicts (supported / refuted / not_enough_info) in the claims-mcp catalog. Use whenever the user asks to audit, verify, validate, or fact-check work against its instructions or spec; build or check requirements traceability; run IEEE 1012 style verification and validation or conformance checks; or confirm that an LLM or agent actually produced what it was told to produce. Requires the `claims` MCP server (claims-mcp).
---

# Claims Audit

Audit an artifact against the instruction or specification it was produced from. Every
conclusion must be traceable: claim -> evidence -> verdict, stored in the claims catalog
through the `claims` MCP tools.

**Core rule:** a verdict is only as good as the evidence you gathered *yourself*. Never
accept the producing agent's own summary, commit message, or "done" report as evidence.

## Tools

| Tool | Use it to |
|---|---|
| `add_claims` | Register a whole claim hierarchy in one call (`@N` links to earlier entries) |
| `add_claim` | Add a single claim discovered later |
| `list_claims` | Find claims by verdict, kind, level, tag, artifact, or text |
| `get_claim` | Read one claim with evidence, verdict history, and children |
| `update_claim` | Fix metadata (tags, artifacts, method, level) |
| `retire_claim` | Withdraw a claim that was wrong or is superseded |
| `add_evidence` | Record one finding with a precise locator and an honest stance |
| `record_verdict` | Append a FEVER verdict that cites the evidence it relies on |
| `trace` | Walk the hierarchy up (why does this exist?) or down (how is it verified?) |
| `audit_report` | Totals, gap lists, and the traceability matrix |

## Workflow

### 1. Scope

- Identify the **artifact** under audit (files, modules, a PR, generated output).
- Identify the **source** it was produced from: the prompt, task, ticket, or spec section.
  Quote or cite it; this becomes the `source` field on every claim.
- Call `list_claims` with a relevant `text` or `artifact` filter first. If claims already
  exist, continue from step 3 rather than registering duplicates.

### 2. Extract claims

Break the source into claims that are:

- **Atomic**: one assertion. If it contains "and", it is probably two claims.
- **Falsifiable**: there is an observation that would show it false.
- **Observable**: phrased as behavior or a property of the artifact, not as intent.

| Weak | Strong |
|---|---|
| "Handles errors well" | "parse_config returns ConfigError::TooLarge for files over 1 MiB" |
| "Fast and secure" | Split into a performance claim and a security claim with concrete bounds |
| "Uses the rmcp crate" | "Cargo.toml depends on rmcp and the server is built with rmcp's tool_router" |

Build the hierarchy from broad to narrow: `system` (the task as a whole) -> `software` ->
`component` -> `unit` -> `artifact`. Register it in **one** `add_claims` call, linking each
entry to its parent with `"derived_from": ["@0"]` (the 0-based index of an earlier entry).

Set on each claim:

- `kind`: functional, non_functional, interface, performance, safety, security,
  documentation, or constraint.
- `integrity_level` (IEEE 1012):

  | Level | Use when failure would cause |
  |---|---|
  | 4 | Safety harm, security breach, or data loss |
  | 3 | Loss of a core function the source explicitly asked for |
  | 2 | Degraded or secondary function |
  | 1 | Cosmetic, style, or documentation issues |

- `verification_method`, matched to the integrity level: `inspection` is acceptable at
  levels 1-2; levels 3-4 need `test`, `analysis`, `demonstration`, or `formal_proof`.
  The report flags violations as `rigor` gaps.
- `artifacts`: the paths the claim is about, so `list_claims` with `artifact` can find it.

### 3. Gather evidence

For each claim (start with the highest integrity level):

1. Inspect the artifact yourself: read the code, run the tests, run the program, check
   output. Use the verification method the claim names.
2. Record **every** relevant finding with `add_evidence`:
   - `locator`: precise and reproducible, such as `src/store.rs:88`,
     `cargo test store::tests::atomic_save`, or the exact command run.
   - `summary`: what you observed, not what you concluded.
   - `stance`: `supports`, `refutes`, or `neutral`. Record refuting and neutral findings
     too; leaving them out biases the audit.
3. One finding per evidence item. Do not merge a passing test and a code reading into one.

### 4. Adjudicate (FEVER)

Call `record_verdict` with `auditor` set to your model id or name:

- **supported**: at least one cited item directly shows the claim holds, and no
  unaddressed refuting evidence exists.
- **refuted**: at least one cited item directly shows the claim does not hold.
- **not_enough_info**: the evidence cannot decide it. This is a legitimate, honest
  outcome; use it instead of guessing.

The server enforces that `supported` cites supporting evidence and `refuted` cites refuting
evidence. Your `rationale` must explain *why* the cited evidence decides the claim.

Never infer support from the absence of failure ("no errors, so it works"). Absence of
evidence is `not_enough_info`.

### 5. Report

Call `audit_report` and summarize for the user:

- Totals by verdict, and the share of level 3-4 claims that are supported.
- Every non-empty gap list, with what each means and a concrete remediation:

  | Gap | Meaning | Remediation |
  |---|---|---|
  | `refuted` | The artifact does not do what was asked | Fix the artifact, then re-audit |
  | `not_enough_info` | Could not be decided | Add a test or analysis that decides it |
  | `unverified` / `no_evidence` | Not yet audited | Continue step 3 |
  | `stale` | Evidence arrived after the verdict | Re-adjudicate |
  | `rigor` | High-integrity claim checked only by inspection | Add a test or analysis |
  | `decomposition` | A parent's sub-claims are not all supported | Resolve the children first |
  | `dangling_links` | Parent was retired | Re-link or retire the child |

- Offer the traceability `matrix` as a table when the user needs a V&V record.

### 6. Re-audit after fixes

Focus on `refuted`, `not_enough_info`, and `stale` claims: gather new evidence, then record
a new verdict. History is append-only, so the earlier verdict remains as the audit trail.

## Anti-patterns

- **Self-certification**: using the producer's claims ("I implemented X") as evidence.
- **Compound claims**: one claim covering several behaviors, which hides partial failure.
- **Moving the goalposts**: editing a judged claim's statement until it passes. Retire it
  with a reason and add a corrected claim instead.
- **Silent refutations**: finding a problem and not recording it as `refutes` evidence.
- **Ignoring rigor gaps** on integrity level 3-4 claims.
- **Batch rubber-stamping**: recording `supported` for many claims after one shallow check.
````

- [ ] **Step 2: Check the tool names match the server**

```bash
for tool in add_claim add_claims get_claim list_claims update_claim retire_claim \
    add_evidence record_verdict trace audit_report; do
    grep -q "\`$tool\`" skills/claims-audit/SKILL.md || echo "missing: $tool"
done
```

Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add skills/
git commit -m "docs: add claims-audit skill

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Registration, README, changelog, final verification

**Files:**
- Create: `.mcp.json`
- Create: `README.md`
- Create: `changelog/<UTC timestamp>.md`

- [ ] **Step 1: Create `.mcp.json`**

```json
{
  "mcpServers": {
    "claims": {
      "command": "cargo",
      "args": ["run", "--release", "--quiet", "--", "--catalog", "claims.json"]
    }
  }
}
```

- [ ] **Step 2: Create `README.md`**

````markdown
# claims-mcp

A stdio [MCP](https://modelcontextprotocol.io) server, built on the
[`rmcp`](https://crates.io/crates/rmcp) crate, that maintains a catalog of **claims** in a single
JSON file you commit alongside your code.

A claim is one atomic, falsifiable assertion about an artifact or requirement. Each claim
carries evidence and an append-only history of FEVER verdicts (`supported`, `refuted`,
`not_enough_info`). Together they give a durable record for:

- **Requirements traceability**: claims link to parent claims (`derived_from`), from system
  level down to individual artifacts.
- **Verification and validation** (IEEE 1012): integrity levels 1-4 and verification methods
  (inspection, analysis, demonstration, test, formal proof).
- **Conformance and LLM output auditing**: check that a produced artifact matches the
  instruction it was produced from.

The server only records; it never runs commands. Verification is done by the agent, guided by
the bundled [`claims-audit`](skills/claims-audit/SKILL.md) skill.

## Build

```bash
cargo build --release
```

## Run

```bash
claims-mcp --catalog path/to/claims.json   # or set CLAIMS_CATALOG
```

| Option | Env | Default | Meaning |
|---|---|---|---|
| `--catalog` | `CLAIMS_CATALOG` | `claims.json` | Catalog file; created on first write |
| | `RUST_LOG` | `warn` | Log filter; logs go to stderr |

The catalog is re-read on every call and written atomically (temp file, fsync, rename) under an
exclusive lock on `.<name>.lock`. Git checkouts, hand edits, and several server instances
sharing one file are all safe. Add `.<name>.lock` and `.<name>.tmp` to `.gitignore`.

## Register with Claude Code

This repository's `.mcp.json` registers the server as `claims`, backed by `claims.json` in the
repository root. For another project, add:

```json
{
  "mcpServers": {
    "claims": {
      "command": "/absolute/path/to/claims-mcp/target/release/claims-mcp",
      "args": ["--catalog", "claims.json"]
    }
  }
}
```

## Install the skill

```bash
mkdir -p ~/.claude/skills
cp -r skills/claims-audit ~/.claude/skills/
```

Then ask Claude to "audit this change against the task", or invoke `/claims-audit`.

## Tools

| Tool | Purpose |
|---|---|
| `add_claim` | Create one claim |
| `add_claims` | Create many claims atomically; `@N` in `derived_from` links to an earlier entry |
| `get_claim` | One claim with evidence, verdict history, current verdict, and children |
| `list_claims` | Filtered summaries (verdict, kind, level, tag, artifact, text, include_retired) |
| `update_claim` | Edit metadata; evidence and verdicts cannot be changed |
| `retire_claim` | Withdraw a claim; it stays so trace links remain valid |
| `add_evidence` | Attach evidence with a locator and a stance (supports / refutes / neutral) |
| `record_verdict` | Append a FEVER verdict; it must cite evidence that backs it |
| `trace` | Ancestors and descendants of a claim, optionally depth-limited |
| `audit_report` | Totals, gap lists, and the traceability matrix |

## Catalog format

See [the design spec](docs/superpowers/specs/2026-10-23-claims-mcp-design.md), section 3,
for the full schema. Ids look like `CLM-0007` and `EVD-0014` and are never reused.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
````

- [ ] **Step 3: Create the changelog entry**

Run `date -u +%Y-%m-%dT%H-%M-%SZ` and create `changelog/<that value>.md` with:

````markdown
# claims-mcp 0.1.0: initial MCP server and claims-audit skill

## Added
- Stdio MCP server on `rmcp` 3.7 with 10 tools: `add_claim`, `add_claims`, `get_claim`,
  `list_claims`, `update_claim`, `retire_claim`, `add_evidence`, `record_verdict`, `trace`,
  `audit_report`.
- Versioned JSON catalog (`schema_version: 1`) with claims, evidence, and append-only FEVER
  verdict history; deterministic output for clean diffs.
- Invariants: no reused ids, acyclic `derived_from`, retired claims frozen, verdicts must cite
  evidence with a matching stance.
- Audit report: totals, gap lists (no_evidence, unverified, refuted, not_enough_info, stale,
  rigor, decomposition, dangling_links), and a traceability matrix.
- Store: exclusive lock file, re-read per call, atomic temp-file + fsync + rename saves.
- `skills/claims-audit/SKILL.md`, `.mcp.json`, and `README.md`.

## Changed
- Logging now goes to stderr only (stdout is the MCP channel).
- Replaced `dotenv` with `dotenvy`.

## Removed
- `console-subscriber` and the `tokio_unstable` rustflag (it bound TCP 6669, which conflicts
  between server instances), plus unused `ctrlc`, `lazy_static`, and `tracing-log`.

## Design
- Spec: `docs/superpowers/specs/2026-10-23-claims-mcp-design.md`
- Plan: `docs/superpowers/plans/2026-10-23-claims-mcp.md`
````

- [ ] **Step 4: Final verification**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo build --release`
Expected: all clean, `46 passed`, release binary at `target/release/claims-mcp`.

Also confirm: `grep -rn '\.unwrap()' src/` prints nothing (tests use `.expect(..)`), and `git status` shows no stray `.claims.json.lock` / `.claims.json.tmp` staged.

- [ ] **Step 5: Commit**

```bash
git add .mcp.json README.md changelog/
git commit -m "docs: add README, MCP registration, and changelog

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
