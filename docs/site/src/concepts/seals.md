# Integrity seals

The catalog is meant to change only through the tools, which enforce its rules: verdicts must
cite evidence, the history is append-only, and claims are retired rather than deleted. To stop
an agent skipping the tools and editing `claims.json` directly, every claim, evidence item and
verdict, plus each of the two id counters, carries a **seal**: an HMAC-SHA256 over the record's
content and its place in the catalog, under a key built into the binary.

- **Every call verifies every seal.** If any check fails, the server **refuses every write**
  until the file is restored. Reads and `audit_report` keep working, so the damage can be
  inspected. There is no option to turn this off.
- **What counts as tampering:** a changed field, an added record without a seal, a deleted
  claim, evidence item or verdict, reordered verdicts, evidence moved to another claim, a
  changed id counter, and a `schema_version` lowered to 1.
- **Recovering:** restore the file from git (`git checkout -- claims.json`, or revert the
  commit that edited it), then redo the change through the tools.
- **Not affected:** reformatting the file or reordering its keys (seals cover content, not
  layout), and git merges of branches that changed *different* claims. Each counter has its
  own seal, so a branch that added a claim merges cleanly with one that added evidence. Two
  branches that both added claims, or both added evidence, hand out the same ids; replay one
  onto the other with [`claims-mcp rebase`](../guides/parallel-branches.md).

## How a seal binds a record

Each seal is `{"by": "server", "mac": "<64 hex digits>"}`, or `"by": "adopt"` for records sealed
during [migration](../guides/migrating.md). The MAC covers the record and its place:

- evidence binds its claim id;
- a verdict binds its claim id, its index, and the previous verdict's MAC, so verdicts can't be
  reordered or dropped;
- a claim binds its own fields, its evidence ids, and its latest verdict's MAC;
- each counter binds `schema_version` and its value. Ids are never reused, so a gap below the
  counter reveals a deleted claim, and lowering the counter to hide the newest claim's deletion
  breaks the counter's seal.

## What seals are not

Seals are a deterrent, not cryptographic security. The key is in the public source, so anyone
who reads it can forge seals. What they stop is the casual edit: in your project, an agent would
have to go and find the key and write code to recompute seals. To keep agents away from the
file entirely, add [deny rules](../guides/migrating.md#keeping-agents-away-from-the-file).

`claims-mcp verify` checks every seal from the command line and exits 1 on any failure, so CI
can reject a hand-edited catalog; see [Checking in CI](../guides/ci.md).
