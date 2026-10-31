# Command line

```text
claims-mcp [--catalog PATH] [SUBCOMMAND]
```

Without a subcommand, claims-mcp runs the MCP server on stdio. MCP clients such as Claude Code
normally start it.

| Option | Env | Default | Meaning |
|---|---|---|---|
| `--catalog` | `CLAIMS_CATALOG` | `claims.json` | Catalog file; created on first write |
| | `RUST_LOG` | `warn` | Log filter; logs go to stderr |

The catalog is re-read on every call and written atomically (temporary file, fsync, rename)
under an exclusive lock on `.<name>.lock`. Git checkouts and several server instances sharing
one file are safe. Add `.<name>.lock` and `.<name>.tmp` to `.gitignore`.

## Subcommands

The subcommands are for people and CI; agents use the [MCP tools](tools.md).

| Subcommand | Purpose | Exit code |
|---|---|---|
| *(none)* | Run the MCP server on stdio | |
| `verify [--json]` | Check every [integrity seal](../concepts/seals.md) | 1 if any check fails |
| `tldr [--limit N] [--json]` | Print the [ranked next steps](../guides/first-audit.md#the-ranked-to-do-list); `--json` matches the `next_steps` tool | 0 whenever the catalog can be read |
| `recheck [--json]` | Compare every anchor with `HEAD`; `--json` matches the `recheck` tool. See [Recheck](../concepts/anchors.md#recheck) | 1 if any anchor is not current |
| `seal --adopt` | Seal unsealed records once, to [migrate a catalog](../guides/migrating.md); maintainers only | |
| `report init [DIR]` | Write the [PDF report](../guides/reports.md) template and a notes file to `DIR` (default `report`) | |
| `schema [--versioned]` | Print the [catalog JSON Schema](catalog-schema.md); `--versioned` uses its pinned URL as `$id` | |

`--catalog` is global, so it can go before or after the subcommand.
