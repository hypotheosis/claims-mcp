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

# Render the catalog as a PDF audit report (requires the Typst CLI). The integrity check
# runs first; a failed check is shown in the report, so its exit code is ignored (`-`).
report catalog="claims.json" out="audit.pdf" notes="report/audit-notes.toml":
    mkdir -p target
    -cargo run --release --quiet -- --catalog {{catalog}} verify --json > target/integrity.json
    typst compile --root . report/claims-report.typ {{out}} \
        --input catalog={{catalog}} \
        --input commit=$(git describe --always --dirty) \
        --input notes={{notes}} \
        --input integrity=target/integrity.json
