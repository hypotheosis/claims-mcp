# PDF report: summary and full detail modes

## Problem

Rendering a large catalog (`bdc.json`: 1,265 claims, 4,094 evidence items) with
`report/claims-report.typ` produces a 1,869-page PDF that nobody can read:

- **Length.** Retired claims take 18 pages, Findings about 230 (418 refuted or not enough
  info), the traceability matrix about 160, and the evidence appendix about 1,400.
- **Overflow.** Source cells in the matrix (`` `architecture.md:2481-2486,2498-2508` ``) have no
  wrap points and run into the Parents column. Markdown backticks print literally.
- **Bunching.** Evidence tables have six columns on a portrait page; `auto` columns for kind,
  stance, and the timestamp squeeze Locator and Summary to about 15 characters.
- **ID walls.** Roll-up evidence locators (`children: CLM-0591, CLM-0592, ...`, up to 197 IDs),
  roll-up summaries that repeat child lists, and the gap summary's Claims column (378 IDs for
  `not_enough_info`). The roll-up locator format comes from `skills/claims-audit/SKILL.md`.

## Goals

- A short summary PDF for most readers, and a full PDF for a reviewer (for example a security
  consultant) who wants every evidence item and verdict.
- Both from one template, selected by an input.
- Legible layout in both modes, including on catalogs already written in the old roll-up
  format.
- The `<audit-report>` metadata and its parity with `audit_report` do not change.

## Design

### The `detail` input

`--input detail=summary|full`, default `summary`. Any other value fails the compile with a
message naming the accepted values.

### Section contents

| Section | `summary` | `full` |
|---|---|---|
| Cover, Result, Assessment | unchanged | unchanged |
| Retired claims | total, then one row per distinct reason with its count; no per-claim table | per-claim table, with wrapping fixed |
| Integrity | status line; adopted records as a count | status line; adopted IDs as compressed ranges |
| Findings | grouped by area (below): refuted claims as cards, not-enough-info claims as table rows (ID, statement, IL, `Fix:` step) | grouped by area; every finding as a card with all cited evidence |
| Points for the evaluator, Coverage | unchanged | unchanged |
| Gap summary | gap, meaning, count | same table, then each non-empty gap's IDs as compressed ranges |
| Claim hierarchy | omitted | tree, unchanged |
| Traceability matrix | omitted | kept, with wrapping fixed and roll-up locators collapsed |
| Evidence appendix | omitted | kept, each evidence item as a full-width block |
| Reproduce | adds the `--input detail=full` command | unchanged plus the `detail` flag |

The `Fix:` step is the text after the last `Fix:` at a word boundary in the latest rationale,
the same rule `next_steps` uses. A rationale with no `Fix:` shows a dash.

### Areas and findings grouping

A claim's **area** is its ancestor exactly one level below a root, reached by following the
first active parent in `derived_from` at each step. An area claim is its own area. A root
claim and claims in a catalog with no second level belong to a single unnamed group.

Findings are the active refuted and not-enough-info claims. Area claims, and root claims that
have active children, are not listed as findings (a root's verdict is the overall result); each heads its group instead, showing its ID, statement, verdict marker, and the
group's finding counts by verdict. Counts are computed from the catalog at render time, never
read from roll-up text. Groups appear in catalog order of their area claims, and groups with no
findings are omitted.

### ID lists

One helper renders a list of claim IDs as compressed ranges: consecutive sequence numbers
collapse to `CLM-0591–0598` (en dash, second ID without its prefix), and ranges and single IDs
are separated by commas.

- **Roll-up locators.** An evidence locator matching `children:` followed by IDs (old format)
  or `children of CLM-NNNN` (new format) renders as
  `children of CLM-0011: 94 active (63 supported, 24 not enough info, 7 refuted: CLM-0594,
  ...)`. Counts and the named children come from the catalog's current active children of the
  claim that owns the evidence; only children that are not supported are named, as compressed
  ranges.
- **Free text.** In summaries, rationales, and reasons, any run of 6 or more claim IDs separated
  by commas (optional spaces) is replaced by its compressed-range form. Other text is unchanged.

### Text rendering

- Backtick spans in statements, sources, reasons, summaries, and rationales render as inline
  code.
- Every free-text table cell gets zero-width break opportunities after `/ _ . : , = -` (the
  existing `wrappable` helper), so no cell overflows into its neighbour.
- In full mode, each evidence item renders as a full-width block: a header line
  `EVD-NNNN · kind · stance · recorded_at`, then the locator, then the summary. This replaces
  the six-column evidence tables in finding cards and the appendix.

### Plumbing

- `justfile`: `report` gains `detail="summary"` and passes `--input detail={{detail}}`.
- CI (`.github/workflows/ci.yml`) renders `audit.pdf` (summary) and `audit-full.pdf` (full);
  the release attaches both.
- README: document the `detail` input and the two release PDFs.
- `skills/claims-audit/SKILL.md`: roll-up evidence uses locator `children of CLM-NNNN` and
  explains coverage in the summary; refuting roll-up evidence names the refuted children in the
  summary. Child lists no longer go in the locator.
- A changelog entry under `changelog/`.

## Testing

In `src/report.rs`:

- `assert_template_matches` compiles each fixture in both modes; each compile must succeed with
  no warnings, and `<audit-report>` must equal `audit_report` in both.
- The template exposes `<report-layout>` metadata: each area with its finding IDs, and each
  rendered roll-up locator string, keyed by evidence ID. Tests query it to check grouping, range
  compression, and locator collapsing without parsing the PDF.
- New fixture: one root, two areas, a roll-up parent whose evidence has an old-style `children:`
  locator listing 10 or more IDs, a refuted leaf, and a not-enough-info leaf.
- A test that `detail=bogus` fails to compile.

Manual check: render `bdc.json` in both modes, compare page counts with the current 1,869, and
inspect sample pages from each section. Summary mode should come in under about 60 pages.

## Out of scope

- Rewriting existing catalogs' roll-up evidence (it is sealed; the report handles the old
  format).
- Any change to `audit_report`, `next_steps`, or the catalog schema.
