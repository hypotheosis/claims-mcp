# MCP tools

All tools return structured JSON. Domain errors, such as a verdict citing the wrong evidence,
come back as tool errors (`is_error: true`) with a clear message, so the agent can recover.
Every call verifies the catalog's [integrity seals](../concepts/seals.md); while any check
fails, write tools refuse and read tools keep working.

| Tool | Purpose | Writes |
|---|---|---|
| [`add_claim`](#add_claim) | Create one claim | yes |
| [`add_claims`](#add_claims) | Create many claims atomically; `@N` in `derived_from` links to an earlier entry | yes |
| [`get_claim`](#get_claim) | One claim with evidence, verdict history, current verdict and children | no |
| [`list_claims`](#list_claims) | Filtered summaries | no |
| [`update_claim`](#update_claim) | Edit metadata; evidence and verdicts can't be changed | yes |
| [`retire_claim`](#retire_claim) | Withdraw a claim; it stays so trace links remain valid | yes |
| [`add_evidence`](#add_evidence) | Attach evidence with a locator, an optional anchor and a stance | yes |
| [`record_verdict`](#record_verdict) | Append a FEVER verdict that cites evidence backing it | yes |
| [`trace`](#trace) | Ancestors and descendants of a claim | no |
| [`audit_report`](#audit_report) | Totals, gaps, integrity and the traceability matrix | no |
| [`next_steps`](#next_steps) | Ranked to-do list: open claims in tiers, each with one next step | no |
| [`recheck`](#recheck) | Anchors that moved, changed or were overturned since they were recorded | no |

## add_claim

| Parameter | Required | Meaning |
|---|---|---|
| `statement` | yes | One atomic, falsifiable assertion |
| `kind` | yes | See [kinds](../concepts/claims.md#kinds) |
| `level` | yes | `system`, `software`, `component`, `unit` or `artifact` |
| `verification_method` | yes | `inspection`, `analysis`, `demonstration`, `test` or `formal_proof` |
| `integrity_level` | no | 1-4 |
| `source` | no | Where the claim came from |
| `source_anchor` | no | `{"path", "lines": {"start", "end"}}`; pinned to the current commit |
| `derived_from` | no | Parent claim ids |
| `artifacts` | no | Paths the claim is about |
| `tags` | no | Labels, such as `area:auth` or `issue:12` |

Returns the full claim with its new id.

## add_claims

`claims`: a list of entries shaped like `add_claim`'s parameters. All are created or none are.
In `derived_from`, `"@N"` refers to the entry at 0-based index N of the same batch, which must
come earlier. Returns the created claims in order.

## get_claim

`id`. Returns the claim plus `current_verdict` and `children` (ids of active claims whose
`derived_from` contains it).

## list_claims

All filters are optional, and all set filters must match: `verdict` (`unverified`, `supported`,
`refuted`, `not_enough_info`), `kind`, `level`, `tag` (exact), `artifact` (substring of any
artifact), `text` (case-insensitive substring of the statement), `include_retired` (default
false). Returns summaries: `id`, `statement`, `kind`, `level`, `integrity_level`,
`current_verdict`, `evidence_count`, `retired`.

## update_claim

`id`, plus any of `statement`, `kind`, `level`, `verification_method`, `integrity_level`,
`source`, `source_anchor`, `derived_from`, `artifacts`, `tags`. Omitted or null fields are
unchanged. List fields replace the whole list (`[]` empties it). `source: ""` removes the
source. `integrity_level` and `source_anchor` can be replaced but not removed. A new
`derived_from` link must point at an active claim. Retired claims can't be updated.

## retire_claim

`id`, `reason`. Retired claims stay in the catalog but accept no further evidence, verdicts or
updates.

## add_evidence

| Parameter | Required | Meaning |
|---|---|---|
| `claim_id` | yes | The claim the evidence is about |
| `kind` | yes | `test`, `inspection`, `analysis`, `demonstration`, `proof`, `log` or `other` |
| `locator` | yes | `path:line`, test name, command or URL |
| `summary` | yes | What was observed |
| `stance` | yes | `supports`, `refutes` or `neutral` |
| `anchor` | no | `{"file": {"path", "lines"?}}` or `{"search": {"pattern", "case_insensitive"?, "scope", "hits"?}}`; see [Anchors](../concepts/anchors.md) |

Returns the evidence item with its id and pinned anchor.

## record_verdict

| Parameter | Required | Meaning |
|---|---|---|
| `claim_id` | yes | The claim being judged |
| `verdict` | yes | `supported`, `refuted` or `not_enough_info` |
| `rationale` | yes | Why the evidence leads to this verdict |
| `auditor` | yes | Person or model id |
| `evidence_ids` | no | Evidence on this claim that the verdict relies on |

`supported` must cite `supports` evidence, and `refuted` must cite `refutes` evidence. See
[Verdicts](../concepts/verdicts.md). Returns the claim with the verdict appended.

## trace

`id`, `direction` (`up`, `down` or `both`, default `both`), `max_depth` (default unlimited).
Returns breadth-first `ancestors` and `descendants` lists of
`{id, statement, current_verdict, depth, retired}`.

## audit_report

`include_matrix` (default true). Returns totals, gaps, the integrity check and the traceability
matrix; see [Traceability](../concepts/traceability.md#the-audit-report).

## next_steps

`limit`: items listed in each of tiers 1 and 2 (default 10; 0 lists everything). Same data as
`claims-mcp tldr --json`; see [the ranked to-do list](../guides/first-audit.md#the-ranked-to-do-list).

## recheck

No parameters. Returns `head`, `checked`, `current`, and `items` for every anchor on an active
claim that is not current, each with `claim`, `role`, `evidence`, `stance`, `anchor`, `status` and
`detail`. See [Recheck](../concepts/anchors.md#recheck).

## Resources

The server also offers read-only resources: the PDF report's files under `claims://report/`
(`claims-report.typ`, `text.typ` and `audit-notes.example.toml`). See
[PDF reports](../guides/reports.md).
