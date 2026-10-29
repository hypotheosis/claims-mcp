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
  version (`audit.pdf`, the summary, and `audit-full.pdf`, every record):

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

  Each archive also contains this README, `LICENSE`, `NOTICE`, and the `skills/` directory. The
  macOS binaries are not signed or notarized; if Gatekeeper blocks one, clear the quarantine flag
  with `xattr -d com.apple.quarantine ~/.local/bin/claims-mcp`.

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
exclusive lock on `.<name>.lock`. Git checkouts and several server instances sharing one file
are safe. Hand edits are detected, and block writes; see [Integrity seals](#integrity-seals).
Add `.<name>.lock` and `.<name>.tmp` to `.gitignore`.

| Subcommand | Purpose |
|---|---|
| *(none)* | Run the MCP server on stdio |
| `verify [--json]` | Check every integrity seal; exit 1 if any check fails |
| `tldr [--limit N] [--json]` | Print the ranked next steps; see [TL;DR of next steps](#tldr-of-next-steps) |
| `recheck [--json]` | Compare every anchor with `HEAD`; exit 1 if any is not current; see [Anchors](#anchors) |
| `seal --adopt` | Seal unsealed records once, to migrate a catalog (maintainers only) |
| `report init [DIR]` | Write the PDF report template and a notes file to `DIR` (default `report`); see [Generate a PDF report](#generate-a-pdf-report) |

## Integrity seals

The catalog is meant to change only through the tools, which enforce its rules: verdicts must
cite evidence, the history is append-only, claims are retired instead of deleted. To stop an
agent from skipping the tools and editing `claims.json` directly, every claim, evidence item,
and verdict, plus each of the two id counters, carries a seal: an HMAC-SHA256 over the record's
content and its place in the catalog, under a key built into the binary.

- **Every call verifies every seal.** If any check fails, the server **refuses every write**
  until the file is restored. Reads and `audit_report` keep working, so the damage can be
  inspected. There is no option to turn this off.
- **What counts as tampering:** a changed field, an added record without a seal, a deleted
  claim, evidence item, or verdict, reordered verdicts, evidence moved to another claim, a
  changed id counter, and a `schema_version` lowered to 1.
- **Recovering:** restore the file from git (`git checkout -- claims.json`, or revert the
  commit that edited it), then redo the change through the tools.
- **Not affected:** reformatting the file or reordering its keys (seals cover content, not
  layout), and git merges of branches that changed *different* claims. Each counter has its
  own seal, so a branch that added a claim merges with one that added evidence.

This is a deterrent, not cryptographic security. The key is in this repository's source, so
anyone who reads it can forge seals. What it stops is the casual edit: in your project, an agent
would have to go find the key and write code to recompute seals.

### Checking in CI

`claims-mcp verify` exits 1 when any check fails and names each failing record, so a hand-edited
catalog fails the PR. This repository's CI runs it. `--json` prints the result as JSON.

```bash
claims-mcp --catalog claims.json verify
```

### Migrating with `seal --adopt`

Catalogs written before seals existed (`schema_version 1`) can still be read, but the server
refuses to write to them until a maintainer runs:

```bash
claims-mcp --catalog claims.json seal --adopt   # then commit claims.json
```

It seals every unsealed record and labels it `adopt`. `verify` and `audit_report` list adopted
records separately from ones the server recorded. Adoption **never** re-seals a record whose seal
is present but wrong, so it cannot hide an edit to a sealed record. It also refuses when a claim
is missing, and when a `schema_version 1` file carries seals (a genuine v1 file has none, so the
version was lowered by hand). Removing a seal by hand and adopting the record is possible, but
the record then shows as adopted in every report and in the git diff. If two branches changed the
*same* claim, keep one side and redo the other side's change through the tools.

Agents should never run `seal --adopt`: the server's own refusal message says so. To keep agents
away from the file entirely, add deny rules to the project's `.claude/settings.json`:

```json
{
  "permissions": {
    "deny": ["Edit(**/claims.json)", "Write(**/claims.json)", "Bash(claims-mcp seal:*)"]
  }
}
```

## Tools

| Tool | Purpose |
|---|---|
| `add_claim` | Create one claim |
| `add_claims` | Create many claims atomically; `@N` in `derived_from` links to an earlier entry |
| `get_claim` | One claim with evidence, verdict history, current verdict, and children |
| `list_claims` | Filtered summaries (verdict, kind, level, tag, artifact, text, include_retired) |
| `update_claim` | Edit metadata; evidence and verdicts cannot be changed |
| `retire_claim` | Withdraw a claim; it stays so trace links remain valid |
| `add_evidence` | Attach evidence with a locator, an optional commit-pinned anchor, and a stance (supports / refutes / neutral) |
| `record_verdict` | Append a FEVER verdict; it must cite evidence that backs it |
| `trace` | Ancestors and descendants of a claim, optionally depth-limited |
| `audit_report` | Totals, gap lists, and the traceability matrix |
| `next_steps` | Ranked TL;DR: open claims in priority tiers, each with one next step |
| `recheck` | Anchors that moved, changed, or were overturned since they were recorded |

The server also offers read-only resources: the PDF report's files, under `claims://report/`.
`claims://report/audit-notes.example.toml` is the commented example of the report's notes file,
so an agent with only the MCP connection can write one.

## Anchors

An evidence `locator` is free text. To make evidence point at an exact, checkable place, add an
`anchor`, which the server pins to the commit checked out at the time:

| Anchor | Shape | Use it for |
|---|---|---|
| file | `{"file": {"path": "src/server.rs", "lines": {"start": 40, "end": 61}}}` | Code or text that shows the finding; omit `lines` for the whole file |
| search | `{"search": {"pattern": "rate.?limit\|throttl", "case_insensitive": true, "scope": ["src"]}}` | A `git grep -E` search; with zero hits, it shows that something does not exist |

Claims take a `source_anchor` (a file anchor) on the text that makes them, such as the README
line that promises a feature. If the claim is refuted, that is the text to fix; `audit_report`
lists refuted claims without one under `refuted_without_source_anchor`. `update_claim` can
replace a `source_anchor` but not remove it.

The server, not the caller, records the commit, and for a search it runs `git grep` itself and
records the hit count (a `hits` value the caller passes must match). It rejects paths outside
the repository, line ranges past the end of the file, and search scopes that do not exist (a
misspelled directory would otherwise "prove" anything absent). Anchors need the catalog to be inside
a git repository; evidence without one works anywhere. An anchor on a file with uncommitted
changes is marked `dirty`.

`claims-mcp recheck` (and the `recheck` tool) compares every anchor on an active claim with
`HEAD`:

| Status | Meaning |
|---|---|
| `moved` | The anchored lines are unchanged but at new line numbers (shown) |
| `changed` | The anchored lines or file changed, or a search's hit count changed |
| `overturned` | An absence search now matches, or a matching search now finds nothing |
| `missing` | The file no longer exists |
| `commit_missing` | The pinned commit was removed by a history rewrite |
| `unpinned` | Recorded on uncommitted changes, so the commit may not show what was seen |

It exits 1 if any anchor is not current. The PDF report takes its `--json` output as the
`recheck` input.

## TL;DR of next steps

On a large catalog, `audit_report` is too long to act on. `next_steps` (MCP) and
`claims-mcp tldr` (CLI) list only what to do next:

```bash
claims-mcp --catalog claims.json tldr             # Markdown, 10 items per tier
claims-mcp --catalog claims.json tldr --limit 0   # every item
claims-mcp --catalog claims.json tldr --json      # same data as the next_steps tool
```

Active **leaf** claims (no active children) are listed when they need work. A parent's verdict
rolls up from its children, so a parent is listed only once every active child is done and its
own verdict is not (missing, refuted, not_enough_info or stale): its step is then to record the
roll-up verdict. Open claims are ranked into tiers:

| Tier | Contents |
|---|---|
| 0. STOP | Shown only when the integrity check fails: make `claims-mcp verify` pass first (usually: restore the catalog from git) |
| 1. Wrong | Refuted claims |
| 2. Unproven | Other open claims at integrity level 3-4 (not_enough_info, unverified, stale, no evidence) |
| 3. Everything else | Counts per `area:` tag |

Tiers 1 and 2 are sorted by integrity level, then id. Each item shows its `issue:` tags and its
next step: the text after the last `Fix:` in the latest verdict rationale (the `claims-audit`
skill requires one on every `refuted` and `not_enough_info` verdict), or else the rationale's
first sentence before the marker. Items beyond `--limit` are counted per area. The command exits
0 whenever the catalog can be read; use `verify` to gate CI.

## Generate a PDF report

A Typst template renders a catalog as a self-contained PDF audit report with
[Typst](https://typst.app). It runs outside the server, which never executes commands. It has
two detail levels:

- **summary** (default): result and assessment, integrity seal status, findings grouped by
  area (refuted claims in full, the rest as one-line next steps), coverage by integrity level,
  and gap counts.
- **full**: everything in the summary, plus every finding in full with its evidence, the claim
  IDs in each gap, the claim hierarchy, the traceability matrix, and an evidence appendix with
  each claim's full verdict history.

Both collapse long claim ID lists into ranges and roll-up evidence into child counts, and both
end with a glossary that defines every term the report uses (verdicts, stances, integrity levels,
verification methods, seals), so readers need not know claims-mcp.

The template is built into the binary, so every install carries the version that matches it.
In the audited project's root:

```bash
claims-mcp report init            # writes report/claims-report.typ, report/text.typ,
                                  # report/audit-notes.example.toml, and report/audit-notes.toml
# uncomment and fill in report/audit-notes.toml as its comments say, then:
claims-mcp --catalog claims.json verify --json > integrity.json
claims-mcp --catalog claims.json recheck --json > recheck.json
typst compile --root . report/claims-report.typ audit.pdf \
  --input catalog=claims.json \
  --input detail=summary \
  --input commit=$(git rev-parse --short HEAD) \
  --input notes=report/audit-notes.toml \
  --input integrity=integrity.json \
  --input recheck=recheck.json
```

`report init` replaces the template files, so they always match the installed binary, but
never replaces an existing `audit-notes.toml`; it prints what it wrote and what it kept. A new
`audit-notes.toml` has every setting of the example commented out, so the example's invented
project cannot end up in a report. Pass a
directory (`claims-mcp report init docs/audit`) to write elsewhere, and adjust the paths above.
Use `--input detail=full` and `audit-full.pdf` for the full record.

In this repository, `just report` renders the summary (`audit.pdf`) and `just report full` the
full record (`audit-full.pdf`) from `claims.json` with claims-mcp's own notes.

Paths are relative to `--root`. All inputs are optional:

| Input | Meaning |
|---|---|
| `catalog` | Catalog to report on (default `claims.json`) |
| `detail` | `summary` (default) or `full`; any other value is an error |
| `integrity` | Output of `claims-mcp verify --json` for that catalog. Typst cannot check seals itself; without this input the report says integrity was not checked. A failed check puts a warning on the result and lists the failing records |
| `recheck` | Output of `claims-mcp recheck --json`. Flags anchors that moved, changed, or were overturned, on the evidence and in an Anchor recheck section; without it the report says anchors were not rechecked |
| `commit` | Catalog commit shown on the cover and in the reproduce section |
| `notes` | TOML sidecar with the audited project's name and the auditor's prose: `artifact`, `version`, `repository`, `method`, `assessment`, and `[[points]]` (`title`, `body`) for the evaluator. Each audited project needs its own: `report init` creates it with the commented example's settings commented out (also the resource `claims://report/audit-notes.example.toml`), which explains every key |
| `date` | Report date (default today) |
| `artifact`, `version`, `repository`, `method`, `assessment` | Override the matching sidecar key |

Tag a claim `issue:N` to link it to issue N of `repository` in the findings. The template applies
the gap rules of spec section 4.1; `cargo test typst_template` checks that its totals, gaps, and
matrix match `audit_report` in both detail levels, and that its text helpers pass
`report/text-tests.typ`. When `typst` is not installed that check is skipped and still
counts as passed, so a green `cargo test` does not prove it ran; set `REQUIRE_TYPST=1` (as CI
should) to make a missing `typst` fail the tests instead.

## Catalog format

See [the design spec](docs/superpowers/specs/2026-10-23-claims-mcp-design.md), section 3,
for the full schema. Ids look like `CLM-0007` and `EVD-0014` and are never reused.

## Security

Report vulnerabilities privately; see [`SECURITY.md`](SECURITY.md).

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
- `claims-mcp verify`, which fails if `claims.json` was edited outside the tools
- `cargo test` on Windows and macOS, and `cargo check` on the minimum supported Rust version
  (`rust-version` in `Cargo.toml`)
- `just report` and `just report full` on the PR head commit; both PDFs (`audit.pdf`, the
  summary, and `audit-full.pdf`, every record) are uploaded as the run's
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
`release.yml` runs the builds as a dry run without publishing. Release tags (`v*`) cannot be
moved or deleted once pushed, so a bad release is fixed with a new version. This includes a
tag that fails the release checks: its version number is used up even though nothing was
published.

## License

Copyright 2026 Hypotheosis. Licensed under the [Apache License, Version 2.0](LICENSE); see
[`NOTICE`](NOTICE). You may use, modify, and redistribute claims-mcp, including commercially,
under the terms of that license. The license grants no rights to the Hypotheosis name or
trademarks.
