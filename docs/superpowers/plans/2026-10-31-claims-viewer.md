# Claims Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A static page at `<docs site>/viewer/` that loads a `claims.json` (from a public GitHub
repository or a local file) and makes it easy to browse: overview, filterable list, claim detail
with hierarchy, verdict history and evidence.

**Architecture:** Five static files in `docs/site/src/viewer/`, which mdBook copies into the built
site unchanged. `model.js` (pure: parse, derive, filter, count, route, anchor links) and
`source.js` (loading) are ES modules with no DOM use, unit-tested with Node's built-in runner;
`app.js` renders with DOM APIs only (never `innerHTML` with catalog data, which is untrusted).

**Tech Stack:** HTML, vanilla JavaScript (ES2022 modules), Pico CSS 2.1.1 (jsDelivr, SRI-pinned),
Google Fonts, `node --test` (Node 24 locally; preinstalled on GitHub's Ubuntu runners), mdBook.

**Spec:** `docs/superpowers/specs/2026-10-31-claims-viewer-design.md`

---

## File map

| Path | Responsibility |
|---|---|
| `docs/site/src/viewer/model.js` | Catalog parsing and normalisation, derived fields, filters, counts, hash routes, anchor links |
| `docs/site/src/viewer/source.js` | Repository input parsing, GitHub raw fetch, local file read, readable errors |
| `docs/site/src/viewer/app.js` | DOM rendering, events, theme, URL state |
| `docs/site/src/viewer/index.html` | Page shell, Pico and fonts |
| `docs/site/src/viewer/viewer.css` | The viewer's theme and layout |
| `docs/site/viewer-tests/model.test.mjs` | Unit tests for `model.js` |
| `docs/site/viewer-tests/source.test.mjs` | Unit tests for `source.js` |
| `docs/site/src/viewer.md`, `SUMMARY.md` | Docs page and sidebar entry |
| `README.md` | One line in the Documentation list |
| `justfile` | `viewer-test` recipe |
| `.github/workflows/ci.yml` | Run the viewer tests in the `check` job |
| `changelog/<timestamp>.md` | Change record |

Test command used throughout: `node --test docs/site/viewer-tests/`.

---

### Task 1: `model.js`: parsing and derived fields

**Files:** Create `docs/site/src/viewer/model.js`, `docs/site/viewer-tests/model.test.mjs`.

- [ ] **Step 1: Write the failing tests**

```js
// docs/site/viewer-tests/model.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  CatalogError, parseCatalog, splitFix, countVerdicts, needsAttention, areaSummary,
  filterClaims, facetValues, parseRoute, formatRoute, fileAnchorUrl, searchCommand,
} from '../src/viewer/model.js';

const claim = (over = {}) => ({ id: 'CLM-0001', statement: 'It works', ...over });
const doc = (claims, extra = {}) => JSON.stringify({ schema_version: 2, claims, ...extra });

test('parses a minimal catalog with defaults', () => {
  const { claims, byId, schemaVersion } = parseCatalog(doc([claim()]));
  assert.equal(schemaVersion, 2);
  assert.equal(claims.length, 1);
  const c = byId.get('CLM-0001');
  assert.equal(c.current, 'unverified');
  assert.deepEqual(c.areas, ['Unassigned']);
  assert.deepEqual(c.issues, []);
  assert.deepEqual(c.children, []);
  assert.equal(c.integrityLevel, null);
  assert.equal(c.retired, false);
  assert.equal(c.sourceAnchor, null);
});

test('parses every optional field', () => {
  const full = claim({
    kind: 'functional', level: 'unit', integrity_level: 3, verification_method: 'test',
    source: 'spec 4', derived_from: [], artifacts: ['src/a.rs'],
    tags: ['area:auth', 'issue:12', 'x'],
    source_anchor: { path: 'README.md', lines: { start: 4, end: 6 }, commit: 'abc1234', dirty: true },
    status: { state: 'retired', reason: 'moved', retired_at: '2026-01-02T00:00:00Z' },
    evidence: [{
      id: 'EVD-0001', kind: 'test', locator: 'cargo test x', summary: 'passes', stance: 'supports',
      recorded_at: '2026-01-01T00:00:00Z',
      anchor: { search: { pattern: 'a|b', case_insensitive: true, scope: ['src'], hits: 0, commit: 'abc1234' } },
    }],
    verdicts: [
      { verdict: 'refuted', rationale: 'No. Fix: add it', auditor: 'm', evidence_ids: ['EVD-0001'], recorded_at: '2026-01-01T00:00:00Z' },
      { verdict: 'supported', rationale: 'Yes', auditor: 'm', evidence_ids: ['EVD-0001'], recorded_at: '2026-01-02T00:00:00Z' },
    ],
  });
  const c = parseCatalog(doc([full])).byId.get('CLM-0001');
  assert.equal(c.kind, 'functional');
  assert.equal(c.integrityLevel, 3);
  assert.equal(c.method, 'test');
  assert.deepEqual(c.areas, ['auth']);
  assert.deepEqual(c.issues, [12]);
  assert.equal(c.retired, true);
  assert.equal(c.retiredReason, 'moved');
  assert.equal(c.current, 'supported');
  assert.deepEqual(c.sourceAnchor, { type: 'file', path: 'README.md', lines: { start: 4, end: 6 }, commit: 'abc1234', dirty: true });
  assert.deepEqual(c.evidence[0].anchor, { type: 'search', pattern: 'a|b', caseInsensitive: true, scope: ['src'], hits: 0, commit: 'abc1234', dirty: false });
  assert.equal(c.verdicts[0].fix, 'add it');
  assert.equal(c.verdicts[1].fix, null);
});

test('reads schema version 1 and ignores unknown fields and seals', () => {
  const text = JSON.stringify({ schema_version: 1, claims: [claim({ seal: { by: 'server', mac: 'x' }, extra: 1 })], other: true });
  const parsed = parseCatalog(text);
  assert.equal(parsed.schemaVersion, 1);
  assert.equal(parsed.claims[0].id, 'CLM-0001');
});

test('reports invalid input', () => {
  assert.throws(() => parseCatalog('{nope'), CatalogError);
  assert.throws(() => parseCatalog('[]'), /claims/);
  assert.throws(() => parseCatalog('{"claims": 3}'), /claims/);
  assert.throws(() => parseCatalog(doc([{ statement: 'x' }])), /Claim 1.*id/);
  assert.throws(() => parseCatalog(doc([{ id: 'CLM-0001' }])), /Claim 1.*statement/);
});

test('builds children from active claims only', () => {
  const { byId } = parseCatalog(doc([
    claim({ id: 'CLM-0001' }),
    claim({ id: 'CLM-0002', derived_from: ['CLM-0001'] }),
    claim({ id: 'CLM-0003', derived_from: ['CLM-0001'], status: { state: 'retired', reason: 'r' } }),
    claim({ id: 'CLM-0004', derived_from: ['CLM-0099'] }),
  ]));
  assert.deepEqual(byId.get('CLM-0001').children, ['CLM-0002']);
});

test('splitFix takes the text after the last Fix:', () => {
  assert.deepEqual(splitFix('Bad. Fix: one. Fix: two'), { body: 'Bad. Fix: one.', fix: 'two' });
  assert.deepEqual(splitFix('Fine.'), { body: 'Fine.', fix: null });
});
```

- [ ] **Step 2: Run them; expect failure.** Run `node --test docs/site/viewer-tests/`. Expected:
  FAIL with `Cannot find module` for `model.js`.

- [ ] **Step 3: Write `model.js` (parsing part)**

```js
// docs/site/src/viewer/model.js
// Pure functions over a claims-mcp catalog: no DOM, no network. app.js renders what these return.

export class CatalogError extends Error {}

const STANCES = new Set(['supports', 'refutes', 'neutral']);
const VERDICTS = new Set(['supported', 'refuted', 'not_enough_info']);

const isObject = (value) => typeof value === 'object' && value !== null && !Array.isArray(value);
const text = (value) => (typeof value === 'string' ? value : '');
const strings = (value) => (Array.isArray(value) ? value.filter((item) => typeof item === 'string') : []);
const list = (value) => (Array.isArray(value) ? value : []);

/** Splits a verdict rationale at its last `Fix:` marker. */
export function splitFix(rationale) {
  const at = rationale.lastIndexOf('Fix:');
  if (at < 0) return { body: rationale, fix: null };
  return { body: rationale.slice(0, at).trim(), fix: rationale.slice(at + 4).trim() || null };
}

function parseLines(value) {
  if (!isObject(value) || !Number.isInteger(value.start) || !Number.isInteger(value.end)) return null;
  return { start: value.start, end: value.end };
}

function parseFileAnchor(value) {
  if (!isObject(value) || typeof value.path !== 'string') return null;
  return { type: 'file', path: value.path, lines: parseLines(value.lines), commit: text(value.commit), dirty: value.dirty === true };
}

function parseAnchor(value) {
  if (!isObject(value)) return null;
  if (isObject(value.file)) return parseFileAnchor(value.file);
  if (isObject(value.search)) {
    const s = value.search;
    return {
      type: 'search', pattern: text(s.pattern), caseInsensitive: s.case_insensitive === true,
      scope: strings(s.scope), hits: Number.isInteger(s.hits) ? s.hits : null,
      commit: text(s.commit), dirty: s.dirty === true,
    };
  }
  return null;
}

function parseEvidence(value) {
  const e = isObject(value) ? value : {};
  return {
    id: text(e.id), kind: text(e.kind), locator: text(e.locator), summary: text(e.summary),
    stance: STANCES.has(e.stance) ? e.stance : 'neutral', anchor: parseAnchor(e.anchor),
    recordedAt: text(e.recorded_at),
  };
}

function parseVerdict(value) {
  const v = isObject(value) ? value : {};
  const rationale = text(v.rationale);
  return {
    verdict: VERDICTS.has(v.verdict) ? v.verdict : 'not_enough_info',
    rationale, fix: splitFix(rationale).fix, auditor: text(v.auditor),
    evidenceIds: strings(v.evidence_ids), recordedAt: text(v.recorded_at),
  };
}

function tagValues(tags, prefix) {
  return tags.filter((tag) => tag.startsWith(prefix)).map((tag) => tag.slice(prefix.length)).filter(Boolean);
}

function parseClaim(value, index) {
  const where = `Claim ${index + 1}`;
  if (!isObject(value)) throw new CatalogError(`${where} is not an object.`);
  if (typeof value.id !== 'string' || !value.id) throw new CatalogError(`${where} has no id.`);
  if (typeof value.statement !== 'string') throw new CatalogError(`${where} (${value.id}) has no statement.`);
  const tags = strings(value.tags);
  const status = isObject(value.status) ? value.status : {};
  const verdicts = list(value.verdicts).map(parseVerdict);
  const areas = tagValues(tags, 'area:');
  return {
    id: value.id, statement: value.statement, kind: text(value.kind), level: text(value.level),
    integrityLevel: Number.isInteger(value.integrity_level) ? value.integrity_level : null,
    method: text(value.verification_method), source: text(value.source),
    sourceAnchor: parseFileAnchor(value.source_anchor),
    derivedFrom: strings(value.derived_from), artifacts: strings(value.artifacts), tags,
    areas: areas.length ? areas : ['Unassigned'],
    issues: tagValues(tags, 'issue:').map(Number).filter(Number.isInteger),
    retired: status.state === 'retired', retiredReason: text(status.reason), retiredAt: text(status.retired_at),
    evidence: list(value.evidence).map(parseEvidence), verdicts,
    current: verdicts.length ? verdicts[verdicts.length - 1].verdict : 'unverified',
    children: [], createdAt: text(value.created_at), updatedAt: text(value.updated_at),
  };
}

/**
 * Parses catalog JSON text. Throws CatalogError with a readable message when the text is not a
 * claims catalog. Seals and unknown fields are ignored.
 */
export function parseCatalog(source) {
  let data;
  try {
    data = JSON.parse(source);
  } catch {
    throw new CatalogError('The file is not valid JSON.');
  }
  if (!isObject(data) || !Array.isArray(data.claims)) {
    throw new CatalogError('This JSON is not a claims catalog: it has no "claims" list.');
  }
  const claims = data.claims.map(parseClaim);
  const byId = new Map(claims.map((c) => [c.id, c]));
  for (const c of claims) {
    if (c.retired) continue;
    for (const parent of c.derivedFrom) byId.get(parent)?.children.push(c.id);
  }
  return { schemaVersion: Number.isInteger(data.schema_version) ? data.schema_version : null, claims, byId };
}
```

The test file already imports Task 2's functions, so append these stubs to `model.js` to let
it load. Task 2 replaces them.

```js
export const countVerdicts = () => {}; export const needsAttention = () => {};
export const areaSummary = () => {}; export const filterClaims = () => {};
export const facetValues = () => {}; export const parseRoute = () => {};
export const formatRoute = () => {}; export const fileAnchorUrl = () => {};
export const searchCommand = () => {};
```

- [ ] **Step 4: Run the tests; expect PASS.** `node --test docs/site/viewer-tests/`: 6 passing.
- [ ] **Step 5: Commit** with message `feat(viewer): parse claims catalogs`.

### Task 2: `model.js`: counts, filters, routes, anchor links

**Files:** Modify `docs/site/src/viewer/model.js` (replace the stubs),
`docs/site/viewer-tests/model.test.mjs` (append).

- [ ] **Step 1: Append the failing tests**

```js
const sample = () => parseCatalog(doc([
  claim({ id: 'CLM-0001', tags: ['area:docs'], integrity_level: 2, kind: 'documentation', level: 'artifact',
    verdicts: [{ verdict: 'supported', rationale: 'ok', auditor: 'm', evidence_ids: [] }] }),
  claim({ id: 'CLM-0002', statement: 'Seals refuse writes', tags: ['area:seals'], integrity_level: 4, kind: 'security', level: 'unit',
    verdicts: [{ verdict: 'refuted', rationale: 'no', auditor: 'm', evidence_ids: [] }] }),
  claim({ id: 'CLM-0003', tags: ['area:seals'], integrity_level: 2 }),
  claim({ id: 'CLM-0004', integrity_level: 3, verdicts: [{ verdict: 'not_enough_info', rationale: '?', auditor: 'm' }] }),
  claim({ id: 'CLM-0005', status: { state: 'retired', reason: 'gone' } }),
])).claims;

test('countVerdicts counts active claims by current verdict', () => {
  assert.deepEqual(countVerdicts(sample()), { total: 4, supported: 1, refuted: 1, not_enough_info: 1, unverified: 1 });
});

test('needsAttention orders refuted, not_enough_info, unverified, then integrity desc, then id', () => {
  assert.deepEqual(needsAttention(sample()).map((c) => c.id), ['CLM-0002', 'CLM-0004', 'CLM-0003']);
});

test('areaSummary groups active claims by area, sorted by name', () => {
  const rows = areaSummary(sample());
  assert.deepEqual(rows.map((r) => [r.area, r.counts.total, r.counts.supported]), [['docs', 1, 1], ['seals', 2, 0], ['Unassigned', 1, 0]]);
});

test('filterClaims applies text, facets and the retired toggle', () => {
  const all = sample();
  assert.deepEqual(filterClaims(all, {}).map((c) => c.id), ['CLM-0001', 'CLM-0002', 'CLM-0003', 'CLM-0004']);
  assert.equal(filterClaims(all, { includeRetired: true }).length, 5);
  assert.deepEqual(filterClaims(all, { text: 'SEALS' }).map((c) => c.id), ['CLM-0002', 'CLM-0003']);
  assert.deepEqual(filterClaims(all, { verdict: 'unverified' }).map((c) => c.id), ['CLM-0003']);
  assert.deepEqual(filterClaims(all, { area: 'seals', integrity: '4' }).map((c) => c.id), ['CLM-0002']);
  assert.deepEqual(filterClaims(all, { kind: 'documentation', level: 'artifact' }).map((c) => c.id), ['CLM-0001']);
});

test('facetValues lists the values present, sorted', () => {
  const f = facetValues(sample());
  assert.deepEqual(f.kinds, ['documentation', 'security']);
  assert.deepEqual(f.integrityLevels, [4, 3, 2]);
  assert.deepEqual(f.areas, ['docs', 'seals', 'Unassigned']);
});

test('routes round-trip through the URL hash', () => {
  assert.deepEqual(parseRoute(''), { view: 'overview' });
  assert.deepEqual(parseRoute('#overview'), { view: 'overview' });
  assert.deepEqual(parseRoute('#CLM-0072'), { view: 'claim', id: 'CLM-0072' });
  assert.deepEqual(parseRoute('#claims?verdict=refuted&area=docs'), { view: 'claims', filter: { verdict: 'refuted', area: 'docs' } });
  assert.equal(formatRoute({ view: 'claims', filter: { verdict: 'refuted', text: '' } }), '#claims?verdict=refuted');
  assert.equal(formatRoute({ view: 'claim', id: 'CLM-0072' }), '#CLM-0072');
  assert.equal(formatRoute({ view: 'overview' }), '#overview');
});

test('fileAnchorUrl links to the pinned commit on GitHub', () => {
  const repo = { owner: 'acme', name: 'widget' };
  const a = { type: 'file', path: 'src/a b.rs', lines: { start: 3, end: 9 }, commit: '0123abcd', dirty: false };
  assert.equal(fileAnchorUrl(repo, a), 'https://github.com/acme/widget/blob/0123abcd/src/a%20b.rs#L3-L9');
  assert.equal(fileAnchorUrl(repo, { ...a, lines: { start: 3, end: 3 } }), 'https://github.com/acme/widget/blob/0123abcd/src/a%20b.rs#L3');
  assert.equal(fileAnchorUrl(repo, { ...a, lines: null }), 'https://github.com/acme/widget/blob/0123abcd/src/a%20b.rs');
  assert.equal(fileAnchorUrl(null, a), null);
  assert.equal(fileAnchorUrl(repo, { ...a, commit: 'not-a-sha' }), null);
});

test('searchCommand renders the git grep a search anchor ran', () => {
  const s = { type: 'search', pattern: "it's|b", caseInsensitive: true, scope: ['src', 'my dir'], hits: 0, commit: 'abc', dirty: false };
  assert.equal(searchCommand(s), "git grep -iE 'it'\\''s|b' -- src 'my dir'");
});

test('smoke: parses this repository\'s own claims.json', (t) => {
  const tldrPath = process.env.CLAIMS_TLDR_JSON;
  if (!tldrPath) {
    t.skip('CLAIMS_TLDR_JSON is not set; run `just viewer-test` to include this check');
    return;
  }
  const catalog = parseCatalog(readFileSync(new URL('../../../claims.json', import.meta.url), 'utf8'));
  const tldr = JSON.parse(readFileSync(tldrPath, 'utf8'));
  assert.equal(countVerdicts(catalog.claims).total, tldr.summary.active_claims);
});
```

- [ ] **Step 2: Run; expect the new tests to FAIL** (the stubs return `undefined`).

- [ ] **Step 3: Replace the stubs in `model.js` with:**

```js
export const ATTENTION = ['refuted', 'not_enough_info', 'unverified'];
const byIntegrityThenId = (a, b) => (b.integrityLevel ?? 0) - (a.integrityLevel ?? 0) || a.id.localeCompare(b.id);
const active = (claims) => claims.filter((c) => !c.retired);

/** Counts active claims by current verdict. */
export function countVerdicts(claims) {
  const counts = { total: 0, supported: 0, refuted: 0, not_enough_info: 0, unverified: 0 };
  for (const c of active(claims)) {
    counts.total += 1;
    counts[c.current] += 1;
  }
  return counts;
}

/** Active claims that are refuted, undecided or unjudged, worst first. */
export function needsAttention(claims) {
  return active(claims)
    .filter((c) => ATTENTION.includes(c.current))
    .sort((a, b) => ATTENTION.indexOf(a.current) - ATTENTION.indexOf(b.current) || byIntegrityThenId(a, b));
}

const sortAreas = (a, b) => (a === 'Unassigned') - (b === 'Unassigned') || a.localeCompare(b);

/** Verdict counts per area tag, areas sorted by name with Unassigned last. */
export function areaSummary(claims) {
  const groups = new Map();
  for (const c of active(claims)) {
    for (const area of c.areas) {
      if (!groups.has(area)) groups.set(area, []);
      groups.get(area).push(c);
    }
  }
  return [...groups.keys()].sort(sortAreas).map((area) => ({ area, counts: countVerdicts(groups.get(area)) }));
}

/**
 * Claims matching every set filter. `filter` fields: text, verdict, kind, level, integrity (a
 * string, as in the URL), area, includeRetired.
 */
export function filterClaims(claims, filter) {
  const needle = (filter.text ?? '').trim().toLowerCase();
  return claims.filter((c) =>
    (filter.includeRetired || !c.retired)
    && (!needle || [c.id, c.statement, ...c.tags].some((s) => s.toLowerCase().includes(needle)))
    && (!filter.verdict || c.current === filter.verdict)
    && (!filter.kind || c.kind === filter.kind)
    && (!filter.level || c.level === filter.level)
    && (!filter.integrity || String(c.integrityLevel ?? '') === filter.integrity)
    && (!filter.area || c.areas.includes(filter.area)));
}

/** The distinct filter values present in a catalog. */
export function facetValues(claims) {
  const distinct = (pick) => [...new Set(claims.flatMap(pick).filter((v) => v !== '' && v !== null))];
  return {
    kinds: distinct((c) => [c.kind]).sort(),
    levels: distinct((c) => [c.level]).sort(),
    integrityLevels: distinct((c) => [c.integrityLevel]).sort((a, b) => b - a),
    areas: distinct((c) => c.areas).sort(sortAreas),
  };
}

const FILTER_KEYS = ['text', 'verdict', 'kind', 'level', 'integrity', 'area', 'includeRetired'];

/** Reads the view and selection from a URL hash. */
export function parseRoute(hash) {
  const raw = hash.replace(/^#/, '');
  if (/^CLM-\d+$/i.test(raw)) return { view: 'claim', id: raw.toUpperCase() };
  if (raw === 'claims' || raw.startsWith('claims?')) {
    const params = new URLSearchParams(raw.slice('claims'.length).replace(/^\?/, ''));
    const filter = {};
    for (const key of FILTER_KEYS) {
      const value = params.get(key);
      if (value) filter[key] = key === 'includeRetired' ? value === 'true' : value;
    }
    return { view: 'claims', filter };
  }
  return { view: 'overview' };
}

/** Writes a route as a URL hash; empty filter values are dropped. */
export function formatRoute(route) {
  if (route.view === 'claim') return `#${route.id}`;
  if (route.view !== 'claims') return '#overview';
  const params = new URLSearchParams();
  for (const key of FILTER_KEYS) {
    const value = route.filter?.[key];
    if (value) params.set(key, String(value));
  }
  const query = params.toString();
  return query ? `#claims?${query}` : '#claims';
}

/** GitHub link for a file anchor at its pinned commit, or null without a repo or valid commit. */
export function fileAnchorUrl(repo, anchor) {
  if (!repo || !anchor || anchor.type !== 'file' || !/^[0-9a-f]{7,40}$/i.test(anchor.commit)) return null;
  const path = anchor.path.split('/').map(encodeURIComponent).join('/');
  const base = `https://github.com/${encodeURIComponent(repo.owner)}/${encodeURIComponent(repo.name)}/blob/${anchor.commit}/${path}`;
  if (!anchor.lines) return base;
  const { start, end } = anchor.lines;
  return start === end ? `${base}#L${start}` : `${base}#L${start}-L${end}`;
}

const shellQuote = (s) => `'${s.replace(/'/g, "'\\''")}'`;
const shellWord = (s) => (/^[A-Za-z0-9_./-]+$/.test(s) ? s : shellQuote(s));

/** The git grep command a search anchor recorded. */
export function searchCommand(anchor) {
  return `git grep ${anchor.caseInsensitive ? '-iE' : '-E'} ${shellQuote(anchor.pattern)} -- ${anchor.scope.map(shellWord).join(' ')}`;
}
```

- [ ] **Step 4: Run; expect PASS** (14 passing, 1 skipped: the smoke test without
  `CLAIMS_TLDR_JSON`).
- [ ] **Step 5: Commit** with message `feat(viewer): counts, filters, routes and anchor links`.

### Task 3: `source.js`

**Files:** Create `docs/site/src/viewer/source.js`, `docs/site/viewer-tests/source.test.mjs`.

- [ ] **Step 1: Write the failing tests**

```js
// docs/site/viewer-tests/source.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SourceError, parseRepoInput, rawUrl, fetchFromGithub } from '../src/viewer/source.js';

test('parseRepoInput accepts owner/name and GitHub URLs', () => {
  assert.deepEqual(parseRepoInput('acme/widget'), { owner: 'acme', name: 'widget' });
  assert.deepEqual(parseRepoInput(' https://github.com/acme/widget.git/ '), { owner: 'acme', name: 'widget' });
  assert.deepEqual(parseRepoInput('github.com/acme/widget/tree/main'), { owner: 'acme', name: 'widget' });
  assert.equal(parseRepoInput('not a repo'), null);
  assert.equal(parseRepoInput('https://example.com/acme/widget'), null);
});

test('rawUrl encodes each part', () => {
  assert.equal(rawUrl({ owner: 'acme', name: 'widget', ref: 'HEAD', path: 'docs/claims.json' }),
    'https://raw.githubusercontent.com/acme/widget/HEAD/docs/claims.json');
  assert.equal(rawUrl({ owner: 'acme', name: 'widget', ref: 'feat/x', path: 'a b.json' }),
    'https://raw.githubusercontent.com/acme/widget/feat/x/a%20b.json');
});

const fakeFetch = (status, body = '') => async () => ({ ok: status >= 200 && status < 300, status, text: async () => body });

test('fetchFromGithub returns the text and the source', async () => {
  const got = await fetchFromGithub({ owner: 'acme', name: 'widget' }, fakeFetch(200, '{"claims":[]}'));
  assert.equal(got.text, '{"claims":[]}');
  assert.deepEqual(got.source, { kind: 'github', owner: 'acme', name: 'widget', ref: 'HEAD', path: 'claims.json' });
});

test('fetchFromGithub explains failures', async () => {
  await assert.rejects(fetchFromGithub({ owner: 'a', name: 'b' }, fakeFetch(404)), (e) => e instanceof SourceError && /private/.test(e.message));
  await assert.rejects(fetchFromGithub({ owner: 'a', name: 'b' }, fakeFetch(500)), /500/);
  await assert.rejects(fetchFromGithub({ owner: 'a', name: 'b' }, async () => { throw new TypeError('x'); }), /reach GitHub/);
});
```

- [ ] **Step 2: Run; expect FAIL** (module not found).

- [ ] **Step 3: Write `source.js`**

```js
// docs/site/src/viewer/source.js
// Loading a catalog: from a public GitHub repository through raw.githubusercontent.com (CORS-open,
// no API rate limit, no token), or from a local file. Failures become SourceError messages.

export class SourceError extends Error {}

const NAME = /^[A-Za-z0-9_.-]+$/;

/** `owner/name` or a github.com URL to {owner, name}; null when it is neither. */
export function parseRepoInput(input) {
  const trimmed = input.trim().replace(/\/+$/, '');
  const match = trimmed.match(/^(?:https?:\/\/)?(?:www\.)?github\.com\/([^/\s]+)\/([^/\s#?]+)/i)
    ?? trimmed.match(/^([^/\s:]+)\/([^/\s]+)$/);
  if (!match) return null;
  const owner = match[1];
  const name = match[2].replace(/\.git$/, '');
  return NAME.test(owner) && NAME.test(name) ? { owner, name } : null;
}

const encodePath = (path) => path.split('/').map(encodeURIComponent).join('/');

/** The raw.githubusercontent.com URL of a file at a ref (a branch, tag, commit or HEAD). */
export function rawUrl({ owner, name, ref, path }) {
  return `https://raw.githubusercontent.com/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/${encodePath(ref)}/${encodePath(path)}`;
}

/** Fetches a catalog from a public repository. `fetchImpl` is injectable for tests. */
export async function fetchFromGithub({ owner, name, ref = 'HEAD', path = 'claims.json' }, fetchImpl = fetch) {
  const source = { kind: 'github', owner, name, ref, path };
  let response;
  try {
    response = await fetchImpl(rawUrl(source));
  } catch {
    throw new SourceError('Could not reach GitHub. Check your connection and try again.');
  }
  if (response.status === 404) {
    throw new SourceError(`${owner}/${name} has no ${path} at ${ref}, or the repository is private. The viewer can only open public repositories.`);
  }
  if (!response.ok) throw new SourceError(`GitHub returned an unexpected response (${response.status}).`);
  return { text: await response.text(), source };
}

/** Reads a local File (from a file input or a drop). */
export async function readFile(file) {
  try {
    return { text: await file.text(), source: { kind: 'file', name: file.name } };
  } catch {
    throw new SourceError(`${file.name} could not be read.`);
  }
}
```

- [ ] **Step 4: Run; expect PASS** (all model and source tests; the smoke test skipped).
- [ ] **Step 5: Commit** with message `feat(viewer): load catalogs from GitHub or a file`.

### Task 4: Page shell and styles

**Files:** Create `docs/site/src/viewer/index.html`, `docs/site/src/viewer/viewer.css`.

- [ ] **Step 1: Write `index.html`.** It contains:
  - The Pico link, pinned with SRI:
    `<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/@picocss/pico@2.1.1/css/pico.min.css" integrity="sha384-L1dWfspMTHU/ApYnFiMz2QID/PlP1xCW9visvBdbEkOLkSSWsP6ZJWhPw6apiXxU" crossorigin="anonymous">`.
  - Google Fonts: Space Grotesk (headings, 500/700), IBM Plex Sans (body, 400/600) and IBM Plex
    Mono (ids, locators, 400).
  - `viewer.css`, and `<script type="module" src="app.js">`.
  - An inline script in `<head>` that applies a saved theme before first paint, to avoid a flash.
    It reads `localStorage['claims-viewer.theme']` inside `try`/`catch` and sets `data-theme` on
    `<html>`.
  - Static landmarks with ids that `app.js` fills:
    - a `header`: the brand, `#source-label`, `#nav` (Overview and Claims links, hidden until a
      catalog loads), `#theme-toggle` and `#change-source`;
    - `main` containing:
      - `#start`: a GitHub form with `#repo-input`, a `<details>` "More options" holding
        `#ref-input` (placeholder `HEAD`) and `#path-input` (placeholder `claims.json`), and a
        submit button; a file `<input type=file accept=".json,application/json" id="file-input">`
        inside the `#drop-zone` label; `#start-error` (`role="alert"`); and a paragraph linking
        `?repo=hypotheosis/claims-mcp` as an example;
      - `#loading` (`aria-busy="true"`, hidden by default);
      - `#overview`, hidden by default;
      - `#claims` (hidden), holding `#filters`, `#claim-list` and `#claim-detail`.
    - a `footer` linking back to the documentation (`../introduction.html`).
- [ ] **Step 2: Write `viewer.css`.**
  - On `:root`, override the Pico variables `--pico-font-family-sans-serif`,
    `--pico-font-family-monospace`, `--pico-primary` (a deep teal) and `--pico-border-radius`.
  - Set headings to Space Grotesk.
  - Verdict tokens (`--v-supported` green, `--v-refuted` red, `--v-nei` amber, `--v-unverified`
    slate), redefined for dark mode both under
    `@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) {...} }` and under
    `[data-theme="dark"]`.
  - Classes: `.badge` plus a `.badge-<verdict>` per verdict, each with a text label; `.stance-*`;
    `.bar` (a stacked bar of flex `span`s); `.list-row` (clickable rows; `.retired` muted with
    reduced opacity); `.fix` (a highlighted callout); `.mono`; `.drop-zone` (dashed border, plus
    `.dragover`).
  - Layout: `#claims` is a grid `minmax(18rem, 26rem) 1fr` from 900px up; below that the list
    and detail stack, and `body.has-selection #claim-list` is hidden.
  - `max-width: 72rem` for main.
- [ ] **Step 3: Verify** `just docs` copies both files to `docs/site/book/viewer/`, then
  **commit** with message `feat(viewer): page shell and theme`.

### Task 5: `app.js`

**Files:** Create `docs/site/src/viewer/app.js`.

- [ ] **Step 1: Write `app.js`** with these units, each a small function:
  - `h(tag, attrs, ...children)`: creates an element. String children become text nodes, and
    `attrs.on*` attach listeners. It never sets `innerHTML`; that is the XSS boundary, because
    catalog text is untrusted.
  - State: `{ catalog, source, repo }`, where `repo` is `{owner, name}` for GitHub sources and
    `null` otherwise.
  - `load(promise)`: shows `#loading`, then on success parses with `parseCatalog`, stores the
    state, sets `#source-label` (`owner/name @ ref · path`, or the file name), shows `#nav` and
    renders. On `SourceError` or `CatalogError`, it shows the start view with the message in
    `#start-error`. Any other error is logged with `console.error` and shown as a generic message.
  - The start view handlers:
    - repo form submit: `parseRepoInput`, then `history.pushState` with
      `?repo=&ref=&path=` (only non-default values), then `load(fetchFromGithub(...))`;
    - file input change: `load(readFile(file))`;
    - drop-zone `dragover`, `dragleave` and `drop`, which toggle `.dragover` and load the first
      file;
    - `#change-source`, which clears the state and the query and hash, and shows `#start`.
  - On startup: read `?repo`, `?ref` and `?path` with `URLSearchParams`. If a repo parses, load
    it; otherwise show `#start`.
  - `render()`: `parseRoute(location.hash)`, then show one section and call `renderOverview`,
    `renderClaims(filter, null)` or `renderClaims(lastFilter, id)`. It runs on `hashchange`. For
    an unknown id, show "No claim CLM-… in this catalog" in the detail pane.
  - `renderOverview()`:
    - a totals `article` with the percentage supported, then `bar(counts)`, then four counts,
      each linking to `#claims?verdict=…`;
    - a "Needs attention" list from `needsAttention`, each row with id, statement, badge and
      `IL n`, linking `#CLM-…`;
    - a "By area" list from `areaSummary`, each row with `supported / total`, then
      `bar(counts)`, linking `#claims?area=…`.
  - `renderFilters(filter)`: a search input and `select`s for verdict (the four values), kind,
    level, integrity and area from `facetValues`, plus an "Include retired" checkbox. Each change
    calls `location.hash = formatRoute({view:'claims', filter: next})`, debounced to 200 ms for
    the text box. Keep focus in the search box across re-renders by updating rows only.
  - `renderList(claims, selectedId)`: rows linking `#CLM-…`. The selected row gets
    `aria-current`, retired rows get `.retired`, and a row count heading shows "N claims".
  - `renderDetail(claim)`:
    - **Header:** a breadcrumb of kind, level, `IL n` and method; the statement as `h2`; the
      badge. A retired banner with the reason and date.
    - **Facts list:** source, then `sourceAnchor` through `anchorNode`. Tags, with `issue:N`
      linked to `https://github.com/<owner>/<name>/issues/N` when `repo` is set. Artifacts in
      `.mono`.
    - **Hierarchy:** parents and children as links. A parent missing from the catalog shows as
      plain text with "(not in catalog)".
    - **Verdict history,** newest first: badge, auditor, date (`toLocaleString`, with the raw
      string if invalid), the rationale body, the `fix` in `.fix` labelled "Fix", and the cited
      evidence ids as `#EVD-…` links. Those scroll to the evidence `li`, whose id is set to the
      evidence id.
    - **Evidence:** each `li` has id, kind, a stance badge, the locator in `.mono`, the summary,
      the date and `anchorNode(evidence.anchor)`.
    - On narrow screens, a "Back to list" link to the claims route.
  - `anchorNode(anchor)`:
    - For a file anchor: a link from `fileAnchorUrl(repo, anchor)` when non-null, otherwise
      text, showing `path:lines @ commit[0..7]`.
    - For a search anchor: the `searchCommand` text in `<code>`, then `→ N hits @ commit`.
    - Either kind adds "(recorded on uncommitted changes)" when `dirty`.
  - `bar(counts)`: a `.bar` with one `span` per verdict, widths as percentages, and an
    `aria-label` listing the counts.
  - Theme toggle: cycles light and dark. It sets `document.documentElement.dataset.theme` and
    saves to `localStorage` in `try`/`catch`. The label shows the mode it switches to.

  Import from `./model.js` and `./source.js` only.
- [ ] **Step 2: Manual check.** Run `just docs-serve` and open
  `http://localhost:3000/viewer/?repo=hypotheosis/claims-mcp`. Check:
  - the overview totals match `claims-mcp tldr` (69 active claims);
  - clicking a needs-attention row opens the claim;
  - the back button returns;
  - filters update the hash;
  - a file anchor opens GitHub at the right lines;
  - the theme toggle survives a reload;
  - drag and drop of the local `claims.json` loads it;
  - the layout works at a 375px width.

  Also test a bad repo (`acme/does-not-exist`) and a non-catalog JSON file, which should show
  readable errors.
- [ ] **Step 3: Commit** with message `feat(viewer): render overview, list and claim detail`.

### Task 6: Docs page, README, justfile, CI

**Files:** Create `docs/site/src/viewer.md`. Modify `docs/site/src/SUMMARY.md`, `README.md`,
`justfile` and `.github/workflows/ci.yml`.

- [ ] **Step 1: Write `viewer.md`** ("Claims viewer"). It covers:
  - what the viewer shows: overview, list and claim detail;
  - how to open a public repository (`owner/name`, plus the optional ref and path) or a local
    file (picked or dropped; it stays on the machine);
  - the `?repo=&ref=&path=` link format;
  - that private repositories aren't supported, and why (no tokens on a shared `github.io`
    origin);
  - that it doesn't check seals or compute gaps, pointing to `claims-mcp verify` and
    [the first-audit guide](guides/first-audit.md);
  - links: [Open the viewer](viewer/index.html) and
    [claims-mcp's own ledger](viewer/index.html?repo=hypotheosis/claims-mcp).
- [ ] **Step 2: Add the page to SUMMARY.md** after the Guides part, as a part of its own:

```markdown
# Tools

- [Claims viewer](viewer.md)
```

- [ ] **Step 3: Update the README.** Add a line to the README's Documentation list:
  `- [Claims viewer](https://hypotheosis.github.io/claims-mcp/viewer/): browse any public repository's claims.json in the browser`.
- [ ] **Step 4: Add to `justfile`**

```just
# Run the claims viewer's unit tests, including the check against this repository's own catalog.
viewer-test:
    mkdir -p target
    cargo run --release --quiet -- --catalog claims.json tldr --json > target/tldr.json
    CLAIMS_TLDR_JSON=target/tldr.json node --test docs/site/viewer-tests/
```

- [ ] **Step 5: Add to `ci.yml`'s `check` job**, after the `verify catalog integrity seals` step:

```yaml
      # The docs site's claims viewer: unit tests, plus a check that it counts this
      # repository's active claims the same way claims-mcp does. Node is preinstalled.
      - name: claims viewer tests
        run: |
          cargo run --locked --quiet -- tldr --json > target/tldr.json
          CLAIMS_TLDR_JSON=target/tldr.json node --test docs/site/viewer-tests/
```

- [ ] **Step 6: Verify:**
  - `just viewer-test`: all pass, smoke test included, 0 skipped;
  - `just docs`: no warnings;
  - local link check: 0 broken;
  - validate `ci.yml` with PyYAML.

  Then **commit** with message `docs(viewer): docs page, README link, just recipe and CI tests`.

### Task 7: Changelog, checks, PR

- [ ] **Step 1: Write `changelog/<UTC timestamp>.md`** (timestamp from
  `date -u +%Y-%m-%dT%H-%M-%SZ`): what the viewer does, where it lives, how it is tested, and that
  it reads public repositories only.
- [ ] **Step 2: Run all checks:**
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `just test` (no Rust
    changed; run anyway);
  - `just viewer-test`;
  - `just docs` plus the link check;
  - `claims-mcp verify` and `recheck`. The site pages anchored by the docs claims aren't edited,
    except `SUMMARY.md`, which has no anchors, so all 20 anchors should stay current. If any
    moved, re-audit per the skill.
- [ ] **Step 3: Commit, push the branch, and open a PR** to `main`, with the attribution footer.
  Watch the checks. Merge only with a merge commit (squash and rebase are disabled).
