# Catalog format

The catalog is one JSON file, `claims.json` by default, meant to be committed alongside the
code. Change it only through the tools: hand edits break its [seals](../concepts/seals.md).

```json
{
  "schema_version": 2,
  "next_claim_seq": 8,
  "next_claim_seq_seal": { "by": "server", "mac": "<64 hex digits>" },
  "next_evidence_seq": 15,
  "next_evidence_seq_seal": { "by": "server", "mac": "<64 hex digits>" },
  "claims": [
    {
      "id": "CLM-0007",
      "statement": "parse_config rejects files larger than 1 MiB",
      "kind": "functional",
      "level": "software",
      "integrity_level": 3,
      "verification_method": "test",
      "source": "prompt: task-42 / spec.md#limits",
      "derived_from": ["CLM-0002"],
      "artifacts": ["src/config.rs"],
      "tags": ["config"],
      "status": { "state": "active" },
      "evidence": [
        {
          "id": "EVD-0014",
          "kind": "test",
          "locator": "tests/config.rs:88 oversized_file_rejected",
          "summary": "test passes; returns ConfigError::TooLarge",
          "stance": "supports",
          "recorded_at": "2026-10-23T14:02:11Z",
          "seal": { "by": "server", "mac": "<64 hex digits>" }
        }
      ],
      "verdicts": [
        {
          "verdict": "supported",
          "rationale": "Direct test covers the boundary and passes.",
          "auditor": "example-model",
          "evidence_ids": ["EVD-0014"],
          "recorded_at": "2026-10-23T14:03:40Z",
          "seal": { "by": "server", "mac": "<64 hex digits>" }
        }
      ],
      "created_at": "2026-10-23T13:58:00Z",
      "updated_at": "2026-10-23T14:03:40Z",
      "seal": { "by": "server", "mac": "<64 hex digits>" }
    }
  ]
}
```

- **Ids** look like `CLM-0007` and `EVD-0014`. They are never reused: the counters only increase,
  and evidence ids are unique across the whole catalog.
- **Deterministic output**: two-space indentation and a trailing newline, claims sorted by id,
  fields in a fixed order, so diffs stay small.
- **Optional fields** (`integrity_level`, `source`, `source_anchor`, evidence `anchor`) are
  omitted when absent. Empty lists are always written.
- **Anchors**, when present, are stored with the commit the server pinned, a `dirty` flag, and
  a search's `hits`; see [Anchors and recheck](../concepts/anchors.md).
- **Retired claims** have `"status": {"state": "retired", "reason": ..., "retired_at": ...}`.
- **`schema_version`** is 2. A version 1 file (written before seals) is read, but the server
  won't write to it until it is [migrated](../guides/migrating.md). Any other value is an error,
  and the file is not touched.

The full schema, with the invariants the server enforces, is in section 3 of the
[design spec](https://github.com/hypotheosis/claims-mcp/blob/main/docs/superpowers/specs/2026-10-23-claims-mcp-design.md).
