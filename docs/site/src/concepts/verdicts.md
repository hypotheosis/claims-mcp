# Verdicts

A **verdict** is the auditor's conclusion about a claim, in the three-way scheme from the FEVER
fact-verification benchmark:

| Verdict | Means | Must cite |
|---|---|---|
| `supported` | Cited evidence shows the claim holds | At least one `supports` evidence item |
| `refuted` | Cited evidence shows the claim does not hold | At least one `refutes` evidence item |
| `not_enough_info` | The evidence can't decide the claim | Nothing required, but the rationale must not be empty |

Cited evidence must belong to the same claim. Every verdict also needs a non-empty `rationale`
(why the evidence decides the claim) and `auditor` (the person or model id).

`not_enough_info` is an honest, legitimate outcome. Use it instead of guessing. Never infer
support from the absence of failure ("no errors, so it works"), and never infer absence from not
having looked.

## History is append-only

`record_verdict` appends to the claim's verdict history; nothing is overwritten. The claim's
**current verdict** is its latest one, or `unverified` if it has none. After a fix, the auditor
gathers new evidence and records a new verdict, and the earlier one stays as the audit trail.

## The `Fix:` convention

A `refuted` or `not_enough_info` rationale should end with `Fix: <one concrete next step>`: what
to change, or which test or evidence would decide the claim. `next_steps` and `claims-mcp tldr`
show the text after the last `Fix:` as the claim's next step. Make it specific and
self-contained: "Fix: add a test that sends a 1024-bit RSA JWK and asserts it is rejected", not
"Fix: add tests" or "same as CLM-0012".

## Parent claims

A parent claim's verdict follows from its children. Judge the leaves first. Once every active
child is `supported`, record one `analysis` evidence item on the parent (locator
`children of CLM-0002`, the parent's own id) explaining how the children cover the parent, then
record the parent's verdict citing it. If any child is `refuted`, record `refutes` evidence on
the parent the same way, naming the refuted children, and mark the parent `refuted`. Until then,
`audit_report` lists the parent under `decomposition`.
