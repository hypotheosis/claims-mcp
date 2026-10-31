# Traceability

Claims form a hierarchy. Each claim can name parent claims in `derived_from`, from a
system-level requirement ("the export feature works") down to unit and artifact claims ("the
CSV writer quotes fields containing commas").

## Rules

- Every `derived_from` id must exist. A new or updated link can't point at a retired claim; an
  existing link to a parent retired later is kept and reported as `dangling_links`.
- Links never form a cycle, including a claim linking to itself.
- `add_claims` registers a whole hierarchy in one all-or-nothing call. Within the batch,
  `"derived_from": ["@0"]` refers to the entry at index 0, which must come earlier.

## Walking the hierarchy

`trace` follows links from a claim: `up` to its ancestors, `down` to its active descendants, or
`both` (the default), optionally limited by `max_depth`. Each claim appears once, at its
shortest depth, with its current verdict.

`get_claim` also lists a claim's active children.

## The audit report

`audit_report` covers active claims and returns:

- **Totals**: counts by current verdict, kind, level and integrity level.
- **Gaps**, each a list of claim ids:

  | Gap | Meaning |
  |---|---|
  | `no_evidence` | No evidence at all |
  | `unverified` | No verdict |
  | `refuted` | Current verdict is `refuted` |
  | `not_enough_info` | Current verdict is `not_enough_info` |
  | `stale` | Evidence was recorded after the latest verdict |
  | `rigor` | Integrity level 3-4 verified only by inspection |
  | `decomposition` | Has active children, and at least one isn't `supported` |
  | `dangling_links` | Links to a retired parent |
  | `refuted_without_source_anchor` | Refuted, with nothing pointing at the text that makes the claim |

- **Integrity**: the result of checking every [seal](seals.md).
- **Traceability matrix** (unless `include_matrix` is false): one row per active claim with its
  parents, children, verification method, evidence locators, refuting locators and current
  verdict.

On a large catalog, the report is too long to act on. `next_steps` (and `claims-mcp tldr`)
reduce it to a ranked to-do list; see [Your first audit](../guides/first-audit.md).
