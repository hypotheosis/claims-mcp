#!/usr/bin/env bash
# Prepares a release locally with cargo-release (release.toml): one commit that bumps the
# version in Cargo.toml, Cargo.lock and report/audit-notes.toml and adds the generated changelog
# entry, plus an annotated `v*` tag on it. Nothing is pushed; `just release-ship` publishes it.
# `just release-prepare` runs it.
#
# Usage: release-prepare.sh LEVEL
#   LEVEL is patch, minor, major, or an exact MAJOR.MINOR.PATCH version.
set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPOSITORY_ROOT
cd "${REPOSITORY_ROOT}"
readonly RELEASE_STEP="release prepare"
# shellcheck source=scripts/release-lib.sh
source scripts/release-lib.sh

readonly LEVEL="${1:-}"
readonly LEVEL_PATTERN='^(patch|minor|major|(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*))$'
[[ $# -eq 1 && "${LEVEL}" =~ ${LEVEL_PATTERN} ]] \
    || fail "usage: release-prepare.sh patch|minor|major|X.Y.Z"

require_clean_main prepare

# A release prepared on a stale main would leave out what has landed since, or fail to merge.
git fetch --quiet origin main
[[ "$(git rev-parse HEAD)" == "$(git rev-parse refs/remotes/origin/main)" ]] \
    || fail "local main is not exactly origin/main; update main before preparing the release"

exec cargo release "${LEVEL}" --execute --no-confirm
