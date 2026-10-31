# claims-mcp

claims-mcp is an [MCP](https://modelcontextprotocol.io) server that keeps a ledger of
**claims** about your software in a single JSON file, committed alongside the code.

A claim is one atomic, falsifiable statement: "`parse_config` rejects files over 1 MiB",
"the README's install command works on macOS", "the agent implemented every item in the task".
Each claim carries the **evidence** an auditor gathered and an append-only history of
**verdicts** in FEVER terms: `supported`, `refuted` or `not_enough_info`. Every verdict must
cite evidence that backs it.

Together they give a durable, reviewable record for:

- **Requirements traceability**: claims link to parent claims (`derived_from`), from the
  system level down to individual artifacts.
- **Verification and validation** (IEEE 1012): integrity levels 1-4 and verification methods
  (inspection, analysis, demonstration, test, formal proof).
- **Conformance and LLM output auditing**: checking that what an agent produced matches the
  instruction it was produced from, and that the docs match the code.

The server only records. It never runs your code. An agent does the verification, guided by
the bundled `claims-audit` skill, and the server enforces the ledger's rules: verdicts must cite
evidence, history is append-only, and a hand-edited ledger is detected and blocks writes.

## Thirty seconds of claims-mcp

An agent auditing a project registers a claim, pointing at the text that makes it:

```json
{"tool": "add_claim", "arguments": {
  "statement": "claims-mcp verify exits 1 when any integrity seal check fails",
  "kind": "functional", "level": "software", "integrity_level": 3,
  "verification_method": "test",
  "source_anchor": {"path": "docs/site/src/reference/cli.md", "lines": {"start": 26, "end": 26}}
}}
```

It inspects the code and runs the test itself, then records what it saw. The server pins the
anchor to the current commit:

```json
{"tool": "add_evidence", "arguments": {
  "claim_id": "CLM-0001", "kind": "test", "stance": "supports",
  "locator": "cargo test verify_reports_tampering",
  "summary": "Test edits a sealed verdict; verify exits 1 and names the record.",
  "anchor": {"file": {"path": "src/main.rs", "lines": {"start": 160, "end": 182}}}
}}
```

It records a verdict that cites the evidence:

```json
{"tool": "record_verdict", "arguments": {
  "claim_id": "CLM-0001", "verdict": "supported", "auditor": "example-model",
  "rationale": "The test exercises the failure path and observes exit code 1.",
  "evidence_ids": ["EVD-0001"]
}}
```

Weeks later, someone changes the anchored code. `claims-mcp recheck` notices:

```text
2 anchors checked against 4e16582: 1 current, 1 need re-audit
  CLM-0001 EVD-0001 changed: lines 160-182 of src/main.rs changed since they were recorded
    src/main.rs:160-182 @ 86a73e2
```

The claim needs re-auditing, and the ledger says exactly where to look. (The ids, line numbers
and test name above are illustrative.)

## Where to go next

- [Why claims-mcp](why/problem.md) explains the problems it solves, and
  [how it works alongside formal verification](why/formal-verification.md).
- [Install claims-mcp](guides/install.md) to try it.
