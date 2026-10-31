# claims-mcp

**Documentation: <https://hypotheosis.github.io/claims-mcp/>**

A stdio [MCP](https://modelcontextprotocol.io) server, built on the
[`rmcp`](https://crates.io/crates/rmcp) crate, that maintains a catalog of **claims** in a single
JSON file you commit alongside your code.

A claim is one atomic, falsifiable assertion about an artifact or requirement. Each claim
carries evidence and an append-only history of FEVER verdicts (`supported`, `refuted`,
`not_enough_info`). Together they give a durable record for:

- **Requirements traceability**: claims link to parent claims (`derived_from`), from system
  level down to individual artifacts.
- **Verification and validation** (IEEE 1012): integrity levels 1-4 and verification methods
  (inspection, analysis, demonstration, test, formal proof).
- **Conformance and LLM output auditing**: check that a produced artifact matches the
  instruction it was produced from.

The server only records; it never runs your code. Verification is done by the agent, guided by
the bundled [`claims-audit`](skills/claims-audit/SKILL.md) skill. Every record is sealed, so a
hand-edited catalog is detected and blocks writes.

## When to use it

- Checking that documentation still matches the code, with evidence pinned to commits and a
  `recheck` that flags drift.
- Auditing what a coding agent produced against the task it was given.
- Keeping a V&V record that links requirements to the evidence for them.

Formal proofs (for example Lean, via Aeneas) show that a function meets a spec for every input.
claims-mcp records whether the spec, the docs, and everything a proof can't reach hold up, and
can hold the proof as its strongest evidence. See
[Alongside formal verification](https://hypotheosis.github.io/claims-mcp/why/formal-verification.html).

## Quickstart

1. **Install** a binary from the [releases](https://github.com/hypotheosis/claims-mcp/releases)
   (Linux and macOS shown; Windows and other options are in the
   [install guide](https://hypotheosis.github.io/claims-mcp/guides/install.html)):

   ```bash
   sha256sum -c claims-mcp-<version>-<target>.tar.gz.sha256   # macOS: shasum -a 256 -c
   tar -xzf claims-mcp-<version>-<target>.tar.gz
   mkdir -p ~/.local/bin
   install -m 755 claims-mcp-<version>-<target>/claims-mcp ~/.local/bin/
   ```

   or `cargo install --locked --git https://github.com/hypotheosis/claims-mcp`.

2. **Register** it with Claude Code from your project's root:

   ```bash
   claude mcp add --scope project claims -- claims-mcp --catalog claims.json
   ```

3. **Install the skill**, then restart Claude Code and ask it to "audit this change against the
   task", or run `/claims-audit`:

   ```bash
   mkdir -p ~/.claude/skills && cp -r skills/claims-audit ~/.claude/skills/
   ```

## Documentation

- [Why claims-mcp](https://hypotheosis.github.io/claims-mcp/why/problem.html): the problems it
  solves, and where it doesn't fit
- [Concepts](https://hypotheosis.github.io/claims-mcp/concepts/claims.html): claims, evidence,
  verdicts, integrity levels, traceability, seals, anchors
- [Guides](https://hypotheosis.github.io/claims-mcp/guides/install.html): install, your first
  audit, CI, PDF reports, migrating
- [Reference](https://hypotheosis.github.io/claims-mcp/reference/tools.html): MCP tools, command
  line, catalog format
- [Development](https://hypotheosis.github.io/claims-mcp/development.html): building, CI,
  releasing

The site's sources are in [`docs/site/`](docs/site/).

## Security

Report vulnerabilities privately; see [`SECURITY.md`](SECURITY.md).

## License

Copyright 2026 Hypotheosis. Licensed under the [Apache License, Version 2.0](LICENSE); see
[`NOTICE`](NOTICE). You may use, modify, and redistribute claims-mcp, including commercially,
under the terms of that license. The license grants no rights to the Hypotheosis name or
trademarks.
