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
