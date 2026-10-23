// PDF audit report for a claims-mcp catalog.
//
//   typst compile --root . report/claims-report.typ audit.pdf \
//     --input catalog=claims.json --input commit=$(git rev-parse --short HEAD) \
//     --input notes=report/audit-notes.toml
//
// Inputs (all optional):
//   catalog     catalog path, relative to --root (default claims.json)
//   commit      catalog commit shown on the cover and in the reproduce section; a "-dirty"
//               suffix (git describe --dirty) marks uncommitted changes
//   notes       TOML sidecar: artifact, version, repository, method, assessment, [[points]]
//   date        report date (default: today)
//   artifact, version, repository, method, assessment
//               override the matching sidecar keys
//
// Gap rules follow spec section 4.1 exactly, as in src/report.rs. The computed totals, gaps,
// and matrix are exposed as <audit-report> metadata so `typst query` can be compared with
// `audit_report`; see the `typst_template` tests in src/report.rs.

// ---------------------------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------------------------

// Typst resolves "/x" against --root and "x" against this file's directory. Catalog and notes
// paths are given relative to the root, like the command-line paths around them.
#let rooted(path) = if path.starts-with("/") { path } else { "/" + path }

#let inputs = sys.inputs
#let catalog-path = inputs.at("catalog", default: "claims.json")
#let catalog = json(rooted(catalog-path))
#let notes = if "notes" in inputs { toml(rooted(inputs.notes)) } else { (:) }
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

#metadata((totals: totals, gaps: gaps, matrix: matrix)) <audit-report>

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

// Zero-width spaces after separators let long paths and locators wrap inside table cells.
#let wrappable(value) = value.replace(regex("[/_.:,=-]"), found => found.text + sym.zws)
#let id-text(id) = raw(id)
#let ids(list) = if list.len() == 0 { [none] } else { list.map(id-text).join(", ") }
#let or-dash(value) = if value == none { [-] } else { [#value] }
#let humanize(name) = name.replace("_", " ")
#let level-text(claim) = or-dash(integrity(claim))
#let all-verdicts = ("supported", "refuted", "not_enough_info", "unverified")

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

// ---------------------------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------------------------

#let auditors = claims.map(claim => claim.verdicts.map(record => record.auditor)).flatten().dedup()

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
  ))
  pagebreak()
}

#let section-result() = {
  heading[Result]
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

  heading(level: 2)[Retired claims]
  [Retired claims are excluded from every total, gap, and table except this list and the
    evidence appendix.]
  if retired.len() == 0 {
    par(emph[No claims are retired.])
  } else {
    data-table(
      columns: (auto, 2fr, 1.4fr, auto),
      header: ([Claim], [Statement], [Reason], [Retired at]),
      ..retired.map(claim => (
        id-text(claim.id),
        claim.statement,
        claim.status.reason,
        wrappable(claim.status.retired_at),
      )),
    )
  }
}

#let finding(claim) = {
  let latest = claim.verdicts.last()
  let cited = claim.evidence.filter(item => item.id in latest.evidence_ids)
  let issues = issue-links(claim)
  block(breakable: false, width: 100%, stroke: 0.5pt + luma(150), inset: 8pt, radius: 3pt, {
    [#strong(id-text(claim.id)) #h(0.4em) #marker(current-verdict(claim))]
    par(claim.statement)
    field-table(
      ([Integrity / method], [IL #level-text(claim), #humanize(claim.verification_method)]),
      ([Rationale], latest.rationale),
      ([Auditor], [#latest.auditor, #latest.recorded_at]),
      ([Linked issue], if issues.len() == 0 { [none] } else { issues.join(", ") }),
    )
    if cited.len() == 0 {
      emph[The verdict cites no evidence.]
    } else {
      data-table(
        columns: (auto, auto, 1.2fr, 2fr),
        header: ([Evidence], [Stance], [Locator], [Summary]),
        ..cited.map(item => (id-text(item.id), item.stance, wrappable(item.locator), item.summary)),
      )
    }
  })
}

#let section-findings() = {
  heading[Findings]
  let findings = active.filter(claim => current-verdict(claim) in ("refuted", "not_enough_info"))
  if findings.len() == 0 {
    emph[No active claim is refuted or undecided.]
  } else {
    findings.map(finding).join(v(0.6em))
  }
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
    [*RIGOR GAP:* these integrity level 3-4 claims are verified only by inspection:
      #ids(gaps.rigor).]
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
    columns: (auto, 1.4fr, auto, 1.6fr),
    header: ([Gap], [Meaning], [Count], [Claims]),
    ..gaps.pairs().map(((name, members)) => (
      raw(name),
      gap-meanings.at(name),
      str(members.len()),
      ids(members),
    )),
  )
}

#let tree-node(id, depth) = {
  let claim = by-id.at(id)
  block(above: 0.5em, below: 0.5em, breakable: false, inset: (left: depth * 1.4em), {
    [#strong(id-text(id)) #marker(current-verdict(claim)) #h(0.3em)]
    text(size: 0.85em)[#claim.level, IL #level-text(claim)]
    linebreak()
    text(size: 0.85em, claim.statement)
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
      claim.statement,
      or-dash(claim.at("source", default: none)),
      ids(claim.derived_from),
      claim.level,
      level-text(claim),
      humanize(claim.verification_method),
      if claim.evidence.len() == 0 {
        [-]
      } else {
        claim.evidence.map(item => wrappable(item.locator)).join(linebreak())
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
  par(claim.statement)
  if claim.evidence.len() == 0 {
    par(emph[No evidence.])
  } else {
    data-table(
      columns: (auto, auto, auto, 1.3fr, 2fr, auto),
      header: ([Evidence], [Kind], [Stance], [Locator], [Summary], [Recorded]),
      ..claim.evidence.map(item => (
        id-text(item.id),
        item.kind,
        item.stance,
        wrappable(item.locator),
        item.summary,
        wrappable(item.recorded_at),
      )),
    )
  }
  if claim.verdicts.len() == 0 {
    par(emph[No verdicts.])
  } else {
    data-table(
      columns: (auto, auto, auto, 2fr, auto),
      header: ([Verdict], [Auditor], [Cites], [Rationale], [Recorded]),
      ..claim.verdicts.map(record => (
        marker(record.verdict),
        record.auditor,
        ids(record.evidence_ids),
        record.rationale,
        wrappable(record.recorded_at),
      )),
    )
  }
}

#let section-evidence() = {
  heading[Evidence appendix]
  [Every evidence item, grouped by claim, followed by the claim's full verdict history in the
    order recorded (the history is append-only, so the last row is the current verdict).]
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
  let commit-flag = if commit == none { "" } else { " \\\n  --input commit=" + commit }
  [Regenerate this PDF:]
  raw(
    block: true,
    lang: "sh",
    checkout
      + "typst compile --root . report/claims-report.typ audit.pdf \\\n  --input catalog="
      + catalog-path
      + commit-flag
      + notes-flag,
  )
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
  section-findings()
  section-points()
  section-coverage()
  section-gaps()
  section-hierarchy()
  section-matrix()
  section-evidence()
  section-reproduce()
}
