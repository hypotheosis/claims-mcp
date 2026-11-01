# Branches that both write the ledger

Claim and evidence ids come from two counters in the ledger. Two branches that both add claims,
or both add evidence, therefore hand out the same ids. Merging them in git goes wrong in one of
two ways:

- **No conflict.** Two branches that each add one evidence item to different claims merge
  cleanly into a ledger with two `EVD-0001` records. `claims-mcp verify` and every tool then
  fail with `catalog is inconsistent: EVD-0001 appears more than once`.
- **A conflict.** With unequal counts, or new claims, git reports a conflict in `claims.json`.
  Resolving it by hand breaks the [seals](../concepts/seals.md).

Never resolve either case by editing the file. `claims-mcp rebase` does it for you instead.

## Rebase the ledger during the merge

Bring `main` into the branch with a merge, and replace whatever git made of `claims.json` with
the branch's records replayed onto `main`'s ledger:

```bash
git fetch origin
git merge --no-commit origin/main      # claims.json may conflict, or merge "cleanly" and break
git checkout HEAD -- claims.json       # the branch's own ledger, as it was before the merge
git show MERGE_HEAD:claims.json > /tmp/main.json
git show "$(git merge-base HEAD MERGE_HEAD)":claims.json > /tmp/base.json || rm /tmp/base.json
claims-mcp --catalog claims.json rebase --onto /tmp/main.json --base /tmp/base.json
claims-mcp --catalog claims.json verify
git add claims.json && git commit      # finishes the merge
```

If `claims.json` did not exist at the merge base, the branch created the ledger. The `rm`
removes the empty file `git show` left, and a missing `--base` counts as an empty ledger.

`rebase` prints which ids changed:

```text
rebased claims.json onto /tmp/main.json: 1 new claim, 2 new evidence items
  CLM-0003 -> CLM-0004
  EVD-0001 -> EVD-0002
  EVD-0002 -> EVD-0003
```

Update the pull request description with it. `--json` prints the same mapping as
`{"claims": [{"from": ..., "to": ...}], "evidence": [...]}`.

## What it replays

`--base` is the ledger both sides started from. Everything the branch did since then is
replayed onto `--onto`:

- **New claims** get the next free claim ids on `--onto`, in their original order.
- **New evidence**, on new or existing claims, gets the next free evidence ids, in id order.
- **New verdicts** on existing claims are appended after any that `--onto` added, and the
  evidence ids they cite are renumbered.
- **Field changes** to existing claims (`update_claim`, `retire_claim`) are applied, unless
  `--onto` changed that claim's fields too. Where both sides made the same change, that's not a
  conflict.

`derived_from` links to renumbered claims are rewritten. Records that existed at `--base` keep
their ids, and anchors keep the commits they were pinned to, so merge the pull request with a
merge commit as usual. The result is resealed and written atomically over `--catalog`.

## When it refuses

`rebase` changes nothing and exits 1 when:

- any of the three ledgers fails its integrity check, so a hand edit can't be laundered;
- the branch or `--onto` doesn't descend from `--base`, which usually means `--base` is not
  the merge base;
- `--onto` already holds the branch's new records, because the branch was already rebased onto
  it;
- both sides changed the same claim's fields differently;
- the branch added evidence or verdicts to a claim that `--onto` retired;
- links from the two sides form a `derived_from` cycle.

To fix a conflict, make the two sides agree through the tools on one of them, then rebase
again.

## Catching it in CI

Run `claims-mcp verify` on the merge result, which GitHub's `pull_request` event checks out by
default (see [Checking in CI](ci.md)). A pull request whose ledger would break `main` then fails
before it merges.
