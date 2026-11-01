# shellcheck shell=bash
# Sourced by the release scripts (and release.yml's publish step). The script that sources it
# sets RELEASE_STEP first ("release prepare", "release ship", ...), which `fail` names.

# The release tag pattern: `v` plus a MAJOR.MINOR.PATCH version, as release.toml's `tag-name`
# writes it and release.yml accepts it.
readonly RELEASE_TAG_GLOB='v[0-9]*.[0-9]*.[0-9]*'

fail() {
    printf '%s failed: %s\n' "${RELEASE_STEP:-release}" "$1" >&2
    exit 1
}

# Refuses unless the current branch is main and the working tree is clean, untracked files
# included. VERB names the step in the message ("prepare", "ship", "abort").
require_clean_main() {
    local branch
    branch="$(git rev-parse --abbrev-ref HEAD)"
    [[ "${branch}" == "main" ]] || fail "must $1 from main, not ${branch}"
    [[ -z "$(git status --porcelain)" ]] || fail "working tree is not clean"
}

# Prints `version` from Cargo.toml's [package] table: the first `version = "..."` line, which
# is the package's own since Cargo.toml opens with [package].
package_version() {
    local version
    version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
    [[ "${version}" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] \
        || fail "Cargo.toml version '${version}' is not a MAJOR.MINOR.PATCH version"
    printf '%s\n' "${version}"
}

# Whether origin has REF. `ls-remote --exit-code` exits 2 for a missing ref and 128 for an
# unreachable remote; only the first means "absent", so an error never passes for one.
origin_has_ref() {
    local status=0
    git ls-remote --exit-code origin "$1" >/dev/null 2>&1 || status=$?
    case "${status}" in
        0) return 0 ;;
        2) return 1 ;;
        *) fail "could not check origin for $1 (git ls-remote exited ${status})" ;;
    esac
}

# Prints the newest release tag reachable from COMMIT: the previous release, when COMMIT is the
# commit a new release is cut from (prepare passes HEAD) or the first parent of a release's
# merge commit (release.yml passes `<tag>^`).
previous_release_tag() {
    git describe --tags --abbrev=0 --match "${RELEASE_TAG_GLOB}" "$1" 2>/dev/null \
        || fail "no release tag is reachable from $1"
}
