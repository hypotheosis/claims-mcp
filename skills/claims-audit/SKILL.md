---
name: claims-audit
description: Audit whether an artifact does what it was supposed to do by recording atomic claims, independently gathered evidence, and FEVER verdicts (supported / refuted / not_enough_info) in the claims-mcp catalog. Use whenever the user asks to audit, verify, validate, or fact-check work against its instructions or spec; build or check requirements traceability; run IEEE 1012 style verification and validation or conformance checks; or confirm that an LLM or agent actually produced what it was told to produce; also when a workflow opening a pull request asks for its change to be audited before the PR opens. Requires the `claims` MCP server (claims-mcp).
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
| `update_claim` | Fix metadata (tags, artifacts, source, source anchor) without changing what the claim means |
| `retire_claim` | Withdraw a claim that was wrong or is superseded |
| `add_evidence` | Record one finding with a precise locator, a commit-pinned anchor, and an honest stance |
| `record_verdict` | Append a FEVER verdict that cites the evidence it relies on |
| `trace` | Walk the hierarchy up (why does this exist?) or down (how is it verified?) |
| `audit_report` | Totals, gap lists, and the traceability matrix |
| `next_steps` | Ranked TL;DR: open claims by tier, each with one next step |
| `recheck` | Find anchors that moved, changed, or were overturned since they were recorded |

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
- Anchors need the catalog to sit in a git repository. The server pins each one to `HEAD` and
  rejects a path outside the repository, a line range past the end of the file, or a search
  `hits` that differs from what it counts itself. Commit the code you audit first: an anchor
  recorded on uncommitted changes is marked `dirty`, and `recheck` reports it as `unpinned`.

## Workflow

### 1. Scope

- Identify the **artifact** under audit (files, modules, a PR, generated output).
- Identify the **source** it was produced from: the prompt, task, ticket, or spec section.
  Quote or cite it; this becomes the `source` field on every claim. When the source is a
  file in the repository (README, spec, docs), also set `source_anchor` to the exact lines
  that make the claim, e.g. `{"path": "README.md", "lines": {"start": 42, "end": 44}}`. If
  the claim is refuted, that is the text someone has to fix, and the report lists refuted
  claims without one under `refuted_without_source_anchor`.
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
   - `anchor`: whenever the finding is in the repository, give a machine-checkable anchor
     next to the free-text locator. Use `{"file": {"path", "lines": {"start", "end"}}}` for
     code or text that shows the finding, and `{"search": {"pattern", "case_insensitive",
     "scope"}}` for a search (see *Absence* below). The server pins it to the current commit.
   - `summary`: what you observed, not what you concluded.
   - `stance`: `supports`, `refutes`, or `neutral`. Record refuting and neutral findings
     too; leaving them out biases the audit.
3. One finding per evidence item. Do not merge a passing test and a code reading into one.

#### Absence: a feature that does not exist

When a document promises something the code lacks, there is no line to point at. Show the
absence instead, so anyone can repeat it:

1. Record `refutes` evidence with a `search` anchor: a `git grep -E` pattern broad enough to
   catch any reasonable implementation (synonyms, config keys, error codes, e.g.
   `rate.?limit|throttl|429`), `case_insensitive: true`, and a `scope` that covers every
   place it could live. The server runs the search and records the hit count; it must be 0.
2. Where the feature would have to plug in (a router, a middleware stack, a config struct),
   add a second `refutes` item with a `file` anchor on those lines, saying what is there
   instead. A search alone can miss an implementation filed under an unexpected name.
3. Set `source_anchor` on the claim (step 1), so the refutation points back at the promise.

`recheck` reruns the search later; if it starts matching, the refutation is `overturned` and
the claim needs re-auditing.

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

Never infer support from the absence of failure ("no errors, so it works"), and never infer
absence from not having looked. If you did not search, or searched without a defined scope
and a pattern that would catch any reasonable implementation, the verdict is
`not_enough_info`. A demonstrated absence, a zero-hit `search` anchor plus a `file` anchor on
where the feature would plug in (see *Absence* above), is evidence and supports `refuted`.

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
  | `refuted_without_source_anchor` | Nothing points at the text that makes a refuted claim | Set `source_anchor` with `update_claim` |

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
     `claims-mcp --catalog <ledger> verify --json > integrity.json` and
     `claims-mcp --catalog <ledger> recheck --json > recheck.json`, then `typst compile --root .
     report/claims-report.typ audit.pdf --input catalog=<ledger> --input
     notes=report/audit-notes.toml --input commit=<sha> --input integrity=integrity.json
     --input recheck=recheck.json`. Add
     `--input detail=full` for the full record (every evidence item and verdict). Paths are
     relative to `--root`, so the template, ledger, and notes must all be under it.
  Tag a refuted claim `issue:N` to link its tracking issue.

### 6. Re-audit after fixes

Call `recheck` first: every anchor it lists as `changed`, `overturned`, `missing`, or
`commit_missing` means the evidence no longer describes the code, so re-audit that claim
(`moved` anchors still hold; re-record them if you want current line numbers). Then focus on
`refuted`, `not_enough_info`, and `stale` claims: gather new evidence, then record a new
verdict. History is append-only, so the earlier verdict remains as the audit trail.

## Auditing a pull request

Use this mode when a workflow that opens a pull request (an issue-to-PR skill, for example)
asks for its change to be audited before the PR opens. It answers two questions: does the
change do what the issue asked and what the PR description says, and did it break anything the
ledger already records as working. The workflow steps above still apply; this section says what
is different.

### Who audits

The **auditor is a fresh subagent on a different model** from the one that wrote the change
(for example `claude-sonnet-5-5` when the author ran on Opus; in Claude Code, the Agent tool
with `model: "sonnet"`). Give it only:

- the issue (URL, title, body, and comments that change the requirements);
- the draft PR description, as a file;
- the commit range (`<base>..<head>`) and the repository path;
- the ledger mode and ledger path (below);
- an instruction to follow this skill and to return the report described under *Hand-back*.

Do not pass it the author's reasoning, plan, or summary of what was done. They are the
producer's account, and the core rule forbids using them as evidence.

### Ledger mode or table-only mode

- **Ledger mode**: `claims.json` is tracked in the repository (`git ls-files --error-unmatch
  claims.json`). Findings are recorded in it through the tools and committed on the branch.
- **Table-only mode**: no tracked ledger. **Make no claims-mcp calls at all.** A write creates the
  ledger, and even a read creates the sidecar `.claims.json.lock` next to where the ledger would
  be, which a later `git add -A` would commit; with no ledger there is nothing to read anyway.
  If either file appears, delete it before committing. Apply the same method and return the
  findings for the PR description only,
  **without claim ids**. Ids in a description would not match the ones a ledger committed later
  assigns.

**Which ledger the server writes.** An MCP client starts claims-mcp once, and a relative
`--catalog` resolves against the directory the session started in. If that is not the checkout
the PR is built in (a git worktree, another clone), the MCP tools would write the wrong file and
pin the wrong commit. In that case pass `catalog` with the ledger's absolute path on **every**
call, reads included:

```json
{"catalog": "/abs/path/to/checkout/claims.json", "tag": "from-issue:42"}
```

The call then reads or writes that ledger and pins anchors to `HEAD` of the repository that
holds it. A call that leaves `catalog` out goes to the session's ledger, so one forgotten
argument writes the wrong file; check with `git status` in both checkouts before committing.

Check the tools' input schemas first: if they have no `catalog` property, the connected server
predates the argument and silently ignores it, so a call would write the session's ledger. (The
released 0.2.4 and earlier lack it, but the version alone doesn't tell you, since unreleased
builds report the last version.) Then make every call through `scripts/claims_call.py`, which starts its own server for the ledger you name:

```bash
python3 <this-skill>/scripts/claims_call.py --catalog <checkout>/claims.json list_claims '{"tag": "from-issue:42"}'
python3 <this-skill>/scripts/claims_call.py --catalog <checkout>/claims.json < calls.json
```

`calls.json` is a JSON list of `{"tool", "arguments"}`; the calls run in order and stop at the
first error. The server pins anchors to `HEAD` of the repository holding the ledger.

### Before the audit

The change must be **committed**: anchors pin `HEAD`, and evidence recorded on uncommitted code
is `unpinned`. The author commits the implementation, then asks for the audit, and commits the
ledger last in a commit of its own.

### Claims to check

1. **Existing claims first.** `list_claims` with the changed paths as `artifact`, and with
   `text` from the issue title. Claims registered while the work was planned get evidence and
   verdicts; do not register duplicates.
2. **The issue's requirements and acceptance criteria.** `source` is the issue URL.
3. **The PR description's statements of fact about the change**: what it changed or removed,
   what was run and passed, how behavior changed, what the tests cover. Quote each one exactly
   in `source`; split statements that join several facts. Skip intentions and plans.
4. Tag every new claim `from-issue:N` (not `issue:N`, which names a refuted claim's tracking
   issue). Link them under an existing parent where one fits, otherwise under one new parent
   for the issue.

Check each claim yourself:

- **Runs**: rerun the command ("tests pass", "clippy clean") on the head commit and record the
  command as the locator, with the counts it printed. A manual check you cannot repeat is
  `not_enough_info`, not `refuted`.
- **Universal claims** ("renamed everywhere", "every variant is tested"): search the whole
  repository at head, not the diff. Leftovers are usually in files the PR did not touch.
- **Fixes and new behavior**: check the code does it, then find a test that exercises the
  defect's input and asserts the fixed outcome. Read the assertion, not the test's name.
- **No behavior change**: every non-test hunk that changes semantics and that the description
  does not mention refutes it.

### Regressions

Call `recheck`. For each anchor it reports as `changed`, `overturned`, or `missing` in a file the
diff touched, re-audit that claim on the head commit and record a new verdict. Anchors that only
`moved` still hold. Report the rest of the `recheck` output without acting on it: drift in files
this PR did not touch is not this PR's finding.

### Ours or elsewhere

Classify every `refuted` claim:

- **Ours**: a claim from the issue or the PR description, or an existing claim this diff broke.
  The author must fix it before the PR opens: fix the code (or, for a description statement
  that is simply wrong, the description), commit, and have those claims re-audited on the new
  head. Resume the same auditor if the harness can (Claude Code's SendMessage resumes a
  finished subagent); otherwise start a fresh auditor on the same non-author model and give it
  the previous hand-back and the claims to re-audit. A new verdict appends to the history;
  nothing is deleted.
- **Elsewhere**: a problem in code this PR did not change, found along the way. Record it as
  `refuted` with its `Fix:` line, leave it in the ledger, and list it in the PR description as
  pre-existing. It does not block the PR.

`not_enough_info` never blocks; list it with what would decide it.

### Hand-back

The auditor returns, per claim: id (ledger mode only), the statement, the verdict, the key
evidence locator, ours or elsewhere, and the `Fix:` step for anything not supported. It also
lists `recheck` findings and the evidence ids it recorded. The author checks it against
`audit_report` before using its numbers.

### Commit and describe

In ledger mode, run `claims-mcp --catalog claims.json verify` and commit `claims.json` alone
(`chore(claims): audit the change for #N`). Then add a `## Claims` section to the PR
description:

```markdown
## Claims

| Claim | Verdict | Evidence |
|-------|---------|----------|
| CLM-0190 "parse_config rejects files over 1 MiB" | supported | `cargo test config::too_large` (1 passed) |
| CLM-0044 "README lists every subcommand" (pre-existing) | refuted | `seal` missing from README.md:120; Fix: add it |
```

In table-only mode, drop the ids and add one line saying the repository has no claims ledger.
In ledger mode, also write: "Merge with a merge commit: the ledger's anchors pin this branch's
commits." A squash or rebase merge leaves them as `commit_missing`.

### When main's ledger moved

Ids come from sealed counters on each branch, so two branches that both added claims or both
added evidence hand out the same ids. Git may report a conflict, or it may **merge cleanly**:
two branches that each added one evidence item to different claims merge without a conflict
into a ledger holding two `EVD-0001` records, which the server then refuses to read at all.

So before pushing, and again before merging, check whether main's ledger changed since the
branch's base (`git diff --quiet <merge-base> origin/main -- claims.json`). If it did, never
merge or hand-edit `claims.json`. Instead:

- **If `claims-mcp rebase --help` works** (releases after 0.2.4), merge main and replay this
  branch's records onto main's ledger in the same step:

  ```bash
  git merge --no-commit origin/main
  git checkout HEAD -- claims.json
  git show MERGE_HEAD:claims.json > "$TMP/main.json"
  git show "$(git merge-base HEAD MERGE_HEAD)":claims.json > "$TMP/base.json" || rm "$TMP/base.json"
  claims-mcp --catalog claims.json rebase --onto "$TMP/main.json" --base "$TMP/base.json"
  claims-mcp --catalog claims.json verify && git add claims.json && git commit
  ```

  It prints each `old -> new` id. If it refuses (both sides changed one claim differently, or
  the branch added evidence to a claim main retired), abort the merge and fall back below.
- **Otherwise**, take main's file (`git checkout origin/main -- claims.json`), record this PR's
  findings again through the tools from the auditor's hand-back, and commit that.

Either way the ids change, so update the PR description's table. A CI step running
`claims-mcp verify` on the PR's merge result catches the case that slipped through. The full
procedure is the docs site's "Branches that both write the ledger" guide.

## Anti-patterns

- **Self-certification**: using the producer's claims ("I implemented X") as evidence.
- **Compound claims**: one claim covering several behaviors, which hides partial failure.
- **Moving the goalposts**: editing a judged claim's statement until it passes, or switching
  `verification_method` / lowering `integrity_level` to clear a `rigor` gap. Retire it with a
  reason and add a corrected claim instead.
- **Silent refutations**: finding a problem and not recording it as `refutes` evidence.
- **Ignoring rigor gaps** on integrity level 3-4 claims.
- **Batch rubber-stamping**: recording `supported` for many claims after one shallow check.
