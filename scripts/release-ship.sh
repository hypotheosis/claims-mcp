#!/usr/bin/env bash
# Publishes the release `just release-prepare` made: pushes its commit to `release/vX.Y.Z`,
# opens a pull request, waits for its checks and its review, merges it with a merge commit,
# moves the local tag to that merge commit and pushes the tag, which release.yml publishes
# from. `just release-ship` runs it.
#
# It stops, leaving the pull request open and nothing tagged, when a check fails or a review
# thread is unresolved; `just release-abort` then cleans up once the fix has landed on main.
#
# Environment (defaults suit GitHub; the tests shorten the waits):
#   RELEASE_REVIEWER       login whose review is awaited (default brooke-reviews, the review
#                          bot); empty to skip waiting and only check the threads.
#   RELEASE_POLL_SECONDS   pause between polls (default 30).
#   RELEASE_POLL_ATTEMPTS  polls before giving up on checks registering or the review arriving
#                          (default 60).
set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPOSITORY_ROOT
cd "${REPOSITORY_ROOT}"
readonly RELEASE_STEP="release ship"
# shellcheck source=scripts/release-lib.sh
source scripts/release-lib.sh

readonly REVIEWER="${RELEASE_REVIEWER-brooke-reviews}"
readonly POLL_SECONDS="${RELEASE_POLL_SECONDS:-30}"
readonly POLL_ATTEMPTS="${RELEASE_POLL_ATTEMPTS:-60}"
[[ "${REVIEWER}" =~ ^[A-Za-z0-9-]*(\[bot\])?$ ]] \
    || fail "RELEASE_REVIEWER '${REVIEWER}' is not a login"

require_clean_main ship

# The tag is picked by the version's name, not by whichever tag on HEAD sorts first, so a stray
# tag on the same commit is never shipped instead; release.yml refuses any other tag anyway.
VERSION="$(package_version)"
readonly VERSION
readonly RELEASE_TAG="v${VERSION}"
readonly RELEASE_BRANCH="release/${RELEASE_TAG}"
grep -qxF -- "${RELEASE_TAG}" <<<"$(git tag --points-at HEAD)" \
    || fail "HEAD does not carry ${RELEASE_TAG}, the Cargo.toml version; \
run 'just release-prepare' first"
! origin_has_ref "refs/tags/${RELEASE_TAG}" \
    || fail "release tag ${RELEASE_TAG} already exists on origin"

# The release commit must sit directly on the current origin/main. This catches a pull request
# merged after preparing, which the generated changelog entry never saw.
git fetch --quiet origin main
REMOTE_MAIN="$(git rev-parse refs/remotes/origin/main)"
readonly REMOTE_MAIN
! git rev-parse --verify --quiet 'HEAD^2' >/dev/null \
    || fail "HEAD must be the single-parent release commit, not a merge"
[[ "$(git rev-parse 'HEAD^')" == "${REMOTE_MAIN}" ]] \
    || fail "the release commit is not directly on current origin/main; abort and prepare again"

git push --quiet origin "HEAD:refs/heads/${RELEASE_BRANCH}"
PR_URL="$(
    gh pr create --base main --head "${RELEASE_BRANCH}" --title "chore: Release ${RELEASE_TAG}" \
        --body "Release commit prepared by \`just release-prepare\`. \`just release-ship\` \
merges it and pushes ${RELEASE_TAG} to the merge commit."
)"
readonly PR_URL
[[ "${PR_URL}" =~ ^https://[^/]+/([^/]+)/([^/]+)/pull/([0-9]+)$ ]] \
    || fail "gh pr create returned '${PR_URL}', not a pull request URL"
readonly PR_OWNER="${BASH_REMATCH[1]}" PR_REPOSITORY="${BASH_REMATCH[2]}"
readonly PR_NUMBER="${BASH_REMATCH[3]}"

count_checks() {
    gh pr checks "${PR_URL}" --json name --jq length
}

count_reviews() {
    gh pr view "${PR_URL}" --json reviews \
        --jq "[.reviews[] | select(.author.login == \"${REVIEWER}\")] | length"
}

# Polls the function COUNTER until it prints a non-zero count; fails naming WHAT otherwise.
wait_for_count() {
    local what="$1" counter="$2" count attempt
    for ((attempt = 0; attempt < POLL_ATTEMPTS; attempt++)); do
        count="$("${counter}" 2>/dev/null || true)"
        [[ "${count:-0}" == 0 ]] || return 0
        sleep "${POLL_SECONDS}"
    done
    fail "${what} did not arrive on ${PR_URL}; it is still open, and nothing was tagged"
}

# Checks take a moment to register on a new pull request; until then `gh pr checks` lists none.
wait_for_count "checks" count_checks
gh pr checks "${PR_URL}" --watch --fail-fast --interval "${POLL_SECONDS}" >/dev/null \
    || fail "checks failed on ${PR_URL}; it is still open, and nothing was tagged"

# main's ruleset requires review threads to be resolved, and the review bot reviews every pull
# request. A thread needs a person, so the release stops for one.
if [[ -n "${REVIEWER}" ]]; then
    wait_for_count "a review by ${REVIEWER}" count_reviews
fi
# The GraphQL variables are gh's, not the shell's, hence the single quotes.
# shellcheck disable=SC2016
UNRESOLVED_THREADS="$(
    gh api graphql -F owner="${PR_OWNER}" -F name="${PR_REPOSITORY}" -F number="${PR_NUMBER}" \
        -f query='query($owner: String!, $name: String!, $number: Int!) {
            repository(owner: $owner, name: $name) { pullRequest(number: $number) {
                reviewThreads(first: 100) { nodes { isResolved } } } } }' \
        --jq '[.data.repository.pullRequest.reviewThreads.nodes[]
            | select(.isResolved | not)] | length'
)" || fail "could not read the review threads of ${PR_URL}"
readonly UNRESOLVED_THREADS
[[ "${UNRESOLVED_THREADS}" == 0 ]] \
    || fail "${PR_URL} has ${UNRESOLVED_THREADS} unresolved review thread(s); \
it is still open, and nothing was tagged"

# main must not have moved since the release commit was checked against it: a change merged
# meanwhile would ship in a release whose changelog entry never saw it.
git fetch --quiet origin main
if [[ "$(git rev-parse refs/remotes/origin/main)" != "${REMOTE_MAIN}" ]]; then
    gh pr close "${PR_URL}" --delete-branch \
        --comment "main moved before this release merged; prepare ${RELEASE_TAG} again from main."
    fail "main moved while the release pull request waited; \
run 'just release-abort', then prepare again"
fi
# A merge commit, as every pull request here lands: claims.json anchors pin branch commits.
gh pr merge "${PR_URL}" --merge --delete-branch

# The tag goes on the merge commit, the commit on main's first-parent history. It is moved
# locally before anything is pushed, since a pushed release tag can never move.
MERGE_COMMIT="$(gh pr view "${PR_URL}" --json mergeCommit --jq '.mergeCommit.oid')"
readonly MERGE_COMMIT
[[ "${MERGE_COMMIT}" =~ ^[0-9a-f]{40}$ ]] || fail "${PR_URL} reports no merge commit"
git fetch --quiet origin main
git merge-base --is-ancestor HEAD "${MERGE_COMMIT}" \
    || fail "merge commit ${MERGE_COMMIT} does not contain the release commit"
TAG_MESSAGE="$(git for-each-ref --format='%(contents:subject)' "refs/tags/${RELEASE_TAG}")"
readonly TAG_MESSAGE
git tag --force --annotate "${RELEASE_TAG}" "${MERGE_COMMIT}" --message "${TAG_MESSAGE}" >/dev/null
git push --quiet origin "refs/tags/${RELEASE_TAG}"
git merge --quiet --ff-only refs/remotes/origin/main

printf 'released %s from %s; release.yml publishes it\n' "${RELEASE_TAG}" "${MERGE_COMMIT}"
