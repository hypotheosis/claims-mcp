# PDF reports

A [Typst](https://typst.app) template renders a catalog as a self-contained PDF audit report. It
runs outside the server, which never executes commands. It has two detail levels:

- **summary** (default): the result and assessment, integrity seal status, findings grouped by
  area (refuted claims in full, the rest as one-line next steps), coverage by integrity level,
  and gap counts.
- **full**: everything in the summary, plus every finding in full with its evidence, the claim
  ids in each gap, the claim hierarchy, the traceability matrix, and an evidence appendix with
  each claim's full verdict history.

Both collapse long claim id lists into ranges and roll-up evidence into child counts. Both end
with a glossary defining every term the report uses (verdicts, stances, integrity levels,
verification methods, seals), so readers need not know claims-mcp.

## Rendering a report

The template is built into the binary, so every install carries the matching version. In the
audited project's root:

```bash
claims-mcp report init            # writes report/claims-report.typ, report/text.typ,
                                  # report/audit-notes.example.toml, and report/audit-notes.toml
# uncomment and fill in report/audit-notes.toml as its comments say, then:
claims-mcp --catalog claims.json verify --json > integrity.json
claims-mcp --catalog claims.json recheck --json > recheck.json
typst compile --root . report/claims-report.typ audit.pdf \
  --input catalog=claims.json \
  --input detail=summary \
  --input commit=$(git rev-parse --short HEAD) \
  --input notes=report/audit-notes.toml \
  --input integrity=integrity.json \
  --input recheck=recheck.json
```

`report init` replaces the template files, so they always match the installed binary, but never
replaces an existing `audit-notes.toml`; it prints what it wrote and what it kept. A new
`audit-notes.toml` has every setting of the example commented out, so the example's invented
project can't end up in a report. Pass a directory (`claims-mcp report init docs/audit`) to
write elsewhere, and adjust the paths above. Use `--input detail=full` and `audit-full.pdf` for
the full record.

An agent with only the MCP connection can read the same files as resources:
`claims://report/claims-report.typ`, `claims://report/text.typ`, and
`claims://report/audit-notes.example.toml`, the commented example of the notes file.

## Inputs

Paths are relative to `--root`. All inputs are optional:

| Input | Meaning |
|---|---|
| `catalog` | Catalog to report on (default `claims.json`) |
| `detail` | `summary` (default) or `full`; any other value is an error |
| `integrity` | Output of `claims-mcp verify --json` for that catalog. Typst can't check seals itself; without this input, the report says integrity was not checked. A failed check puts a warning on the result and lists the failing records |
| `recheck` | Output of `claims-mcp recheck --json`. Flags anchors that moved, changed or were overturned, on the evidence and in an anchor recheck section; without it, the report says anchors were not rechecked |
| `commit` | Catalog commit shown on the cover and in the reproduce section |
| `notes` | TOML sidecar with the audited project's name and the auditor's prose: `artifact`, `version`, `repository`, `method`, `assessment`, and `[[points]]` (`title`, `body`) for the evaluator. Each audited project needs its own; the commented example explains every key |
| `date` | Report date (default today) |
| `artifact`, `version`, `repository`, `method`, `assessment` | Override the matching sidecar key |

Tag a claim `issue:N` to link it to issue N of `repository` in the findings.

## Writing the notes

The `assessment` should say what was audited, whether the project meets it, and which findings
decide that, citing claim ids and issues. Use `[[points]]` for caveats the reader should weigh:
whether the audit was independent of the builder, heuristic evidence, and hierarchy changes
during the audit. Don't restate counts or verdicts; the report computes them.
