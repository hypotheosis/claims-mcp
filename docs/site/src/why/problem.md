# The problem

Software makes promises in many places: specs, READMEs, tickets, prompts given to coding
agents. Over time, three things go wrong.

## Documentation drifts from code

A README says the server rate-limits requests; a refactor removed the limiter a year ago. Nobody
notices, because nothing connects the sentence to the code. Reviews check the change, not the
promises the change breaks.

**What claims-mcp does:** each promise becomes a claim with a `source_anchor` on the exact lines
that make it. Evidence carries anchors too: a `file` anchor on the code that shows the claim
holds, or a `search` anchor (a `git grep` pattern with a recorded hit count) showing that
something is absent. The server pins every anchor to a commit, and
[`recheck`](../concepts/anchors.md) reports anchors that have since moved, changed or been
overturned. Drift becomes a finding instead of a surprise.

## LLM output is accepted without checking

An agent reports "I implemented all five requirements". That is the producer certifying its own
work. Checking it properly means splitting the instruction into separate assertions, checking
each one independently and keeping the result.

**What claims-mcp does:** the [`claims-audit` skill](../guides/first-audit.md) breaks the
instruction into [atomic, falsifiable claims](../concepts/claims.md). The auditor gathers its own
[evidence](../concepts/evidence.md) for each one and records a [FEVER verdict](../concepts/verdicts.md)
that must cite that evidence. `not_enough_info` is an allowed outcome, so "I didn't check" can't
pass as "it works".

## Traceability is lost

Which requirement does this test cover? Which sub-requirements make up "the export feature
works"? If a low-level check fails, which high-level promise is now broken? Spreadsheets and
ticket links answer this badly, and go stale.

**What claims-mcp does:** claims form a hierarchy through `derived_from`, from system level
down to individual artifacts. [`trace`](../concepts/traceability.md) walks it in either direction,
and `audit_report` produces a traceability matrix and lists the gaps: unverified claims,
high-integrity claims checked only by inspection, and parents whose children aren't all
supported.

## Why a ledger, and why sealed

All of this lives in one `claims.json`, committed with the code, so it is reviewed, diffed
and versioned like the code. Verdict history is append-only: a new verdict doesn't erase the
old one. Every record carries an [integrity seal](../concepts/seals.md). If anyone, an agent
included, edits the file by hand instead of going through the tools, the server notices and
refuses further writes until the file is restored.

Next: [how claims-mcp relates to formal verification](formal-verification.md).
