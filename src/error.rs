//! Error type shared by every layer of the claims catalog.

use std::path::PathBuf;

use crate::model::{ClaimId, EvidenceId};
use crate::seal::IntegrityProblem;

/// At most this many failed checks are listed in an error message; the rest are counted.
const LISTED_PROBLEMS: usize = 10;

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

    /// The catalog failed integrity verification, so the server refuses every write.
    #[error(
        "catalog {path} was changed outside claims-mcp, so every write is refused until it is \
         restored. Failed checks: {}. Never edit the catalog file directly. To recover, restore \
         it from git (`git checkout -- {path}`, or revert the commit that edited it), then \
         redo the intended change through the claims tools. Reads and audit_report still work.",
        list_problems(.problems)
    )]
    CatalogTampered {
        /// Catalog file that failed verification.
        path: PathBuf,
        /// Every failed check.
        problems: Vec<IntegrityProblem>,
    },

    /// The catalog uses the unsealed schema version 1, so the server refuses every write until
    /// a maintainer migrates it.
    #[error(
        "catalog {path} uses schema_version 1, which has no integrity seals, so every write is \
         refused. A maintainer (not an agent) must migrate it once with `claims-mcp --catalog \
         {path} seal --adopt` and commit the result. Reads and audit_report still work."
    )]
    UnsealedCatalog {
        /// Catalog file that needs migrating.
        path: PathBuf,
    },

    /// `seal --adopt` found records whose seal is present but wrong, or deleted claims.
    #[error(
        "refusing to adopt: these records were changed after they were sealed, or are missing: \
         {}. Restore the catalog from git instead.",
        list_problems(.problems)
    )]
    AdoptRefused {
        /// Every record that blocks adoption.
        problems: Vec<IntegrityProblem>,
    },

    /// `claims-mcp rebase` cannot replay the branch's records onto the other ledger: an input
    /// fails its integrity check, the inputs do not descend from `--base`, or both sides
    /// changed the same claim differently.
    #[error("cannot rebase: {0}")]
    Rebase(String),

    /// A git command needed for an anchor or a recheck failed.
    #[error("git {command} failed: {message}")]
    Git {
        /// The git subcommand and its arguments.
        command: String,
        /// git's error output, or why it could not be run.
        message: String,
    },

    /// Serializing the catalog or a tool response failed.
    #[error("failed to encode JSON: {0}")]
    Encode(#[from] serde_json::Error),

    /// The blocking worker thread running a store operation panicked or was cancelled.
    #[error("catalog worker task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// `EVD-0014 (seal does not match its content); ...`, truncated after [`LISTED_PROBLEMS`].
fn list_problems(problems: &[IntegrityProblem]) -> String {
    let mut listed: Vec<String> = problems
        .iter()
        .take(LISTED_PROBLEMS)
        .map(|item| format!("{} ({})", item.record, item.problem))
        .collect();
    if problems.len() > LISTED_PROBLEMS {
        listed.push(format!("and {} more", problems.len() - LISTED_PROBLEMS));
    }
    listed.join("; ")
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, ClaimsError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_problem_lists_are_truncated_with_a_count() {
        let problems: Vec<IntegrityProblem> = (1..=12)
            .map(|seq| IntegrityProblem {
                record: EvidenceId::new(seq).to_string(),
                problem: "has no seal".to_owned(),
            })
            .collect();
        let listed = list_problems(&problems);
        assert!(listed.starts_with("EVD-0001 (has no seal); EVD-0002"));
        assert!(listed.contains("EVD-0010 (has no seal)"));
        assert!(!listed.contains("EVD-0011"));
        assert!(listed.ends_with("and 2 more"));
    }
}
