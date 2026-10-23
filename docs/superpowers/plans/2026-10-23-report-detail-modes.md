# Report Detail Modes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Typst PDF audit report readable on large catalogs by adding a `summary`
(default) and `full` detail mode, grouping findings by area, collapsing ID lists, and fixing
cell overflow.

**Architecture:** All rendering stays in `report/claims-report.typ`. Pure text helpers move to a
new `report/text.typ`, checked by assertions in `report/text-tests.typ`. The template exposes a
second metadata label, `<report-layout>`, so Rust tests in `src/report.rs` can check grouping
and roll-up locator collapsing through `typst query`. `<audit-report>` is unchanged.

**Tech Stack:** Typst 0.14 (template), Rust (`cargo test` drives the Typst CLI), `just`, GitHub
Actions.

**Spec:** `docs/superpowers/specs/2026-10-23-report-detail-modes-design.md`

**Verified ahead of time:** the template, `text.typ`, and `text-tests.typ` below were prototyped
and compiled against `claims.json`, an empty catalog, and `bdc.json` (1,265 claims) in both
modes with no warnings. On `bdc.json`, summary renders 41 pages and full 795 (was 1,869).

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `report/text.typ` | create | Pure text helpers: ID range compression, ID-run compression in free text, `Fix:` step, retired-reason key, wrap points, backtick spans |
| `report/text-tests.typ` | create | Assertions over `text.typ`; compiling it is the test |
| `report/claims-report.typ` | rewrite | `detail` input, areas and findings grouping, `<report-layout>`, roll-up locators, per-mode sections, record blocks |
| `src/report.rs` (tests module) | modify | Harness renders both modes and queries both labels; new fixture and tests |
| `justfile` | modify | `report` takes `detail` first; `out` defaults by mode |
| `.gitignore` | modify | ignore `/audit-full.pdf` |
| `.github/workflows/ci.yml` | modify | render and upload both PDFs (the release reuses CI's artifacts) |
| `README.md` | modify | document `detail` and the two PDFs |
| `skills/claims-audit/SKILL.md` | modify | new roll-up locator format |
| `changelog/<timestamp>.md` | create | changelog entry (CLAUDE.md requires one per iteration) |

---

### Task 1: Text helpers module and its assertions

**Files:**
- Create: `report/text.typ`
- Create: `report/text-tests.typ`
- Modify: `src/report.rs` (tests module)

- [ ] **Step 1: Write the failing Rust test**

In `src/report.rs`, inside `mod tests`, change `use std::path::Path;` to:

```rust
    use std::path::{Path, PathBuf};
```

Then add, directly above `fn assert_template_matches`:

```rust
    /// Whether the Typst CLI is on `PATH`. It is an external tool, so the template checks are
    /// skipped (with a note on stderr) without it, unless `REQUIRE_TYPST=1` is set (as CI
    /// should), which turns the skip into a failure.
    fn typst_available() -> bool {
        let installed = Command::new("typst")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        if !installed {
            // Rust note: `env::var` returns `Result`; `is_ok_and` is false for unset variables.
            let required = std::env::var("REQUIRE_TYPST").is_ok_and(|value| value == "1");
            assert!(!required, "REQUIRE_TYPST=1 but `typst` is not on PATH");
            eprintln!("skipping Typst template check: `typst` is not on PATH");
        }
        installed
    }

    /// Path of a file under `report/` in this repository.
    fn report_file(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("report").join(name)
    }
```

And add this test after `typst_template_matches_every_gap_and_retired_claims`:

```rust
    /// Compiles `report/text-tests.typ`, whose assertions check the template's text helpers.
    #[test]
    fn typst_template_text_helpers() {
        if !typst_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("temp dir");
        let output = Command::new("typst")
            .arg("compile")
            .arg(report_file("text-tests.typ"))
            .arg(dir.path().join("text-tests.pdf"))
            .output()
            .expect("run typst");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "text helper assertions failed: {stderr}");
        assert!(stderr.trim().is_empty(), "typst warned: {stderr}");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test typst_template_text_helpers`
Expected: FAIL with `text helper assertions failed: error: file not found` (for
`text-tests.typ`).

- [ ] **Step 3: Create `report/text.typ`**

```typst
// Text helpers for claims-report.typ. They take and return plain values and read no catalog,
// so report/text-tests.typ can check them; `cargo test typst_template` compiles that file.

// "CLM-0042" -> 42 and back, keeping the four-digit minimum width of `ClaimId`'s Display.
#let claim-number(id) = int(id.slice(4))
#let claim-id(number) = {
  let digits = str(number)
  "CLM-" + "0" * calc.max(0, 4 - digits.len()) + digits
}

// Claim ids as sorted, comma-separated ranges: "CLM-0591–0598, CLM-0600". Empty for no ids.
#let compress-ids(ids) = {
  let ranges = ()
  for number in ids.map(claim-number).sorted().dedup() {
    if ranges.len() > 0 and ranges.last().at(1) + 1 == number {
      ranges.last().at(1) = number
    } else {
      ranges.push((number, number))
    }
  }
  ranges
    .map(((first, last)) => if first == last {
      claim-id(first)
    } else {
      claim-id(first) + "–" + claim-id(last).slice(4)
    })
    .join(", ", default: "")
}

// Runs of six or more comma-separated claim ids in free text become compressed ranges.
#let id-run = regex("CLM-\d+(?:,\s*CLM-\d+){5,}")
#let compress-runs(value) = value.replace(
  id-run,
  found => compress-ids(found.text.split(regex(",\s*"))),
)

// Spec section 5, as in src/report/next_steps.rs: the trimmed text after the last `Fix:` that
// starts the rationale or follows whitespace; none when there is no marker or no text after it.
#let fix-step(rationale) = {
  let markers = rationale.matches(regex("(?:^|\s)Fix:"))
  if markers.len() == 0 { return none }
  let step = rationale.slice(markers.last().end).trim()
  if step == "" { none } else { step }
}

// Retired reasons with their claim ids replaced, so "replaced by CLM-0661, CLM-0662" and
// "replaced by CLM-0663" count as one reason.
#let reason-key(reason) = reason.replace(
  regex("CLM-\d+(?:(?:,\s*|\s+and\s+)CLM-\d+)*"),
  "[claim ids]",
)

// Zero-width spaces after separators let long paths and locators wrap inside table cells.
#let wrappable(value) = value.replace(regex("[/_.:,=-]"), found => found.text + sym.zws)

// Catalog free text: id runs compressed, `backtick` spans as inline code, and wrap points
// throughout. Unbalanced backticks are printed as written.
#let rich(value) = {
  let parts = compress-runs(value).split("`")
  if calc.even(parts.len()) { return wrappable(parts.join("`")) }
  parts
    .enumerate()
    .map(((index, part)) => if calc.odd(index) { raw(wrappable(part)) } else { wrappable(part) })
    .join()
}
```

- [ ] **Step 4: Create `report/text-tests.typ`**

```typst
// Checks for report/text.typ. `cargo test typst_template_text_helpers` compiles this file; a
// failed assertion fails the compile and names both values.

#import "text.typ": compress-ids, compress-runs, fix-step, reason-key, rich

// compress-ids: sorted, deduplicated, consecutive runs as ranges, five-digit ids kept whole.
#assert.eq(
  compress-ids(("CLM-0003", "CLM-0001", "CLM-0002", "CLM-0002", "CLM-0005", "CLM-12345",
    "CLM-12346")),
  "CLM-0001–0003, CLM-0005, CLM-12345–12346",
)
#assert.eq(compress-ids(("CLM-0007",)), "CLM-0007")
#assert.eq(compress-ids(()), "")

// compress-runs: six or more ids collapse; shorter runs and other text are untouched.
#assert.eq(
  compress-runs(
    "Open: CLM-0001, CLM-0002,CLM-0003, CLM-0004, CLM-0005, CLM-0009. "
      + "See CLM-0010, CLM-0011.",
  ),
  "Open: CLM-0001–0005, CLM-0009. See CLM-0010, CLM-0011.",
)
#assert.eq(compress-runs("CLM-0001, CLM-0002, CLM-0003"), "CLM-0001, CLM-0002, CLM-0003")

// fix-step: the last marker at a word boundary, trimmed; none without a usable one.
#assert.eq(fix-step("HotFix: no. Fix: first. Fix: second step. "), "second step.")
#assert.eq(fix-step("Fix: at the start"), "at the start")
#assert.eq(fix-step("HotFix: not a marker"), none)
#assert.eq(fix-step("Nothing follows. Fix:  "), none)
#assert.eq(fix-step("no marker"), none)

// reason-key: id lists of any length, with commas or "and", become one placeholder.
#assert.eq(
  reason-key("replaced by CLM-0661, CLM-0662 and CLM-0663."),
  "replaced by [claim ids].",
)
#assert.eq(reason-key("replaced by CLM-0670."), reason-key("replaced by CLM-0661, CLM-0662."))

// rich: balanced backticks become raw elements; unbalanced ones stay text.
#assert(repr(rich("see `a.md:1` here")).contains("raw("))
#assert(not repr(rich("odd ` backtick")).contains("raw("))
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test typst_template_text_helpers`
Expected: PASS. Also check that a wrong expectation fails: temporarily change
`compress-ids(("CLM-0007",)), "CLM-0007"` to `"CLM-0008"`, rerun (expect FAIL with
`equality assertion failed`), and revert.

- [ ] **Step 6: Commit**

```bash
git add report/text.typ report/text-tests.typ src/report.rs
git commit -m "test: add Typst text helpers for the report, with assertions"
```

---

### Task 2: Test harness renders both modes and reads `<report-layout>`

**Files:**
- Modify: `src/report.rs` (tests module: replace `assert_template_matches`, add fixture and tests)

- [ ] **Step 1: Replace the harness**

Change the imports at the top of `mod tests` to:

```rust
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use chrono::DateTime;
    use serde_json::{json, Value};

    use super::*;
    use crate::catalog::tests::{at, evidence, new_claim, verdict};
    use crate::catalog::{NewClaim, NewEvidence, NewVerdict};
    use crate::model::{ClaimRef, EvidenceId, EvidenceKind, Stance, Verdict};
```

Then replace the whole of `fn assert_template_matches` (its doc comment through its closing brace)
with:

```rust
    /// Detail modes of `report/claims-report.typ`; every template check renders both.
    const DETAILS: [&str; 2] = ["summary", "full"];

    /// The template's two metadata labels for one detail mode, as `typst query` emits them.
    struct Rendered {
        /// `<audit-report>`, which must equal `audit_report`.
        report: Value,
        /// `<report-layout>`: the findings grouping and the collapsed roll-up locators.
        layout: Value,
    }

    /// A catalog and its integrity result written to a temp dir, ready to render.
    struct TemplateRun {
        // Rust note: dropping a `TempDir` deletes the directory, so it is kept for as long as
        // the run, even though only the paths below are read.
        dir: tempfile::TempDir,
        catalog_input: OsString,
        integrity_input: OsString,
    }

    impl TemplateRun {
        fn new(catalog: &Catalog) -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let catalog_path = dir.path().join("claims.json");
            std::fs::write(
                &catalog_path,
                serde_json::to_vec(catalog).expect("serialize catalog"),
            )
            .expect("write catalog");
            // The template cannot compute HMACs, so it takes the integrity result as an input,
            // as `just report` passes it from `claims-mcp verify --json`.
            let integrity_path = dir.path().join("integrity.json");
            std::fs::write(
                &integrity_path,
                serde_json::to_vec(&seal::verify(catalog)).expect("serialize integrity"),
            )
            .expect("write integrity");
            // Rust note: `OsString` is the platform's native string type; `Command` args take
            // it directly, so paths need no lossy conversion to `String`.
            let mut catalog_input = OsString::from("catalog=");
            catalog_input.push(&catalog_path);
            let mut integrity_input = OsString::from("integrity=");
            integrity_input.push(&integrity_path);
            Self {
                dir,
                catalog_input,
                integrity_input,
            }
        }

        fn pdf(&self) -> PathBuf {
            self.dir.path().join("audit.pdf")
        }

        /// Runs `typst <subcommand>` on the template at `detail` and returns its raw output.
        fn typst(&self, subcommand: &str, detail: &str, extra: &[&OsStr]) -> Output {
            // `--root /` lets the template read both the repo template and the temp catalog.
            Command::new("typst")
                .args([subcommand, "--root", "/"])
                .arg(report_file("claims-report.typ"))
                .args(extra)
                .arg("--input")
                .arg(&self.catalog_input)
                .arg("--input")
                .arg(&self.integrity_input)
                .arg("--input")
                .arg(format!("detail={detail}"))
                .output()
                .expect("run typst")
        }

        /// Like `typst`, but asserts success with no warnings and returns stdout.
        fn typst_ok(&self, subcommand: &str, detail: &str, extra: &[&OsStr]) -> Vec<u8> {
            let output = self.typst(subcommand, detail, extra);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "typst {subcommand} ({detail}) failed: {stderr}"
            );
            assert!(
                stderr.trim().is_empty(),
                "typst {subcommand} ({detail}) warned: {stderr}"
            );
            output.stdout
        }

        /// Compiles the PDF at `detail`, then queries both metadata labels.
        fn render(&self, detail: &str) -> Rendered {
            self.typst_ok("compile", detail, &[self.pdf().as_os_str()]);
            let query = |label: &str| -> Value {
                let stdout = self.typst_ok(
                    "query",
                    detail,
                    &[label, "--field", "value", "--one"].map(OsStr::new),
                );
                serde_json::from_slice(&stdout).expect("typst query emits JSON")
            };
            Rendered {
                report: query("<audit-report>"),
                layout: query("<report-layout>"),
            }
        }
    }

    /// Renders the template over `catalog` in every detail mode and checks that each compile is
    /// warning-free and that its `<audit-report>` metadata equals `audit_report`. Returns the
    /// summary and full renders, or `None` when `typst` is missing and the check was skipped.
    fn assert_template_matches(catalog: &Catalog) -> Option<[Rendered; 2]> {
        if !typst_available() {
            return None;
        }
        let run = TemplateRun::new(catalog);
        let expected = serde_json::to_value(audit_report(catalog, true)).expect("serialize");
        // Rust note: `array::map` builds a new fixed-size array: one render per mode.
        Some(DETAILS.map(|detail| {
            let rendered = run.render(detail);
            assert_eq!(rendered.report, expected, "detail={detail}");
            assert_eq!(rendered.layout["detail"], detail);
            rendered
        }))
    }

    /// root <- areas A and B. A <- parent <- ten leaves: the first refuted, the second not
    /// enough info, the rest supported. The parent is not enough info, citing roll-up evidence
    /// in the old `children: CLM-..., ...` locator format. B <- one not-enough-info leaf whose
    /// rationale names a `Fix:` step. Ids: root CLM-0001, areas CLM-0002 and CLM-0003, parent
    /// CLM-0004, leaves CLM-0005 to CLM-0014, B's leaf CLM-0015. Returns the catalog and the
    /// roll-up evidence id.
    fn areas_fixture() -> (Catalog, EvidenceId) {
        let mut catalog = Catalog::default();
        let root = catalog
            .add_claim(new_claim("root"), at(0))
            .expect("root")
            .id;
        let area_a = catalog
            .add_claim(child_of("area a", root), at(0))
            .expect("area a")
            .id;
        let area_b = catalog
            .add_claim(child_of("area b", root), at(0))
            .expect("area b")
            .id;
        let parent = catalog
            .add_claim(child_of("parent", area_a), at(0))
            .expect("parent")
            .id;
        let leaves: Vec<ClaimId> = (0..10)
            .map(|index| {
                catalog
                    .add_claim(child_of(&format!("leaf {index}"), parent), at(0))
                    .expect("leaf")
                    .id
            })
            .collect();
        let undecided = catalog
            .add_claim(child_of("undecided", area_b), at(0))
            .expect("undecided")
            .id;
        for (index, &leaf) in leaves.iter().enumerate() {
            let (stance, outcome) = match index {
                0 => (Stance::Refutes, Verdict::Refuted),
                1 => (Stance::Neutral, Verdict::NotEnoughInfo),
                _ => (Stance::Supports, Verdict::Supported),
            };
            let item = catalog
                .add_evidence(leaf, evidence(stance), at(1))
                .expect("leaf evidence");
            catalog
                .record_verdict(leaf, verdict(outcome, vec![item.id]), at(2))
                .expect("leaf verdict");
        }
        let listed: Vec<String> = leaves.iter().map(ToString::to_string).collect();
        let rollup = catalog
            .add_evidence(
                parent,
                NewEvidence {
                    kind: EvidenceKind::Analysis,
                    locator: format!("children: {}", listed.join(", ")),
                    summary: "Two leaves are open.".to_owned(),
                    stance: Stance::Neutral,
                },
                at(3),
            )
            .expect("roll-up evidence");
        catalog
            .record_verdict(
                parent,
                verdict(Verdict::NotEnoughInfo, vec![rollup.id]),
                at(4),
            )
            .expect("parent verdict");
        catalog
            .record_verdict(
                undecided,
                NewVerdict {
                    rationale: "No test covers it. Fix: add one.".to_owned(),
                    ..verdict(Verdict::NotEnoughInfo, Vec::new())
                },
                at(2),
            )
            .expect("undecided verdict");
        (catalog, rollup.id)
    }
```

- [ ] **Step 2: Add the new tests**

After `typst_template_text_helpers`, add:

```rust
    #[test]
    fn typst_template_groups_findings_by_area() {
        let (catalog, _) = areas_fixture();
        let Some(renders) = assert_template_matches(&catalog) else {
            return;
        };
        // The root and the areas head the groups; the roll-up parent is an ordinary finding.
        let expected = json!([
            {"area": "CLM-0002", "findings": ["CLM-0004", "CLM-0005", "CLM-0006"]},
            {"area": "CLM-0003", "findings": ["CLM-0015"]},
        ]);
        for rendered in &renders {
            assert_eq!(rendered.layout["areas"], expected);
        }
    }

    #[test]
    fn typst_template_collapses_rollup_locators() {
        let (catalog, rollup) = areas_fixture();
        let Some(renders) = assert_template_matches(&catalog) else {
            return;
        };
        // Counts come from the catalog; only the children that are not supported are named.
        let expected = "children of CLM-0004: 10 active (8 supported, 1 refuted, 1 not enough \
                        info). Not supported: CLM-0005–0006";
        for rendered in &renders {
            assert_eq!(
                rendered.layout["rollup_locators"][rollup.to_string().as_str()],
                expected
            );
        }
    }

    #[test]
    fn typst_template_rejects_unknown_detail() {
        if !typst_available() {
            return;
        }
        let run = TemplateRun::new(&Catalog::default());
        let output = run.typst("compile", "bogus", &[run.pdf().as_os_str()]);
        assert!(!output.status.success(), "detail=bogus must not compile");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("detail must be"), "unexpected error: {stderr}");
    }
```

- [ ] **Step 3: Run the template tests to verify they fail**

Run: `cargo test typst_template`
Expected: `typst_template_text_helpers` PASSES. Every other `typst_template_*` test FAILS. The
old template has no `<report-layout>` label, so the query fails (`typst query (summary)
failed`). `typst_template_rejects_unknown_detail` fails with `detail=bogus must not compile`.

- [ ] **Step 4: Commit**

```bash
git add src/report.rs
git commit -m "test: render the report template in both detail modes and check its layout"
```

---

### Task 3: Rewrite the template

**Files:**
- Modify (full replacement): `report/claims-report.typ`

The rewrite does the following:
- Adds the `detail` input and its validation.
- Imports `text.typ`.
- Adds the area, findings, and roll-up analysis and `<report-layout>`.
- Renders all free text through `rich`.
- Replaces evidence and verdict tables with full-width record blocks.
- Makes finding cards breakable.
- Adds summary-mode retired, integrity, findings, and gap rendering.
- Gates the hierarchy, matrix, and appendix on `full`.
- Adds `detail` to the cover and the reproduce command.

The `Catalog analysis` section and `<audit-report>` metadata are unchanged.

- [ ] **Step 1: Replace `report/claims-report.typ` with**

```typst
// PDF audit report for a claims-mcp catalog.
//
//   typst compile --root . report/claims-report.typ audit.pdf \
//     --input catalog=claims.json --input commit=$(git rev-parse --short HEAD) \
//     --input notes=report/audit-notes.toml --input integrity=target/integrity.json
//
// Inputs (all optional):
//   catalog     catalog path, relative to --root (default claims.json)
//   detail      summary (default) or full. summary groups findings by area, shows refuted
//               claims in full and the rest as one-line next steps, and omits the hierarchy,
//               matrix, and evidence appendix; full includes everything.
//   integrity   output of `claims-mcp verify --json` for that catalog, relative to --root.
//               Typst cannot check HMAC seals itself; without this input the report says
//               integrity was not checked.
//   commit      catalog commit shown on the cover and in the reproduce section; a "-dirty"
//               suffix (git describe --dirty) marks uncommitted changes
//   notes       TOML sidecar: artifact, version, repository, method, assessment, [[points]]
//   date        report date (default: today)
//   artifact, version, repository, method, assessment
//               override the matching sidecar keys
//
// Gap rules follow spec section 4.1 exactly, as in src/report.rs. The computed totals, gaps,
// and matrix, plus the integrity input, are exposed as <audit-report> metadata so
// `typst query` can be compared with `audit_report`; see the `typst_template` tests in
// src/report.rs. <report-layout> exposes the findings grouping and collapsed roll-up
// locators for the same tests.

// ---------------------------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------------------------

#import "text.typ": compress-ids, compress-runs, fix-step, reason-key, rich, wrappable

// Typst resolves "/x" against --root and "x" against this file's directory. Catalog and notes
// paths are given relative to the root, like the command-line paths around them.
#let rooted(path) = if path.starts-with("/") { path } else { "/" + path }

#let inputs = sys.inputs
#let detail = inputs.at("detail", default: "summary")
#assert(
  detail in ("summary", "full"),
  message: "detail must be \"summary\" or \"full\", not \"" + detail + "\"",
)
#let full = detail == "full"
#let catalog-path = inputs.at("catalog", default: "claims.json")
#let catalog = json(rooted(catalog-path))
#let notes = if "notes" in inputs { toml(rooted(inputs.notes)) } else { (:) }
// `verified`, `records`, `problems` (record, problem), `adopted`; none when not checked.
#let seal-check = if "integrity" in inputs { json(rooted(inputs.integrity)) } else { none }
#let tampered = seal-check != none and not seal-check.verified
#let setting(key, default) = inputs.at(key, default: notes.at(key, default: default))

#let artifact = setting("artifact", "unnamed artifact")
#let version = setting("version", none)
// A trailing slash would turn issue links into `.../repo//issues/N`.
#let repository = {
  let url = setting("repository", none)
  if url == none { none } else { url.trim("/", at: end) }
}
#let method = setting("method", "skills/claims-audit")
#let assessment = setting("assessment", none)
#let commit = inputs.at("commit", default: none)
#let report-date = inputs.at("date", default: datetime.today().display())
#let points = notes.at("points", default: ())

// ---------------------------------------------------------------------------------------------
// Catalog analysis (mirrors src/report.rs)
// ---------------------------------------------------------------------------------------------

// List fields have serde defaults, so a hand-edited catalog may omit them.
#let list-defaults = (derived_from: (), artifacts: (), tags: (), evidence: (), verdicts: ())
#let claims = catalog.at("claims", default: ()).map(claim => list-defaults + claim)
#let active = claims.filter(claim => claim.status.state == "active")
#let retired = claims.filter(claim => claim.status.state != "active")
#let retired-set = retired.map(claim => (claim.id, true)).to-dict()
#let by-id = claims.map(claim => (claim.id, claim)).to-dict()

#let current-verdict(claim) = if claim.verdicts.len() == 0 {
  "unverified"
} else {
  claim.verdicts.last().verdict
}

#let integrity(claim) = claim.at("integrity_level", default: none)
#let integrity-key(claim) = {
  let level = integrity(claim)
  if level == none { "unassigned" } else { str(level) }
}

// Parent id -> ids of its active children, in catalog order.
#let children = {
  let index = (:)
  for claim in active {
    for parent in claim.derived_from {
      index.insert(parent, index.at(parent, default: ()) + (claim.id,))
    }
  }
  index
}
#let children-of(id) = children.at(id, default: ())

// RFC 3339 timestamp -> fixed-width UTC key that compares correctly as a string, including
// nanoseconds and non-UTC offsets.
#let instant(timestamp) = {
  let parts = timestamp.match(regex(
    "^(\d{4})-(\d{2})-(\d{2})[Tt ](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?([Zz]|[+-]\d{2}:\d{2})$",
  ))
  assert(parts != none, message: "unrecognized timestamp: " + timestamp)
  let (year, month, day, hour, minute, second, fraction, offset) = parts.captures
  let local = datetime(
    year: int(year), month: int(month), day: int(day),
    hour: int(hour), minute: int(minute), second: int(second),
  )
  let utc = if offset in ("Z", "z") {
    local
  } else {
    let sign = if offset.starts-with("-") { -1 } else { 1 }
    let minutes = int(offset.slice(1, 3)) * 60 + int(offset.slice(4, 6))
    local - duration(minutes: sign * minutes)
  }
  let nanos = if fraction == none { "" } else { fraction.slice(0, calc.min(9, fraction.len())) }
  utc.display("[year][month][day][hour][minute][second]") + nanos + "0" * (9 - nanos.len())
}

#let is-stale(claim) = if claim.verdicts.len() == 0 {
  false
} else {
  let latest = instant(claim.verdicts.last().recorded_at)
  claim.evidence.any(item => instant(item.recorded_at) > latest)
}

// Spec section 4.1, in the order `audit_report` serializes them.
#let gap-rules = (
  no_evidence: claim => claim.evidence.len() == 0,
  unverified: claim => current-verdict(claim) == "unverified",
  refuted: claim => current-verdict(claim) == "refuted",
  not_enough_info: claim => current-verdict(claim) == "not_enough_info",
  stale: is-stale,
  rigor: claim => integrity(claim) in (3, 4) and claim.verification_method == "inspection",
  decomposition: claim => children-of(claim.id).any(
    child => current-verdict(by-id.at(child)) != "supported",
  ),
  dangling_links: claim => claim.derived_from.any(parent => parent in retired-set),
)
#let gaps = gap-rules.pairs().map(((name, rule)) => (
  name,
  active.filter(rule).map(claim => claim.id),
)).to-dict()

#let count-by(items, key) = {
  let counts = (:)
  for item in items {
    let name = key(item)
    counts.insert(name, counts.at(name, default: 0) + 1)
  }
  counts
}

#let totals = (
  active_claims: active.len(),
  retired_claims: retired.len(),
  by_verdict: count-by(active, current-verdict),
  by_kind: count-by(active, claim => claim.kind),
  by_level: count-by(active, claim => claim.level),
  by_integrity_level: count-by(active, integrity-key),
)

#let matrix = active.map(claim => (
  id: claim.id,
  statement: claim.statement,
  derived_from: claim.derived_from,
  children: children-of(claim.id),
  verification_method: claim.verification_method,
  evidence_locators: claim.evidence.map(item => item.locator),
  current_verdict: current-verdict(claim),
))

#metadata((totals: totals, gaps: gaps, integrity: seal-check, matrix: matrix)) <audit-report>

// ---------------------------------------------------------------------------------------------
// Layout analysis: areas, findings, and roll-up locators
// ---------------------------------------------------------------------------------------------

#let all-verdicts = ("supported", "refuted", "not_enough_info", "unverified")
#let humanize(name) = name.replace("_", " ")

// The first parent that is an active claim, or none for a root.
#let first-active-parent(claim) = claim.derived_from.find(
  parent => parent in by-id and parent not in retired-set,
)

// A claim's area: its ancestor one level below a root, following first active parents. An
// area claim is its own area; a root has none. Revisiting a claim stops the walk, so a
// hand-edited cycle cannot loop forever.
#let area-of(claim) = {
  let path = (claim.id,)
  let parent = first-active-parent(claim)
  while parent != none and parent not in path {
    path.push(parent)
    parent = first-active-parent(by-id.at(parent))
  }
  if path.len() >= 2 { path.at(-2) } else { none }
}
#let area = active.map(claim => (claim.id, area-of(claim))).to-dict()

// Roots and areas that have active children head the findings and the result instead of being
// listed as findings themselves.
#let is-heading-rollup(claim) = {
  children-of(claim.id).len() > 0 and area.at(claim.id) in (none, claim.id)
}
#let findings = active.filter(claim => {
  current-verdict(claim) in ("refuted", "not_enough_info") and not is-heading-rollup(claim)
})

// Findings grouped by area: claims under a root but in no area first, then each area in catalog
// order. Groups without findings are left out.
#let finding-groups = {
  let grouped = (:)
  for claim in findings {
    let key = area.at(claim.id)
    let key = if key == none { "" } else { key }
    grouped.insert(key, grouped.at(key, default: ()) + (claim,))
  }
  let area-ids = active.filter(claim => area.at(claim.id) == claim.id).map(claim => claim.id)
  (("",) + area-ids).filter(key => key in grouped).map(key => (
    area: if key == "" { none } else { key },
    findings: grouped.at(key),
  ))
}

// Roll-up evidence names its claim's children: `children: CLM-0003, CLM-0004` (old skill
// format) or `children of CLM-0002` (current). Either renders from the catalog's current
// children, never from the locator text.
#let rollup-pattern = regex("^\s*children(?::|\s+of\s+CLM-\d+)")
#let is-rollup(item) = item.locator.match(rollup-pattern) != none
#let rollup-locator(claim) = {
  let kids = children-of(claim.id)
  let prefix = "children of " + claim.id + ": "
  if kids.len() == 0 { return prefix + "no active children" }
  let counts = count-by(kids, id => current-verdict(by-id.at(id)))
  let breakdown = all-verdicts
    .filter(verdict => verdict in counts)
    .map(verdict => str(counts.at(verdict)) + " " + humanize(verdict))
    .join(", ")
  let open = kids.filter(id => current-verdict(by-id.at(id)) != "supported")
  let named = if open.len() == 0 { "" } else { ". Not supported: " + compress-ids(open) }
  prefix + str(kids.len()) + " active (" + breakdown + ")" + named
}
#let locator-text(claim, item) = if is-rollup(item) { rollup-locator(claim) } else { item.locator }

#let rollup-locators = {
  let rendered = (:)
  for claim in claims {
    for item in claim.evidence.filter(is-rollup) {
      rendered.insert(item.id, rollup-locator(claim))
    }
  }
  rendered
}

#metadata((
  detail: detail,
  areas: finding-groups.map(group => (
    area: group.area,
    findings: group.findings.map(claim => claim.id),
  )),
  rollup_locators: rollup-locators,
)) <report-layout>

// ---------------------------------------------------------------------------------------------
// Presentation helpers
// ---------------------------------------------------------------------------------------------

// Every marker carries a text label, so verdicts survive grayscale printing.
#let verdict-styles = (
  supported: (label: "SUPPORTED", fill: rgb("#d5ecd9")),
  refuted: (label: "REFUTED", fill: rgb("#f3d0d0")),
  not_enough_info: (label: "NOT ENOUGH INFO", fill: rgb("#f5e6bf")),
  unverified: (label: "UNVERIFIED", fill: luma(225)),
)
#let marker(verdict) = {
  let style = verdict-styles.at(verdict)
  box(
    fill: style.fill,
    stroke: 0.5pt + luma(80),
    inset: (x: 3pt, y: 1.5pt),
    outset: (y: 1pt),
    radius: 2pt,
    text(size: 0.75em, weight: "bold", style.label),
  )
}

#let id-text(id) = raw(id)
#let ids(list) = if list.len() == 0 { [none] } else { list.map(id-text).join(", ") }
#let or-dash(value) = if value == none { [-] } else { [#value] }
#let level-text(claim) = or-dash(integrity(claim))

// Tags of the form `issue:N` link a claim to an issue in `repository`.
#let issue-links(claim) = claim.tags.filter(tag => tag.starts-with("issue:")).map(tag => {
    let number = tag.slice(6)
    let label = "issue #" + number
    if repository == none { label } else { link(repository + "/issues/" + number, label) }
  })

#let field-table(..rows) = table(
  columns: (auto, 1fr),
  stroke: none,
  inset: (x: 0pt, y: 3pt),
  column-gutter: 1.2em,
  ..rows.pos().map(((name, value)) => (strong(name), value)).flatten(),
)

#let data-table(columns: auto, header: (), ..rows) = table(
  columns: columns,
  stroke: 0.4pt + luma(150),
  inset: 4pt,
  fill: (_, row) => if row == 0 { luma(235) },
  table.header(..header.map(strong)),
  ..rows.pos().flatten(),
)

// A full-width record: a header line of short fields, then the long text on its own lines, so
// no column squeezes it.
#let record-block(header, ..lines) = block(
  width: 100%,
  breakable: true,
  above: 0.6em,
  below: 0.6em,
  inset: (left: 6pt, y: 2pt),
  stroke: (left: 1.5pt + luma(170)),
  {
    text(size: 0.9em, header.join[ #sym.dot.c ])
    for line in lines.pos() { linebreak() + line }
  },
)

#let evidence-block(claim, item) = record-block(
  (strong(id-text(item.id)), item.kind, item.stance, wrappable(item.recorded_at)),
  text(fill: luma(70), rich(locator-text(claim, item))),
  rich(item.summary),
)

#let verdict-block(record) = record-block(
  (
    marker(record.verdict),
    record.auditor,
    wrappable(record.recorded_at),
    [cites #ids(record.evidence_ids)],
  ),
  rich(record.rationale),
)

// ---------------------------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------------------------

#let auditors = claims.map(claim => claim.verdicts.map(record => record.auditor)).flatten().dedup()

#let integrity-status() = if seal-check == none {
  [not checked]
} else if seal-check.verified {
  [verified (#seal-check.records records)]
} else {
  let count = seal-check.problems.len()
  strong[FAILED (#count #if count == 1 [problem] else [problems])]
}

#let cover() = {
  set align(center)
  v(4cm)
  text(size: 24pt, weight: "bold")[Claims Audit Report]
  v(0.6em)
  text(size: 15pt, artifact + if version != none { " " + version })
  v(2.5cm)
  set align(left)
  block(width: 100%, inset: (x: 2cm), field-table(
    ([Date], report-date),
    ([Catalog], raw(catalog-path)),
    ([Catalog commit], or-dash(if commit != none { raw(commit) })),
    ([Auditors], if auditors.len() == 0 { [none recorded] } else { auditors.join(", ") }),
    ([Method], raw(method)),
    ([Claims], [#active.len() active, #retired.len() retired]),
    ([Integrity], integrity-status()),
    ([Detail], if full [full] else [summary (render with `detail=full` for every record)]),
  ))
  pagebreak()
}

#let retired-claims() = {
  heading(level: 2)[Retired claims]
  [Retired claims are excluded from every total, gap, and table except this list and the
    evidence appendix.]
  if retired.len() == 0 {
    par(emph[No claims are retired.])
  } else if full {
    data-table(
      columns: (auto, 2fr, 1.4fr, auto),
      header: ([Claim], [Statement], [Reason], [Retired at]),
      ..retired.map(claim => (
        id-text(claim.id),
        rich(claim.statement),
        rich(claim.status.reason),
        wrappable(claim.status.retired_at),
      )),
    )
  } else {
    let reasons = count-by(retired, claim => reason-key(claim.status.reason))
    data-table(
      columns: (auto, 1fr),
      header: ([Claims], [Reason]),
      ..reasons
        .pairs()
        .sorted(key: ((_, count)) => -count)
        .map(((reason, count)) => (str(count), rich(reason))),
      ([*#retired.len()*], [*Total retired*]),
    )
  }
}

#let section-result() = {
  heading[Result]
  if tampered {
    block(width: 100%, fill: rgb("#f3d0d0"), stroke: 0.8pt + luma(60), inset: 8pt, radius: 3pt)[
      *INTEGRITY CHECK FAILED.* The catalog was changed outside claims-mcp (see the Integrity
      section). Its verdicts and evidence cannot be trusted, so this report is not a clean
      result.
    ]
  } else if seal-check == none {
    par(emph[Integrity seals were not checked. Render with `just report` to include them.])
  }
  let evidence = active.map(claim => claim.evidence).flatten()
  let stances = count-by(evidence, item => item.stance)
  grid(
    columns: (1fr, 1fr),
    column-gutter: 1.5em,
    data-table(
      columns: (1fr, auto),
      header: ([Active claims by verdict], [Count]),
      ..all-verdicts.map(verdict => (
        marker(verdict),
        str(totals.by_verdict.at(verdict, default: 0)),
      )),
      ([*Total active*], [*#active.len()*]),
    ),
    data-table(
      columns: (1fr, auto),
      header: ([Evidence by stance], [Count]),
      ..("supports", "refutes", "neutral").map(stance => (
        stance,
        str(stances.at(stance, default: 0)),
      )),
      ([*Total evidence*], [*#evidence.len()*]),
    ),
  )
  heading(level: 2)[Assessment]
  if assessment == none { emph[No assessment provided.] } else { assessment }
  retired-claims()
}

#let finding(claim) = {
  let latest = claim.verdicts.last()
  let cited = claim.evidence.filter(item => item.id in latest.evidence_ids)
  let issues = issue-links(claim)
  block(breakable: true, width: 100%, stroke: 0.5pt + luma(150), inset: 8pt, radius: 3pt, {
    [#strong(id-text(claim.id)) #h(0.4em) #marker(current-verdict(claim))]
    par(rich(claim.statement))
    field-table(
      ([Integrity / method], [IL #level-text(claim), #humanize(claim.verification_method)]),
      ([Rationale], rich(latest.rationale)),
      ([Auditor], [#latest.auditor, #wrappable(latest.recorded_at)]),
      ([Linked issue], if issues.len() == 0 { [none] } else { issues.join(", ") }),
    )
    if cited.len() == 0 {
      emph[The verdict cites no evidence.]
    } else {
      cited.map(item => evidence-block(claim, item)).join()
    }
  })
}

#let section-integrity() = {
  heading[Integrity]
  [claims-mcp seals every claim, evidence item, and verdict when a tool records it. A record
    whose seal fails was changed outside the server.]
  if seal-check == none {
    par(emph[Not checked: no `integrity` input was given.])
    return
  }
  if seal-check.verified {
    par[All #seal-check.records sealed records verified.]
  } else {
    data-table(
      columns: (auto, 1fr),
      header: ([Record], [Problem]),
      ..seal-check.problems.map(item => (raw(item.record), item.problem)),
    )
  }
  if seal-check.adopted.len() > 0 {
    par[#seal-check.adopted.len() records were sealed by `claims-mcp seal --adopt` (migrated)
      rather than recorded through the server#if full [:] else [.]]
    if full { text(size: 7.5pt, seal-check.adopted.map(raw).join(", ")) }
  }
}

#let finding-counts(claims) = {
  let counts = count-by(claims, current-verdict)
  ("refuted", "not_enough_info")
    .filter(verdict => verdict in counts)
    .map(verdict => str(counts.at(verdict)) + " " + humanize(verdict))
    .join(", ")
}

#let next-step-row(claim) = {
  let step = fix-step(claim.verdicts.last().rationale)
  (id-text(claim.id), rich(claim.statement), level-text(claim), or-dash(if step != none {
    rich(step)
  }))
}

#let finding-group(group) = {
  if group.area == none {
    heading(level: 2)[Top level]
  } else {
    let head = by-id.at(group.area)
    heading(level: 2)[#id-text(head.id) #h(0.3em) #marker(current-verdict(head))]
    par(rich(head.statement))
  }
  par(emph[#group.findings.len() findings: #finding-counts(group.findings).])
  let refuted = group.findings.filter(claim => current-verdict(claim) == "refuted")
  let cards = if full { group.findings } else { refuted }
  cards.map(finding).join(v(0.6em))
  if not full and refuted.len() < group.findings.len() {
    v(0.6em)
    data-table(
      columns: (auto, 1fr, auto, 1fr),
      header: ([Claim], [Statement], [IL], [Next step]),
      ..group
        .findings
        .filter(claim => current-verdict(claim) != "refuted")
        .map(next-step-row),
    )
  }
}

#let section-findings() = {
  heading[Findings]
  if findings.len() == 0 {
    emph[No active claim is refuted or undecided.]
    return
  }
  [Active claims that are refuted or not enough info, grouped by area: the claim one level
    below the root. Roots and areas head the groups instead of being listed.]
  if not full {
    [ Refuted claims are shown in full; the rest show the next step from their latest
      rationale.]
  }
  finding-groups.map(finding-group).join()
}

#let section-points() = {
  heading[Points for the evaluator]
  if points.len() == 0 {
    emph[No evaluator notes provided.]
  } else {
    for point in points {
      terms.item(point.title, point.body)
    }
  }
}

#let section-coverage() = {
  heading[Coverage by integrity level]
  let rows = ("4", "3", "2", "1", "unassigned").map(key => {
    let group = active.filter(claim => integrity-key(claim) == key)
    let verdicts = count-by(group, current-verdict)
    let methods = count-by(group, claim => claim.verification_method)
    (
      if key == "unassigned" { [unassigned] } else { [IL #key] },
      str(group.len()),
      str(verdicts.at("supported", default: 0)),
      str(verdicts.at("refuted", default: 0)),
      str(group.len() - verdicts.at("supported", default: 0) - verdicts.at("refuted", default: 0)),
      if methods.len() == 0 {
        [-]
      } else {
        methods.pairs().map(((name, count)) => humanize(name) + " " + str(count)).join(", ")
      },
    )
  })
  data-table(
    columns: (auto, auto, auto, auto, auto, 1fr),
    header: ([Level], [Claims], [Supported], [Refuted], [Other], [Verification methods]),
    ..rows,
  )
  [_Other_ counts claims that are unverified or not enough info.]
  heading(level: 2)[Rigor check]
  if gaps.rigor.len() == 0 {
    [No integrity level 3-4 claim is verified only by inspection.]
  } else {
    [*RIGOR GAP:* #gaps.rigor.len() integrity level 3-4 claims are verified only by
      inspection: #compress-ids(gaps.rigor).]
  }
}

#let gap-meanings = (
  no_evidence: [Claim has no evidence at all.],
  unverified: [Claim has no verdict.],
  refuted: [Current verdict is refuted.],
  not_enough_info: [Current verdict is not enough info.],
  stale: [Evidence was recorded after the latest verdict.],
  rigor: [Integrity level 3-4 claim verified only by inspection.],
  decomposition: [At least one active child is not supported.],
  dangling_links: [Claim derives from a retired claim.],
)

#let section-gaps() = {
  heading[Gap summary]
  [All eight gap categories of spec section 4.1 were checked, over active claims only.]
  data-table(
    columns: (auto, 1fr, auto),
    header: ([Gap], [Meaning], [Count]),
    ..gaps.pairs().map(((name, members)) => (
      raw(name),
      gap-meanings.at(name),
      str(members.len()),
    )),
  )
  let listed = gaps.pairs().filter(((_, members)) => members.len() > 0)
  if full and listed.len() > 0 {
    heading(level: 2)[Claims per gap]
    for (name, members) in listed {
      terms.item(raw(name), compress-ids(members))
    }
  }
}

#let tree-node(id, depth) = {
  let claim = by-id.at(id)
  block(above: 0.5em, below: 0.5em, breakable: false, inset: (left: depth * 1.4em), {
    [#strong(id-text(id)) #marker(current-verdict(claim)) #h(0.3em)]
    text(size: 0.85em)[#claim.level, IL #level-text(claim)]
    linebreak()
    text(size: 0.85em, rich(claim.statement))
  })
  for child in children-of(id) {
    tree-node(child, depth + 1)
  }
}

#let section-hierarchy() = {
  heading[Claim hierarchy]
  [Active claims, indented under their parents by `derived_from`. A claim with several parents
    appears under each. Roots are claims with no active parent.]
  let roots = active.filter(claim => claim.derived_from.all(parent => parent in retired-set))
  for root in roots {
    tree-node(root.id, 0)
  }
}

#let section-matrix() = page(flipped: true, {
  heading[Traceability matrix]
  set text(size: 7.5pt)
  // Rows are never split across pages; the header repeats on every page.
  set table.cell(breakable: false)
  data-table(
    columns: (auto, 2.6fr, 1.3fr, auto, auto, auto, auto, 2.2fr, auto),
    header: (
      [ID], [Statement], [Source], [Parents], [Level], [IL], [Method], [Evidence locators],
      [Verdict],
    ),
    ..active.map(claim => (
      id-text(claim.id),
      rich(claim.statement),
      {
        let source = claim.at("source", default: none)
        if source == none { [-] } else { rich(source) }
      },
      ids(claim.derived_from),
      claim.level,
      level-text(claim),
      humanize(claim.verification_method),
      if claim.evidence.len() == 0 {
        [-]
      } else {
        claim.evidence.map(item => rich(locator-text(claim, item))).join(linebreak())
      },
      marker(current-verdict(claim)),
    )),
  )
})

#let evidence-entry(claim) = {
  heading(level: 2, {
    id-text(claim.id)
    if claim.status.state != "active" { [ (retired)] }
  })
  par(rich(claim.statement))
  if claim.evidence.len() == 0 {
    par(emph[No evidence.])
  } else {
    claim.evidence.map(item => evidence-block(claim, item)).join()
  }
  if claim.verdicts.len() == 0 {
    par(emph[No verdicts.])
  } else {
    claim.verdicts.map(verdict-block).join()
  }
}

#let section-evidence() = {
  heading[Evidence appendix]
  [Every evidence item, grouped by claim, followed by the claim's full verdict history in the
    order recorded (the history is append-only, so the last entry is the current verdict).]
  set text(size: 8.5pt)
  claims.map(evidence-entry).join()
}

#let section-reproduce() = {
  heading[Reproduce]
  // `git describe --dirty` appends "-dirty" when the catalog had uncommitted changes; that
  // state cannot be checked out, so say so instead of printing an invalid ref.
  let checkout = if commit == none {
    ""
  } else if commit.ends-with("-dirty") {
    let base = commit.trim("-dirty", at: end)
    "# rendered from uncommitted changes on top of this commit\ngit checkout " + base + "\n"
  } else {
    "git checkout " + commit + "\n"
  }
  let notes-flag = if "notes" in inputs { " \\\n  --input notes=" + inputs.notes } else { "" }
  let integrity-flag = if "integrity" in inputs {
    " \\\n  --input integrity=" + inputs.integrity
  } else { "" }
  let commit-flag = if commit == none { "" } else { " \\\n  --input commit=" + commit }
  [Regenerate this PDF:]
  raw(
    block: true,
    lang: "sh",
    checkout
      + (if "integrity" in inputs {
        "claims-mcp --catalog " + catalog-path + " verify --json > " + inputs.integrity + "\n"
      } else { "" })
      + "typst compile --root . report/claims-report.typ audit.pdf \\\n  --input catalog="
      + catalog-path
      + " \\\n  --input detail="
      + detail
      + commit-flag
      + notes-flag
      + integrity-flag,
  )
  if not full {
    [This is the summary. Pass `--input detail=full` (or `just report detail=full`) for every
      finding in full, every claim per gap, the claim hierarchy, the traceability matrix, and the
      evidence appendix with each claim's verdict history.]
  }
  [Re-run the checks. `cargo test` runs the server's tests; `cargo test typst_template` also
    confirms that this template's totals, gaps, and matrix match `audit_report` (it needs the
    Typst CLI on `PATH`). Each evidence locator names the test, command, or file to re-inspect.]
  raw(block: true, lang: "sh", "cargo test\ncargo test typst_template")
}

// ---------------------------------------------------------------------------------------------
// Document
// ---------------------------------------------------------------------------------------------

#set document(title: "Claims Audit Report: " + artifact)
#set text(size: 10pt)
#set par(justify: false)
#set heading(numbering: "1.")
#set page(
  paper: "a4",
  margin: 2cm,
  numbering: "1 / 1",
  header: context if here().page() > 1 {
    set text(size: 8pt, fill: luma(90))
    [Claims audit: #artifact #h(1fr) #if commit != none { raw(commit) }]
  },
)
#show heading.where(level: 1): set block(above: 1.6em, below: 0.9em)

#cover()

#if claims.len() == 0 {
  heading[Result]
  [This catalog contains no claims. There is nothing to report.]
} else {
  section-result()
  section-integrity()
  section-findings()
  section-points()
  section-coverage()
  section-gaps()
  if full {
    section-hierarchy()
    section-matrix()
    section-evidence()
  }
  section-reproduce()
}
```

- [ ] **Step 2: Run the template tests to verify they pass**

Run: `cargo test typst_template`
Expected: all 7 `typst_template_*` tests PASS.

- [ ] **Step 3: Run the full suite**

Run: `cargo test`
Expected: all tests PASS.

- [ ] **Step 4: Commit**

```bash
git add report/claims-report.typ
git commit -m "feat: summary and full detail modes for the PDF report

Summary (default) groups findings by area, shows refuted claims in full and
the rest as next steps, and omits the hierarchy, matrix, and appendix. Both
modes collapse roll-up locators and long ID runs, render backticks as code,
wrap every cell, and show evidence and verdicts as full-width blocks."
```

---

### Task 4: `just report`, CI, and `.gitignore`

**Files:**
- Modify: `justfile` (the `report` recipe)
- Modify: `.gitignore`
- Modify: `.github/workflows/ci.yml` (the `report` job)

- [ ] **Step 1: Update the `report` recipe**

Replace the `report` recipe and its comment in `justfile` with:

```just
# Render the catalog as a PDF audit report (requires the Typst CLI): `just report` for the
# summary (audit.pdf), `just report full` for every record (audit-full.pdf). The integrity
# check runs first; a failed check is shown in the report, so its exit code is ignored (`-`).
report detail="summary" catalog="claims.json" out=(if detail == "full" { "audit-full.pdf" } else { "audit.pdf" }) notes="report/audit-notes.toml":
    mkdir -p target
    -cargo run --release --quiet -- --catalog {{catalog}} verify --json > target/integrity.json
    typst compile --root . report/claims-report.typ {{out}} \
        --input catalog={{catalog}} \
        --input detail={{detail}} \
        --input commit=$(git describe --always --dirty) \
        --input notes={{notes}} \
        --input integrity=target/integrity.json
```

- [ ] **Step 2: Ignore the full PDF**

In `.gitignore`, add on the line after `/audit.pdf`:

```
/audit-full.pdf
```

- [ ] **Step 3: Verify both recipes**

Run: `just report && just report full && ls -l audit.pdf audit-full.pdf && git status --short`
Expected: both files exist; `git status` shows neither.

- [ ] **Step 4: Render and upload both PDFs in CI**

In `.github/workflows/ci.yml`, in the `report` job, replace `- run: just report` with:

```yaml
      - run: just report
      - run: just report full
```

and replace the upload step's `path: audit.pdf` with:

```yaml
          path: |
            audit.pdf
            audit-full.pdf
```

The release job downloads every CI artifact (`merge-multiple: true`) and attaches `dist/*`,
so no change to `release.yml` is needed. In the header comment of `ci.yml` (lines 3-4), change
"and the rendered audit report as a downloadable artifact" to "and the rendered audit reports
(summary and full) as a downloadable artifact", rewrapping at 100 columns.

- [ ] **Step 5: Commit**

```bash
git add justfile .gitignore .github/workflows/ci.yml
git commit -m "build: render summary and full audit reports locally and in CI"
```

---

### Task 5: README and the claims-audit skill

**Files:**
- Modify: `README.md` (`## Generate a PDF report`, and the release bullet near line 26)
- Modify: `skills/claims-audit/SKILL.md` (`#### Parent claims`)

- [ ] **Step 1: README report section**

Replace the first paragraph and the code block of `## Generate a PDF report` with:

````markdown
[`report/claims-report.typ`](report/claims-report.typ) renders a catalog as a self-contained
PDF audit report with [Typst](https://typst.app). It runs outside the server, which never
executes commands. It has two detail levels:

- **summary** (default): result and assessment, integrity seal status, findings grouped by
  area (refuted claims in full, the rest as one-line next steps), coverage by integrity level,
  and gap counts.
- **full**: everything in the summary, plus every finding in full with its evidence, the claim
  IDs in each gap, the claim hierarchy, the traceability matrix, and an evidence appendix with
  each claim's full verdict history.

Both collapse long claim ID lists into ranges and roll-up evidence into child counts.

```bash
just report                       # summary: claims.json -> audit.pdf
just report full                  # full: claims.json -> audit-full.pdf
# or, without just:
claims-mcp --catalog claims.json verify --json > target/integrity.json
typst compile --root . report/claims-report.typ audit.pdf \
  --input catalog=claims.json \
  --input detail=summary \
  --input commit=$(git rev-parse --short HEAD) \
  --input notes=report/audit-notes.toml \
  --input integrity=target/integrity.json
```
````

Add this row to the inputs table directly under the `catalog` row:

```markdown
| `detail` | `summary` (default) or `full`; any other value is an error |
```

In the paragraph after the table, change "checks that its totals, gaps, and matrix match
`audit_report`" to "checks that its totals, gaps, and matrix match `audit_report` in both detail
levels, and that its text helpers pass `report/text-tests.typ`".

In the release bullet near line 26, change "and the audit report for that version" to "and the
audit report for that version (`audit.pdf`, the summary, and `audit-full.pdf`, every record)".

- [ ] **Step 2: Skill roll-up format**

In `skills/claims-audit/SKILL.md`, replace the paragraph under `#### Parent claims` with:

```markdown
Judge leaf claims first. Once every active child of a parent is `supported`, record one
`analysis` evidence item on the parent with locator `children of CLM-0002` (the parent's own
id; do not list the children, since the report counts them from the catalog) and a summary
explaining how the children together cover the parent, then record the parent's verdict citing
it. If any child is `refuted`, record `refutes` evidence on the parent with the same locator,
naming the refuted children in the summary, and mark the parent `refuted`. Until then the
report lists the parent under `decomposition`.
```

- [ ] **Step 3: Check for other mentions of the old format**

Run: `grep -rn "children: CLM" README.md skills docs/reference src/server.rs`
Expected: no matches. If any remain, change them to the `children of CLM-NNNN` form.

- [ ] **Step 4: Commit**

```bash
git add README.md skills/claims-audit/SKILL.md
git commit -m "docs: document report detail levels and the new roll-up locator"
```

---

### Task 6: Verify on bdc.json, changelog, and final checks

**Files:**
- Create: `changelog/<UTC timestamp>.md`

- [ ] **Step 1: Render `bdc.json` in both modes**

`bdc.json` is untracked, so run it outside git:

```bash
cargo run --release --quiet -- --catalog bdc.json verify --json > target/bdc-integrity.json
for detail in summary full; do
  typst compile --root . report/claims-report.typ target/bdc-$detail.pdf \
    --input catalog=bdc.json --input detail=$detail \
    --input integrity=target/bdc-integrity.json --input notes=report/audit-notes.toml
done
mutool info target/bdc-summary.pdf target/bdc-full.pdf | grep Pages
```

Expected: about 41 pages (summary) and 795 (full). Render and inspect the following pages:
- summary: pages 2, 3, and 5
- full: one matrix page and one appendix page

Use `mutool draw -r 80 -o target/p%d.png target/bdc-full.pdf <pages>`, finding the pages with
`mutool draw -F txt`. Check for:
- no text crossing a cell border
- no ID list longer than a line or two
- backticked text shown as code

- [ ] **Step 2: Write the changelog entry**

Run `date -u +%Y-%m-%dT%H-%M-%SZ` and create `changelog/<that>.md`:

```markdown
# PDF report detail levels

- `report/claims-report.typ` takes `--input detail=summary|full` (default `summary`; anything
  else fails the compile). Summary groups findings by area (the claim one level below the
  root), shows refuted claims in full and not-enough-info claims as one-line next steps (the
  text after `Fix:`), shows retired claims as counts per reason, and omits the hierarchy,
  matrix, and evidence appendix. Full keeps every section.
- Both levels: roll-up locators (`children: CLM-..., ...` and the new `children of CLM-NNNN`)
  render from the catalog as child counts plus the children that are not supported; runs of six
  or more IDs in free text are compressed to ranges; backtick spans render as code; every cell
  can wrap; evidence and verdicts are full-width blocks instead of narrow tables; finding cards
  can break across pages.
- Pure text helpers moved to `report/text.typ`, checked by `report/text-tests.typ`.
- New `<report-layout>` metadata (findings grouping, collapsed roll-up locators). Template
  tests render both levels and check it. `<audit-report>` is unchanged.
- `just report [summary|full]`: `detail` is now the first parameter, so `just report <catalog>`
  calls must become `just report summary <catalog>`. CI uploads `audit.pdf` and `audit-full.pdf`,
  and releases attach both.
- `claims-audit` skill: roll-up evidence uses locator `children of CLM-NNNN` and does not list
  children.
- On `bdc.json` (1,265 claims): 1,869 pages before; 41 (summary) and 795 (full) after.
- Spec: `docs/superpowers/specs/2026-10-23-report-detail-modes-design.md`. Plan:
  `docs/superpowers/plans/2026-10-23-report-detail-modes.md`.
```

Put the actual page counts from Step 1 in the bdc line.

- [ ] **Step 3: Run the pre-commit checklist**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && REQUIRE_TYPST=1 cargo test`
Expected: all succeed with no warnings. If `cargo fmt --check` fails, run `cargo fmt` and
include the result in the commit.

- [ ] **Step 4: Commit**

```bash
git add changelog/
git commit -m "docs: changelog for report detail levels"
```
