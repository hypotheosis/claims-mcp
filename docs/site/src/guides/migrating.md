# Migrating with `seal --adopt`

Catalogs written before [integrity seals](../concepts/seals.md) existed (`schema_version` 1) can
still be read, but the server refuses to write to them until a maintainer runs:

```bash
claims-mcp --catalog claims.json seal --adopt   # then commit claims.json
```

It seals every unsealed record and labels it `adopt`. `verify` and `audit_report` list adopted
records separately from ones the server recorded.

Adoption **never** re-seals a record whose seal is present but wrong, so it can't hide an edit to
a sealed record. It also refuses when a claim is missing, and when a `schema_version` 1 file
carries seals (a genuine v1 file has none, so the version was lowered by hand). Removing a seal by
hand and adopting the record is possible, but the record then shows as adopted in every report
and in the git diff.

If two branches changed the *same* claim, the merge breaks that claim's seal. Keep one side, and
redo the other side's change through the tools.

## Keeping agents away from the file

Agents should never run `seal --adopt`; the server's own refusal message says so. To keep agents
away from the file entirely, add deny rules to the project's `.claude/settings.json`:

```json
{
  "permissions": {
    "deny": ["Edit(**/claims.json)", "Write(**/claims.json)", "Bash(claims-mcp seal:*)"]
  }
}
```
