#!/usr/bin/env bash
# cargo-release's pre-release hook (release.toml): writes the release's changelog entry, `#
# Release vVERSION` and the notes scripts/release-notes.sh prints for the previous release tag
# to HEAD, into changelog/ and stages it, so it lands in the release commit. release.yml
# publishes the same notes, for the previous tag to the release tag, as the GitHub release body.
#
# Usage: release-changelog.sh VERSION (cargo-release passes {{version}} and sets DRY_RUN).
set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPOSITORY_ROOT
cd "${REPOSITORY_ROOT}"
readonly RELEASE_STEP="release changelog"
# shellcheck source=scripts/release-lib.sh
source scripts/release-lib.sh

readonly VERSION="${1:?usage: release-changelog.sh VERSION}"
ENTRY="changelog/$(date -u +%Y-%m-%dT%H-%M-%SZ).md"
readonly ENTRY

# The notes are copied byte for byte, so the entry and the GitHub release body match exactly.
NOTES="$(mktemp)"
readonly NOTES
trap 'rm -f -- "${NOTES}"' EXIT
# A plain assignment, so a failure stops the hook (an argument's `$(...)` would not).
PREVIOUS_TAG="$(previous_release_tag HEAD)"
readonly PREVIOUS_TAG
scripts/release-notes.sh "${PREVIOUS_TAG}" HEAD >"${NOTES}"

# Without --execute, cargo-release only shows what it would do.
if [[ "${DRY_RUN:-true}" != "false" ]]; then
    printf 'would write %s:\n# Release v%s\n\n' "${ENTRY}" "${VERSION}" >&2
    cat -- "${NOTES}" >&2
    exit 0
fi

[[ ! -e "${ENTRY}" ]] || fail "${ENTRY} already exists"
{
    printf '# Release v%s\n\n' "${VERSION}"
    cat -- "${NOTES}"
} >"${ENTRY}"
git add "${ENTRY}"
