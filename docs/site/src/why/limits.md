# Where it doesn't fit

claims-mcp is useful because it is honest about what a verdict means. Here is the same honesty
about the tool itself.

- **A verdict is a judgment, not a proof.** `supported` means an auditor looked at cited
  evidence and concluded the claim holds. The server enforces that the evidence exists and
  points the right way (`supported` must cite `supports` evidence). It cannot check that the
  auditor read the evidence correctly. When you need "holds for every input", use
  [formal verification](formal-verification.md) and record the proof as evidence.
- **The ledger is only as good as its evidence.** A vague locator, or an inspection where a
  test was needed, gives a weak verdict. `audit_report` flags some of this (high-integrity
  claims checked only by inspection show up as `rigor` gaps), but it can't judge quality.
- **The server never runs your code.** It records what the auditor observed. The only commands
  it runs are read-only `git` queries for anchors (`rev-parse`, `status`, `grep`, `diff`,
  `cat-file`). Running tests,
  programs and proof checkers is the auditor's job.
- **Seals deter hand edits; they are not cryptographic security.** The HMAC key that seals each
  record is in the public source, so someone determined can forge seals. What seals stop is
  the casual edit, in particular an agent "fixing" a verdict by editing the JSON. See
  [Integrity seals](../concepts/seals.md).
- **Anchors need git.** Evidence without anchors works anywhere, but commit pinning and
  `recheck` need the catalog to be inside a git repository.
- **Claims must be atomic to be useful.** One claim covering five behaviours hides partial
  failure. Splitting claims well takes judgment; the skill guides it, but nothing enforces it.
- **One catalog per server.** Each server instance keeps one `claims.json`, over stdio only.
