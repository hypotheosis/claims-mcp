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

## Install

### 1. Install the server

Pick one:

- **Download a release.** Each [GitHub release](https://github.com/hypotheosis/claims-mcp/releases)
  has an archive per platform, a `.sha256` checksum for each, and the audit report for that
  version:

  | Platform | Archive |
  |---|---|
  | Linux x86_64 (static) | `claims-mcp-<version>-x86_64-unknown-linux-musl.tar.gz` |
  | Linux ARM64 (static) | `claims-mcp-<version>-aarch64-unknown-linux-musl.tar.gz` |
  | Windows x86_64 | `claims-mcp-<version>-x86_64-pc-windows-msvc.zip` |
  | macOS Intel | `claims-mcp-<version>-x86_64-apple-darwin.tar.gz` |
  | macOS Apple Silicon | `claims-mcp-<version>-aarch64-apple-darwin.tar.gz` |

  On Linux and macOS:

  ```bash
  sha256sum -c claims-mcp-<version>-<target>.tar.gz.sha256   # macOS: shasum -a 256 -c
  tar -xzf claims-mcp-<version>-<target>.tar.gz
  mkdir -p ~/.local/bin
  install -m 755 claims-mcp-<version>-<target>/claims-mcp ~/.local/bin/
  ```

  On Windows (PowerShell), compare the hash with the one in the `.sha256` file, then extract
  and put `claims-mcp.exe` in a directory on your `PATH`:

  ```powershell
  $archive = "claims-mcp-<version>-x86_64-pc-windows-msvc"
  (Get-FileHash "$archive.zip" -Algorithm SHA256).Hash   # must match "$archive.zip.sha256"
  Expand-Archive "$archive.zip" -DestinationPath .
  ```

  Each archive also contains this README and the `skills/` directory. The macOS binaries are
  not signed or notarized; if Gatekeeper blocks one, clear the quarantine flag with
  `xattr -d com.apple.quarantine ~/.local/bin/claims-mcp`.

- **Install with cargo** (Rust 1.89 or newer):

  ```bash
  cargo install --locked --git https://github.com/hypotheosis/claims-mcp
  ```

  This puts `claims-mcp` in `~/.cargo/bin`.

- **Build from a clone**:

  ```bash
  cargo build --release   # binary at target/release/claims-mcp
  ```

### 2. Register the server with Claude Code

With `claims-mcp` on your `PATH`, register it from the root of the project whose catalog it
should keep:

```bash
claude mcp add --scope project claims -- claims-mcp --catalog claims.json
```

`--scope project` writes `.mcp.json` so everyone working in the repository gets the server;
use `--scope user` to make it available in all your projects, or leave the flag off to register
it for this project only. A relative `--catalog` resolves against the directory Claude Code is
started in. The resulting `.mcp.json` entry, if you prefer to write it by hand (use an absolute
`command` path if the binary is not on `PATH`):

```json
{
  "mcpServers": {
    "claims": {
      "command": "claims-mcp",
      "args": ["--catalog", "claims.json"]
    }
  }
}
```

This repository's own `.mcp.json` uses `cargo run --release` instead, so the first start
compiles the server; run `cargo build --release` once beforehand to avoid a client startup
timeout.

### 3. Install the skill

The [`claims-audit`](skills/claims-audit/SKILL.md) skill teaches Claude the audit workflow. Copy
it from a clone or a release archive, either for yourself (all projects):

```bash
mkdir -p ~/.claude/skills
cp -r skills/claims-audit ~/.claude/skills/
```

or into a project, committed so collaborators get it too:

```bash
mkdir -p .claude/skills
cp -r skills/claims-audit .claude/skills/
```

On Windows the user directory is `%USERPROFILE%\.claude\skills`. Restart Claude Code, then ask
it to "audit this change against the task", or invoke `/claims-audit`.

## Run

The server speaks MCP over stdio, so Claude Code normally starts it. To run it directly:

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

### Continuous integration

`.github/workflows/ci.yml` runs on every pull request and push to `main`:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, a build with
  `RUSTFLAGS=-D warnings`, and `cargo test` with Typst installed and `REQUIRE_TYPST=1`, so the
  Typst template checks cannot skip silently
- `cargo test` on Windows and macOS, and `cargo check` on the minimum supported Rust version
  (`rust-version` in `Cargo.toml`)
- `just report` on the PR head commit; the PDF is uploaded as the run's
  `audit-report-pr-<number>-<commit>` artifact (open the run's summary page to download it)

### Releasing

1. In a pull request, set `version` in `Cargo.toml` to the new version, update
   `report/audit-notes.toml` to match, and merge. `main` only accepts changes through pull
   requests that pass CI and are up to date with `main`.
2. Tag the merge commit and push the tag:

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

`.github/workflows/release.yml` checks that the tag equals `v` plus the `Cargo.toml` version
and points at a commit on `main` (and fails without publishing otherwise), reruns CI on the tagged commit, builds the five
archives listed under [Install](#1-install-the-server), and publishes a GitHub release with
the archives, their `.sha256` files, and the audit report. A pull request that changes
`release.yml` runs the builds as a dry run without publishing. Release tags (`v*`) cannot be moved or deleted once
pushed, so a bad release is fixed with a new version.

## Security

Report vulnerabilities privately; see [`SECURITY.md`](SECURITY.md).
