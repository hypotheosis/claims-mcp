# Security

Report vulnerabilities privately, through
[GitHub's private vulnerability reporting](https://github.com/hypotheosis/claims-mcp/security/advisories/new),
not in a public issue. The full policy, including supported versions, is in
[`SECURITY.md`](https://github.com/hypotheosis/claims-mcp/blob/main/SECURITY.md).

Two properties matter when you decide how far to trust a catalog:

- **Seals are a deterrent, not cryptographic security.** The HMAC key is in the public source.
  See [Integrity seals](../concepts/seals.md#what-seals-are-not).
- **The server doesn't execute your code.** It only reads and writes the catalog and runs
  read-only `git` queries for [anchors](../concepts/anchors.md). Search scopes and file paths
  outside the repository are rejected.
- **A tool call can name its own ledger.** The [`catalog` parameter](tools.md#choosing-the-ledger)
  lets the client point a call at any path the server's user can write, where it creates the
  ledger and its `.<name>.lock` sidecar. That is no more than the client that started the
  server can already do, but don't expose the server to callers you wouldn't let write files.
