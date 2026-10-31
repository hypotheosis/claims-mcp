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

export const ATTENTION = ['refuted', 'not_enough_info', 'unverified'];

const byIntegrityThenId = (a, b) =>
  (b.integrityLevel ?? 0) - (a.integrityLevel ?? 0) || a.id.localeCompare(b.id);
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
    .sort((a, b) =>
      ATTENTION.indexOf(a.current) - ATTENTION.indexOf(b.current) || byIntegrityThenId(a, b));
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
  return [...groups.keys()]
    .sort(sortAreas)
    .map((area) => ({ area, counts: countVerdicts(groups.get(area)) }));
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
  const distinct = (pick) =>
    [...new Set(claims.flatMap(pick).filter((v) => v !== '' && v !== null))];
  return {
    kinds: distinct((c) => [c.kind]).sort(),
    levels: distinct((c) => [c.level]).sort(),
    integrityLevels: distinct((c) => [c.integrityLevel]).sort((a, b) => b - a),
    areas: distinct((c) => c.areas).sort(sortAreas),
  };
}

const FILTER_KEYS = ['text', 'verdict', 'kind', 'level', 'integrity', 'area', 'includeRetired'];

/**
 * Reads the view and selection from a URL hash: `#overview` (or empty), `#claims?<filters>`, or
 * any other value as a URL-encoded claim id, so catalogs with ids other than `CLM-NNNN` work.
 */
export function parseRoute(hash) {
  const raw = hash.replace(/^#/, '');
  if (raw === '' || raw === 'overview') return { view: 'overview' };
  if (raw === 'claims' || raw.startsWith('claims?')) {
    const params = new URLSearchParams(raw.slice('claims'.length).replace(/^\?/, ''));
    const filter = {};
    for (const key of FILTER_KEYS) {
      const value = params.get(key);
      if (value) filter[key] = key === 'includeRetired' ? value === 'true' : value;
    }
    return { view: 'claims', filter };
  }
  try {
    return { view: 'claim', id: decodeURIComponent(raw) };
  } catch {
    return { view: 'overview' };
  }
}

/** Writes a route as a URL hash; empty filter values are dropped. */
export function formatRoute(route) {
  if (route.view === 'claim') return `#${encodeURIComponent(route.id)}`;
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
  if (!repo || !anchor || anchor.type !== 'file' || !/^[0-9a-f]{7,40}$/i.test(anchor.commit)) {
    return null;
  }
  const path = anchor.path.split('/').map(encodeURIComponent).join('/');
  const owner = encodeURIComponent(repo.owner);
  const name = encodeURIComponent(repo.name);
  const base = `https://github.com/${owner}/${name}/blob/${anchor.commit}/${path}`;
  if (!anchor.lines) return base;
  const { start, end } = anchor.lines;
  return start === end ? `${base}#L${start}` : `${base}#L${start}-L${end}`;
}

const shellQuote = (s) => `'${s.replace(/'/g, "'\\''")}'`;
const shellWord = (s) => (/^[A-Za-z0-9_./-]+$/.test(s) ? s : shellQuote(s));

/** The git grep command a search anchor recorded. */
export function searchCommand(anchor) {
  const flags = anchor.caseInsensitive ? '-iE' : '-E';
  return `git grep ${flags} ${shellQuote(anchor.pattern)} -- ${anchor.scope.map(shellWord).join(' ')}`;
}
