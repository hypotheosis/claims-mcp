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

test('routes accept any claim id, not only CLM-NNNN', () => {
  assert.deepEqual(parseRoute('#REQ-7'), { view: 'claim', id: 'REQ-7' });
  assert.deepEqual(parseRoute('#clm-0001'), { view: 'claim', id: 'clm-0001' });
  assert.deepEqual(parseRoute('#a%20b%3Fc'), { view: 'claim', id: 'a b?c' });
  assert.equal(formatRoute({ view: 'claim', id: 'a b?c' }), '#a%20b%3Fc');
  assert.deepEqual(parseRoute('#%E0%A4'), { view: 'overview' });
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
