// The claims viewer's page logic: loading, rendering and navigation. Catalog text is untrusted,
// so every element is built with DOM APIs and text nodes; nothing here sets innerHTML.

import {
  CatalogError, parseCatalog, countVerdicts, needsAttention, areaSummary, filterClaims,
  facetValues, parseRoute, formatRoute, fileAnchorUrl, searchCommand, splitFix,
} from './model.js';
import { SourceError, parseRepoInput, fetchFromGithub, readFile } from './source.js';

const VERDICT_LABELS = {
  supported: 'Supported',
  refuted: 'Refuted',
  not_enough_info: 'Not enough info',
  unverified: 'Unverified',
};
const VERDICT_ORDER = ['supported', 'refuted', 'not_enough_info', 'unverified'];
const SECTIONS = ['start', 'loading', 'overview', 'claims'];
const THEME_KEY = 'claims-viewer.theme';

// `loadedSearch` is the query string the current catalog came from ('' for a file), so that
// history navigation reloads only when the source changes, not on every hash change.
const state = { catalog: null, source: null, repo: null, filter: {}, loadedSearch: null };
const $ = (id) => document.getElementById(id);

/** Creates an element. String children become text nodes; `on*` attributes become listeners. */
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith('on')) el.addEventListener(key.slice(2), value);
    else if (key === 'class') el.className = value;
    else if (key === 'hidden' || key === 'selected' || key === 'checked') el[key] = Boolean(value);
    else el.setAttribute(key, value === true ? '' : String(value));
  }
  for (const child of children.flat(Infinity)) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return el;
}

const show = (id) => SECTIONS.forEach((name) => { $(name).hidden = name !== id; });

/** A row of small facts separated by middle dots; empty parts are skipped. */
function meta(...parts) {
  const kept = parts.flat().filter((part) => part !== null && part !== undefined && part !== '');
  return h('div', { class: 'detail-meta' },
    kept.flatMap((part, i) => [
      i ? h('span', { 'aria-hidden': 'true' }, '·') : null,
      part instanceof Node ? part : h('span', {}, part),
    ]));
}

function formatDate(value) {
  if (!value) return '';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

const badge = (verdict) =>
  h('span', { class: `badge badge-${verdict}` }, VERDICT_LABELS[verdict] ?? verdict);

const integrity = (claim) =>
  (claim.integrityLevel === null ? null : h('span', { class: 'il', title: 'Integrity level' }, `IL ${claim.integrityLevel}`));

function bar(counts) {
  const label = VERDICT_ORDER.map((v) => `${counts[v]} ${VERDICT_LABELS[v].toLowerCase()}`).join(', ');
  return h('div', { class: 'bar', role: 'img', 'aria-label': label },
    VERDICT_ORDER.map((v) => h('span', {
      class: v,
      style: `width: ${counts.total ? (100 * counts[v]) / counts.total : 0}%`,
    })));
}

const percent = (part, total) => (total ? Math.round((100 * part) / total) : 0);

/* ---------- Theme ---------- */

function currentTheme() {
  const set = document.documentElement.dataset.theme;
  if (set === 'light' || set === 'dark') return set;
  return matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function updateThemeButton() {
  $('theme-toggle').textContent = currentTheme() === 'dark' ? 'Light' : 'Dark';
}

function toggleTheme() {
  const next = currentTheme() === 'dark' ? 'light' : 'dark';
  document.documentElement.dataset.theme = next;
  try {
    localStorage.setItem(THEME_KEY, next);
  } catch {
    // Storage unavailable: the choice lasts for this page only.
  }
  updateThemeButton();
}

/* ---------- Loading ---------- */

function sourceLabel(source) {
  if (source.kind === 'file') return source.name;
  return `${source.owner}/${source.name} @ ${source.ref} · ${source.path}`;
}

function showStartError(message) {
  show('start');
  $('start-error').textContent = message;
  $('start-error').hidden = !message;
}

async function load(pending) {
  state.loadedSearch = location.search;
  show('loading');
  try {
    const { text, source } = await pending;
    state.catalog = parseCatalog(text);
    state.source = source;
    state.repo = source.kind === 'github' ? { owner: source.owner, name: source.name } : null;
    state.filter = {};
    $('source-label').textContent = sourceLabel(source);
    $('nav').hidden = false;
    $('change-source').hidden = false;
    $('start-error').hidden = true;
    buildFilters();
    render();
  } catch (error) {
    if (error instanceof SourceError || error instanceof CatalogError) {
      showStartError(error.message);
    } else {
      console.error('Loading the catalog failed', error);
      showStartError('Something went wrong while opening this catalog. Please try again.');
    }
  }
}

function loadFromQuery() {
  const params = new URLSearchParams(location.search);
  state.loadedSearch = location.search;
  const repo = parseRepoInput(params.get('repo') ?? '');
  if (!repo) {
    resetToStart();
    if (params.has('repo')) showStartError(`"${params.get('repo')}" isn't a GitHub repository name.`);
    return;
  }
  load(fetchFromGithub({
    ...repo,
    ref: params.get('ref') || undefined,
    path: params.get('path') || undefined,
  }));
}

function resetToStart() {
  state.catalog = null;
  state.loadedSearch = location.search;
  $('source-label').textContent = '';
  $('nav').hidden = true;
  $('change-source').hidden = true;
  document.body.classList.remove('has-selection');
  show('start');
}

function onRepoSubmit(event) {
  event.preventDefault();
  const repo = parseRepoInput($('repo-input').value);
  if (!repo) {
    showStartError('Enter a repository as owner/name or a https://github.com/owner/name link.');
    return;
  }
  const params = new URLSearchParams({ repo: `${repo.owner}/${repo.name}` });
  const ref = $('ref-input').value.trim();
  const path = $('path-input').value.trim();
  if (ref) params.set('ref', ref);
  if (path) params.set('path', path);
  history.pushState(null, '', `?${params}#overview`);
  load(fetchFromGithub({ ...repo, ref: ref || undefined, path: path || undefined }));
}

function openFile(file) {
  if (!file) return;
  history.pushState(null, '', `${location.pathname}#overview`);
  load(readFile(file));
}

function wireStart() {
  $('repo-form').addEventListener('submit', onRepoSubmit);
  $('file-input').addEventListener('change', (event) => openFile(event.target.files[0]));
  const zone = $('drop-zone');
  zone.addEventListener('dragover', (event) => {
    event.preventDefault();
    zone.classList.add('dragover');
  });
  zone.addEventListener('dragleave', () => zone.classList.remove('dragover'));
  zone.addEventListener('drop', (event) => {
    event.preventDefault();
    zone.classList.remove('dragover');
    openFile(event.dataTransfer.files[0]);
  });
  $('change-source').addEventListener('click', () => {
    history.pushState(null, '', location.pathname);
    resetToStart();
  });
}

/* ---------- Routing ---------- */

function render() {
  if (!state.catalog) return;
  const route = parseRoute(location.hash);
  for (const link of $('nav').querySelectorAll('a')) {
    const active = link.dataset.view === (route.view === 'overview' ? 'overview' : 'claims');
    if (active) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  }
  document.body.classList.toggle('has-selection', route.view === 'claim');
  if (route.view === 'overview') {
    show('overview');
    renderOverview();
    return;
  }
  if (route.view === 'claims') state.filter = route.filter;
  show('claims');
  syncFilters();
  renderList(route.view === 'claim' ? route.id : null);
  renderDetail(route.view === 'claim' ? route.id : null);
  if (route.view === 'claim') window.scrollTo({ top: 0 });
}

const go = (route) => { location.hash = formatRoute(route); };

/* ---------- Overview ---------- */

function renderOverview() {
  const claims = state.catalog.claims;
  const counts = countVerdicts(claims);
  const attention = needsAttention(claims);
  const areas = areaSummary(claims);

  const totals = h('article', {},
    h('p', { class: 'headline' }, `${percent(counts.supported, counts.total)}% supported`),
    h('p', { class: 'hint' }, `${counts.total} active claims`),
    bar(counts),
    h('div', { class: 'totals' }, VERDICT_ORDER.map((v) =>
      h('a', { class: 'total', href: formatRoute({ view: 'claims', filter: { verdict: v } }) },
        h('strong', {}, counts[v]), badge(v)))));

  const attentionCard = h('article', {},
    h('h2', {}, `Needs attention (${attention.length})`),
    attention.length
      ? h('ul', { class: 'rows' }, attention.map((c) => h('li', {}, claimRow(c, false))))
      : h('p', { class: 'hint' }, 'Every active claim is supported.'));

  const areaCard = h('article', {},
    h('h2', {}, 'By area'),
    h('ul', { class: 'rows' }, areas.map(({ area, counts: ac }) => h('li', {},
      h('a', { class: 'area-row', href: formatRoute({ view: 'claims', filter: { area } }) },
        h('div', { class: 'area-head' }, h('strong', {}, area), h('span', {}, `${ac.supported} of ${ac.total} supported`)),
        bar(ac))))));

  $('overview').replaceChildren(totals, h('div', { class: 'overview-grid' }, attentionCard, areaCard));
}

/* ---------- Claims list ---------- */

function claimRow(claim, selected) {
  return h('a', {
    class: `list-row${claim.retired ? ' retired' : ''}`,
    href: formatRoute({ view: 'claim', id: claim.id }),
    'aria-current': selected ? 'true' : null,
  },
  h('span', { class: 'id' }, claim.id, claim.retired ? ' · retired' : ''),
  h('span', { class: 'meta' }, integrity(claim), badge(claim.current)),
  h('span', { class: 'statement' }, claim.statement));
}

function select(name, label, options, labels = {}) {
  return h('select', { name, 'aria-label': label },
    h('option', { value: '' }, label),
    options.map((value) => h('option', { value: String(value) }, labels[value] ?? String(value))));
}

/** Builds the filter form once per catalog, so typing in the search box keeps its focus. */
function buildFilters() {
  const facets = facetValues(state.catalog.claims);
  const search = h('input', {
    type: 'search', name: 'text', placeholder: 'Search ids, statements, tags', 'aria-label': 'Search',
  });
  let timer;
  search.addEventListener('input', () => {
    clearTimeout(timer);
    timer = setTimeout(() => go({ view: 'claims', filter: { ...state.filter, text: search.value } }), 200);
  });
  const selects = [
    select('verdict', 'Any verdict', VERDICT_ORDER, VERDICT_LABELS),
    select('area', 'Any area', facets.areas),
    select('kind', 'Any kind', facets.kinds),
    select('level', 'Any level', facets.levels),
    select('integrity', 'Any integrity level', facets.integrityLevels,
      Object.fromEntries(facets.integrityLevels.map((n) => [n, `IL ${n}`]))),
  ];
  for (const el of selects) {
    el.addEventListener('change', () =>
      go({ view: 'claims', filter: { ...state.filter, [el.name]: el.value } }));
  }
  const retired = h('input', { type: 'checkbox', name: 'includeRetired', role: 'switch' });
  retired.addEventListener('change', () =>
    go({ view: 'claims', filter: { ...state.filter, includeRetired: retired.checked } }));
  $('filters').replaceChildren(search, ...selects,
    h('label', { class: 'checkbox' }, retired, ' Include retired claims'));
}

function syncFilters() {
  const form = $('filters');
  for (const el of form.elements) {
    if (el === document.activeElement) continue;
    if (el.type === 'checkbox') el.checked = Boolean(state.filter[el.name]);
    else el.value = state.filter[el.name] ?? '';
  }
}

function renderList(selectedId) {
  const claims = filterClaims(state.catalog.claims, state.filter);
  $('claim-list').replaceChildren(
    h('p', { class: 'count', style: 'padding: 0 0.75rem' }, `${claims.length} ${claims.length === 1 ? 'claim' : 'claims'}`),
    claims.length
      ? h('ul', { class: 'rows' }, claims.map((c) => h('li', {}, claimRow(c, c.id === selectedId))))
      : h('p', { class: 'hint', style: 'padding: 0 0.75rem' }, 'No claims match these filters.'));
  $('claim-list').querySelector('[aria-current="true"]')?.scrollIntoView({ block: 'nearest' });
}

/* ---------- Claim detail ---------- */

function anchorNode(anchor) {
  if (!anchor) return null;
  const dirty = anchor.dirty ? h('em', {}, ' (recorded on uncommitted changes)') : null;
  const commit = anchor.commit ? anchor.commit.slice(0, 7) : 'unpinned';
  if (anchor.type === 'search') {
    const hits = anchor.hits === null ? '' : ` → ${anchor.hits} ${anchor.hits === 1 ? 'hit' : 'hits'}`;
    return h('div', { class: 'anchor' }, h('code', {}, searchCommand(anchor)), `${hits} @ ${commit}`, dirty);
  }
  const { lines } = anchor;
  const range = lines ? `:${lines.start === lines.end ? lines.start : `${lines.start}-${lines.end}`}` : '';
  const label = `${anchor.path}${range} @ ${commit}`;
  const url = fileAnchorUrl(state.repo, anchor);
  return h('div', { class: 'anchor mono' },
    url ? h('a', { href: url, target: '_blank', rel: 'noopener' }, label) : label, dirty);
}

function claimLink(id) {
  return state.catalog.byId.has(id)
    ? h('a', { href: formatRoute({ view: 'claim', id }) }, id)
    : h('span', {}, id, h('em', { class: 'hint' }, ' (not in catalog)'));
}

function tagNode(tag) {
  const issue = tag.match(/^issue:(\d+)$/);
  if (issue && state.repo) {
    const url = `https://github.com/${encodeURIComponent(state.repo.owner)}/${encodeURIComponent(state.repo.name)}/issues/${issue[1]}`;
    return h('a', { class: 'chip', href: url, target: '_blank', rel: 'noopener' }, tag);
  }
  return h('span', { class: 'chip' }, tag);
}

function scrollToEvidence(event, id) {
  event.preventDefault();
  const item = $(`evidence-${id}`);
  if (!item) return;
  item.scrollIntoView({ behavior: 'smooth', block: 'center' });
  item.classList.remove('flash');
  void item.offsetWidth; // Restart the highlight animation when clicked again.
  item.classList.add('flash');
}

function fact(label, value) {
  if (value === null || value === '' || (Array.isArray(value) && !value.length)) return [];
  return [h('dt', {}, label), h('dd', {}, value)];
}

function verdictItem(claim, record) {
  const { body } = splitFix(record.rationale);
  return h('li', {},
    meta(badge(record.verdict), record.auditor, formatDate(record.recordedAt)),
    body ? h('p', {}, body) : null,
    record.fix ? h('div', { class: 'fix' }, h('strong', {}, 'Fix: '), record.fix) : null,
    record.evidenceIds.length
      ? h('small', {}, 'Cites ', record.evidenceIds.flatMap((id, i) => [
        i ? ', ' : '',
        h('a', { href: formatRoute({ view: 'claim', id: claim.id }), onclick: (e) => scrollToEvidence(e, id) }, id),
      ]))
      : null);
}

function evidenceItem(evidence) {
  return h('li', { id: `evidence-${evidence.id}` },
    meta(h('span', { class: 'mono' }, evidence.id), evidence.kind || 'evidence',
      h('strong', { class: `stance-${evidence.stance}` }, evidence.stance),
      formatDate(evidence.recordedAt)),
    evidence.locator ? h('p', { class: 'locator' }, evidence.locator) : null,
    evidence.summary ? h('p', {}, evidence.summary) : null,
    anchorNode(evidence.anchor));
}

function renderDetail(id) {
  const pane = $('claim-detail');
  if (!id) {
    pane.replaceChildren(h('p', { class: 'empty' }, 'Select a claim to see its verdicts and evidence.'));
    return;
  }
  const claim = state.catalog.byId.get(id);
  const back = h('a', { class: 'back-link', href: formatRoute({ view: 'claims', filter: state.filter }) }, '← Back to the list');
  if (!claim) {
    pane.replaceChildren(back, h('p', { class: 'empty' }, `No claim ${id} in this catalog.`));
    return;
  }
  const newestFirst = [...claim.verdicts].reverse();
  pane.replaceChildren(back, h('article', {},
    meta(h('span', { class: 'mono' }, claim.id), claim.kind, claim.level, claim.method,
      integrity(claim)),
    h('h2', {}, claim.statement),
    h('p', {}, badge(claim.current)),
    claim.retired
      ? h('div', { class: 'banner' }, h('strong', {}, 'Retired'),
        claim.retiredAt ? ` on ${formatDate(claim.retiredAt)}` : '',
        claim.retiredReason ? `: ${claim.retiredReason}` : '')
      : null,
    h('dl', { class: 'facts' },
      fact('Source', claim.source),
      fact('Source text', anchorNode(claim.sourceAnchor)),
      fact('Tags', claim.tags.length ? h('span', { class: 'chips' }, claim.tags.map(tagNode)) : null),
      fact('Artifacts', claim.artifacts.length ? h('span', { class: 'mono' }, claim.artifacts.join(', ')) : null),
      fact('Parents', claim.derivedFrom.length ? claim.derivedFrom.flatMap((p, i) => [i ? ', ' : '', claimLink(p)]) : null),
      fact('Children', claim.children.length ? claim.children.flatMap((c, i) => [i ? ', ' : '', claimLink(c)]) : null)),
    h('h3', {}, `Verdict history (${newestFirst.length})`),
    newestFirst.length
      ? h('ul', { class: 'history' }, newestFirst.map((record) => verdictItem(claim, record)))
      : h('p', { class: 'hint' }, 'No verdict recorded yet.'),
    h('h3', {}, `Evidence (${claim.evidence.length})`),
    claim.evidence.length
      ? h('ul', { class: 'evidence' }, claim.evidence.map(evidenceItem))
      : h('p', { class: 'hint' }, 'No evidence recorded yet.')));
}

/* ---------- Start-up ---------- */

wireStart();
$('theme-toggle').addEventListener('click', toggleTheme);
updateThemeButton();
window.addEventListener('hashchange', render);
// Browsers fire popstate on every fragment navigation as well as on back/forward, so only a
// changed query string (a different source) reloads; hashchange handles everything else.
window.addEventListener('popstate', () => {
  if (location.search !== state.loadedSearch) loadFromQuery();
});
loadFromQuery();
