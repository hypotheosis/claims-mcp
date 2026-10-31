# Your first audit

With the server and skill [installed](install.md), ask Claude to audit something: "audit this
change against the task", "check the README against the code", or run `/claims-audit`. The
skill walks the agent through six steps. This page explains them so you know what to expect,
and what to check.

## 1. Scope

The agent identifies the **artifact** under audit (files, a module, a PR, generated output) and
the **source** it was produced from (the prompt, ticket or spec section). When the source is a
file in the repository, each claim gets a `source_anchor` on the lines that make it. The agent
calls `list_claims` first, so an existing audit continues instead of duplicating claims.

## 2. Extract claims

The source is broken into [atomic, falsifiable claims](../concepts/claims.md) and registered as a
hierarchy in one `add_claims` call: the task as a whole at the top, narrower claims below,
each linked with `derived_from`. Each claim gets a kind, an
[integrity level and a verification method](../concepts/integrity-and-methods.md).

**What to check:** compound claims ("handles X and Y") and vague ones ("works well"). They hide
partial failures.

## 3. Gather evidence

Starting with the highest integrity level, the agent reads the code, runs the tests and runs the
program, then records each finding with `add_evidence`: a precise locator, an
[anchor](../concepts/anchors.md) whenever the finding is in the repository, a summary of what it
observed, and an honest stance. Refuting and neutral findings are recorded too.

For a promised feature that doesn't exist, it records a zero-hit `search` anchor plus a `file`
anchor on where the feature would plug in. See
[Showing that something is absent](../concepts/evidence.md#showing-that-something-is-absent).

## 4. Adjudicate

The agent records a [FEVER verdict](../concepts/verdicts.md) per claim, leaves first, citing the
evidence that decides it. `refuted` and `not_enough_info` verdicts end with a `Fix:` line naming
one concrete next step. Parent claims get their verdict once their children are judged.

## 5. Report

The agent summarizes `audit_report`: totals, every non-empty gap with a remediation, and the
traceability matrix if you need a V&V record. On a large catalog it starts from `next_steps`.

### The ranked to-do list

`next_steps` (MCP) and `claims-mcp tldr` (CLI) list only what to do next:

```bash
claims-mcp --catalog claims.json tldr             # Markdown, 10 items per tier
claims-mcp --catalog claims.json tldr --limit 0   # every item
claims-mcp --catalog claims.json tldr --json      # same data as the next_steps tool
```

Active **leaf** claims (no active children) are listed when they need work. A parent is listed
only once every active child is done and its own verdict is not (missing, refuted,
not_enough_info or stale): its step is then to record the roll-up verdict. Open claims are
ranked into tiers:

| Tier | Contents |
|---|---|
| 0. STOP | Shown only when the integrity check fails: make `claims-mcp verify` pass first (usually by restoring the catalog from git) |
| 1. Wrong | Refuted claims |
| 2. Unproven | Other open claims at integrity level 3-4 (not_enough_info, unverified, stale, no evidence) |
| 3. Everything else | Counts per `area:` tag |

Tiers 1 and 2 are sorted by integrity level, then id. Each item shows its `issue:` tags and its
next step: the text after the last `Fix:` in the latest verdict rationale, or else the
rationale's first sentence. Items beyond `--limit` are counted per area. The command exits 0
whenever the catalog can be read; use `verify` to gate CI.

For a fixed record a reviewer can read, render a [PDF report](reports.md).

## 6. Re-audit after fixes

After the code is fixed, the agent calls `recheck` first. Every anchor that is `changed`,
`overturned`, `missing` or `commit_missing` means that evidence no longer describes the code, so
the claim is re-audited. Then it works through `refuted`, `not_enough_info` and `stale` claims:
new evidence, new verdict. The earlier verdicts remain as the audit trail.

## Commit the catalog

`claims.json` is meant to be committed with the code, so the audit is reviewed and versioned
like everything else. Add `claims-mcp verify` to CI to catch hand edits; see
[Checking in CI](ci.md).
