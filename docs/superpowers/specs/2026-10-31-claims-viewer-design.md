# Claims viewer: design

Date: 2026-10-31
Status: approved in conversation, awaiting spec review

## Goal

A web page, published with the documentation site, that loads a `claims.json` and makes it easy
to explore and read: what was claimed, what the verdicts are, why, on what evidence, and how
claims relate. It is for people reading an audit, not for running one. It does not check seals,
compute audit gaps, or change the catalog.

## Decisions

| Question | Decision |
|---|---|
| Scope | Display only: parse, browse, filter, count. No seal checking, no `audit_report` gap rules |
| Technology | Static HTML, vanilla JavaScript (ES modules) and Pico CSS with its own stylesheet; no build step, no framework, no Node toolchain for the site |
| Hosting | Files in `docs/site/src/viewer/`, which mdBook copies through unchanged, served at `<site>/viewer/` by the existing docs workflow |
| Sources | A public GitHub repository, or a local file (picker or drag and drop) |
| Private repositories | Not supported: no access tokens. Every project site under one `github.io` owner shares an origin, so a stored token would be readable by other projects' pages |
| Prior art | An earlier React prototype informs layout and wording; no code is carried over |

Rejected: compiling the Rust core to WASM (only needed for seal checking and exact gap rules,
which are out of scope), and porting the React prototype as is (framework, Node toolchain, and it
conflicts with the repository's front-end rules).

## Files

```
docs/site/src/viewer/
  index.html     page shell: header, source picker, overview, list, detail; loads Pico and fonts
  viewer.css     the viewer's own theme on top of Pico: typography, verdict colours, layout
  model.js       pure functions, no DOM: parse and normalise a catalog, filter, count, hierarchy,
                 anchor links
  source.js      loading: GitHub fetch, local file read; turns failures into readable messages
  app.js         DOM and state: renders views, wires events, keeps the URL hash in sync
docs/site/src/viewer.md      docs page introducing the viewer, linked from SUMMARY.md
docs/site/viewer-tests/      node --test unit tests for model.js (outside src/, so not published)
```

`model.js` and `source.js` are ES modules with no browser globals at import time, so Node can
import `model.js` for tests. `app.js` is the only file that touches the DOM.

## Loading a catalog

The start view offers two sources.

- **GitHub repository**: a text field accepting `owner/name` or a `https://github.com/owner/name`
  URL, plus optional fields (behind a "More options" disclosure) for the branch or commit
  (default `HEAD`, the default branch) and the catalog path (default `claims.json`). The viewer
  fetches `https://raw.githubusercontent.com/<owner>/<name>/<ref>/<path>`. That host sends
  `Access-Control-Allow-Origin: *` and costs no GitHub API rate limit.
- **Local file**: a file input and a drop zone that accept one JSON file. Works in every
  browser; the file never leaves the machine.

Query parameters open a catalog directly: `?repo=owner/name`, with optional `&ref=` and
`&path=`. The docs page links `viewer/?repo=hypotheosis/claims-mcp` as a live example of this
project's own ledger.

Errors are shown in place with a plain explanation: repository or file not found (404, with a
hint that private repositories aren't supported), network failure, invalid JSON, or JSON that
isn't a claims catalog (no `claims` list).

## Parsing

`model.js` normalises the snake_case catalog into plain objects. It is tolerant: unknown fields
are ignored, missing optional fields get defaults, and only a missing `claims` list or a claim
without `id` and `statement` is an error, reported with the claim's index. It reads
`schema_version` 1 and 2. Seals are ignored.

Each claim gets:

- `current`: the latest verdict, or `unverified`;
- `areas`: the values of its `area:<name>` tags, or `["Unassigned"]`;
- `issues`: the numbers from its `issue:<N>` tags;
- `children`: ids of active claims whose `derived_from` contains it (built in one pass, O(n + e)).

## Views

The header shows the source (repository and ref, or file name), a link back to the start view,
and a light/dark theme toggle.

### Overview

- Totals: active claims, the share supported, and counts per current verdict, with a stacked bar.
- **Needs attention**: active claims that are refuted, then not enough info, then unverified,
  each sorted by integrity level (highest first), then id. Each row links to the claim.
- **By area**: per `area:` value, supported out of total, with a stacked bar; clicking an area
  filters the list.

### Claims list

A search box (case-insensitive substring of id, statement and tags) and filters: current
verdict, kind, level, integrity level, area. A toggle includes retired claims, shown muted.
Rows show id, statement, verdict badge and integrity level. Filter values come from the loaded
catalog, so the viewer doesn't hard-code the enums.

### Claim detail

- Id, statement, current verdict badge, kind, level, integrity level, verification method,
  source, tags (`issue:N` linked to the repository's issue N when loaded from GitHub), and
  artifacts.
- Retired claims show a banner with the reason and date.
- **Hierarchy**: parents (`derived_from`) and children as links, so the tree can be walked.
- **Verdict history**, newest first: verdict, auditor, date, rationale, and the cited evidence
  ids, which link to the evidence below. The `Fix:` line of a rationale is highlighted.
- **Evidence**: id, kind, stance badge, locator, summary and date. A `file` anchor renders as a
  link to `https://github.com/<owner>/<name>/blob/<commit>/<path>#L<start>-L<end>` when loaded
  from GitHub, and as text otherwise. A `search` anchor shows the `git grep` command, the hit
  count and the commit. `dirty` anchors carry a note that they were recorded on uncommitted
  changes.
- The claim's `source_anchor` renders the same way as a file anchor.

## Navigation and state

The URL hash holds the view and selection (`#overview`, `#claims?verdict=refuted&area=docs`,
`#CLM-0072`), so views are shareable and the back button works. Query parameters select the
source; the hash selects what to show within it. Rendering is idempotent from the hash plus the
loaded catalog.

## Look

- Pico CSS 2.1.1 from jsDelivr, pinned with a Subresource Integrity hash
  (`sha384-L1dWfspMTHU/ApYnFiMz2QID/PlP1xCW9visvBdbEkOLkSSWsP6ZJWhPw6apiXxU`), plus
  `viewer.css` that sets the viewer's own palette and type, so it doesn't look like stock Pico.
- Fonts from Google Fonts: a distinctive sans for headings and a readable one for body text,
  plus a monospace for ids, locators and commands.
- Adaptive light and dark themes following the system setting, with a toggle in the header; the
  choice is remembered in `localStorage` (a per-viewer convenience; the page works without it).
- Verdict colours are consistent everywhere and paired with text labels, never colour alone.
- Layout works from phone width up: list and detail stack on narrow screens and sit side by
  side on wide ones.

## Documentation

`docs/site/src/viewer.md` ("Claims viewer") explains what the viewer shows, how to open a
repository or file, that private repositories aren't supported and why, and links to the
viewer and the live example. `SUMMARY.md` lists it after the Guides. The README's
"Documentation" list gains a line for it.

## Testing

- `docs/site/viewer-tests/model.test.mjs`, run with `node --test docs/site/viewer-tests/`
  (Node's built-in runner, no packages):
  - parsing: a minimal catalog, a full one with every optional field, schema version 1,
    unknown fields ignored, and each error case;
  - `current`, `areas`, `issues` and `children` derivation, including retired parents;
  - filters and search;
  - counts and the needs-attention ordering;
  - anchor link building for file anchors with and without lines, and search anchors;
  - a smoke test that parses the repository's own `claims.json` and checks its active-claim count
    against `summary.active_claims` in `claims-mcp tldr --json` output. The test reads that output
    from the file named by the `CLAIMS_TLDR_JSON` environment variable, and is skipped, with a
    message, when the variable is unset.
- `ci.yml`'s `fmt, clippy, build, test` job, which already builds the binary, gains two steps:
  write `claims-mcp tldr --json` to a file, then run `node --test docs/site/viewer-tests/` with
  `CLAIMS_TLDR_JSON` pointing at it. Node is preinstalled on GitHub's Ubuntu runners, so no setup
  step or packages are needed. A `just viewer-test` recipe does the same locally.
- The existing lychee link check covers the new docs page and the viewer's relative links.
- A manual check before the PR: open the built site locally with `just docs-serve`, load this
  repository's ledger from GitHub and from a file, in light and dark modes, at phone and desktop
  widths.

## Out of scope

- Seal checking, audit gaps, the ranked to-do list and anchor rechecking (the CLI and the PDF
  report cover these).
- Private repositories and access tokens.
- Editing the catalog.
- Loading more than one catalog at once, or comparing catalogs.
