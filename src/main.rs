//! `claims-mcp`: a stdio MCP server that maintains a JSON catalog of verifiable claims.

mod catalog;
mod error;
mod model;
mod report;
mod seal;
mod server;
mod store;

use std::borrow::Cow;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use crate::report::next_steps::{self, AreaCount, NextSteps, Tier};
use crate::seal::IntegrityReport;
use crate::server::ClaimsServer;
use crate::store::CatalogStore;

/// Statements longer than this many `char`s are cut in the TL;DR.
const STATEMENT_CHARS: usize = 100;

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
    /// Print a TL;DR: open leaf claims ranked into priority tiers, each with its next step.
    Tldr {
        /// Items listed in each of tiers 1 and 2; the rest are counted per area. 0 lists
        /// everything.
        #[arg(long, default_value_t = next_steps::DEFAULT_LIMIT)]
        limit: usize,
        /// Print JSON (the `next_steps` tool's output) instead of Markdown.
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
        Some(Command::Tldr { limit, json }) => {
            tldr(&store, limit, json).await.map(|()| ExitCode::SUCCESS)
        }
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

/// Prints the next-steps TL;DR as Markdown, or as JSON with `json`. Gaps are the expected
/// content, so the exit code is 0 whenever the catalog could be read.
async fn tldr(store: &CatalogStore, limit: usize, json: bool) -> anyhow::Result<()> {
    let steps = store
        .read(move |catalog| Ok(next_steps::next_steps(catalog, limit)))
        .await
        .context("failed to read the catalog")?;
    // Render fully before writing so a closed pipe is seen in exactly one place.
    let mut buffer = Vec::new();
    if json {
        serde_json::to_writer(&mut buffer, &steps)?;
        writeln!(buffer)?;
    } else {
        write_tldr(&mut buffer, &store.path().display().to_string(), &steps)?;
    }
    write_ignoring_broken_pipe(&mut std::io::stdout().lock(), &buffer)?;
    Ok(())
}

/// Writes `bytes` to `out` in one call. A closed reader (`claims-mcp tldr | head`) is not an
/// error.
///
/// # Errors
///
/// Any I/O error other than `BrokenPipe`.
fn write_ignoring_broken_pipe(out: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    // Rust note: `match` on `Result` handles each case; the guard `if ...` narrows the `Err`
    // arm to one error kind, and every other error falls through to the last arm unchanged.
    match out.write_all(bytes) {
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

/// Markdown form of the TL;DR (spec section 6).
fn write_tldr(out: &mut impl Write, path: &str, steps: &NextSteps) -> std::io::Result<()> {
    let summary = &steps.summary;
    let leaves = summary.done_leaves + summary.open_leaves;
    writeln!(out, "# TL;DR: {path}\n")?;
    writeln!(
        out,
        "{} active {}: {} {} ({} done, {} open), {} {}.\n",
        summary.active_claims,
        plural(summary.active_claims, "claim", "claims"),
        leaves,
        plural(leaves, "leaf", "leaves"),
        summary.done_leaves,
        summary.open_leaves,
        summary.rollup_claims,
        plural(summary.rollup_claims, "roll-up", "roll-ups")
    )?;
    if !steps.integrity.verified {
        writeln!(out, "## 0. STOP: integrity check failed\n")?;
        let failed = steps.integrity.problems.len();
        writeln!(
            out,
            "{failed} {} failed the seal check. Nothing below can be trusted until \
             `claims-mcp verify` passes.\n",
            plural(failed, "record", "records")
        )?;
        if let Some(first) = steps.integrity.problems.first() {
            writeln!(out, "First problem: {}: {}\n", first.record, first.problem)?;
        }
        writeln!(
            out,
            "The usual cause is a hand edit; fix it by restoring the catalog from git.\n"
        )?;
    }
    write_tier(out, "1. Wrong", &steps.wrong)?;
    write_tier(out, "2. Unproven, integrity 3-4", &steps.unproven)?;
    let other_total: usize = steps.other.iter().map(|entry| entry.count).sum();
    writeln!(out, "## 3. Everything else ({other_total})\n")?;
    if steps.other.is_empty() {
        writeln!(out, "None.")
    } else {
        writeln!(out, "{}", area_list(&steps.other))
    }
}

/// One tier: a bullet per listed item, then the per-area count of the rest.
fn write_tier(out: &mut impl Write, heading: &str, tier: &Tier) -> std::io::Result<()> {
    writeln!(out, "## {heading} ({})\n", tier.total)?;
    if tier.items.is_empty() {
        return writeln!(out, "None.\n");
    }
    for item in &tier.items {
        write!(out, "- **{}**", item.id)?;
        if let Some(level) = item.integrity_level {
            write!(out, " IL{}", level.get())?;
        }
        if !item.issues.is_empty() {
            write!(out, " [#{}]", item.issues.join(", #"))?;
        }
        writeln!(
            out,
            " {}\n  - Next: {}",
            truncated(&single_line(&item.statement), STATEMENT_CHARS),
            single_line(&item.next_step)
        )?;
    }
    if !tier.omitted.is_empty() {
        let hidden = tier.total - tier.items.len();
        writeln!(out, "\n...and {hidden} more: {}", area_list(&tier.omitted))?;
    }
    writeln!(out)
}

/// `singular` when `count` is exactly 1, otherwise `plural`.
fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 {
        singular
    } else {
        plural
    }
}

/// `text` with every `\n` and `\r` replaced by a space, so it cannot break a Markdown list item.
///
/// Borrows the input unchanged (no allocation) when it has no line break.
fn single_line(text: &str) -> Cow<'_, str> {
    if text.contains(['\n', '\r']) {
        Cow::Owned(text.replace(['\n', '\r'], " "))
    } else {
        Cow::Borrowed(text)
    }
}

/// `OPS 30, POL 21, ...`
fn area_list(counts: &[AreaCount]) -> String {
    counts
        .iter()
        .map(|entry| format!("{} {}", entry.area, entry.count))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `text` cut to `max_chars` `char`s with `...` appended, or unchanged when it fits.
///
/// Rust note: `Cow` (clone-on-write) borrows the input when it fits and only allocates a new
/// `String` when it has to cut. `char_indices` walks whole Unicode scalar values, so the cut
/// never splits a multibyte character.
fn truncated(text: &str, max_chars: usize) -> Cow<'_, str> {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => Cow::Owned(format!("{}...", &text[..cut])),
        None => Cow::Borrowed(text),
    }
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
    use crate::model::{ClaimId, CurrentVerdict, IntegrityLevel};
    use crate::report::next_steps::{NextStep, NextStepsSummary};
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

    fn integrity(problems: usize) -> IntegrityReport {
        IntegrityReport {
            verified: problems == 0,
            records: 9,
            problems: (0..problems)
                .map(|index| IntegrityProblem {
                    record: format!("CLM-000{index}"),
                    problem: "seal does not match its content".to_owned(),
                })
                .collect(),
            adopted: Vec::new(),
        }
    }

    /// One refuted item listed with one more omitted, an empty tier 2, and two tier 3 claims.
    fn sample_steps(problems: usize) -> NextSteps {
        NextSteps {
            integrity: integrity(problems),
            summary: NextStepsSummary {
                active_claims: 6,
                rollup_claims: 1,
                open_leaves: 4,
                done_leaves: 1,
            },
            wrong: Tier {
                total: 2,
                items: vec![NextStep {
                    id: ClaimId::new(12),
                    integrity_level: Some(IntegrityLevel::try_from(4).expect("level")),
                    verdict: CurrentVerdict::Refuted,
                    stale: false,
                    area: "AUTH".to_owned(),
                    statement: "Tokens expire.".to_owned(),
                    next_step: "Add an expiry test.".to_owned(),
                    issues: vec!["360".to_owned(), "361".to_owned()],
                }],
                omitted: vec![AreaCount {
                    area: "OPS".to_owned(),
                    count: 1,
                }],
            },
            unproven: Tier::default(),
            other: vec![AreaCount {
                area: "(none)".to_owned(),
                count: 2,
            }],
        }
    }

    /// A writer whose reader has gone away, or that fails some other way.
    struct FailingWriter(std::io::ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_is_success_and_other_errors_propagate() {
        let mut closed = FailingWriter(std::io::ErrorKind::BrokenPipe);
        let mut denied = FailingWriter(std::io::ErrorKind::PermissionDenied);

        assert!(write_ignoring_broken_pipe(&mut closed, b"x").is_ok());
        let error = write_ignoring_broken_pipe(&mut denied, b"x").expect_err("must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    fn rendered_tldr(steps: &NextSteps) -> String {
        let mut out = Vec::new();
        write_tldr(&mut out, "claims.json", steps).expect("write to memory");
        String::from_utf8(out).expect("UTF-8")
    }

    #[test]
    fn tldr_renders_every_tier() {
        assert_eq!(
            rendered_tldr(&sample_steps(0)),
            "# TL;DR: claims.json\n\n\
             6 active claims: 5 leaves (1 done, 4 open), 1 roll-up.\n\n\
             ## 1. Wrong (2)\n\n\
             - **CLM-0012** IL4 [#360, #361] Tokens expire.\n  \
             - Next: Add an expiry test.\n\n\
             ...and 1 more: OPS 1\n\n\
             ## 2. Unproven, integrity 3-4 (0)\n\n\
             None.\n\n\
             ## 3. Everything else (2)\n\n\
             (none) 2\n"
        );
    }

    #[test]
    fn tldr_opens_with_stop_when_integrity_fails() {
        let text = rendered_tldr(&sample_steps(2));
        let stop = text
            .find("## 0. STOP: integrity check failed\n\n2 records failed")
            .expect("STOP section");
        let first = text
            .find("First problem: CLM-0000: seal does not match its content\n")
            .expect("first problem line");
        assert!(stop < first);
        assert!(text.contains("until `claims-mcp verify` passes"));
        assert!(text.contains("restoring the catalog from git"));
        assert!(!text.contains("CLM-0001: seal"));
        let wrong = text.find("## 1. Wrong").expect("tier 1");
        assert!(stop < wrong);
        assert!(!rendered_tldr(&sample_steps(0)).contains("STOP"));
    }

    #[test]
    fn tldr_uses_singular_nouns_for_a_count_of_one() {
        let mut steps = sample_steps(1);
        steps.summary.active_claims = 1;
        steps.summary.rollup_claims = 1;
        steps.summary.done_leaves = 0;
        steps.summary.open_leaves = 1;

        let text = rendered_tldr(&steps);

        assert!(text.contains("1 active claim: 1 leaf (0 done, 1 open), 1 roll-up.\n"));
        assert!(text.contains("\n\n1 record failed the seal check."));
    }

    #[test]
    fn tldr_flattens_line_breaks_in_statements_and_steps() {
        let mut steps = sample_steps(0);
        steps.wrong.items[0].statement = "Tokens\r\nexpire.\nSoon.".to_owned();
        steps.wrong.items[0].next_step = "Add a test.\nThen fix.".to_owned();

        let text = rendered_tldr(&steps);

        assert!(text.contains("Tokens  expire. Soon.\n  - Next: Add a test. Then fix.\n"));
        assert!(matches!(single_line("one line"), Cow::Borrowed(_)));
    }

    #[test]
    fn plural_picks_by_count() {
        assert_eq!(plural(0, "leaf", "leaves"), "leaves");
        assert_eq!(plural(1, "leaf", "leaves"), "leaf");
        assert_eq!(plural(2, "leaf", "leaves"), "leaves");
    }

    #[test]
    fn tldr_omits_missing_level_and_issues() {
        let mut steps = sample_steps(0);
        steps.wrong.items[0].integrity_level = None;
        steps.wrong.items[0].issues.clear();
        assert!(rendered_tldr(&steps).contains("- **CLM-0012** Tokens expire.\n"));
    }

    #[test]
    fn truncated_cuts_on_char_boundaries() {
        let long = "\u{e9}".repeat(STATEMENT_CHARS + 1);
        let exact = "\u{e9}".repeat(STATEMENT_CHARS);

        assert_eq!(truncated(&long, STATEMENT_CHARS), format!("{exact}..."));
        assert!(matches!(
            truncated(&exact, STATEMENT_CHARS),
            Cow::Borrowed(_)
        ));
    }
}
