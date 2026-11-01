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
- `just release-test` and `shellcheck` on the release scripts, and on a `release/v*` pull
  request a check that the branch names the `Cargo.toml` version (see [Releasing](#releasing));
- `just report` and `just report full` on the PR head commit. Both PDFs are uploaded as the
  run's `audit-report-pr-<number>-<commit>` artifact; open the run's summary page to download
  it.

`.github/workflows/docs.yml` builds this site and checks its internal links on pull requests
that touch `docs/site/`, and deploys it to GitHub Pages from `main`.

## Releasing

Releases are cut by three `just` recipes, the same way every time, with no hand-written text.
They need [cargo-release](https://github.com/crate-ci/cargo-release) and an authenticated `gh`.

```bash
just release-prepare patch   # or minor, major, or an exact version such as 0.3.0
just release-ship
```

`just release-prepare <level>` runs cargo-release with `release.toml`. It makes one local
commit, `chore: Release vX.Y.Z`, which sets the version in `Cargo.toml`, `Cargo.lock` and
`report/audit-notes.toml` and adds the release's changelog entry, and an annotated tag
`vX.Y.Z` on it. It pushes nothing. It refuses to run off `main`, with a dirty working tree, or
when local `main` is not exactly `origin/main`.

`just release-ship` pushes the release commit to `release/vX.Y.Z` and opens a pull request,
then:

1. waits for its checks, and stops if one fails;
2. waits for the review bot's review, and stops if any review thread is unresolved;
3. closes the pull request and stops if `main` moved meanwhile;
4. merges it with a merge commit, moves the local tag to that merge commit, pushes the tag, and
   fast-forwards local `main`. The tag starts `release.yml`, which publishes the release.

When it stops on a check or a review, the pull request stays open and no tag is pushed (the
local tag from `just release-prepare` stays where it is). If the pull request merges but the
tag push fails, it stops and prints the commands that finish the release:
`git push origin refs/tags/vX.Y.Z && git merge --ff-only origin/main`. It refuses to start unless
`HEAD` carries the tag for the `Cargo.toml` version, that tag is not on `origin`, and `HEAD` is a
single-parent commit directly on the current `origin/main`.

`just release-abort` abandons a release that has not been published: it closes the release pull
request and deletes its branch if they exist, deletes the local tag, and resets local `main` to
`origin/main`. It refuses off `main`, with a dirty working tree, when `HEAD` does not carry the
version's tag, when the tag is already on `origin` or the pull request has merged, and when local
`main` has commits besides the release commit that are not on `origin`. After a stopped ship,
land the fix through its own pull request, then abort, prepare and ship again.

### Release notes

`scripts/release-notes.sh <from> <to>` (or `just release-notes <from> <to>`) prints the notes
from git history alone: one line per pull request merged into `main` in the range, linked to it
and grouped by the conventional-commit type of its title (Breaking changes, Features, Fixes,
Documentation, Dependencies, and Other, as written). The release's own pull request is left out.
`just release-prepare` writes them, from the previous tag to `HEAD`, as the changelog entry
`# Release vX.Y.Z`; `release.yml` publishes them, from the previous tag to the new one, as the
GitHub release body. The two cover the same pull requests, so they are the same text.

`report/audit-notes.toml` needs no edit per release: its assessment is fixed, and
`just release-prepare` sets its `version`. The report computes counts, verdicts and gaps from the
ledger.

### Checks

`.github/workflows/release.yml` checks that the tag equals `v` plus the `Cargo.toml` version and
points at a commit on `main` (and fails without publishing otherwise). It reruns CI on the tagged
commit, builds the five archives listed under [Install](guides/install.md#download-a-release),
and publishes a GitHub release with the archives, their `.sha256` files and the audit report. A
pull request that changes `release.yml` runs the builds as a dry run without publishing. CI also
fails a `release/v*` pull request whose branch does not name its `Cargo.toml` version.

`just release-test` tests the scripts against throwaway git repositories and a fake `gh`,
including the real cargo-release when it is installed. CI runs it in the `release scripts` job.

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
