//! `claims-mcp`: a stdio MCP server that maintains a JSON catalog of verifiable claims.

mod catalog;
mod error;
mod model;

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
