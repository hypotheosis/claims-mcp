// PDF audit report for a claims-mcp catalog. The rendered text is written for readers of the
// audited project, who may not know claims-mcp: it introduces the tool once (`about-ledger`)
// and refers to nothing that exists only in the claims-mcp repository.
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
// Gap rules follow claims-mcp's design spec, section 4.1, as in src/report.rs. The computed totals, gaps,
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
#let method = setting("method", "claims-audit skill (claims-mcp)")
#let assessment = setting("assessment", none)
#let commit = inputs.at("commit", default: none)
#let report-date = inputs.at("date", default: datetime.today().display())
// Where readers of another project's report can find the tool that recorded the ledger.
#let claims-mcp-url = "https://github.com/hypotheosis/claims-mcp"
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
// hand-edited cycle cannot loop forever; a claim whose walk ends in a cycle has no root above
// it, so it is treated as a root (no area) and is still listed when it is a finding.
#let area-of(claim) = {
  let path = (claim.id,)
  let parent = first-active-parent(claim)
  while parent != none and parent not in path {
    path.push(parent)
    parent = first-active-parent(by-id.at(parent))
  }
  if parent != none or path.len() < 2 { none } else { path.at(-2) }
}
#let area = active.map(claim => (claim.id, area-of(claim))).to-dict()

#let is-open(claim) = current-verdict(claim) in ("refuted", "not_enough_info")

// Areas with active children head their group instead of being listed as findings. Roots and
// childless areas are ordinary findings, so no refuted or undecided claim is hidden.
#let is-heading-rollup(claim) = {
  area.at(claim.id) == claim.id and children-of(claim.id).len() > 0
}
#let findings = active.filter(claim => is-open(claim) and not is-heading-rollup(claim))

// Findings grouped by area. Roots and childless areas (claims directly below a root, with no
// children) form the top-level group, listed first; then each heading area in catalog order. A
// group is kept when it has findings or when its head is itself refuted or not enough info.
#let finding-groups = {
  let grouped = (:)
  for claim in findings {
    let key = area.at(claim.id)
    let key = if key == none or key == claim.id { "" } else { key }
    grouped.insert(key, grouped.at(key, default: ()) + (claim,))
  }
  let top = if "" in grouped { ((area: none, findings: grouped.at("")),) } else { () }
  let heads = active.filter(claim => {
    is-heading-rollup(claim) and (claim.id in grouped or is-open(claim))
  })
  top + heads.map(head => (area: head.id, findings: grouped.at(head.id, default: ())))
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
    ([Detail], if full [full] else [summary (see Reproduce for the full report)]),
  ))
  pagebreak()
}

#let retired-claims() = {
  heading(level: 2)[Retired claims]
  [Retired claims are excluded from every total, gap, and table except this list#if full [ and
    the evidence appendix].]
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

// The one place the report explains claims-mcp; later sections name it without explanation.
#let about-ledger() = par[
  This report summarizes a claims ledger kept with
  #link(claims-mcp-url)[claims-mcp], an open-source requirements-traceability tool. Each
  requirement of #artifact is recorded as an atomic _claim_; an auditor attaches _evidence_
  (tests, analysis, inspection, demonstration) and records a _verdict_ on each claim:
  supported, refuted, or not enough info. Claims can be derived from broader claims, which
  forms the hierarchy used to group the findings.
]

#let section-result() = {
  heading[Result]
  about-ledger()
  if tampered {
    block(width: 100%, fill: rgb("#f3d0d0"), stroke: 0.8pt + luma(60), inset: 8pt, radius: 3pt)[
      *INTEGRITY CHECK FAILED.* The claims ledger was changed outside claims-mcp (see the
      Integrity section). Its verdicts and evidence cannot be trusted, so this report is not a clean
      result.
    ]
  } else if seal-check == none {
    par(emph[Integrity seals were not checked; see the Integrity section.])
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
  [claims-mcp seals every claim, evidence item, and verdict as it is recorded. A record whose
    seal fails was changed afterwards by other means, such as editing the ledger file by hand.]
  if seal-check == none {
    par(emph[Not checked: the report was rendered without the result of `claims-mcp verify`.])
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
    par[#seal-check.adopted.len() records were sealed when an older ledger was migrated
      (`claims-mcp seal --adopt`) rather than as they were recorded#if full [:] else [.]]
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
  if group.findings.len() == 0 {
    par(emph[No findings below this area.])
  } else {
    par(emph[#group.findings.len() findings: #finding-counts(group.findings).])
  }
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
  if finding-groups.len() == 0 {
    emph[No active claim is refuted or undecided.]
    return
  }
  [Active claims that are refuted or not enough info, grouped by area: the claim one level
    below the root. Areas with children head their group instead of being listed; roots and
    claims directly below a root with no children are under Top level.]
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
  [Each claim's _integrity level_ (IL, from the IEEE 1012 verification standard) rates the
    consequence if it is wrong: 4 safety, security, or data loss; 3 a core function fails; 2 a
    function is degraded; 1 cosmetic or documentation. Higher levels call for more rigorous
    verification.]
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
  let critical = active.filter(claim => integrity(claim) in (3, 4)).len()
  [Claims at integrity level 3 or 4 should be verified by test, analysis, demonstration, or
    formal proof, not by _inspection_ alone (someone reading the code or documents). ]
  if critical == 0 {
    [No active claim has integrity level 3 or 4.]
  } else if gaps.rigor.len() == 0 {
    [All #critical level 3-4 claims meet this: none relies on inspection alone.]
  } else {
    [*RIGOR GAP:* #gaps.rigor.len() of the #critical level 3-4 claims rely on inspection
      alone: #compress-ids(gaps.rigor).]
  }
}

#let gap-meanings = (
  no_evidence: [Claim has no evidence at all.],
  unverified: [Claim has no verdict.],
  refuted: [Current verdict is refuted.],
  not_enough_info: [Current verdict is not enough info.],
  stale: [Evidence was recorded after the latest verdict.],
  rigor: [Integrity level 3-4 claim that relies on inspection alone.],
  decomposition: [At least one active child is not supported.],
  dangling_links: [Claim derives from a retired claim.],
)

#let section-gaps() = {
  heading[Gap summary]
  [Every active claim was checked against all eight gap categories below.]
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
  [Active claims, each indented under the claims it was derived from. A claim with several
    parents appears under each. Roots are claims with no active parent.]
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
  [Regenerate this PDF with the #link(claims-mcp-url)[claims-mcp] command-line tool and its
    report template (`report/claims-report.typ` in the claims-mcp repository; adjust the path
    to where it is on your machine) using #link("https://typst.app")[Typst]:]
  raw(
    block: true,
    lang: "sh",
    checkout
      + (if "integrity" in inputs {
        "claims-mcp --catalog " + catalog-path + " verify --json > " + inputs.integrity + "\n"
      } else { "" })
      + "typst compile --root . report/claims-report.typ "
      + (if full { "audit-full.pdf" } else { "audit.pdf" })
      + " \\\n  --input catalog="
      + catalog-path
      + " \\\n  --input detail="
      + detail
      + commit-flag
      + notes-flag
      + integrity-flag,
  )
  if not full {
    par[This is the summary. Render with `--input detail=full` for every finding in full, every
      claim per gap, the claim hierarchy, the traceability matrix, and the evidence appendix
      with each claim's verdict history.]
  }
  par[To re-check a verdict, follow the locators of its evidence: each names the test, command,
    or file in #artifact to re-run or re-inspect.]
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
// External links (claims-mcp, Typst, issues) are underlined so print readers can spot them.
#show link: underline

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
