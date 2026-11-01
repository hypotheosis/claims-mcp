# Checking in CI

`claims-mcp verify` checks every [integrity seal](../concepts/seals.md) in the catalog. It exits 1
when any check fails and names each failing record, so a hand-edited catalog fails the pull
request:

```bash
claims-mcp --catalog claims.json verify
```

`--json` prints the result as JSON. The [PDF report](reports.md) takes that output as its
`integrity` input.

## A GitHub Actions step

Install a release binary (see [Install](install.md)), then:

```yaml
- name: Check the claims catalog's integrity seals
  run: claims-mcp --catalog claims.json verify
```

## Also worth gating

`claims-mcp recheck` exits 1 when any anchor is no longer current. Whether to fail CI on it is a
policy choice. Failing makes every change to anchored code trigger a re-audit. Not failing (and
reporting it instead) keeps drift visible without blocking merges. `claims-mcp tldr` always exits
0 while the catalog can be read, so use it for reporting, not gating.

## Branches that both write the ledger

Ids come from counters in the ledger, so two branches that both add claims, or both add
evidence, hand out the same ids. Git does not always see a conflict: two branches that each add
one evidence item to different claims merge cleanly into a ledger with two records of the same
id, and the server then refuses to read it. Run `claims-mcp verify` on the result of the merge,
which GitHub's `pull_request` event checks out by default, so a PR that would break the ledger
fails before it merges. To fix such a branch, take the main branch's `claims.json` and record the
branch's findings again through the tools; never edit or merge the file by hand.
