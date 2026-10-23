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

// Runs of six or more comma-separated claim ids in free text become compressed ranges. A run
// ends at a word boundary, so an id glued to other text ("CLM-0007x") is left out of it.
#let id-run = regex("CLM-\d+(?:,\s*CLM-\d+){5,}\b")
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
// "replaced by CLM-0663" count as one reason. Ids may be separated by commas, "and", or both
// (the Oxford comma ", and").
#let reason-key(reason) = reason.replace(
  regex("CLM-\d+(?:(?:,\s*(?:and\s+)?|\s+and\s+)CLM-\d+)*"),
  "[claim ids]",
)

// Zero-width spaces after separators let long paths and locators wrap inside table cells. A
// hyphen after an uppercase letter is skipped, so ids like `CLM-0042` and `EVD-0007` never
// split; the hyphen pattern matches the character before it too, which is kept in place.
#let wrappable(value) = value.replace(
  regex("[/_.:,=]|[^A-Z]-"),
  found => found.text + sym.zws,
)

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
