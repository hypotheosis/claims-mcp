# Development tasks. Run `just --list` for a summary.

# Format all Rust code.
fmt:
    cargo fmt

# Lint with every warning treated as an error.
clippy:
    cargo clippy --all-targets -- -D warnings

# Run all tests. The Typst template checks are skipped when `typst` is not installed,
# unless REQUIRE_TYPST=1 is set.
test:
    cargo test

# Regenerate the latest and format-version-pinned catalog schemas.
schema:
    mkdir -p docs/site/src/schema/v2
    cargo run --quiet -- schema > docs/site/src/schema/claims.schema.json
    cp docs/site/src/schema/claims.schema.json docs/site/src/schema/v2/claims.schema.json

# Render the catalog as a PDF audit report (requires the Typst CLI): `just report` for the
# summary (audit.pdf), `just report full` for every record (audit-full.pdf). The integrity
# check and the anchor recheck run first; their findings are shown in the report, so their
# exit codes are ignored (`-`).
report detail="summary" catalog="claims.json" out=(if detail == "full" { "audit-full.pdf" } else { "audit.pdf" }) notes="report/audit-notes.toml":
    mkdir -p target
    -cargo run --release --quiet -- --catalog {{catalog}} verify --json > target/integrity.json
    -cargo run --release --quiet -- --catalog {{catalog}} recheck --json > target/recheck.json
    typst compile --root . report/claims-report.typ {{out}} \
        --input catalog={{catalog}} \
        --input detail={{detail}} \
        --input commit=$(git describe --always --dirty) \
        --input notes={{notes}} \
        --input integrity=target/integrity.json \
        --input recheck=target/recheck.json

# Build the documentation site into docs/site/book (requires mdbook).
docs:
    mdbook build docs/site

# Serve the documentation site with live reload at http://localhost:3000.
docs-serve:
    mdbook serve docs/site

# Run the claims viewer's unit tests, including the check against this repository's own catalog.
viewer-test:
    mkdir -p target
    cargo run --release --quiet -- --catalog claims.json tldr --json > target/tldr.json
    CLAIMS_TLDR_JSON=target/tldr.json node --test 'docs/site/viewer-tests/*.test.mjs'
