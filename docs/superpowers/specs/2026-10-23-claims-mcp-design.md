# claims-mcp Design

Date: 2026-10-23
Status: Approved (brainstorming), pending spec review

## 1. Purpose

`claims-mcp` is a stdio MCP server, built on the `rmcp` crate, that maintains a catalog of
**claims** in a single JSON file committed to the repository. A claim is one atomic, falsifiable
assertion about an artifact or a requirement. The catalog gives a durable, reviewable format for:

- **Requirements traceability**: a hierarchy of claims (`derived_from` links) from system level
  down to artifact level, with IEEE 1012 integrity levels and verification methods.
- **Verification and validation / conformance testing**: evidence attached to each claim and an
  append-only verdict history.
- **LLM output auditing (FEVER style)**: each claim extracted from an instruction is judged against
  the produced artifact as `supported`, `refuted`, or `not_enough_info`, and every verdict must cite
  evidence.

Both workflows carry equal weight. The server is a **pure ledger**: it never executes commands or
reads artifacts. The agent, guided by the `claims-audit` skill, does the verification and records
the results.

A companion skill (`skills/claims-audit/SKILL.md`) teaches an agent the end-to-end auditing process
using these tools.

## 2. Non-goals

- Running tests or shell commands from the server.
- Multiple catalogs per server instance. One server serves one catalog file; run more instances for
  more catalogs.
- Full revision history of claim text. Git history covers that; only verdicts are append-only.
- An HTTP transport. Only stdio is supported.

## 3. Data model

### 3.1 File format (`claims.json`)

```jsonc
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
          "auditor": "claude-opus-5-5",
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

- **Determinism**: the file is pretty-printed with 2-space indentation and a trailing newline.
  Claims are sorted by numeric id and fields are emitted in declaration order, so diffs are minimal
  and stable.
- **Ids** are never reused. `next_claim_seq` and `next_evidence_seq` only ever increase. Evidence ids
  are unique across the whole catalog.
- **Optional fields** (`integrity_level`, `source`, `source_anchor`, evidence `anchor`) are
  omitted when absent. Empty arrays are always
  emitted, so the shape is uniform.
- `schema_version` is `2`. A `1` (written before integrity seals, issue #9) is still read, but
  the server refuses to write to it until `claims-mcp seal --adopt` migrates it. Any other value
  is an error and the file is not touched.
- **Integrity seals** (issue #9): each id counter (`next_*_seq_seal`) and every claim, evidence
  item, and verdict (`seal`) carry `{by, mac}`, an HMAC-SHA256 under a key built into the binary. `by` is `server`, or
  `adopt` for records sealed by `seal --adopt`. Each MAC covers the record and its place in the
  catalog: evidence binds its claim id; a verdict binds its claim id, index, and the previous
  verdict's MAC; a claim binds its own fields, its evidence ids, and its latest verdict's MAC;
  each counter binds `schema_version` and its value. Any gap below `next_claim_seq` means a
  deleted claim, and lowering the counter to hide the newest claim's deletion breaks its seal.
  Sealing the counters separately lets a branch that added a claim merge with one that added
  evidence. `seal --adopt` refuses a v1 file that carries seals (a lowered version). The server verifies every seal on every call and refuses all writes while any
  check fails; reads and `audit_report` still work. This deters casual hand edits; it is not
  cryptographic security, because the key is in the public source. See `src/seal.rs` and the
  README's "Integrity seals" section.
- **Anchors** (issue #21): an evidence item may carry `anchor`, and a claim `source_anchor`,
  pinned to the commit at `HEAD` of the catalog's git repository when recorded:
  `{"file": {"path", "lines"?: {"start", "end"}, "commit", "dirty"?}}` or (evidence only)
  `{"search": {"pattern", "case_insensitive"?, "scope": [..], "hits", "commit", "dirty"?}}`.
  The server sets `commit`, `dirty` (the anchored paths had uncommitted changes), and a
  search's `hits` (from `git grep -c -E` at that commit); callers never supply them. Both
  fields are omitted when absent, so records written before them keep their seals, and the
  schema version stays 2. `src/anchor.rs` validates and pins; `src/recheck.rs` compares with
  `HEAD`.

### 3.2 Enumerations (serialized snake_case)

| Field | Values |
|---|---|
| `kind` | `functional`, `non_functional`, `interface`, `performance`, `safety`, `security`, `documentation`, `constraint` |
| `level` | `system`, `software`, `component`, `unit`, `artifact` |
| `verification_method` | `inspection`, `analysis`, `demonstration`, `test`, `formal_proof` |
| evidence `kind` | `test`, `inspection`, `analysis`, `demonstration`, `proof`, `log`, `other` |
| evidence `stance` | `supports`, `refutes`, `neutral` |
| `verdict` | `supported`, `refuted`, `not_enough_info` |
| `status.state` | `active`, `retired` (retired also carries `reason` and `retired_at`) |

`integrity_level` is a newtype over `u8` that is valid only from 1 to 4 (the IEEE 1012 integrity
levels; 4 is the highest consequence).

### 3.3 Rust types

- `ClaimId(u32)` and `EvidenceId(u32)` are newtypes with custom `Serialize`/`Deserialize`/`FromStr`
  and `Display` (`CLM-{:04}` and `EVD-{:04}`; widths grow past 9999). Parsing accepts the prefix in
  any case. Ordering is numeric.
- `ClaimStatus` is an enum: `Active` or `Retired { reason: String, retired_at: DateTime<Utc> }`,
  serialized with `#[serde(tag = "state")]`.
- The **current verdict** is derived, not stored: it is the last `verdicts` entry, or `Unverified`
  when the list is empty. Reports and filters use a `CurrentVerdict` enum
  (`Unverified | Supported | Refuted | NotEnoughInfo`).
- Timestamps are `chrono::DateTime<Utc>`, serialized as RFC 3339.

### 3.4 Invariants enforced by `Catalog`

1. Every `derived_from` id exists in the catalog. It may be retired, but a new or updated link may
   not point at a retired claim. An existing link to a parent retired later may be kept (it is
   reported as `dangling_links`).
2. `derived_from` never forms a cycle, including self-links. This is checked by DFS on every add or
   update that changes links.
3. Retired claims reject `add_evidence`, `record_verdict`, and `update_claim`.
4. `record_verdict` rules:
   - Every cited `evidence_id` must belong to the same claim.
   - `supported` must cite at least one evidence item with stance `supports`.
   - `refuted` must cite at least one evidence item with stance `refutes`.
   - `not_enough_info` may cite zero evidence, but must have a non-empty rationale.
   - `rationale` and `auditor` must be non-empty after trimming.
5. `statement` must be non-empty after trimming. Duplicate tags and artifacts are removed (first
   occurrence wins).
6. `add_claims` is all-or-nothing. Claims within one batch may reference each other through
   `derived_from` using the batch-local placeholder `@N`, where N is the 0-based index in the batch. N must be smaller than the referring entry's own index.
   This lets a hierarchy be registered in one call.

## 4. MCP tool surface

All tools return structured JSON (`CallToolResult` with JSON text content). Domain errors are
returned as tool errors (`is_error: true`) with a clear message, so the agent can recover and the
server does not fail the protocol request.

| Tool | Parameters | Returns |
|---|---|---|
| `add_claim` | `statement`, `kind`, `level`, `verification_method`, optional `integrity_level`, `source`, `source_anchor` (`{path, lines?}`), `derived_from[]`, `artifacts[]`, `tags[]` | the full claim |
| `add_claims` | `claims[]` (same shape as `add_claim`; `derived_from` accepts `@N`) | the created claims, in order |
| `get_claim` | `id` | the claim, plus `current_verdict` and `children[]` (ids of active claims whose `derived_from` contains it) |
| `list_claims` | optional `verdict` (includes `unverified`), `kind`, `level`, `tag`, `artifact` (substring), `text` (case-insensitive substring of the statement), `include_retired` (default false) | summaries: `id`, `statement`, `kind`, `level`, `integrity_level`, `current_verdict`, `evidence_count`, `retired` (bool) |
| `update_claim` | `id` plus any optional metadata field; list fields replace the whole list | the updated claim |
| `retire_claim` | `id`, `reason` | the updated claim |
| `add_evidence` | `claim_id`, `kind`, `locator`, `summary`, `stance`, optional `anchor` (`{file: {path, lines?}}` or `{search: {pattern, case_insensitive?, scope[], hits?}}`) | the evidence item, including its id and pinned anchor |
| `record_verdict` | `claim_id`, `verdict`, `rationale`, `auditor`, `evidence_ids[]` | the claim, with the new verdict appended |
| `trace` | `id`, `direction` (`up` / `down` / `both`, default `both`), optional `max_depth` (default unlimited) | breadth-first `ancestors` and `descendants` lists of `{id, statement, current_verdict, depth, retired}` nodes (each claim once, at its shortest depth) |
| `audit_report` | optional `include_matrix` (default true) | the report described in 4.1 |
| `recheck` | none | `head`, `checked`, `current`, and `items[]` (`claim`, `role`, `evidence`?, `stance`?, `anchor`, `status`, `detail`) for every anchor on an active claim that is not current; `status` is `moved`, `changed`, `overturned`, `missing`, `commit_missing`, or `unpinned` |

The server also lists read-only resources, one per PDF report file compiled into the binary
(`src/report/files.rs`, issue #18): `claims://report/claims-report.typ`,
`claims://report/text.typ`, and `claims://report/audit-notes.example.toml`. Each is served as
text, byte-identical to the file in `report/`. The server instructions name the notes example
and the `claims-mcp report init` command, which writes the same files into a project.

### 4.1 `audit_report` contents (active claims only)

- `totals`: counts by current verdict, kind, level, and integrity level (including `unassigned`).
- `gaps`:
  - `no_evidence`: claims with zero evidence.
  - `unverified`: claims with no verdict.
  - `refuted`: claims whose current verdict is `refuted`.
  - `not_enough_info`: claims whose current verdict is `not_enough_info`.
  - `stale`: claims where any evidence `recorded_at` is later than the latest verdict's
    `recorded_at`.
  - `rigor`: claims with integrity level 3 or 4 whose `verification_method` is `inspection`.
    IEEE 1012 expects analysis, test, or formal methods at higher integrity levels.
  - `decomposition`: claims that have active children where at least one child's current verdict
    is not `supported`. A parent is only as verified as its decomposition.
  - `dangling_links`: active claims whose `derived_from` includes a retired claim.
  - `refuted_without_source_anchor`: refuted claims with no `source_anchor`, so nothing points
    at the text that makes the claim.
- `matrix` (traceability matrix): one row per active claim with `id`, `statement`, `derived_from`,
  `children`, `verification_method`, `evidence_locators`, `refuting_locators` (each `refutes`
  evidence item's anchor in short form, or its locator when it has none), and
  `current_verdict`.

Each gap lists claim ids. The report is computed in one pass over the claims plus one child-index
build, which is O(n + e) where e is the number of links.

## 5. Architecture

```
src/
  main.rs     CLI (clap): --catalog <path> (env CLAIMS_CATALOG, default ./claims.json);
              tracing to stderr; builds CatalogStore and ClaimsServer; serves rmcp over stdio.
  error.rs    ClaimsError (thiserror): NotFound, Validation, Cycle, Retired, Io, Parse,
              UnsupportedSchema, Join.
  model.rs    ids, enums, Claim, Evidence, VerdictRecord, ClaimStatus, IntegrityLevel, and
              CurrentVerdict.
  catalog.rs  Catalog: pure in-memory operations and invariant checks. No I/O.
  report.rs   audit_report and trace: pure functions over &Catalog.
  store.rs    CatalogStore: locking, loading, atomic saving; read/mutate helpers.
  server.rs   ClaimsServer: rmcp #[tool_router]/#[tool_handler], schemars parameter structs,
              ClaimsError to tool-error mapping.
```

Dependencies, in order:

- `main` depends on `server` and `store`.
- `server` depends on `store`, `catalog`, `report`, and `model`.
- `store` depends on `catalog`.
- `catalog` and `report` depend on `model`.

Only `store` touches the filesystem, apart from the `report init` CLI command
(`report::files::init`), and only `server` knows about MCP.

### 5.1 Store data flow

```
tool call -> server handler -> store.mutate(|catalog| catalog.op(...))
    spawn_blocking:
        open/create  <dir>/.<file_name>.lock   ; File::lock() (exclusive, std >= 1.89)
        read <catalog> (missing file => Catalog::default())
        serde_json::from_slice, check schema_version (1 or 2)
        verify every seal (any failure => UnsealedCatalog / CatalogTampered, nothing written)
        run closure  (Err => nothing written)
        re-seal, as the server, every record whose seal no longer matches
        serialize (pretty, trailing newline) -> <dir>/.<file_name>.tmp
        fsync tmp, rename tmp -> catalog
        lock released on drop
-> serialize closure result -> CallToolResult
```

`store.read` takes the same lock, loads the catalog, and never writes. The catalog is re-read on
every call, so `git checkout` and concurrent server instances are always respected. Hand edits
are detected by the integrity seals and block writes until the file is restored.
The cost is O(file size) per call, which is acceptable for expected catalog sizes (up to the low
thousands of claims).

### 5.2 Template changes

- Remove `console-subscriber` (it binds TCP 6669, which conflicts across instances) and the
  `tokio_unstable` rustflag.
- Remove `ctrlc` and `lazy_static` (unused; stdio EOF ends the server).
- Replace `dotenv` with `dotenvy`.
- Add:
  - `rmcp` (`server`, `macros`, `transport-io`)
  - `schemars` 1.x
  - `serde_json`
  - `chrono` (`serde`, `clock`)
  - `clap` (`derive`, `env`)
  - `anyhow`
  - `tempfile` (dev-dependency)
- Tracing writes to **stderr** only (stdout is the MCP channel), with default filter `warn`.
- `.gitignore`: add `.claims.json.lock` and `.claims.json.tmp`.

`rayon` and SIMD are deliberately not used: the workload is small and bound by file I/O.

## 6. Error handling

- `ClaimsError` (thiserror) covers domain and I/O failures. `server.rs` maps them to a tool result
  with `is_error: true` and the `Display` message (for example `claim CLM-0099 not found`, or
  `verdict 'supported' requires at least one cited evidence item with stance 'supports'`).
- A parse failure or unsupported schema refuses to touch the file and reports the path and the
  serde error location.
- `main` uses `anyhow` with `.context()` for startup failures (bad catalog path, transport errors)
  and logs with `tracing::error!`.
- There is no `.unwrap()` outside tests.

## 7. Testing

- `model.rs`: id parse and display round trips (case, width growth, bad input), integrity level
  bounds, and enum serde names.
- `catalog.rs`:
  - every operation succeeds on the happy path;
  - every invariant in 3.4 is rejected when violated (cycle, self-link, missing parent,
    retired-target link, verdict/evidence mismatch, foreign evidence id, empty rationale);
  - `@N` batch references work, and a failing batch leaves the catalog unchanged;
  - ids are never reused after retirement.
- `report.rs`: each gap category, the totals, matrix rows, and `trace` in all directions with depth
  limits.
- `store.rs` (with `tempfile`): a missing file creates a catalog on first mutate; write/read round
  trip; output is byte-identical across two saves; a failed closure leaves the file untouched; a
  bad schema version errors; no tmp file remains after a save.
- `server.rs`: handlers are called directly against a temp catalog to cover JSON shapes and the
  error mapping.

## 8. Skill: `skills/claims-audit/SKILL.md`

Frontmatter `name: claims-audit`, with a description that triggers on claim auditing, verification
and validation, traceability, FEVER, IEEE 1012, conformance, and "verify the agent did what it was
told".

Workflow:

1. **Scope**: identify the artifact under audit and the instruction or spec it came from. Record
   the source string.
2. **Extract**: break the instruction into atomic, falsifiable claims (one assertion, no "and",
   observable condition). Build the hierarchy (system -> software -> component -> unit -> artifact)
   and register it in one `add_claims` call using `@N` links. Assign integrity levels (guidance
   table: 4 = safety, security, or data loss; 3 = core function; 2 = degraded function; 1 = cosmetic
   or docs) and verification methods that match the integrity level.
3. **Gather evidence**: for each claim, inspect code, run tests, and read output independently,
   without trusting the producing agent's own summary. Record each finding with `add_evidence`,
   using precise locators and an honest stance, including refuting and neutral findings.
4. **Adjudicate**: apply FEVER rules. Record `supported` only with direct supporting evidence,
   `refuted` when contradicting evidence exists, and `not_enough_info` otherwise. Never infer
   support from absence of failure.
5. **Report**: run `audit_report` and summarize totals and every gap category with remediation
   proposals.
6. **Re-audit**: after fixes, target `stale`, `refuted`, and `not_enough_info` claims only.

Anti-patterns section: verdicts without independent evidence, compound claims, editing a claim's
statement to make it pass (retire and re-add instead), and ignoring `rigor` gaps.

## 9. Deliverables

- Source as in section 5, with doc comments on every public item.
- `skills/claims-audit/SKILL.md`.
- `.mcp.json` registering `claims` with
  `cargo run --release --quiet -- --catalog claims.json`.
- `README.md`: build, configuration, tool reference, and skill installation
  (`cp -r skills/claims-audit ~/.claude/skills/`).
- `changelog/<timestamp>.md` per the CLAUDE.md rule.
- `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt --check` all pass.
