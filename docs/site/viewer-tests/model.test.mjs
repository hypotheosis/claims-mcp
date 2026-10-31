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
