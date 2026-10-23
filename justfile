# Development tasks. Run `just --list` for a summary.

# Format all Rust code.
fmt:
    cargo fmt

# Lint with every warning treated as an error.
clippy:
    cargo clippy --all-targets -- -D warnings

# Run all tests (the Typst template checks are skipped when `typst` is not installed).
test:
    cargo test

# Render the catalog as a PDF audit report (requires the Typst CLI).
report catalog="claims.json" out="audit.pdf" notes="report/audit-notes.toml":
    typst compile --root . report/claims-report.typ {{out}} \
        --input catalog={{catalog}} \
        --input commit=$(git rev-parse --short HEAD) \
        --input notes={{notes}}
