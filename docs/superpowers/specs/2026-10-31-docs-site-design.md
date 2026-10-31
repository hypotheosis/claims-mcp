# claims-mcp documentation site: design

Date: 2026-10-31
Status: approved in conversation, awaiting spec review

## Goal

Publish a documentation site for claims-mcp on GitHub Pages. It is written first for
**evaluators**, people deciding whether claims-mcp fits their problem, and second for adopters.
It explains what claims-mcp is, where it fits, and how it works alongside formal verification:
where it adds value when formal proofs fall short, and where it doesn't. The site becomes the
single canonical home of the detailed documentation, and the README shrinks to a front door.

## Decisions

| Question | Decision |
|---|---|
| Primary reader | Evaluators first, then adopters |
| README | Slimmed to a front door; the site is canonical for detail |
| Generator | mdBook (Rust-native single binary, Markdown, built-in search and themes) |
| Hosting | GitHub Pages, source "GitHub Actions", deployed from `main` |
| Formal-verification content | A dedicated page under "Why claims-mcp", citing public sources only |
| Dogfooding | The site's factual statements become claims in the repo's `claims.json` |

Rejected: MkDocs Material/Zensical (Python toolchain in a Rust repo, upstream churn) and Astro
Starlight (Node toolchain, heaviest to maintain). Both look more polished, but mdBook keeps one
toolchain and suits content-first pages.

## Layout in the repository

```
docs/site/
  book.toml            mdBook config: title, repo link, edit-url template, search on
  src/
    SUMMARY.md         sidebar, in the order below
    introduction.md
    why/…              problem.md, formal-verification.md, limits.md
    concepts/…         claims.md, evidence.md, verdicts.md, integrity-and-methods.md,
                       traceability.md, seals.md, anchors.md
    guides/…           install.md, first-audit.md, ci.md, reports.md, migrating.md
    reference/…        tools.md, cli.md, catalog-format.md, security.md
    development.md
```

`docs/site/book/` (build output) is git-ignored. `docs/superpowers/` and `docs/reference/` stay
as they are and are not part of the site.

## Page contents

### 1. Introduction (`introduction.md`)

What claims-mcp is, in one screen. A 30-second worked example: one atomic claim, one piece of
evidence with a commit-pinned anchor, a `supported` verdict citing it, and what `recheck` reports
after the anchored lines change. Links to "Why claims-mcp" and to the install guide.

### 2. Why claims-mcp

- **The problem (`why/problem.md`)**: docs drift from code, LLM output is accepted unchecked,
  and requirement-to-artifact traceability is lost. Shows how a ledger of falsifiable claims
  with evidence and append-only verdicts addresses each one.
- **Alongside formal verification (`why/formal-verification.md`)**: the page this work started
  from.
  - What formal verification gives you: for example Rust translated to Lean by Aeneas, then a
    machine-checked proof for every input, trusted through a small kernel.
  - What it can't tell you, and where claims-mcp covers it:
    - *the spec gap*: does the formal spec state the requirement? A proof can be valid and
      say nothing meaningful;
    - *the model gap*: does the model behave like the shipped binary?
    - *the trusted base*: escape hatches such as `sorry`, `admit` and `axiom`, the
      translator, and the compiler;
    - *coverage*: I/O, async, concurrency, `unsafe`, configuration, documentation and LLM
      output, which proofs rarely reach.
  - A side-by-side comparison table: question answered, scope, who judges, drift, cost.
  - *Using them together*: a proof as the evidence for a high-integrity claim with the
    `formal_proof` method; a zero-hit search anchor for `sorry|admit|axiom` (an absence
    refutation that `recheck` reports as `overturned` if one appears); a differential-testing
    run as evidence that the model matches the binary; and a spec-fidelity claim judged by a
    reviewer other than the spec's author. This mirrors the four questions Leo de Moura
    suggests for vetting a "formally verified" claim.
  - LLMs now write Lean proofs, which makes proofs cheaper and auditing the spec more
    important.
  - Sources, all public: the Aeneas repository, lean-lang.org, and the Data Exchange interview
    with Leo de Moura (2026-10).
- **Where it doesn't fit (`why/limits.md`)**: honest limits. A verdict is an auditor's judgment
  backed by cited evidence, not a proof. The server records and never runs your code. The
  ledger's quality is the evidence's quality. Seals deter casual hand edits but are not
  cryptographic security: the HMAC key is in the public source.

### 3. Concepts

One page each, moved and expanded from the README and the design spec: claims (atomic,
falsifiable, kinds, levels); evidence (locator, stance, anchors); FEVER verdicts (append-only
history, must cite evidence); integrity levels 1-4 and verification methods (IEEE 1012);
traceability (`derived_from`, `trace`, the matrix); integrity seals; anchors and recheck
(statuses `moved`, `changed`, `overturned`, `missing`, `commit_missing`, `unpinned`).

### 4. Guides

Install and register with Claude Code (from README "Install"); running a first audit with the
`claims-audit` skill; checking seals in CI; generating the PDF report; migrating an existing
catalog with `seal --adopt`.

### 5. Reference

MCP tools (the README table, one subsection per tool), the CLI (`verify`, `tldr`, `recheck`,
`seal`, `report init`, and running the server), the catalog format, and security (from
`SECURITY.md`, linked rather than duplicated where possible).

### 6. Development

Building, the pre-commit checklist, CI, and releasing (from the README's "Development").

## The slimmed README

About 80 lines, in this order:

1. What claims-mcp is (the current opening paragraph and bullets).
2. "When to use it": three or four lines, including two on formal verification, linking to
   the site's comparison page.
3. Quickstart: one install path (release download), registering with Claude Code, installing
   the skill, with each linking to the full guide.
4. A links section into the site: concepts, guides, reference.
5. License.

Every section removed from the README must exist on the site before the README change lands.
Release archives keep shipping `README.md`; it stays useful offline because it links to the
site and to `skills/claims-audit/SKILL.md`.

## Build and deployment

- New workflow `.github/workflows/docs.yml`:
  - On pull requests touching `docs/site/**` or the workflow: build only (`mdbook build`),
    failing on broken internal links (mdBook's link checking, or a pinned link checker on the
    output).
  - On pushes to `main`: build, then `actions/upload-pages-artifact` and
    `actions/deploy-pages`, with `permissions: pages: write, id-token: write` on the deploy
    job only.
  - Following the existing workflows: actions pinned by commit SHA with a version comment,
    a pinned exact mdBook version installed from its release tarball and verified by
    checksum, and `concurrency` set so only the latest deploy runs.
- `just docs` builds the site locally, and `just docs-serve` runs `mdbook serve`.
- The site URL goes in `book.toml`, the README, and the repository's "homepage" field.

**Repository settings (outward-facing, done only with the owner's go-ahead at implementation
time):** enable Pages with source "GitHub Actions", and set the repository homepage to the
site URL.

## Dogfooding: claims for the docs

The site's checkable statements become claims in the repo's `claims.json`, added through the
claims MCP tools and never by hand edits:

- One parent claim, "The documentation site describes the shipped behaviour", with child
  claims for concrete statements: tool names and parameters, CLI commands and exit codes,
  recheck statuses, the seal behaviour on hand edits.
- Each claim gets a `source_anchor` on the docs line that makes it, and evidence with a file
  anchor into `src/` that supports it, so `recheck` flags drift on either side.
- Positioning statements ("formal proofs can't show spec fidelity") are not code claims and are
  not registered.

## Testing and acceptance

- `mdbook build` succeeds with no warnings and no broken internal links, locally and in CI.
- Every README section removed has a matching site page (checked against the old README's
  heading list).
- The formal-verification page cites only public sources and contains no private hostnames,
  people or systems.
- `claims-mcp verify` passes after the docs claims are added, and `claims-mcp recheck` reports
  every new anchor as current.
- After the first deploy, the Pages URL serves the site and search works.
- A changelog entry under `changelog/` records the change, as the repo's conventions require.

## Out of scope

- A custom theme, logo or landing-page hero (mdBook defaults, light and dark).
- Versioned docs per release.
- API docs from `cargo doc` (the crate is a binary; the MCP tools and CLI are the interface).
- Translating the Typst report docs beyond the existing guide content.
