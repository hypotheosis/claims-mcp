#!/usr/bin/env bash
# Abandons a release `just release-prepare` made that has not been published: closes its pull
# request and deletes its `release/v*` branch if they exist, deletes the local tag, and resets
# local main to origin/main. `just release-abort` runs it.
#
# For when the release pull request's checks or review stopped `just release-ship`: land the
# fix on main through its own pull request, abort, then prepare and ship again.
set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPOSITORY_ROOT
cd "${REPOSITORY_ROOT}"
readonly RELEASE_STEP="release abort"
# shellcheck source=scripts/release-lib.sh
source scripts/release-lib.sh

require_clean_main abort

VERSION="$(package_version)"
readonly VERSION
readonly RELEASE_TAG="v${VERSION}"
readonly RELEASE_BRANCH="release/${RELEASE_TAG}"
grep -qxF -- "${RELEASE_TAG}" <<<"$(git tag --points-at HEAD)" \
    || fail "HEAD does not carry ${RELEASE_TAG}, the Cargo.toml version; nothing to abort"

# A pushed tag is a published release, and a merged pull request is one whose tag only needs
# pushing; neither is undone here.
! origin_has_ref "refs/tags/${RELEASE_TAG}" \
    || fail "release tag ${RELEASE_TAG} already exists on origin; it is published"
MERGED_PR_URL="$(
    gh pr list --head "${RELEASE_BRANCH}" --state merged --json url --jq '.[0].url // empty'
)"
readonly MERGED_PR_URL
[[ -z "${MERGED_PR_URL}" ]] \
    || fail "${MERGED_PR_URL} already merged ${RELEASE_TAG}; it cannot be aborted"

# The reset may drop only the release commit: anything else local is someone's work.
git fetch --quiet origin main
UNPUBLISHED_COMMITS="$(git rev-list --count refs/remotes/origin/main..HEAD)"
readonly UNPUBLISHED_COMMITS
((UNPUBLISHED_COMMITS <= 1)) \
    || fail "main has ${UNPUBLISHED_COMMITS} commits not on origin/main; \
only the release commit may be dropped"

OPEN_PR_URL="$(
    gh pr list --head "${RELEASE_BRANCH}" --state open --json url --jq '.[0].url // empty'
)"
readonly OPEN_PR_URL
if [[ -n "${OPEN_PR_URL}" ]]; then
    gh pr close "${OPEN_PR_URL}" --delete-branch \
        --comment "Release ${RELEASE_TAG} aborted; it will be prepared again from main."
elif origin_has_ref "refs/heads/${RELEASE_BRANCH}"; then
    git push --quiet origin --delete "refs/heads/${RELEASE_BRANCH}"
fi

git tag --delete "${RELEASE_TAG}" >/dev/null
git reset --quiet --hard refs/remotes/origin/main

printf 'aborted %s; main is at origin/main (%s)\n' "${RELEASE_TAG}" "$(git rev-parse --short HEAD)"
