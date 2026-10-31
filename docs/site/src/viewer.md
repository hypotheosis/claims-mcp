# Claims viewer

The [claims viewer](viewer/index.html) is a page for reading a `claims.json` in the browser. It is
for people reviewing an audit: what was claimed, what the verdicts are and why, what evidence
backs them, and how the claims fit together.

**Try it on [claims-mcp's own ledger](viewer/index.html?repo=hypotheosis/claims-mcp).**

## What it shows

- **Overview**: the share of active claims that are supported, counts per verdict, the claims
  that need attention (refuted, then not enough info, then unverified, highest integrity level
  first), and progress per `area:` tag.
- **Claims**: a searchable list with filters for verdict, area, kind, level and integrity level,
  and an option to include retired claims.
- **A claim**: its statement, metadata and tags (`issue:N` tags link to the repository's issues);
  its parent and child claims, so you can walk the hierarchy; the verdict history, newest first,
  with each rationale's `Fix:` step highlighted; and its evidence. File anchors link to the exact
  lines on GitHub at the commit they were pinned to. Search anchors show the `git grep` command
  they ran and its hit count.

Every view has its own address, so you can share a link to a filtered list or to one claim, and
the browser's back button works.

## Opening a ledger

- **From GitHub**: enter `owner/name` or a repository URL. Under "More options" you can choose a
  branch, tag or commit (the default branch otherwise) and a catalog path (`claims.json`
  otherwise).
- **From a file**: choose a `claims.json` or drop it on the page. The file is read in your browser
  and never uploaded.

A link can open a ledger directly:

```text
viewer/index.html?repo=owner/name
viewer/index.html?repo=owner/name&ref=release-1.2&path=audit/claims.json
```

## Limits

- **Public repositories only.** The viewer fetches files from `raw.githubusercontent.com` without
  credentials. It doesn't accept access tokens: every project site under one `github.io` account
  shares a browser origin, so a stored token could be read by other projects' pages. For a
  private repository, download the file and open it from disk.
- **It shows the ledger as recorded.** It doesn't check [integrity seals](concepts/seals.md),
  recompute audit gaps or re-run anchors. Use `claims-mcp verify`, `claims-mcp recheck` and
  `claims-mcp tldr` for those; see [Your first audit](guides/first-audit.md).
- **Read-only.** Change a ledger only through the MCP tools.
