# Anchors and recheck

An evidence `locator` is free text. To make evidence point at an exact, checkable place, add an
**anchor**. The server pins it to the commit checked out at the time.

| Anchor | Shape | Use it for |
|---|---|---|
| file | `{"file": {"path": "src/server.rs", "lines": {"start": 40, "end": 61}}}` | Code or text that shows the finding; omit `lines` for the whole file |
| search | `{"search": {"pattern": "rate.?limit\|throttl", "case_insensitive": true, "scope": ["src"]}}` | A `git grep -E` search; with zero hits, it shows that something does not exist |

Claims take a `source_anchor` (a file anchor) on the text that makes them, such as the README
line that promises a feature. If the claim is refuted, that is the text to fix.

## What the server records

The server, not the caller, records the commit (`git rev-parse HEAD`). For a search, it runs
`git grep` itself and records the hit count; a `hits` value the caller passes must match.

It rejects paths outside the repository, line ranges past the end of the file, and search scopes
that don't exist. A misspelled directory would otherwise "prove" anything absent. Searches and
the dirty check skip the catalog file and its lock and temporary files, so a committed
`claims.json` never matches its own recorded patterns.

An anchor on a file with uncommitted changes is marked `dirty`. Anchors need the catalog to be
inside a git repository; evidence without one works anywhere.

## Recheck

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

The command prints one line per anchor that isn't current and exits 1 if there are any.
`--json` prints the same data as the tool, which the [PDF report](../guides/reports.md) takes as
its `recheck` input.

A `moved` anchor still holds; re-record it if you want current line numbers. Anything
`changed`, `overturned`, `missing` or `commit_missing` means the evidence no longer describes the
code, so re-audit that claim.

## Anchors recorded on a branch

An anchor pins whatever commit is checked out, so an audit run on a feature branch pins
branch commits. A true merge commit keeps them reachable from the main branch. A squash or
rebase merge rewrites them, and once the branch is deleted, fresh clones report every such
anchor as `commit_missing`. Either merge pull requests that carry anchored evidence with a
merge commit, or re-record the evidence on the main branch after merging.
