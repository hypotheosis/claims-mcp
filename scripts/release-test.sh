#!/usr/bin/env bash
# Tests the release scripts (release-notes, -prepare, -ship, -abort, -branch-check) against
# throwaway git repositories with local bare remotes, and a fake `gh` standing in for GitHub, so
# nothing leaves the machine. `just release-test` runs it, and so does CI.
#
# The end-to-end case runs the real cargo-release on a throwaway crate. Without cargo-release it
# is skipped, unless REQUIRE_CARGO_RELEASE=1 (set in CI), which makes its absence a failure.
set -Eeuo pipefail

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly SOURCE_ROOT
SCRATCH="$(mktemp -d)"
readonly SCRATCH
trap 'rm -rf -- "${SCRATCH}"' EXIT
readonly FAKE_BIN="${SCRATCH}/fake-bin"
readonly PR_URL="https://github.com/example/repo/pull/1"
# The tests only ever wait for polls that answer at once, or for ones that never do.
export RELEASE_POLL_SECONDS=0 RELEASE_POLL_ATTEMPTS=2
TESTS_RUN=0

fail() {
    printf 'release test failed: %s\n' "$1" >&2
    exit 1
}

pass() {
    TESTS_RUN=$((TESTS_RUN + 1))
    printf 'ok %s\n' "$1"
}

configure_identity() {
    git -C "$1" config user.name 'Release Test'
    git -C "$1" config user.email 'release-test@example.invalid'
}

# Writes a crate at VERSION into the current directory: enough for cargo-release.
write_crate() {
    mkdir -p src report changelog
    printf '[package]\nname = "release-test"\nversion = "%s"\nedition = "2021"\npublish = false\n' \
        "$1" >Cargo.toml
    printf 'artifact = "release-test"\nversion = "%s"\n' "$1" >report/audit-notes.toml
    printf '//! Throwaway crate.\n' >src/lib.rs
    printf '# First entry\n' >changelog/2000-01-01T00-00-00Z.md
    printf '/target\n' >.gitignore
}

# Merges a pull request into main the way GitHub writes it: a merge commit whose subject names
# the pull request and whose body is its title. Run inside a clone, on main.
merge_pull_request() {
    local number="$1" title="$2"
    git checkout --quiet -b "pr-${number}"
    git commit --quiet --allow-empty -m "work for #${number}"
    git checkout --quiet main
    git merge --quiet --no-ff "pr-${number}" \
        -m "Merge pull request #${number} from example/pr-${number}" -m "${title}"
    git branch --quiet -D "pr-${number}"
}

# Creates remote NAME.git and clone NAME holding this repository's release scripts and a crate
# released as v1.2.2 (tagged on a pull request's merge commit), plus one pull request merged
# since. Prints the clone's path.
create_repository() {
    local name="$1"
    local remote="${SCRATCH}/${name}.git" work="${SCRATCH}/${name}"
    git init --quiet --bare --initial-branch=main "${remote}"
    git clone --quiet "${remote}" "${work}" 2>/dev/null
    # An empty clone's branch follows the client's init.defaultBranch; these tests need main.
    git -C "${work}" symbolic-ref HEAD refs/heads/main
    configure_identity "${work}"
    (
        cd "${work}"
        mkdir scripts
        cp "${SOURCE_ROOT}"/scripts/release-{lib,notes,changelog,prepare,ship,abort}.sh \
            "${SOURCE_ROOT}/scripts/release-branch-check.sh" scripts/
        cp "${SOURCE_ROOT}/release.toml" .
        write_crate 1.2.2
        cargo generate-lockfile --quiet --offline
        git add .
        git commit --quiet -m initial
        merge_pull_request 1 'chore: Release v1.2.2'
        git tag -a v1.2.2 -m 'Release v1.2.2'
        merge_pull_request 2 'feat: add the thing'
        git push --quiet origin main refs/tags/v1.2.2
    )
    printf '%s\n' "${work}"
}

# Commits the version bump and tags it, as release-prepare's cargo-release run would.
prepare_release() {
    (
        cd "$1"
        sed -i 's/^version = "1.2.2"$/version = "1.2.3"/' Cargo.toml
        git commit --quiet -am 'chore: Release v1.2.3'
        git tag -a v1.2.3 -m 'Release v1.2.3'
    )
}

# Clones NAME's remote as a second checkout, standing in for GitHub's side or another person.
second_clone() {
    local clone="${SCRATCH}/$1-$2"
    git clone --quiet "${SCRATCH}/$1.git" "${clone}" 2>/dev/null
    configure_identity "${clone}"
    printf '%s\n' "${clone}"
}

# Lands a pull request on NAME's remote from another clone, as one merged meanwhile would.
land_upstream_change() {
    local clone
    clone="$(second_clone "$1" upstream)"
    (cd "${clone}" && merge_pull_request 9 'fix: landed meanwhile' && git push --quiet origin main)
}

# Runs SCRIPT [ARGS...] in clone WORK with the fake gh first on PATH, capturing its status and
# combined output in RUN_STATUS and RUN_OUTPUT.
run_script() {
    local work="$1"
    shift
    set +e
    RUN_OUTPUT="$(cd "${work}" && PATH="${FAKE_BIN}:${PATH}" "$@" 2>&1)"
    RUN_STATUS=$?
    set -e
}

assert_succeeded() {
    ((RUN_STATUS == 0)) || fail "$1 failed (${RUN_STATUS}): ${RUN_OUTPUT}"
}

assert_rejected() {
    ((RUN_STATUS != 0)) || fail "$1 was accepted: ${RUN_OUTPUT}"
    [[ "${RUN_OUTPUT}" == *"$2"* ]] || fail "$1: missing diagnostic '$2' in: ${RUN_OUTPUT}"
}

remote_has_ref() {
    git --git-dir="${SCRATCH}/$1.git" rev-parse --verify --quiet "$2" >/dev/null
}

# The fake gh. Every call is logged to TEST_GH_LOG when set. `pr merge` merges the release
# branch on the remote with --no-ff from TEST_INTEGRATION_CLONE, as GitHub's merge would, and
# `pr view --json mergeCommit` reports that merge.
create_fake_gh() {
    mkdir -p "${FAKE_BIN}"
    cat >"${FAKE_BIN}/gh" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
[[ -z "${TEST_GH_LOG:-}" ]] || printf '%s\n' "$*" >>"${TEST_GH_LOG}"
case "$1 $2" in
    'pr create')
        # TEST_UPSTREAM_CLONE: a pull request lands on main while the release one is open.
        if [[ -n "${TEST_UPSTREAM_CLONE:-}" ]]; then
            git -C "${TEST_UPSTREAM_CLONE}" commit --quiet --allow-empty -m 'landed meanwhile'
            git -C "${TEST_UPSTREAM_CLONE}" push --quiet origin main
        fi
        # TEST_PR_URL: what gh prints instead of the pull request's URL.
        printf '%s\n' "${TEST_PR_URL-https://github.com/example/repo/pull/1}"
        ;;
    'pr checks')
        # TEST_CHECK_COUNT checks registered (default 1); watching them exits with
        # TEST_CHECKS_STATUS (default 0, passed).
        if [[ " $* " == *' --json '* ]]; then
            printf '%s\n' "${TEST_CHECK_COUNT:-1}"
        else
            exit "${TEST_CHECKS_STATUS:-0}"
        fi
        ;;
    'pr view')
        case " $* " in
            *' reviews '*) printf '%s\n' "${TEST_REVIEW_COUNT:-1}" ;;
            # TEST_MERGE_COMMIT, when set (even empty), replaces the real merge commit.
            *' mergeCommit '*)
                if [[ -n "${TEST_MERGE_COMMIT+set}" ]]; then
                    printf '%s\n' "${TEST_MERGE_COMMIT}"
                else
                    git -C "${TEST_INTEGRATION_CLONE}" rev-parse HEAD
                fi
                ;;
            *) printf 'unexpected gh pr view: %s\n' "$*" >&2; exit 64 ;;
        esac
        ;;
    'api graphql')
        # TEST_GRAPHQL_STATUS: the exit status of the review threads query (default 0).
        printf '%s\n' "${TEST_UNRESOLVED_THREADS:-0}"
        exit "${TEST_GRAPHQL_STATUS:-0}"
        ;;
    'pr close')
        ;;
    'pr merge')
        clone="${TEST_INTEGRATION_CLONE}"
        git -C "${clone}" fetch --quiet origin
        git -C "${clone}" reset --quiet --hard origin/main
        git -C "${clone}" merge --quiet --no-ff "origin/${TEST_RELEASE_BRANCH}" \
            -m "Merge pull request #1 from example/${TEST_RELEASE_BRANCH}" \
            -m "chore: Release ${TEST_RELEASE_BRANCH#release/}"
        git -C "${clone}" push --quiet origin main
        ;;
    'pr list')
        # The release branch's pull request: open (TEST_OPEN_PR_URL) or merged
        # (TEST_MERGED_PR_URL); none by default.
        case " $* " in
            *' --state open '*) printf '%s' "${TEST_OPEN_PR_URL:-}" ;;
            *' --state merged '*) printf '%s' "${TEST_MERGED_PR_URL:-}" ;;
            *) printf 'unexpected gh pr list: %s\n' "$*" >&2; exit 64 ;;
        esac
        ;;
    *)
        printf 'unexpected gh invocation: %s\n' "$*" >&2
        exit 64
        ;;
esac
EOF
    chmod +x "${FAKE_BIN}/gh"
}

# ---- release-notes.sh -------------------------------------------------------------------

test_notes_grouping_and_range() {
    local work expected first_pull_request
    work="$(create_repository notes)"
    first_pull_request="$(git -C "${work}" rev-parse HEAD)"
    (
        cd "${work}"
        merge_pull_request 3 'fix(viewer): contain the filters'
        merge_pull_request 4 'docs: explain anchors'
        merge_pull_request 5 'build(deps): bump rmcp from 3.7.0 to 3.8.0'
        merge_pull_request 6 'feat(api)!: rename the tool'
        merge_pull_request 7 'Add a plain sentence title'
        merge_pull_request 8 'chore: tidy the justfile'
        # A merge inside a pull request's branch is not on main's first-parent history.
        git checkout --quiet -b pr-10
        git checkout --quiet -b side
        git commit --quiet --allow-empty -m 'side work'
        git checkout --quiet pr-10
        git merge --quiet --no-ff side -m 'Merge pull request #99 from example/side' \
            -m 'feat: hidden'
        git checkout --quiet main
        git merge --quiet --no-ff pr-10 -m 'Merge pull request #10 from example/pr-10' \
            -m 'fix: nested merge'
        git merge --quiet --no-ff "$(git commit-tree -p HEAD -m x 'HEAD^{tree}')" \
            -m "Merge branch 'hotfix'"
        merge_pull_request 11 'chore: Release v1.2.3'
    )
    expected="$(cat <<'EOF'
## Breaking changes

- (api) rename the tool ([#6](https://github.com/hypotheosis/claims-mcp/pull/6))

## Features

- add the thing ([#2](https://github.com/hypotheosis/claims-mcp/pull/2))

## Fixes

- (viewer) contain the filters ([#3](https://github.com/hypotheosis/claims-mcp/pull/3))
- nested merge ([#10](https://github.com/hypotheosis/claims-mcp/pull/10))

## Documentation

- explain anchors ([#4](https://github.com/hypotheosis/claims-mcp/pull/4))

## Dependencies

- bump rmcp from 3.7.0 to 3.8.0 ([#5](https://github.com/hypotheosis/claims-mcp/pull/5))

## Other

- Add a plain sentence title ([#7](https://github.com/hypotheosis/claims-mcp/pull/7))
- chore: tidy the justfile ([#8](https://github.com/hypotheosis/claims-mcp/pull/8))
- Merge branch 'hotfix'
EOF
)"
    run_script "${work}" scripts/release-notes.sh v1.2.2 HEAD
    assert_succeeded 'release notes'
    [[ "${RUN_OUTPUT}" == "${expected}" ]] || fail "unexpected notes:
${RUN_OUTPUT}
--- expected:
${expected}"
    # The bytes end in exactly one newline (the command substitution above strips them).
    (cd "${work}" && scripts/release-notes.sh v1.2.2 HEAD) >"${SCRATCH}/notes.md"
    [[ "$(tail -c 2 "${SCRATCH}/notes.md" | od -An -tx1)" =~ ^\ [0-9a-f]{2}\ 0a$ \
        && "$(tail -c 2 "${SCRATCH}/notes.md" | od -An -tx1)" != ' 0a 0a' ]] \
        || fail 'the notes do not end in exactly one newline'
    pass 'notes: grouped by type, first-parent merges only, release pull request left out'

    # Ranges: from the previous tag, not from the start of history.
    run_script "${work}" scripts/release-notes.sh v1.2.2 "${first_pull_request}"
    assert_succeeded 'a narrower range'
    [[ "${RUN_OUTPUT}" == '## Features'*'#2'* && "${RUN_OUTPUT}" != *'#3'* ]] \
        || fail "the range to pull request #2 printed: ${RUN_OUTPUT}"
    run_script "${work}" scripts/release-notes.sh \
        "$(git -C "${work}" rev-list --max-parents=0 HEAD)" v1.2.2
    assert_rejected 'a range holding only the release pull request' 'no pull requests were merged'
    pass 'notes: ranges'
}

test_notes_refusals() {
    local work
    work="$(create_repository notes-refusals)"
    run_script "${work}" scripts/release-notes.sh HEAD HEAD
    assert_rejected 'an empty range' 'no pull requests were merged between HEAD and HEAD'
    run_script "${work}" scripts/release-notes.sh v0.0.0 HEAD
    assert_rejected 'a missing ref' 'v0.0.0 is not a commit'
    run_script "${work}" scripts/release-notes.sh HEAD
    assert_rejected 'one argument' 'usage: release-notes.sh FROM-REF TO-REF'
    pass 'notes: refuses an empty range, a missing ref, and a wrong argument count'
}

# ---- release-prepare.sh -----------------------------------------------------------------

test_prepare_refusals() {
    local work
    work="$(create_repository prepare)"
    run_script "${work}" scripts/release-prepare.sh
    assert_rejected 'no level' 'usage: release-prepare.sh patch|minor|major|X.Y.Z'
    run_script "${work}" scripts/release-prepare.sh bogus
    assert_rejected 'a bad level' 'usage: release-prepare.sh'

    git -C "${work}" checkout --quiet -b topic
    run_script "${work}" scripts/release-prepare.sh patch
    assert_rejected 'a non-main branch' 'must prepare from main, not topic'
    git -C "${work}" checkout --quiet main

    printf 'in progress\n' >"${work}/notes.txt"
    run_script "${work}" scripts/release-prepare.sh patch
    assert_rejected 'a dirty tree' 'working tree is not clean'
    rm "${work}/notes.txt"

    git -C "${work}" commit --quiet --allow-empty -m 'local only'
    run_script "${work}" scripts/release-prepare.sh patch
    assert_rejected 'local main ahead of origin' 'local main is not exactly origin/main'
    git -C "${work}" reset --quiet --hard origin/main

    land_upstream_change prepare
    run_script "${work}" scripts/release-prepare.sh patch
    assert_rejected 'local main behind origin' 'local main is not exactly origin/main'
    pass 'prepare: refuses a bad level, off main, a dirty tree, and main not at origin/main'
}

# ---- release-prepare.sh and release-ship.sh end to end ----------------------------------

# The real cargo-release: one commit with every version and the changelog entry, an annotated
# tag, nothing pushed; then a ship whose tag lands on the merge commit, and whose GitHub notes
# match the changelog entry byte for byte.
test_prepare_and_ship_end_to_end() {
    if ! cargo release --version >/dev/null 2>&1; then
        [[ "${REQUIRE_CARGO_RELEASE:-0}" != 1 ]] || fail 'cargo-release is not installed'
        printf 'skip prepare and ship end to end: cargo-release is not installed\n'
        return 0
    fi
    local work integration entries entry remote_main_before
    work="$(create_repository e2e)"
    integration="$(second_clone e2e integration)"
    remote_main_before="$(git --git-dir="${SCRATCH}/e2e.git" rev-parse main)"

    CARGO_NET_OFFLINE=true run_script "${work}" scripts/release-prepare.sh patch
    assert_succeeded 'release-prepare patch'
    (
        cd "${work}"
        [[ "$(git log -1 --format=%s)" == 'chore: Release v1.2.3' ]] || fail 'no release commit'
        [[ "$(git rev-parse HEAD^)" == "${remote_main_before}" ]] \
            || fail 'prepare made more than one commit'
        [[ "$(git cat-file -t v1.2.3)" == tag ]] || fail 'v1.2.3 is not an annotated tag'
        [[ "$(git rev-parse 'v1.2.3^{commit}')" == "$(git rev-parse HEAD)" ]] \
            || fail 'v1.2.3 is not on the release commit'
        grep -qx 'version = "1.2.3"' Cargo.toml || fail 'Cargo.toml was not bumped'
        grep -qx 'version = "1.2.3"' report/audit-notes.toml \
            || fail 'audit-notes.toml was not bumped'
        grep -A1 -x 'name = "release-test"' Cargo.lock | grep -qx 'version = "1.2.3"' \
            || fail 'Cargo.lock was not bumped'
        [[ -z "$(git status --porcelain)" ]] || fail 'prepare left the tree dirty'
        git diff --name-only HEAD^ HEAD | grep -qx 'changelog/.*\.md' \
            || fail 'the changelog entry is not in the release commit'
    ) || exit 1
    [[ "$(git --git-dir="${SCRATCH}/e2e.git" rev-parse main)" == "${remote_main_before}" ]] \
        || fail 'prepare pushed main'
    ! remote_has_ref e2e refs/tags/v1.2.3 || fail 'prepare pushed the tag'
    entries="$(git -C "${work}" diff --name-only --diff-filter=A HEAD^ HEAD -- changelog)"
    entry="${work}/${entries}"
    [[ "$(sed -n 1p "${entry}")" == '# Release v1.2.3' && -z "$(sed -n 2p "${entry}")" ]] \
        || fail 'the entry does not open with its heading and a blank line'
    pass 'prepare: one release commit with every version and the changelog entry, a local tag'

    TEST_INTEGRATION_CLONE="${integration}" TEST_RELEASE_BRANCH='release/v1.2.3' \
        run_script "${work}" scripts/release-ship.sh
    assert_succeeded 'release-ship'
    git -C "${work}" fetch --quiet origin --tags
    (
        cd "${work}"
        tail -n +3 "${entry}" >"${SCRATCH}/entry-notes.md"
        scripts/release-notes.sh "$(git describe --tags --abbrev=0 --match 'v*' 'v1.2.3^')" v1.2.3 \
            >"${SCRATCH}/release-notes.md"
        cmp -s "${SCRATCH}/entry-notes.md" "${SCRATCH}/release-notes.md" \
            || fail "the GitHub notes differ from the changelog entry:
$(diff "${SCRATCH}/entry-notes.md" "${SCRATCH}/release-notes.md")"
    ) || exit 1
    pass 'ship: the release body matches the changelog entry byte for byte'
}

# ---- release-ship.sh --------------------------------------------------------------------

test_ship_refusals() {
    local work
    work="$(create_repository ship)"

    run_script "${work}" scripts/release-ship.sh
    assert_rejected 'an untagged HEAD' 'HEAD does not carry v1.2.2, the Cargo.toml version'
    git -C "${work}" tag -a v9.9.9 -m 'Release v9.9.9'
    run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a tag other than the version' 'HEAD does not carry v1.2.2'
    git -C "${work}" tag -d v9.9.9 >/dev/null

    prepare_release "${work}"
    git -C "${work}" checkout --quiet -b topic
    run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a non-main branch' 'must ship from main, not topic'
    git -C "${work}" checkout --quiet main
    printf 'in progress\n' >"${work}/notes.txt"
    run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a dirty tree' 'working tree is not clean'
    rm "${work}/notes.txt"

    git -C "${work}" remote set-url origin "${SCRATCH}/missing.git"
    run_script "${work}" scripts/release-ship.sh
    assert_rejected 'an unreachable origin' 'could not check origin for refs/tags/v1.2.3'
    git -C "${work}" remote set-url origin "${SCRATCH}/ship.git"
    pass 'ship: refuses off main, a dirty tree, HEAD without the version tag, unreachable origin'
}

test_ship_refuses_tag_on_origin() {
    local work
    work="$(create_repository ship-pushed)"
    prepare_release "${work}"
    git -C "${work}" push --quiet origin refs/tags/v1.2.3
    run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a pushed tag' 'release tag v1.2.3 already exists on origin'
    pass 'ship: refuses a tag already on origin'
}

test_ship_refuses_merge_head() {
    local work
    work="$(create_repository ship-merge-head)"
    (
        cd "${work}"
        sed -i 's/^version = "1.2.2"$/version = "1.2.3"/' Cargo.toml
        git commit --quiet -am 'chore: Release v1.2.3'
        git checkout --quiet -b side HEAD^
        git commit --quiet --allow-empty -m side
        git checkout --quiet main
        git merge --quiet --no-ff side -m 'merge side'
        git tag -a v1.2.3 -m 'Release v1.2.3'
    )
    run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a merge commit HEAD' 'HEAD must be the single-parent release commit'
    pass 'ship: refuses a merge commit as the release commit'
}

test_ship_refuses_stale_release() {
    local work
    work="$(create_repository ship-stale)"
    prepare_release "${work}"
    land_upstream_change ship-stale
    run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a release not on current origin/main' \
        'the release commit is not directly on current origin/main'
    ! remote_has_ref ship-stale refs/heads/release/v1.2.3 || fail 'the release branch was pushed'
    pass 'ship: refuses a release commit that is not directly on current origin/main'
}

# Runs a ship in a fresh repository NAME with the given fake-gh settings (as VAR=value words),
# expecting it to stop with DIAGNOSTIC, the pull request open, nothing merged or closed, and no
# tag on origin.
assert_ship_stops_open() {
    local name="$1" diagnostic="$2" work gh_log
    shift 2
    work="$(create_repository "${name}")"
    gh_log="${SCRATCH}/${name}-gh.log"
    prepare_release "${work}"
    run_script "${work}" env "$@" TEST_GH_LOG="${gh_log}" scripts/release-ship.sh
    assert_rejected "${name}" "${diagnostic}"
    [[ "${RUN_OUTPUT}" == *'it is still open, and no tag was pushed'* ]] \
        || fail "${name}: the diagnostic does not say the pull request is left open"
    ! grep -qE '^pr (merge|close)' "${gh_log}" \
        || fail "${name}: the release pull request was merged or closed: $(<"${gh_log}")"
    ! remote_has_ref "${name}" refs/tags/v1.2.3 || fail "${name}: a release was tagged"
    remote_has_ref "${name}" refs/heads/release/v1.2.3 || fail "${name}: no release branch"
}

test_ship_stops_on_failed_checks() {
    assert_ship_stops_open ship-red "checks failed on ${PR_URL}" TEST_CHECKS_STATUS=1
    grep -q -- '^pr checks .* --watch --fail-fast' "${SCRATCH}/ship-red-gh.log" \
        || fail 'the checks were not watched with --fail-fast'
    assert_ship_stops_open ship-no-checks "checks did not arrive on ${PR_URL}" TEST_CHECK_COUNT=0
    pass 'ship: failing or missing checks stop it with the pull request open'
}

test_ship_stops_on_review() {
    assert_ship_stops_open ship-threads "${PR_URL} has 2 unresolved review thread(s)" \
        TEST_UNRESOLVED_THREADS=2
    assert_ship_stops_open ship-no-review "a review by brooke-reviews did not arrive" \
        TEST_REVIEW_COUNT=0
    pass 'ship: unresolved review threads, or no review, stop it with the pull request open'
}

test_ship_closes_when_main_moves() {
    local work upstream gh_log
    work="$(create_repository ship-moved)"
    upstream="$(second_clone ship-moved upstream)"
    gh_log="${SCRATCH}/ship-moved-gh.log"
    prepare_release "${work}"
    TEST_UPSTREAM_CLONE="${upstream}" TEST_GH_LOG="${gh_log}" \
        run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a release whose main moved' 'main moved while the release pull request waited'
    grep -q "^pr close ${PR_URL} --delete-branch" "${gh_log}" \
        || fail "the release pull request was not closed: $(<"${gh_log}")"
    ! grep -q '^pr merge' "${gh_log}" || fail 'the release pull request was merged'
    ! remote_has_ref ship-moved refs/tags/v1.2.3 || fail 'a release was tagged after main moved'
    pass 'ship: main moving mid-ship closes the pull request and tags nothing'
}

# Ship's remaining guards: a bad RELEASE_REVIEWER, gh pr create printing no URL, and a failed
# review threads query stop before any merge; a missing merge commit, or one that does not
# contain the release commit, stop after the merge and before the tag moves or is pushed.
test_ship_guards() {
    local work integration unrelated
    work="$(create_repository ship-guards)"
    prepare_release "${work}"

    RELEASE_REVIEWER='not a login!' run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a bad reviewer login' "RELEASE_REVIEWER 'not a login!' is not a login"
    TEST_PR_URL='created, see the web UI' run_script "${work}" scripts/release-ship.sh
    assert_rejected 'no pull request URL' \
        "gh pr create returned 'created, see the web UI', not a pull request URL"
    TEST_GRAPHQL_STATUS=1 run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a failed review threads query' \
        "could not read the review threads of ${PR_URL}"
    ! remote_has_ref ship-guards refs/tags/v1.2.3 || fail 'a guard let the tag through'

    for case_name in ship-no-merge-commit ship-unrelated-merge-commit; do
        work="$(create_repository "${case_name}")"
        integration="$(second_clone "${case_name}" integration)"
        prepare_release "${work}"
        if [[ "${case_name}" == ship-no-merge-commit ]]; then
            TEST_MERGE_COMMIT='' TEST_INTEGRATION_CLONE="${integration}" \
                TEST_RELEASE_BRANCH='release/v1.2.3' run_script "${work}" scripts/release-ship.sh
            assert_rejected 'no merge commit' "${PR_URL} reports no merge commit"
        else
            unrelated="$(git -C "${work}" rev-parse 'v1.2.2^{commit}')"
            TEST_MERGE_COMMIT="${unrelated}" TEST_INTEGRATION_CLONE="${integration}" \
                TEST_RELEASE_BRANCH='release/v1.2.3' run_script "${work}" scripts/release-ship.sh
            assert_rejected 'an unrelated merge commit' \
                "merge commit ${unrelated} does not contain the release commit"
        fi
        ! remote_has_ref "${case_name}" refs/tags/v1.2.3 || fail "${case_name}: tag pushed"
        [[ "$(git -C "${work}" rev-parse 'v1.2.3^{commit}')" \
            == "$(git -C "${work}" rev-parse HEAD)" ]] || fail "${case_name}: the local tag moved"
    done
    pass 'ship: a bad reviewer login, no PR URL, a failed thread query, and a bad merge commit'
}

# A tag push refused after the merge (here by a pre-receive hook) names the commands that
# finish the release, and running them does.
test_ship_names_recovery_when_tag_push_fails() {
    local work integration hook command
    work="$(create_repository ship-tag-push)"
    integration="$(second_clone ship-tag-push integration)"
    hook="${SCRATCH}/ship-tag-push.git/hooks/pre-receive"
    prepare_release "${work}"
    printf '#!/usr/bin/env bash\nif grep -q " refs/tags/"; then exit 1; fi\n' >"${hook}"
    chmod +x "${hook}"
    TEST_INTEGRATION_CLONE="${integration}" TEST_RELEASE_BRANCH='release/v1.2.3' \
        run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a refused tag push' "${PR_URL} merged, but pushing v1.2.3 failed"
    command="${RUN_OUTPUT##*Finish the release with: }"
    [[ "${command}" == 'git push origin refs/tags/v1.2.3 && git merge --ff-only origin/main' ]] \
        || fail "unexpected recovery command: ${command}"
    rm "${hook}"
    (cd "${work}" && eval "${command}" >/dev/null 2>&1) || fail 'the recovery command failed'
    [[ "$(git --git-dir="${SCRATCH}/ship-tag-push.git" rev-parse 'v1.2.3^{commit}')" \
        == "$(git --git-dir="${SCRATCH}/ship-tag-push.git" rev-parse main)" ]] \
        || fail 'the recovery did not push the tag on the merge commit'
    [[ "$(git -C "${work}" rev-parse HEAD)" \
        == "$(git --git-dir="${SCRATCH}/ship-tag-push.git" rev-parse main)" ]] \
        || fail 'the recovery did not fast-forward local main'
    pass 'ship: a failed tag push after the merge names the commands that finish the release'
}

test_ship_tags_merge_commit() {
    local work integration gh_log tagged
    work="$(create_repository ship-ok)"
    integration="$(second_clone ship-ok integration)"
    gh_log="${SCRATCH}/ship-ok-gh.log"
    prepare_release "${work}"
    TEST_INTEGRATION_CLONE="${integration}" TEST_RELEASE_BRANCH='release/v1.2.3' \
        TEST_GH_LOG="${gh_log}" run_script "${work}" scripts/release-ship.sh
    assert_succeeded 'release-ship'
    grep -q "^pr create --base main --head release/v1.2.3 --title chore: Release v1.2.3" \
        "${gh_log}" || fail "no release pull request was opened: $(<"${gh_log}")"
    grep -q "^pr merge ${PR_URL} --merge --delete-branch" "${gh_log}" \
        || fail 'the release pull request was not merged with a merge commit'
    tagged="$(git --git-dir="${SCRATCH}/ship-ok.git" rev-parse 'v1.2.3^{commit}')"
    [[ "${tagged}" == "$(git --git-dir="${SCRATCH}/ship-ok.git" rev-parse main)" ]] \
        || fail 'the tag is not on the merge commit at the tip of main'
    git --git-dir="${SCRATCH}/ship-ok.git" rev-parse --verify --quiet "${tagged}^2" >/dev/null \
        || fail 'the tagged commit is not a merge commit'
    [[ "$(git --git-dir="${SCRATCH}/ship-ok.git" cat-file -t v1.2.3)" == tag ]] \
        || fail 'the pushed tag is not annotated'
    [[ "$(git --git-dir="${SCRATCH}/ship-ok.git" for-each-ref --format='%(contents:subject)' \
        refs/tags/v1.2.3)" == 'Release v1.2.3' ]] || fail 'the tag message was not kept'
    [[ "$(git -C "${work}" rev-parse HEAD)" == "${tagged}" ]] \
        || fail 'local main was not fast-forwarded to the merge commit'
    pass 'ship: merges with a merge commit, tags that commit, fast-forwards main'
}

# ---- release-branch-check.sh -----------------------------------------------------------

test_branch_check() {
    local work
    work="$(create_repository branch-check)"
    run_script "${work}" scripts/release-branch-check.sh release/v1.2.2
    assert_succeeded 'a matching release branch'
    run_script "${work}" scripts/release-branch-check.sh release/v1.2.3
    assert_rejected 'a mismatched release branch' \
        'branch release/v1.2.3 does not match the Cargo.toml version 1.2.2'
    pass 'branch check: a release branch must name the Cargo.toml version'
}

# ---- release-abort.sh -------------------------------------------------------------------

# Asserts an abort left nothing behind locally: no tag, and main exactly at origin/main.
assert_aborted() {
    ! git -C "$1" rev-parse --verify --quiet refs/tags/v1.2.3 >/dev/null \
        || fail 'the local release tag survived the abort'
    [[ "$(git -C "$1" rev-parse HEAD)" == "$(git -C "$1" rev-parse origin/main)" ]] \
        || fail 'local main was not reset to origin/main'
}

# Asserts a refused abort changed nothing: the local tag and HEAD are as they were.
assert_untouched() {
    git -C "$1" rev-parse --verify --quiet refs/tags/v1.2.3 >/dev/null \
        || fail 'a refused abort deleted the local release tag'
    [[ "$(git -C "$1" rev-parse HEAD)" == "$2" ]] || fail 'a refused abort moved HEAD'
}

test_abort_refusals() {
    local work head
    work="$(create_repository abort)"
    run_script "${work}" scripts/release-abort.sh
    assert_rejected 'an untagged HEAD' 'HEAD does not carry v1.2.2'

    prepare_release "${work}"
    head="$(git -C "${work}" rev-parse HEAD)"
    git -C "${work}" checkout --quiet -b topic
    run_script "${work}" scripts/release-abort.sh
    assert_rejected 'a non-main branch' 'must abort from main, not topic'
    git -C "${work}" checkout --quiet main
    printf 'in progress\n' >"${work}/notes.txt"
    run_script "${work}" scripts/release-abort.sh
    assert_rejected 'a dirty tree' 'working tree is not clean'
    [[ -f "${work}/notes.txt" ]] || fail 'a refused abort removed an untracked file'
    rm "${work}/notes.txt"
    assert_untouched "${work}" "${head}"

    TEST_MERGED_PR_URL="${PR_URL}" run_script "${work}" scripts/release-abort.sh
    assert_rejected 'a merged release pull request' "${PR_URL} already merged v1.2.3"
    assert_untouched "${work}" "${head}"

    git -C "${work}" push --quiet origin refs/tags/v1.2.3
    run_script "${work}" scripts/release-abort.sh
    assert_rejected 'a published tag' 'release tag v1.2.3 already exists on origin'
    assert_untouched "${work}" "${head}"
    pass 'abort: refuses off main, a dirty tree, an untagged HEAD, a pushed tag, a merged PR'
}

test_abort_refuses_other_local_commits() {
    local work head
    work="$(create_repository abort-local)"
    git -C "${work}" commit --quiet --allow-empty -m 'local work'
    prepare_release "${work}"
    head="$(git -C "${work}" rev-parse HEAD)"
    run_script "${work}" scripts/release-abort.sh
    assert_rejected 'unpublished local work' 'main has 2 commits not on origin/main'
    assert_untouched "${work}" "${head}"
    pass 'abort: refuses when main has commits besides the release commit'
}

test_abort_cleans_up() {
    local work gh_log
    work="$(create_repository abort-unshipped)"
    gh_log="${SCRATCH}/abort-unshipped-gh.log"
    prepare_release "${work}"
    TEST_GH_LOG="${gh_log}" run_script "${work}" scripts/release-abort.sh
    assert_succeeded 'abort of an unshipped release'
    assert_aborted "${work}"
    ! grep -q '^pr close' "${gh_log}" || fail 'a pull request was closed with none open'

    work="$(create_repository abort-open)"
    gh_log="${SCRATCH}/abort-open-gh.log"
    prepare_release "${work}"
    TEST_CHECKS_STATUS=1 run_script "${work}" scripts/release-ship.sh
    assert_rejected 'a release with failing checks' 'checks failed'
    TEST_OPEN_PR_URL="${PR_URL}" TEST_GH_LOG="${gh_log}" \
        run_script "${work}" scripts/release-abort.sh
    assert_succeeded 'abort of a stopped ship'
    grep -q "^pr close ${PR_URL} --delete-branch" "${gh_log}" \
        || fail 'the open release pull request was not closed'
    assert_aborted "${work}"

    work="$(create_repository abort-leftover)"
    prepare_release "${work}"
    git -C "${work}" push --quiet origin 'HEAD:refs/heads/release/v1.2.3'
    land_upstream_change abort-leftover
    run_script "${work}" scripts/release-abort.sh
    assert_succeeded 'abort with a leftover branch'
    ! remote_has_ref abort-leftover refs/heads/release/v1.2.3 || fail 'the branch was not deleted'
    assert_aborted "${work}"
    pass 'abort: drops the release, closes its open PR or deletes its branch, resets main'
}

main() {
    create_fake_gh
    test_notes_grouping_and_range
    test_notes_refusals
    test_prepare_refusals
    test_prepare_and_ship_end_to_end
    test_ship_refusals
    test_ship_refuses_tag_on_origin
    test_ship_refuses_merge_head
    test_ship_refuses_stale_release
    test_ship_stops_on_failed_checks
    test_ship_stops_on_review
    test_ship_closes_when_main_moves
    test_ship_guards
    test_ship_names_recovery_when_tag_push_fails
    test_ship_tags_merge_commit
    test_branch_check
    test_abort_refusals
    test_abort_refuses_other_local_commits
    test_abort_cleans_up
    printf 'release tests passed (%d)\n' "${TESTS_RUN}"
}

main "$@"
