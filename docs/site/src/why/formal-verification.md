# Alongside formal verification

Formal verification proves that code meets a specification for every possible input. It is
the strongest assurance software can have, and AI is making it much cheaper. So why keep a
ledger of claims judged by an auditor?

Because a proof answers one narrow question very well, and most of what you need to trust
lies outside it. claims-mcp records everything else, and can hold the proof as its strongest
evidence.

## What formal verification gives you

Take a Rust project verified with [Aeneas](https://github.com/AeneasVerif/aeneas). Charon
extracts the crate's MIR to an intermediate form (LLBC). Aeneas turns that into a pure
functional model in a proof assistant such as [Lean](https://lean-lang.org). You write a
specification in Lean, for example "`sort` returns a permutation of its input, in order", and
prove that the model meets it. Lean's small kernel checks the proof. If it checks, the property
holds for every input, not just the ones a test tried.

That is a guarantee no amount of testing or auditing can match. claims-mcp doesn't compete
with it.

## What a proof can't tell you

A proof shows that *the model* meets *the spec*, assuming *the checker* is sound. Each of those
three words is a gap, and there is a fourth gap around all of them.

### The spec gap: does the spec say what you meant?

A proof can be valid and say nothing useful. "Returns a sorted list" holds for a function that
always returns `[]`. A spec with contradictory assumptions proves anything. When an AI writes
both the code and the spec, it may quietly weaken the theorem until the proof goes through.
Whether a formal spec captures the requirement is a judgment about meaning, and no kernel
checks meaning.

**In claims-mcp:** a claim such as "`sort_spec` states requirement REQ-7", whose evidence
compares the Lean statement with the requirement text, judged by someone other than the
spec's author.

### The model gap: does the model behave like the binary?

The proof is about a model of the code produced by a translator. Translators have bugs and
unsupported features, and the shipped binary goes through a compiler the proof never saw.

**In claims-mcp:** a claim that the model matches the program, with **differential testing** as
evidence: run the model and the real binary on the same inputs and compare the outputs.

### The trusted base: what did the proof assume?

Proof assistants have escape hatches. In Lean, `sorry` and `admit` mark unfinished proofs,
an `axiom` can assert anything, and `native_decide` trusts compiled code instead of the kernel.
The toolchain (Charon, Aeneas, rustc) is also trusted.

**In claims-mcp:** an *absence* claim. A `search` anchor for the escape hatches over the proof
directory, recorded with zero hits, is evidence that none are used. If one appears later,
[`recheck`](../concepts/anchors.md) reports the anchor as `overturned`. A separate claim lists
the trust assumptions explicitly.

### Coverage: what can't be proved at all?

Proof tools reach a subset of code. Aeneas handles safe, sequential Rust; it doesn't handle
`unsafe`, concurrency or I/O. No one formally verifies a README, a deployment config, an
install script or an agent's report of what it did. In practice, most failures come from
requirements and integration, not from the algorithmic core.

**In claims-mcp:** ordinary claims, with evidence from tests, inspection and demonstration, for
everything the proof doesn't reach. That is most of the system.

## Side by side

| | Formal verification (for example Aeneas and Lean) | claims-mcp |
|---|---|---|
| Question answered | Does this function meet this spec, for every input? | What did we claim, what evidence backs each claim, and what was the verdict? |
| Scope | A few critical functions: narrow and deep | A whole system and its documentation: broad |
| Who judges | A proof checker's kernel | An auditor, citing recorded evidence |
| Strength of "yes" | Mathematical, given the spec and the trusted base | As strong as the cited evidence |
| When code changes | The proof breaks and must be repaired | `recheck` flags anchors that moved, changed or were overturned |
| Cost | High; needs proof-assistant skills | Low |

## Using them together

The two fit naturally. A proof becomes the evidence for a high-integrity claim, and the audit
covers the questions around it. A verified function might carry these claims:

```json
{"tool": "add_claims", "arguments": {"claims": [
  {"statement": "parse_header never reads past its input and rejects malformed lengths",
   "kind": "safety", "level": "unit", "integrity_level": 4,
   "verification_method": "formal_proof", "artifacts": ["src/header.rs"]},
  {"statement": "The Lean spec parse_header_spec states the header requirement",
   "kind": "documentation", "level": "artifact", "integrity_level": 4,
   "verification_method": "inspection", "derived_from": ["@0"]},
  {"statement": "The proofs use no sorry, admit or axiom",
   "kind": "constraint", "level": "artifact", "integrity_level": 4,
   "verification_method": "analysis", "derived_from": ["@0"]},
  {"statement": "The Lean model of parse_header matches the compiled function on 10,000 random inputs",
   "kind": "functional", "level": "artifact", "integrity_level": 4,
   "verification_method": "test", "derived_from": ["@0"]}
]}}
```

The evidence for each:

- **The proof itself**: `kind: "proof"`, with a `file` anchor on the theorem in the `.lean`
  file and the proof checker's output as the locator.
- **No escape hatches**: a `search` anchor, recorded with zero hits:

  ```json
  {"search": {"pattern": "\\bsorry\\b|\\badmit\\b|^axiom |native_decide", "scope": ["proofs"]}}
  ```

- **Model fidelity**: the differential-testing run, as test evidence.
- **Spec fidelity**: an inspection comparing the spec with the requirement, recorded by a
  reviewer who did not write the spec.

The `integrity_level` of 4 matters. `audit_report` flags level 3-4 claims verified only by
inspection as `rigor` gaps, so the proof claim needs `formal_proof` (or `test`, `analysis` or
`demonstration`). The spec-fidelity claim is inspection by nature, so it shows up as a gap
that a person has to look at. That is deliberate.

## Questions for any "formally verified" claim

Leo de Moura, who created Lean, suggests asking four questions of any claim that something
is formally verified. Each maps onto a claim you can record and recheck:

| Question | As a claim in the ledger |
|---|---|
| Show me the specifications. | "Spec X states requirement Y", judged by an independent reviewer |
| Which properties were verified? | "Theorem T proves property P", anchored to the `.lean` file |
| Which proof checker and kernel? | "The proofs pass Lean's kernel and an independent checker", with CI logs as evidence |
| How large is the trusted base? | The zero-hit escape-hatch search, plus a claim listing the toolchain assumptions |

## AI makes this more important, not less

LLMs are getting good at writing Lean. Proofs that took experts months can be drafted quickly,
and proof repair after a code change, historically the biggest cost of formal methods, is
exactly the kind of task models do well. Because the kernel rejects wrong proofs, AI-written
proofs can be trusted in a way AI-written code can't.

The weak point moves to the spec. If a model writes the spec, the code and the proof, you get
a verified program that may do the wrong thing. The human's job shifts from reviewing code to
reviewing specs, and the review needs to be recorded, independent and rechecked as things
change. That is what an audit ledger is for.

## Sources

- Aeneas: <https://github.com/AeneasVerif/aeneas>
- Lean: <https://lean-lang.org>
- Ben Lorica and Leo de Moura, "When AI Writes the Code, Who Proves It Works?", The Data
  Exchange, October 2026: <https://thedataexchange.media/leo-de-moura-lean-aws/>
