import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SourceError, parseRepoInput, rawUrl, fetchFromGithub, readFile } from '../src/viewer/source.js';

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

const fakeFetch = (status, body = '') => async () =>
  ({ ok: status >= 200 && status < 300, status, text: async () => body });

test('fetchFromGithub returns the text and the source', async () => {
  const got = await fetchFromGithub({ owner: 'acme', name: 'widget' }, fakeFetch(200, '{"claims":[]}'));
  assert.equal(got.text, '{"claims":[]}');
  assert.deepEqual(got.source, { kind: 'github', owner: 'acme', name: 'widget', ref: 'HEAD', path: 'claims.json' });
});

test('fetchFromGithub explains failures', async () => {
  await assert.rejects(fetchFromGithub({ owner: 'a', name: 'b' }, fakeFetch(404)),
    (e) => e instanceof SourceError && /private/.test(e.message));
  await assert.rejects(fetchFromGithub({ owner: 'a', name: 'b' }, fakeFetch(500)), /500/);
  await assert.rejects(fetchFromGithub({ owner: 'a', name: 'b' }, async () => { throw new TypeError('x'); }),
    /reach GitHub/);
});

test('readFile returns the text and the file name', async () => {
  const got = await readFile(new File(['{"claims":[]}'], 'claims.json'));
  assert.deepEqual(got, { text: '{"claims":[]}', source: { kind: 'file', name: 'claims.json' } });
});

test('readFile explains an unreadable file', async () => {
  const broken = { name: 'x.json', text: async () => { throw new Error('gone'); } };
  await assert.rejects(readFile(broken), (e) => e instanceof SourceError && /x\.json/.test(e.message));
});

test('fetchFromGithub rejects ref and path segments that could leave the repository', async () => {
  const neverCalled = async () => { throw new Error('fetch must not be called'); };
  for (const bad of [{ ref: '../../c/d/main' }, { ref: 'a/./b' }, { ref: 'a//b' }, { path: '../x.json' }, { path: '/claims.json' }, { path: 'docs/' }]) {
    await assert.rejects(fetchFromGithub({ owner: 'a', name: 'b', ...bad }, neverCalled),
      (e) => e instanceof SourceError && /isn't a valid/.test(e.message), JSON.stringify(bad));
  }
  const ok = await fetchFromGithub({ owner: 'a', name: 'b', ref: 'release/1.2', path: 'audit/claims.json' }, fakeFetch(200, '{}'));
  assert.equal(ok.source.ref, 'release/1.2');
});
