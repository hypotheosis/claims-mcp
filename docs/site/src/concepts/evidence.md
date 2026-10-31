# Evidence

**Evidence** is one finding the auditor gathered, attached to a claim. The auditor reads the
code, runs the test or program, and records what it observed. The producer saying "I
implemented it" is not evidence.

Record one finding per evidence item. Record findings that refute the claim, and neutral ones,
as well as supporting ones; leaving them out biases the audit.

## Fields

| Field | Meaning |
|---|---|
| `id` | `EVD-0014`; unique across the whole catalog and never reused |
| `kind` | The form of the evidence (below) |
| `locator` | Where to find it, precisely enough to repeat: `src/store.rs:88`, a test name, the exact command run, or a URL |
| `summary` | What was observed, not what was concluded |
| `stance` | `supports`, `refutes` or `neutral` toward the claim |
| `anchor` | Optional machine-checkable location, pinned to a commit; see [Anchors and recheck](anchors.md) |
| `recorded_at` | Set by the server |

### Kinds

| `kind` | Is |
|---|---|
| `test` | The result of an executed test |
| `inspection` | An observation from reading an artifact |
| `analysis` | The result of reasoning or static analysis |
| `demonstration` | An observation from running the system |
| `proof` | A proof artifact or proof-checker output |
| `log` | A captured log or command output |
| `other` | Anything else |

### Stance

The stance uses FEVER terminology. `supports` means the evidence shows the claim holds,
`refutes` that it does not, and `neutral` that the evidence is relevant but decides nothing.
[Verdicts](verdicts.md) must cite evidence with the matching stance.

## Showing that something is absent

When a document promises a feature the code lacks, there is no line to point at. Record
`refutes` evidence with a `search` anchor instead: a `git grep -E` pattern broad enough to catch
any reasonable implementation, over a scope that covers every place it could live. The server
runs the search itself and records the hit count, which must be zero. Add a second `refutes`
item with a `file` anchor on where the feature would have to plug in. A search alone can miss
an implementation filed under an unexpected name.

A demonstrated absence supports `refuted`. Not having looked does not: that is
`not_enough_info`.

## Evidence is permanent

Evidence can't be edited or removed. If the code changes, add new evidence and record a new
verdict. When evidence is recorded after a claim's latest verdict, `audit_report` lists the
claim as `stale` until it is judged again.
