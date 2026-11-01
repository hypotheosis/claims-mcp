#!/usr/bin/env bash
# Prints a release's notes as Markdown, built only from git history: one line per pull request
# merged into main between FROM and TO, grouped by the conventional-commit type of its title.
# `just release-prepare` writes them into the release's changelog entry (scripts/
# release-changelog.sh), and release.yml publishes them as the GitHub release body.
#
# Usage: release-notes.sh FROM-REF TO-REF
#
# The pull requests are the first-parent merge commits in FROM..TO. GitHub writes each as
# `Merge pull request #N from OWNER/BRANCH`, with the pull request's title as the body. The
# release's own pull request (title `chore: Release ...`, release.toml's
# `pre-release-commit-message`) is left out; change both together. A merge commit in another
# form goes under Other, as written, with no link.
set -Eeuo pipefail

REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPOSITORY_ROOT
readonly RELEASE_STEP="release notes"
# shellcheck source=scripts/release-lib.sh
source "${REPOSITORY_ROOT}/scripts/release-lib.sh"

# The pull request links. Fixed rather than read from the environment, so the notes prepare
# writes locally and the ones release.yml publishes are the same bytes.
readonly REPOSITORY_URL="https://github.com/hypotheosis/claims-mcp"

# The sections in order, as "bucket:heading". Only these buckets are ever filled.
readonly SECTIONS=(
    'breaking:Breaking changes'
    'feat:Features'
    'fix:Fixes'
    'docs:Documentation'
    'deps:Dependencies'
    'other:Other'
)
# Bucket -> its rendered lines.
declare -A ENTRIES=()

[[ $# -eq 2 ]] || fail "usage: release-notes.sh FROM-REF TO-REF"
readonly FROM_REF="$1" TO_REF="$2"
git rev-parse --quiet --verify "${FROM_REF}^{commit}" >/dev/null \
    || fail "${FROM_REF} is not a commit"
git rev-parse --quiet --verify "${TO_REF}^{commit}" >/dev/null || fail "${TO_REF} is not a commit"

# Files one pull request TITLE (number NUMBER, empty for a merge in another form) under its
# bucket. A named bucket drops the `type(scope)!: ` prefix but keeps the scope; Other keeps the
# title as written.
add_entry() {
    local title="$1" number="$2" bucket="other" line="$1"
    if [[ "${title}" =~ ^([a-z]+)(\(([^\)]*)\))?(!)?:[[:space:]]+(.+)$ ]]; then
        local type="${BASH_REMATCH[1]}" scope="${BASH_REMATCH[3]}"
        if [[ -n "${BASH_REMATCH[4]}" ]]; then
            bucket="breaking"
        elif [[ "${type}" == "build" && "${scope}" == "deps" ]]; then
            bucket="deps"
        elif [[ "${type}" == "feat" || "${type}" == "fix" || "${type}" == "docs" ]]; then
            bucket="${type}"
        fi
        if [[ "${bucket}" != "other" ]]; then
            line="${BASH_REMATCH[5]}"
            # Dependencies already names the scope.
            [[ -z "${scope}" || "${bucket}" == "deps" ]] || line="(${scope}) ${line}"
        fi
    fi
    [[ -z "${number}" ]] || line+=" ([#${number}](${REPOSITORY_URL}/pull/${number}))"
    ENTRIES["${bucket}"]+="- ${line}"$'\n'
}

# Records are separated by \x1e and their fields (subject, body) by \x1f, so a title may hold
# any printable text.
while IFS=$'\x1f' read -r -d $'\x1e' subject body; do
    # git log puts a newline between records; it belongs to no field.
    subject="${subject#$'\n'}"
    if [[ "${subject}" =~ ^Merge\ pull\ request\ \#([0-9]+)\ from\  ]]; then
        number="${BASH_REMATCH[1]}"
        # The title is the body's first line; GitHub leaves the body empty for an empty title.
        title="${body%%$'\n'*}"
        [[ -n "${title}" ]] || title="${subject}"
    else
        number=""
        title="${subject}"
    fi
    [[ "${title}" != "chore: Release "* ]] || continue
    add_entry "${title}" "${number}"
done < <(git log --first-parent --merges --reverse --format='%s%x1f%b%x1e' \
    "${FROM_REF}..${TO_REF}")

(( ${#ENTRIES[@]} > 0 )) || fail "no pull requests were merged between ${FROM_REF} and ${TO_REF}"

# Sections are separated by one blank line, and the notes end with exactly one newline.
separator=""
for section in "${SECTIONS[@]}"; do
    bucket="${section%%:*}"
    [[ -n "${ENTRIES["${bucket}"]:-}" ]] || continue
    printf '%s## %s\n\n%s' "${separator}" "${section#*:}" "${ENTRIES["${bucket}"]}"
    separator=$'\n'
done
