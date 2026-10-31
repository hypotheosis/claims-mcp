# Catalog JSON Schema

The generated [JSON Schema](../schema/claims.schema.json) is the official machine-readable
definition of `claims.json`. It uses JSON Schema 2020-12 and describes catalog schema version 2.
The stable URL always describes the latest catalog format:

```text
https://hypotheosis.github.io/claims-mcp/schema/claims.schema.json
```

Consumers that need a format-version pin can use
[`schema/v2/claims.schema.json`](../schema/v2/claims.schema.json). Every GitHub release also
contains `claims.schema.json`, so consumers can pin to a claims-mcp release.

To associate `claims.json` with the schema in VS Code, add this to `settings.json`:

```json
{
  "json.schemas": [
    {
      "fileMatch": ["claims.json"],
      "url": "https://hypotheosis.github.io/claims-mcp/schema/claims.schema.json"
    }
  ]
}
```

Maintainers regenerate both committed copies with `just schema`. The parity test fails if the
Rust model and committed schema differ.

## Rules beyond JSON Schema

The server enforces relationships and history that a schema cannot express:

- Integrity seals authenticate records and counters, bind evidence to its claim and position,
  and chain each verdict to the preceding verdict.
- Claim and evidence ids are never reused; both sequence counters only increase.
- Every `derived_from` id exists, cannot refer to a claim newly retired by the same operation,
  and the resulting graph has no cycle.
- A `supported` verdict cites at least one `supports` evidence item, and a `refuted` verdict
  cites at least one `refutes` item.
- `not_enough_info` may omit evidence; other verdicts may not.
- The server, rather than tool input, sets anchor `commit`, `dirty`, and `hits` fields.
