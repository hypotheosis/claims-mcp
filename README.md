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
repository root. It uses `cargo run --release`, so the first start compiles the server; run
`cargo build --release` once beforehand to avoid a client startup timeout. For another project, add:

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

## Generate a PDF report

[`report/claims-report.typ`](report/claims-report.typ) renders a catalog as a self-contained
PDF audit report with [Typst](https://typst.app): result and assessment, findings, coverage by
integrity level, all eight gap categories, the claim hierarchy, the traceability matrix, and an
evidence appendix with the full verdict history. It runs outside the server, which never
executes commands.

```bash
just report                       # claims.json -> audit.pdf, stamped with the current commit
# or, without just:
typst compile --root . report/claims-report.typ audit.pdf \
  --input catalog=claims.json \
  --input commit=$(git rev-parse --short HEAD) \
  --input notes=report/audit-notes.toml
```

Paths are relative to `--root`. All inputs are optional:

| Input | Meaning |
|---|---|
| `catalog` | Catalog to report on (default `claims.json`) |
| `commit` | Catalog commit shown on the cover and in the reproduce section |
| `notes` | TOML sidecar with `artifact`, `version`, `repository`, `method`, `assessment`, and `[[points]]` (`title`, `body`) for the evaluator; see [`report/audit-notes.toml`](report/audit-notes.toml) |
| `date` | Report date (default today) |
| `artifact`, `version`, `repository`, `method`, `assessment` | Override the matching sidecar key |

Tag a claim `issue:N` to link it to issue N of `repository` in the findings. The template applies
the gap rules of spec section 4.1; `cargo test typst_template` checks that its totals, gaps, and
matrix match `audit_report`. When `typst` is not installed that check is skipped and still
counts as passed, so a green `cargo test` does not prove it ran; set `REQUIRE_TYPST=1` (as CI
should) to make a missing `typst` fail the tests instead.

## Catalog format

See [the design spec](docs/superpowers/specs/2026-10-23-claims-mcp-design.md), section 3,
for the full schema. Ids look like `CLM-0007` and `EVD-0014` and are never reused.

## Development

```bash
just fmt      # cargo fmt
just clippy   # cargo clippy --all-targets -- -D warnings
just test     # cargo test
```
