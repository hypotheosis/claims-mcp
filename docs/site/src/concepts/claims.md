# Claims

A **claim** is one atomic, falsifiable assertion about an artifact or a requirement.

- **Atomic**: one assertion. If it contains "and", it is probably two claims. A compound claim
  hides partial failure.
- **Falsifiable**: some observation would show it false.
- **Observable**: it describes behaviour or a property of the artifact, not an intention.

| Weak | Strong |
|---|---|
| "Handles errors well" | "`parse_config` returns `ConfigError::TooLarge` for files over 1 MiB" |
| "Fast and secure" | A performance claim and a security claim, each with concrete bounds |
| "Uses the rmcp crate" | "`Cargo.toml` depends on rmcp and the server is built with rmcp's `tool_router`" |

## Fields

| Field | Meaning |
|---|---|
| `id` | `CLM-0007`; assigned by the server and never reused |
| `statement` | The assertion; must not be empty |
| `kind` | What sort of property it asserts (below) |
| `level` | Where it sits in the requirements hierarchy (below) |
| `verification_method` | How it is meant to be verified; see [Integrity levels and methods](integrity-and-methods.md) |
| `integrity_level` | Optional, 1-4; see [Integrity levels and methods](integrity-and-methods.md) |
| `source` | Optional free text: where the claim came from (prompt, spec section, ticket) |
| `source_anchor` | Optional [file anchor](anchors.md) on the exact text that makes the claim |
| `derived_from` | Parent claims; see [Traceability](traceability.md) |
| `artifacts` | Paths the claim is about, so `list_claims` can find it by artifact |
| `tags` | Free-form labels; see below |
| `status` | `active`, or `retired` with a reason and time |

Duplicate tags and artifacts are removed (the first occurrence wins).

### Kinds

| `kind` | Asserts |
|---|---|
| `functional` | Observable behaviour or output |
| `non_functional` | A quality attribute such as usability or maintainability |
| `interface` | A contract with another component, an API or a user |
| `performance` | A latency, throughput or resource bound |
| `safety` | Protection against harm to people or property |
| `security` | A confidentiality, integrity or availability property |
| `documentation` | A statement about documentation content |
| `constraint` | An imposed limitation, such as a language, licence or dependency rule |

### Levels

From broadest to narrowest: `system` (the whole task or system), `software`, `component`,
`unit` (one function or type) and `artifact` (a concrete produced thing).

### Tags with meaning

Tags are free-form, but two prefixes are used by the tools:

- `area:<name>` groups claims. `next_steps` and `claims-mcp tldr` count the remaining work
  per area.
- `issue:<N>` links a claim to issue N. The PDF report links it to the repository's issue
  tracker.

## Source anchors

When the claim comes from a file in the repository, such as a README line that promises a
feature, set `source_anchor` to those lines. If the claim is refuted, that is the text someone
has to fix. `audit_report` lists refuted claims without one under
`refuted_without_source_anchor`. `update_claim` can replace a source anchor but not remove it.

## Changing and retiring claims

`update_claim` edits metadata. Evidence and verdicts can't be edited. To change what a judged
claim *means*, retire it with `retire_claim` and add a corrected one. Editing a judged claim's
statement until it passes would rewrite history.

Retired claims stay in the catalog, so `derived_from` links that point at them remain valid,
but they accept no more evidence, verdicts or updates.
