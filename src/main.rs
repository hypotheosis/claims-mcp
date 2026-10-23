//! `claims-mcp`: a stdio MCP server that maintains a JSON catalog of verifiable claims.

mod catalog;
mod error;
mod model;
mod report;
mod seal;
mod server;
mod store;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use crate::seal::IntegrityReport;
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
    #[arg(
        long,
        global = true,
        env = "CLAIMS_CATALOG",
        default_value = "claims.json"
    )]
    catalog: PathBuf,
    /// What to do; without a subcommand the MCP server runs on stdio.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Maintenance subcommands. None of them is exposed as an MCP tool.
#[derive(Debug, Subcommand)]
enum Command {
    /// Check every integrity seal in the catalog; exits 1 if any check fails.
    Verify {
        /// Print the result as JSON (the `integrity` input of report/claims-report.typ).
        #[arg(long)]
        json: bool,
    },
    /// Seal records that have no seal, for maintainers migrating a catalog.
    Seal {
        /// Seal every unsealed record as adopted. Required: adoption is the only sealing
        /// mode, and it refuses records whose existing seal is wrong.
        #[arg(long, required = true)]
        adopt: bool,
    },
}

/// Rust note: `current_thread` runs the async reactor on one thread. That is enough for a
/// stdio server; blocking file work is still sent to tokio's separate blocking pool.
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // A `.env` file is optional, so a missing one is not an error.
    let _ = dotenvy::dotenv();
    init_tracing();
    let cli = Cli::parse();
    let store = CatalogStore::new(cli.catalog);
    let outcome = match cli.command {
        None => serve(store).await.map(|()| ExitCode::SUCCESS),
        Some(Command::Verify { json }) => verify(&store, json).await,
        Some(Command::Seal { adopt: _ }) => adopt(&store).await.map(|()| ExitCode::SUCCESS),
    };
    outcome.unwrap_or_else(|error| {
        // `{:#}` prints the whole anyhow context chain on one line.
        tracing::error!("{error:#}");
        ExitCode::FAILURE
    })
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
async fn serve(store: CatalogStore) -> anyhow::Result<()> {
    tracing::info!(catalog = %store.path().display(), "starting claims-mcp");
    let server = ClaimsServer::new(store);
    let service = server
        .serve(rmcp::transport::stdio_pipes())
        .await
        .context("failed to start the MCP stdio transport")?;
    service.waiting().await.context("MCP service task failed")?;
    Ok(())
}

/// Prints the catalog's integrity report; the exit code is 1 when any check fails, so CI can
/// gate on it.
async fn verify(store: &CatalogStore, json: bool) -> anyhow::Result<ExitCode> {
    let report = store
        .read(|catalog| Ok(seal::verify(catalog)))
        .await
        .context("failed to read the catalog")?;
    let path = store.path().display();
    // Rust note: locking stdout once avoids re-acquiring its mutex for every line.
    let mut out = std::io::stdout().lock();
    if json {
        serde_json::to_writer(&mut out, &report)?;
        writeln!(out)?;
    } else {
        write_report(&mut out, &path.to_string(), &report)?;
    }
    Ok(if report.verified {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Human-readable form of an integrity report.
fn write_report(out: &mut impl Write, path: &str, report: &IntegrityReport) -> std::io::Result<()> {
    let status = if report.verified {
        "verified"
    } else {
        "FAILED"
    };
    writeln!(
        out,
        "{path}: integrity {status} ({} records checked)",
        report.records
    )?;
    for problem in &report.problems {
        writeln!(out, "  {}: {}", problem.record, problem.problem)?;
    }
    if !report.adopted.is_empty() {
        writeln!(
            out,
            "{} records were sealed by `seal --adopt`, not by the server: {}",
            report.adopted.len(),
            report.adopted.join(", ")
        )?;
    }
    Ok(())
}

/// Seals unsealed records as adopted and lists them.
async fn adopt(store: &CatalogStore) -> anyhow::Result<()> {
    let adopted = store.adopt().await.context("seal --adopt failed")?;
    let mut out = std::io::stdout().lock();
    if adopted.is_empty() {
        writeln!(
            out,
            "nothing to adopt: every record already has a valid seal"
        )?;
    } else {
        writeln!(out, "sealed {} records as adopted:", adopted.len())?;
        for record in &adopted {
            writeln!(out, "  {record}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seal::IntegrityProblem;

    fn rendered(report: &IntegrityReport) -> String {
        let mut out = Vec::new();
        write_report(&mut out, "claims.json", report).expect("write to memory");
        String::from_utf8(out).expect("UTF-8")
    }

    #[test]
    fn report_lists_problems_and_adopted_records() {
        let report = IntegrityReport {
            verified: false,
            records: 5,
            problems: vec![IntegrityProblem {
                record: "EVD-0002".to_owned(),
                problem: "seal does not match its content".to_owned(),
            }],
            adopted: vec!["CLM-0001".to_owned()],
        };
        assert_eq!(
            rendered(&report),
            "claims.json: integrity FAILED (5 records checked)\n  \
             EVD-0002: seal does not match its content\n\
             1 records were sealed by `seal --adopt`, not by the server: CLM-0001\n"
        );
    }

    #[test]
    fn verified_report_is_one_line() {
        let report = IntegrityReport {
            verified: true,
            records: 3,
            problems: Vec::new(),
            adopted: Vec::new(),
        };
        assert_eq!(
            rendered(&report),
            "claims.json: integrity verified (3 records checked)\n"
        );
    }
}
