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
