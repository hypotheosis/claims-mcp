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
