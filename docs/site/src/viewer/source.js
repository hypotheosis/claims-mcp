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
  const repo = `${encodeURIComponent(owner)}/${encodeURIComponent(name)}`;
  return `https://raw.githubusercontent.com/${repo}/${encodePath(ref)}/${encodePath(path)}`;
}

/** Fetches a catalog from a public repository. `fetchImpl` is injectable for tests. */
export async function fetchFromGithub(
  { owner, name, ref = 'HEAD', path = 'claims.json' },
  fetchImpl = fetch,
) {
  // Empty, `.` and `..` segments would let the URL resolve to another repository's file while
  // the page still names this one.
  for (const [label, value] of [['branch, tag or commit', ref], ['catalog path', path]]) {
    if (value.split('/').some((segment) => segment === '' || segment === '.' || segment === '..')) {
      throw new SourceError(`"${value}" isn't a valid ${label}.`);
    }
  }
  const source = { kind: 'github', owner, name, ref, path };
  let response;
  try {
    response = await fetchImpl(rawUrl(source));
  } catch {
    throw new SourceError('Could not reach GitHub. Check your connection and try again.');
  }
  if (response.status === 404) {
    throw new SourceError(
      `${owner}/${name} has no ${path} at ${ref}, or the repository is private. `
      + 'The viewer can only open public repositories.',
    );
  }
  if (!response.ok) {
    throw new SourceError(`GitHub returned an unexpected response (${response.status}).`);
  }
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
