#!/usr/bin/env bash
# Fails unless BRANCH, a release pull request's head branch, is `release/v` plus the Cargo.toml
# version, so a release pull request whose version and name disagree fails before it merges
# (release.yml would refuse its tag after). CI runs it on pull requests from `release/v*`.
#
# Usage: release-branch-check.sh BRANCH
set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPOSITORY_ROOT
cd "${REPOSITORY_ROOT}"
readonly RELEASE_STEP="release branch check"
# shellcheck source=scripts/release-lib.sh
source scripts/release-lib.sh

readonly BRANCH="${1:?usage: release-branch-check.sh BRANCH}"
VERSION="$(package_version)"
readonly VERSION
[[ "${BRANCH}" == "release/v${VERSION}" ]] \
    || fail "branch ${BRANCH} does not match the Cargo.toml version ${VERSION}"
printf 'branch %s matches the Cargo.toml version\n' "${BRANCH}"
