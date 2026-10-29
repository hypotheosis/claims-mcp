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
