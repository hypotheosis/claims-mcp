---
name: claims-audit
description: Audit whether an artifact does what it was supposed to do by recording atomic claims, independently gathered evidence, and FEVER verdicts (supported / refuted / not_enough_info) in the claims-mcp catalog. Use whenever the user asks to audit, verify, validate, or fact-check work against its instructions or spec; build or check requirements traceability; run IEEE 1012 style verification and validation or conformance checks; or confirm that an LLM or agent actually produced what it was told to produce. Requires the `claims` MCP server (claims-mcp).
---

# Claims Audit

Audit an artifact against the instruction or specification it was produced from. Every
conclusion must be traceable: claim -> evidence -> verdict, stored in the claims catalog
through the `claims` MCP tools.

**Core rule:** a verdict is only as good as the evidence you gathered *yourself*. Never
accept the producing agent's own summary, commit message, or "done" report as evidence.

## Tools

| Tool | Use it to |
|---|---|
| `add_claims` | Register a whole claim hierarchy in one call (`@N` links to earlier entries) |
| `add_claim` | Add a single claim discovered later |
| `list_claims` | Find claims by verdict, kind, level, tag, artifact, or text |
| `get_claim` | Read one claim with evidence, verdict history, and children |
| `update_claim` | Fix metadata (tags, artifacts, source) without changing what the claim means |
| `retire_claim` | Withdraw a claim that was wrong or is superseded |
| `add_evidence` | Record one finding with a precise locator and an honest stance |
| `record_verdict` | Append a FEVER verdict that cites the evidence it relies on |
| `trace` | Walk the hierarchy up (why does this exist?) or down (how is it verified?) |
| `audit_report` | Totals, gap lists, and the traceability matrix |
| `next_steps` | Ranked TL;DR: open claims by tier, each with one next step |

## Errors and constraints

- **Never edit the catalog file (`claims.json`) directly**, not even to fix a typo. Every record
  carries an integrity seal. If one fails, the server refuses every write until the file is
  restored.
- If a write fails with "was changed outside claims-mcp", **stop and tell the user**. Restore the
  file from git (`git checkout -- claims.json`, or revert the commit that edited it), then redo
  the intended change through the tools. Treat the failed checks as an audit finding. Never run
  `claims-mcp seal --adopt` to clear the error: adoption is a maintainer's one-time migration,
  and the records it seals are labeled as adopted in every report.

- A failed call returns an error message and changes nothing. A failed `add_claims` writes
  nothing; the error names the entry as `claims[i]: ...` - fix that entry and resend the whole
  batch.
- `@N` must point at an earlier entry of the same batch, and only `add_claims` accepts it.
- Retired claims reject `update_claim`, `add_evidence`, and `record_verdict`; add a replacement
  claim instead.
- A new `derived_from` link cannot point at a retired claim. An existing link to a parent retired
  later is kept and reported under `dangling_links`.
- Evidence cited by a verdict must belong to the same claim; `supported` needs cited `supports`
  evidence and `refuted` needs cited `refutes` evidence.
- `update_claim`: omitted fields are unchanged and list fields replace the whole list.

## Workflow

### 1. Scope

- Identify the **artifact** under audit (files, modules, a PR, generated output).
- Identify the **source** it was produced from: the prompt, task, ticket, or spec section.
  Quote or cite it; this becomes the `source` field on every claim.
- Call `list_claims` with a relevant `text` or `artifact` filter first. If claims already
  exist, continue from step 3 rather than registering duplicates.

### 2. Extract claims

Break the source into claims that are:

- **Atomic**: one assertion. If it contains "and", it is probably two claims.
- **Falsifiable**: there is an observation that would show it false.
- **Observable**: phrased as behavior or a property of the artifact, not as intent.

| Weak | Strong |
|---|---|
| "Handles errors well" | "parse_config returns ConfigError::TooLarge for files over 1 MiB" |
| "Fast and secure" | Split into a performance claim and a security claim with concrete bounds |
| "Uses the rmcp crate" | "Cargo.toml depends on rmcp and the server is built with rmcp's tool_router" |

Build the hierarchy from broad to narrow: `system` (the task as a whole) -> `software` ->
`component` -> `unit` -> `artifact`. Register it in **one** `add_claims` call, linking each
entry to its parent with `"derived_from": ["@0"]` (the 0-based index of an earlier entry).

Set on each claim:

- `kind`: functional, non_functional, interface, performance, safety, security,
  documentation, or constraint.
- `integrity_level` (IEEE 1012):

  | Level | Use when failure would cause |
  |---|---|
  | 4 | Safety harm, security breach, or data loss |
  | 3 | Loss of a core function the source explicitly asked for |
  | 2 | Degraded or secondary function |
  | 1 | Cosmetic, style, or documentation issues |

- `verification_method`, matched to the integrity level: `inspection` is acceptable at
  levels 1-2; levels 3-4 need `test`, `analysis`, `demonstration`, or `formal_proof`.
  The report flags violations as `rigor` gaps.
- `artifacts`: the paths the claim is about, so `list_claims` with `artifact` can find it.

### 3. Gather evidence

For each claim (start with the highest integrity level):

1. Inspect the artifact yourself: read the code, run the tests, run the program, check
   output. Use the verification method the claim names.
2. Record **every** relevant finding with `add_evidence`:
   - `locator`: precise and reproducible, such as `src/store.rs:88`,
     `cargo test store::tests::atomic_save`, or the exact command run.
   - `summary`: what you observed, not what you concluded.
   - `stance`: `supports`, `refutes`, or `neutral`. Record refuting and neutral findings
     too; leaving them out biases the audit.
3. One finding per evidence item. Do not merge a passing test and a code reading into one.

### 4. Adjudicate (FEVER)

Call `record_verdict` with `auditor` set to your model id or name:

- **supported**: at least one cited item directly shows the claim holds, and no
  unaddressed refuting evidence exists.
- **refuted**: at least one cited item directly shows the claim does not hold.
- **not_enough_info**: the evidence cannot decide it. This is a legitimate, honest
  outcome; use it instead of guessing.

The server enforces that `supported` cites supporting evidence and `refuted` cites refuting
evidence. Your `rationale` must explain *why* the cited evidence decides the claim.

A `refuted` or `not_enough_info` rationale must end with `Fix: <one concrete next step>`: what
to change, or which test or evidence would decide the claim. `next_steps` and
`claims-mcp tldr` show the text after the last `Fix:` as the claim's next step, so make it
specific enough to act on without rereading the rationale ("Fix: add a test that sends a
1024-bit RSA JWK and asserts it is rejected", not "Fix: add tests"). The step must also stand
alone, because items are re-sorted and truncated: never refer to other claims or text ("As
above", "same as CLM-0012").

Never infer support from the absence of failure ("no errors, so it works"). Absence of
evidence is `not_enough_info`.

#### Parent claims

Judge leaf claims first. Once every active child of a parent is `supported`, record one
`analysis` evidence item on the parent with locator `children of CLM-0002` (the parent's own
id; do not list the children, since the report counts them from the catalog) and a summary
explaining how the children together cover the parent, then record the parent's verdict citing
it. If any child is `refuted`, record `refutes` evidence on the parent with the same locator,
naming the refuted children in the summary, and mark the parent `refuted`. Until then the
report lists the parent under `decomposition`.

### 5. Report

Call `audit_report` and summarize for the user. On a large catalog, call `next_steps` first and
lead the report with tiers 0-2:

- Totals by verdict, and the share of level 3-4 claims that are supported.
- Every non-empty gap list, with what each means and a concrete remediation:

  | Gap | Meaning | Remediation |
  |---|---|---|
  | `refuted` | The artifact does not do what was asked | Fix the artifact, then re-audit |
  | `not_enough_info` | Could not be decided | Add a test or analysis that decides it |
  | `unverified` / `no_evidence` | Not yet audited | Continue step 3 |
  | `stale` | Evidence arrived after the verdict | Re-adjudicate |
  | `rigor` | High-integrity claim checked only by inspection | Add a test or analysis |
  | `decomposition` | A parent's sub-claims are not all supported | Resolve the children first |
  | `dangling_links` | Parent was retired | Re-link or retire the child |

- Offer the traceability `matrix` as a table when the user needs a V&V record.
- Optionally, when a reviewer needs a fixed record of the audit, offer a PDF report:
  1. Run `claims-mcp report init` from the project root. It writes the report template to
     `report/` and creates `report/audit-notes.toml`: the commented example with every setting
     commented out (it never replaces an existing notes file). Without a shell, read the MCP
     resource `claims://report/audit-notes.example.toml` and write the notes file from it.
     Uncomment each setting and replace the example's values, as its comments say: set `artifact`, `version`, and `repository`; write an
     `assessment` that says what was audited, whether the project meets it, and which
     findings decide that, citing claim ids and issues;
     and add `[[points]]` for caveats the reader should weigh, such as whether the audit was
     independent of the builder, heuristic evidence, and hierarchy changes during the audit.
     Do not restate counts or verdicts; the report computes them.
  2. Build the PDF from a shell, not through this server (it never executes commands):
     `claims-mcp --catalog <ledger> verify --json > integrity.json`, then `typst compile --root .
     report/claims-report.typ audit.pdf --input catalog=<ledger> --input
     notes=report/audit-notes.toml --input commit=<sha> --input integrity=integrity.json`. Add
     `--input detail=full` for the full record (every evidence item and verdict). Paths are
     relative to `--root`, so the template, ledger, and notes must all be under it.
  Tag a refuted claim `issue:N` to link its tracking issue.

### 6. Re-audit after fixes

Focus on `refuted`, `not_enough_info`, and `stale` claims: gather new evidence, then record
a new verdict. History is append-only, so the earlier verdict remains as the audit trail.

## Anti-patterns

- **Self-certification**: using the producer's claims ("I implemented X") as evidence.
- **Compound claims**: one claim covering several behaviors, which hides partial failure.
- **Moving the goalposts**: editing a judged claim's statement until it passes, or switching
  `verification_method` / lowering `integrity_level` to clear a `rigor` gap. Retire it with a
  reason and add a corrected claim instead.
- **Silent refutations**: finding a problem and not recording it as `refutes` evidence.
- **Ignoring rigor gaps** on integrity level 3-4 claims.
- **Batch rubber-stamping**: recording `supported` for many claims after one shallow check.
