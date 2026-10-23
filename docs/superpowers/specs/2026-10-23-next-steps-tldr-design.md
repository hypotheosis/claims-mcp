# Next-steps TL;DR Design

Date: 2026-10-23
Status: Approved (brainstorming), pending spec review

## 1. Purpose

`audit_report` and the PDF report list every gap in a catalog. On a large catalog that is too
much to act on: `bdc.json` (bolt_deployment_controller) has 1,049 active claims, 40 refuted and
378 `not_enough_info`. The next step for almost every one of them is already written down: 407 of
the 418 non-supported verdict rationales end with `Fix: <step>`. But those steps are spread across
hundreds of records.

The TL;DR mode turns a catalog into a short, ranked list of exact next steps.

## 2. Surfaces

One shared function, two entry points:

- `report::next_steps::next_steps(catalog: &Catalog, limit: usize) -> NextSteps` in
  `src/report/next_steps.rs`, a submodule of `src/report.rs` that reuses its child index and
  its `is_stale` / `is_high_integrity` helpers.
- **MCP tool `next_steps`** with a `limit` parameter (optional, default 10). It returns
  `NextSteps` as JSON.
- **CLI subcommand `claims-mcp [--catalog PATH] tldr [--limit N] [--json]`**. It prints Markdown
  by default and the same JSON as the MCP tool with `--json`. It exits 0 even when there are
  gaps; the integrity banner (section 4, tier 0) appears in the output.

`audit_report` is unchanged.

`limit` caps the items listed in each of tiers 1 and 2. `0` means no cap.

## 3. Scope: actionable claims

Only **active leaf claims** are listed: active claims with no active children. A parent's verdict
is a roll-up of its children, so fixing the children fixes the parent. Active parents are counted
in `rollup_claims` and do not appear in any tier.

A leaf is **open** when any of these hold:

| Condition | Meaning (same rules as `Gaps` in spec section 4.1) |
|---|---|
| `refuted` | current verdict is `refuted` |
| `not_enough_info` | current verdict is `not_enough_info` |
| `unverified` | no verdict |
| `stale` | evidence newer than the latest verdict |
| `no_evidence` | no evidence at all |

A leaf that is `supported`, not stale and has evidence is done and is not listed.

## 4. Tiers

| Tier | Name | Contents |
|---|---|---|
| 0 | Stop | Present only when `seal::verify` fails: the catalog was edited outside the server, nothing below can be trusted, restore it from git. |
| 1 | Wrong | Open leaves whose current verdict is `refuted`. |
| 2 | Unproven, high integrity | Every other open leaf with integrity level 3 or 4. |
| 3 | Everything else | Remaining open leaves (integrity 1, 2 or unassigned). Counts only, per area. |

Order within tiers 1 and 2: integrity level descending (unassigned last), then claim id
ascending. When a tier has more than `limit` items, the first `limit` are listed and the rest are
reported as `omitted` with a per-area count.

**Area** is the suffix of the claim's first `area:` tag (`area:AUTH` gives `AUTH`), or
`(none)` when the claim has no such tag. Per-area counts are sorted by count descending, then
by area name.

## 5. The next step for each item

`next_step` is chosen in this order:

1. The latest verdict's rationale contains `Fix:`: the text after the **last** `Fix:`, trimmed.
   If that text is empty, fall through.
2. The latest verdict has a rationale: the first sentence (up to and including the first `. `,
   or the whole text), trimmed, of the text before the last `Fix:` (the whole rationale when
   there is no marker). If that is empty (for example the rationale is just `Fix:`), the step
   is `"Rationale names no next step; read the latest verdict."`.
3. No verdict: a fixed phrase for the gap, `"No evidence attached; gather evidence, then record a
   verdict."` or `"Evidence attached but no verdict; record one."`.

A stale claim whose step comes from rule 1 or 2 gets the prefix `"Re-verify (new evidence since
the last verdict): "`, because its rationale predates the evidence.

## 6. Output shape

```rust
pub struct NextSteps {
    pub integrity: IntegrityReport,   // tier 0 is rendered when !integrity.verified
    pub summary: NextStepsSummary,
    pub wrong: Tier,                  // tier 1
    pub unproven: Tier,               // tier 2
    pub other: Vec<AreaCount>,        // tier 3
}

pub struct NextStepsSummary {
    pub active_claims: usize,
    pub rollup_claims: usize,
    pub open_leaves: usize,
    pub done_leaves: usize,
}

pub struct Tier {
    pub total: usize,
    pub items: Vec<NextStep>,         // at most `limit` (all when limit is 0)
    pub omitted: Vec<AreaCount>,      // per-area counts of the items not listed
}

pub struct NextStep {
    pub id: ClaimId,
    pub integrity_level: Option<IntegrityLevel>,
    pub verdict: CurrentVerdict,
    pub stale: bool,
    pub area: String,
    pub statement: String,            // full; the Markdown renderer truncates
    pub next_step: String,
    pub issues: Vec<String>,          // `issue:NNN` tag suffixes, in tag order
}

pub struct AreaCount { pub area: String, pub count: usize }
```

### Markdown rendering (CLI)

```markdown
# TL;DR: claims.json

1049 active claims: 1038 leaves (631 done, 407 open), 11 roll-ups.

## 1. Wrong (32)

- **CLM-0612** IL4 [#360] No secret value survives redaction, across chunk boundaries.
  - Next: <full text after the last `Fix:`>

...and 22 more: PLAN 9, CHG-C 6, ...

## 2. Unproven, integrity 3-4 (266)

...

## 3. Everything else (109)

OPS 30, POL 21, ...
```

- Statements longer than 100 `char`s (Unicode scalar values) are cut to 100 with `...`
  appended.
- `next_step` is printed in full: it is the actionable part.
- Issue tags are shown as `[#360, #361]`. Without issues the bracket is omitted.
- When tier 0 applies, the document opens with a `## 0. STOP` section that names the problem
  count and says to restore the catalog from git.
- An empty tier prints `None.`

## 7. Making `Fix:` official

`skills/claims-audit/SKILL.md` says: a `refuted` or `not_enough_info` rationale must end with
`Fix: <one concrete next step>` (what to change, or what test or evidence would decide the claim).
The README documents that `next_steps` and `tldr` rely on it.

## 8. Complexity

Same pattern as `audit_report`: one `children_index` build (O(n + e)), one pass over the claims,
and a sort of each listed tier (O(k log k)). `Fix:` extraction is one `rfind` per rationale.
Only the first `limit` items per tier allocate owned strings; the overflow is counted in a
`BTreeMap<&str, usize>` that borrows the area from the tags.

## 9. Testing

Unit tests in `src/report.rs`:

- `next_step` extraction: one `Fix:`, several `Fix:` (last wins), `Fix:` with empty text (falls
  back to the first sentence), no `Fix:`, no verdict with and without evidence, the stale prefix.
- Tier assignment for every open condition and integrity level, and done leaves left out.
- Roll-up parents left out and counted; a parent whose only children are retired counts as a
  leaf.
- Sort order, `limit` truncation, `omitted` per-area counts, `limit = 0`.
- Area: first `area:` tag wins, `(none)` fallback.

Server test: `next_steps` round-trips through the MCP handler.

CLI tests in `src/main.rs`: Markdown rendering of a small catalog (tiers, truncation, issue
brackets, `None.`), the tier 0 banner for a failed integrity report, and multibyte statement
truncation.

## 10. Non-goals

- Grouping by issue, by area or by fix type. Areas appear only in counts.
- Writing anything to the catalog, or changing the schema (no `next_step` field on verdicts).
- Changing `audit_report` or the Typst PDF.
