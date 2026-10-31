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
  return {
    type: 'file', path: value.path, lines: parseLines(value.lines),
    commit: text(value.commit), dirty: value.dirty === true,
  };
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
  return tags
    .filter((tag) => tag.startsWith(prefix))
    .map((tag) => tag.slice(prefix.length))
    .filter(Boolean);
}

function parseClaim(value, index) {
  const where = `Claim ${index + 1}`;
  if (!isObject(value)) throw new CatalogError(`${where} is not an object.`);
  if (typeof value.id !== 'string' || !value.id) throw new CatalogError(`${where} has no id.`);
  if (typeof value.statement !== 'string') {
    throw new CatalogError(`${where} (${value.id}) has no statement.`);
  }
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
    retired: status.state === 'retired', retiredReason: text(status.reason),
    retiredAt: text(status.retired_at),
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
  const schemaVersion = Number.isInteger(data.schema_version) ? data.schema_version : null;
  return { schemaVersion, claims, byId };
}

export const countVerdicts = () => {}; export const needsAttention = () => {};
export const areaSummary = () => {}; export const filterClaims = () => {};
export const facetValues = () => {}; export const parseRoute = () => {};
export const formatRoute = () => {}; export const fileAnchorUrl = () => {};
export const searchCommand = () => {};
