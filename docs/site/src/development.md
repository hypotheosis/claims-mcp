# Development

claims-mcp is a Rust crate (minimum Rust version in `Cargo.toml`'s `rust-version`) built on the
[`rmcp`](https://crates.io/crates/rmcp) crate. Common tasks use [`just`](https://github.com/casey/just):

```bash
just fmt          # cargo fmt
just clippy       # cargo clippy --all-targets -- -D warnings
just test         # cargo test
just report       # render audit.pdf from the repository's own claims.json
just report full  # render audit-full.pdf, every record
just docs         # build this site into docs/site/book
just docs-serve   # serve it with live reload
```

The repository's own `.mcp.json` runs the server with `cargo run --release`, so the first start
compiles it. Run `cargo build --release` once beforehand to avoid a client startup timeout.

claims-mcp audits itself: `claims.json` in the repository holds claims about its own
specification and documentation, including this site.

## Template checks

`cargo test typst_template` checks that the PDF template's totals, gaps and matrix match
`audit_report` in both detail levels, and that its text helpers pass `report/text-tests.typ`.
When `typst` is not installed, that check is skipped and still counts as passed. Set
`REQUIRE_TYPST=1` (as CI does) to make a missing `typst` fail the tests instead.

## Continuous integration

`.github/workflows/ci.yml` runs on every pull request and push to `main`:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, a build with
  `RUSTFLAGS=-D warnings`, and `cargo test` with Typst installed and `REQUIRE_TYPST=1`;
- `claims-mcp verify`, which fails if `claims.json` was edited outside the tools;
- `cargo test` on Windows and macOS, and `cargo check` on the minimum supported Rust version;
- `just report` and `just report full` on the PR head commit. Both PDFs are uploaded as the
  run's `audit-report-pr-<number>-<commit>` artifact; open the run's summary page to download
  it.

`.github/workflows/docs.yml` builds this site and checks its internal links on pull requests
that touch `docs/site/`, and deploys it to GitHub Pages from `main`.

## Releasing

1. In a pull request, set `version` in `Cargo.toml` to the new version, update
   `report/audit-notes.toml` to match, and merge. `main` only accepts changes through pull
   requests that pass CI and are up to date with `main`.
2. Tag the merge commit and push the tag:

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

`.github/workflows/release.yml` checks that the tag equals `v` plus the `Cargo.toml` version and
points at a commit on `main` (and fails without publishing otherwise). It reruns CI on the tagged
commit, builds the five archives listed under [Install](guides/install.md#download-a-release),
and publishes a GitHub release with the archives, their `.sha256` files and the audit report. A
pull request that changes `release.yml` runs the builds as a dry run without publishing.

Release tags (`v*`) can't be moved or deleted once pushed, so a bad release is fixed with a new
version. That includes a tag that fails the release checks: its version number is used up even
though nothing was published.

The release also includes `claims.schema.json`. If the catalog format changes, bump
`SCHEMA_VERSION` in `src/catalog.rs` and run `just schema` in the same pull request. That command
regenerates the latest schema and creates the current format-version-pinned copy without changing
older pins.

## Merging pull requests

Merge pull requests with a **merge commit**, not a squash or rebase merge. The anchors in
`claims.json` pin the commits they were recorded on, which for a pull request are branch commits.
A squash or rebase merge rewrites them, and `recheck` would then report those anchors as
`commit_missing`; see
[Anchors recorded on a branch](concepts/anchors.md#anchors-recorded-on-a-branch).
