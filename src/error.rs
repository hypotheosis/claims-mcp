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

    /// A `derived_from` link would point at a retired claim.
    #[error("cannot link to retired claim {0}")]
    RetiredParent(ClaimId),

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
