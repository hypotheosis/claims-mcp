# Integrity levels and methods

claims-mcp borrows two ideas from IEEE 1012, the standard for system and software verification
and validation: how much a failure would matter, and how a claim is verified.

## Integrity levels

`integrity_level` is optional and ranges from 1 to 4. Any other value is rejected.

| Level | Use when failure would cause |
|---|---|
| 4 | Safety harm, a security breach or data loss |
| 3 | Loss of a core function the source explicitly asked for |
| 2 | A degraded or secondary function |
| 1 | Cosmetic, style or documentation issues |

`audit_report` counts claims per level (including `unassigned`), and `next_steps` lists open
level 3-4 claims ahead of the rest.

## Verification methods

`verification_method` says how the claim is meant to be verified:

| Method | Means |
|---|---|
| `inspection` | Reading code, documents or output |
| `analysis` | Reasoning, calculation or static analysis |
| `demonstration` | Running the system and observing behaviour without instrumentation |
| `test` | Executing a defined test with pass/fail criteria |
| `formal_proof` | A machine-checked or rigorous mathematical proof |

The first four are the IEEE 1012 and INCOSE methods. `formal_proof` is added for claims backed
by proof assistants such as Lean; see
[Alongside formal verification](../why/formal-verification.md).

## The rigor rule

Inspection is acceptable at levels 1-2. Levels 3-4 need `test`, `analysis`, `demonstration` or
`formal_proof`. `audit_report` lists level 3-4 claims whose method is `inspection` as `rigor`
gaps. Resolve a rigor gap by adding a test or analysis, not by lowering the level or switching
the method on a judged claim. That would be moving the goalposts; retire the claim and add a
corrected one instead.
