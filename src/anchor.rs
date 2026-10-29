//! Commit-pinned anchors: tool input types, validation, pinning to `HEAD`, and the git probe
//! that pinning and `recheck` read the repository through.
//!
//! The catalog never runs git. The server pins an anchor (validates it, records `HEAD`, and
//! for a search counts the hits itself) and hands the catalog a finished [`Anchor`].

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::{ClaimsError, Result};
use crate::model::{line_count_text, short_commit, Anchor, FileAnchor, LineRange, SearchAnchor};
use crate::store::CatalogStore;

/// Tool input for a file anchor; the server adds the commit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct NewFileAnchor {
    /// Path relative to the repository root, e.g. `src/server.rs` or `README.md`.
    pub path: String,
    /// Inclusive 1-based line range; omit to anchor the whole file.
    #[serde(default)]
    pub lines: Option<LineRange>,
}

/// Tool input for a search anchor; the server runs the search and records the hits.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
pub struct NewSearchAnchor {
    /// POSIX extended regular expression (`git grep -E`). Make it broad enough to catch any
    /// reasonable implementation, e.g. `rate.?limit|throttl|429`.
    pub pattern: String,
    /// Ignore case (`git grep -i`).
    #[serde(default)]
    pub case_insensitive: bool,
    /// Paths to search, relative to the repository root; `.` searches everything.
    pub scope: Vec<String>,
    /// Matching lines you expect. Optional; if given, it must equal what the server counts.
    #[serde(default)]
    pub hits: Option<u32>,
}

/// Tool input for an evidence anchor.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NewAnchor {
    /// Code or text that shows the finding.
    File(NewFileAnchor),
    /// A search; zero hits demonstrates that something is absent.
    Search(NewSearchAnchor),
}

/// Where to read a file from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revision<'a> {
    /// The file as committed at this commit.
    Commit(&'a str),
    /// The file as it is on disk now.
    WorkTree,
}

/// One hunk of a diff, on the old side: `count` lines starting at `start`. A pure insertion
/// has `count == 0` and goes after line `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    /// First old line replaced, or the line the insertion follows.
    pub start: u32,
    /// Old lines replaced.
    pub count: u32,
    /// New lines in their place.
    pub new_count: u32,
}

impl Hunk {
    /// Returns `true` if the hunk changes, removes, or inserts lines inside `range`.
    pub fn touches(self, range: LineRange) -> bool {
        if self.count == 0 {
            // An insertion between two lines of the range splits it.
            range.start <= self.start && self.start < range.end
        } else {
            range.overlaps(LineRange {
                start: self.start,
                end: self.start + self.count - 1,
            })
        }
    }
}

/// Read access to the repository the catalog audits. [`GitCli`] is the real implementation;
/// tests use a fake.
pub trait RepoProbe {
    /// Full commit id of `HEAD`.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Git`] if git fails, e.g. on a repository with no commits.
    fn head(&self) -> Result<String>;

    /// Whether any of `paths` has uncommitted or untracked changes.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Git`] if git fails.
    fn is_dirty(&self, paths: &[String]) -> Result<bool>;

    /// Number of lines in `path` at `revision`, or `None` if it is not a file there.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Git`] or [`ClaimsError::Io`] if the file cannot be read.
    fn line_count(&self, revision: Revision<'_>, path: &str) -> Result<Option<u32>>;

    /// Whether `path` names a file or directory at `commit`.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Git`] if git cannot be run.
    fn path_exists(&self, commit: &str, path: &str) -> Result<bool>;

    /// Lines in `scope` at `commit` matching `pattern` (`git grep -c -E`).
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Git`] for an invalid pattern or a git failure.
    fn grep_count(
        &self,
        commit: &str,
        pattern: &str,
        case_insensitive: bool,
        scope: &[String],
    ) -> Result<u32>;

    /// Whether `commit` exists in the repository (history rewrites can remove it).
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Git`] if git cannot be run.
    fn commit_exists(&self, commit: &str) -> Result<bool>;

    /// Hunks that change `path` between two commits, in old-file order.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Git`] if git fails.
    fn hunks(&self, from: &str, to: &str, path: &str) -> Result<Vec<Hunk>>;
}

/// Validates `input` and pins it to `HEAD`. The file must exist and the lines must be inside
/// it: in the work tree if it has uncommitted changes (then `dirty` is set), otherwise at
/// `HEAD`.
///
/// # Errors
///
/// [`ClaimsError::Validation`] for a bad path or line range, or a missing file;
/// [`ClaimsError::Git`] if the repository cannot be read.
pub fn pin_file(probe: &impl RepoProbe, input: NewFileAnchor) -> Result<FileAnchor> {
    let path = clean_path("path", &input.path)?
        .ok_or_else(|| validation("path must name a file, not the repository root"))?;
    if let Some(lines) = input.lines {
        if lines.start == 0 || lines.end < lines.start {
            return Err(validation(format!(
                "lines must satisfy 1 <= start <= end, got {}..{}",
                lines.start, lines.end
            )));
        }
    }
    let commit = probe.head()?;
    let dirty = probe.is_dirty(std::slice::from_ref(&path))?;
    let revision = if dirty {
        Revision::WorkTree
    } else {
        Revision::Commit(&commit)
    };
    let line_count = probe
        .line_count(revision, &path)?
        .ok_or_else(|| validation(format!("{path} is not a file in the repository")))?;
    if let Some(lines) = input.lines {
        if lines.end > line_count {
            return Err(validation(format!(
                "lines {lines} are outside {path}, which has {line_count} lines"
            )));
        }
    }
    Ok(FileAnchor {
        path,
        lines: input.lines,
        commit,
        dirty,
    })
}

/// Validates `input`, runs the search at `HEAD`, and records the hits the server counted.
///
/// # Errors
///
/// [`ClaimsError::Validation`] for an empty pattern or scope, a bad scope path, or `hits`
/// that differs from the real count; [`ClaimsError::Git`] for an invalid pattern.
pub fn pin_search(probe: &impl RepoProbe, input: NewSearchAnchor) -> Result<SearchAnchor> {
    let pattern = input.pattern.trim();
    if pattern.is_empty() {
        return Err(validation("pattern must not be empty"));
    }
    if input.scope.is_empty() {
        return Err(validation(
            "scope must list at least one path (`.` for everything)",
        ));
    }
    let mut scope = input
        .scope
        .iter()
        .map(|entry| Ok(clean_path("scope", entry)?.unwrap_or_else(|| ".".to_owned())))
        .collect::<Result<Vec<_>>>()?;
    scope.sort();
    scope.dedup();
    let commit = probe.head()?;
    // A misspelled scope matches nothing, which would pass for a proven absence.
    for path in scope.iter().filter(|path| *path != ".") {
        if !probe.path_exists(&commit, path)? {
            return Err(validation(format!(
                "scope path {path} does not exist at {}",
                short_commit(&commit)
            )));
        }
    }
    let dirty = probe.is_dirty(&scope)?;
    let hits = probe.grep_count(&commit, pattern, input.case_insensitive, &scope)?;
    if let Some(expected) = input.hits {
        if expected != hits {
            return Err(validation(format!(
                "the search matches {} at {}, not {expected}",
                line_count_text(hits),
                short_commit(&commit)
            )));
        }
    }
    Ok(SearchAnchor {
        pattern: pattern.to_owned(),
        case_insensitive: input.case_insensitive,
        scope,
        hits,
        commit,
        dirty,
    })
}

/// Pins either kind of anchor; see [`pin_file`] and [`pin_search`].
///
/// # Errors
///
/// As for [`pin_file`] and [`pin_search`].
pub fn pin(probe: &impl RepoProbe, input: NewAnchor) -> Result<Anchor> {
    match input {
        NewAnchor::File(file) => pin_file(probe, file).map(Anchor::File),
        NewAnchor::Search(search) => pin_search(probe, search).map(Anchor::Search),
    }
}

/// Normalizes a repository-relative path to `a/b/c` form. `None` means the root (`.`).
/// Absolute paths and `..` are rejected so an anchor cannot point outside the repository.
fn clean_path(field: &str, raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(validation(format!("{field} must not be empty")));
    }
    let mut parts = Vec::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => parts
                .push(part.to_str().ok_or_else(|| {
                    validation(format!("{field} '{trimmed}' is not valid UTF-8"))
                })?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(validation(format!(
                    "{field} '{trimmed}' must be relative to the repository root, without '..'"
                )));
            }
        }
    }
    Ok((!parts.is_empty()).then(|| parts.join("/")))
}

fn validation(message: impl Into<String>) -> ClaimsError {
    ClaimsError::Validation(message.into())
}

/// [`RepoProbe`] backed by the `git` command line, rooted at a repository's top level.
#[derive(Debug, Clone)]
pub struct GitCli {
    root: PathBuf,
    /// Repository-relative paths of the catalog and its lock and temp files. Searches and the
    /// dirty check skip them: the catalog records every search pattern, so a committed
    /// catalog would match its own absence searches, and it changes on every write.
    excluded: Vec<String>,
}

impl GitCli {
    /// Finds the repository that contains the store's catalog.
    ///
    /// # Errors
    ///
    /// [`ClaimsError::Git`] if git is not installed or the catalog is not in a repository.
    pub fn for_store(store: &CatalogStore) -> Result<Self> {
        let command = "rev-parse --show-toplevel --show-prefix";
        let output = git_command(store.directory())
            .args(["rev-parse", "--show-toplevel", "--show-prefix"])
            .output()
            .map_err(|source| spawn_error(command, &source))?;
        let stdout = checked_stdout(command, &output)?;
        // Line 1 is the repository root; line 2 is the catalog directory relative to it,
        // with a trailing `/` (empty at the root).
        let mut lines = stdout.lines();
        let root = lines.next().unwrap_or_default();
        let prefix = lines.next().unwrap_or_default();
        let excluded = store
            .own_files()
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| format!("{prefix}{}", name.to_string_lossy()))
            .collect();
        Ok(Self {
            root: PathBuf::from(root),
            excluded,
        })
    }

    fn is_excluded(&self, path: &str) -> bool {
        self.excluded.iter().any(|excluded| excluded == path)
    }

    /// Runs `git <args>` in the repository root.
    fn run(&self, args: &[&str]) -> Result<Output> {
        git_command(&self.root)
            .args(args)
            .output()
            .map_err(|source| spawn_error(&args.join(" "), &source))
    }
}

/// `git -C <directory> --literal-pathspecs`: paths are taken literally, never as pathspec
/// magic such as `:(exclude)`.
fn git_command(directory: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(directory)
        .arg("--literal-pathspecs")
        // Rust note: `env` affects only the child process, not this one.
        .env("GIT_OPTIONAL_LOCKS", "0");
    command
}

fn spawn_error(command: &str, source: &std::io::Error) -> ClaimsError {
    ClaimsError::Git {
        command: command.to_owned(),
        message: format!("could not run git: {source}"),
    }
}

/// Stdout of a successful command as text, or its stderr as a [`ClaimsError::Git`].
fn checked_stdout(command: &str, output: &Output) -> Result<String> {
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(ClaimsError::Git {
            command: command.to_owned(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// `(path, count)` from `git grep -c <commit>` output, whose lines are `<commit>:<path>:<count>`.
/// A path may contain `:`, so the commit prefix is stripped and the count split off the end.
fn grep_counts<'out>(stdout: &'out str, commit: &str) -> impl Iterator<Item = (&'out str, u32)> {
    let prefix = format!("{commit}:");
    stdout.lines().filter_map(move |line| {
        let (path, count) = line.strip_prefix(&prefix)?.rsplit_once(':')?;
        Some((path, count.parse().ok()?))
    })
}

/// Paths from `git status --porcelain -z`: NUL-separated `XY <path>` entries, where a rename
/// or copy (`R`/`C`) is followed by one more entry holding the original path.
fn status_paths(stdout: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    let mut entries = stdout.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        let (Some(status), Some(path)) = (entry.get(..2), entry.get(3..)) else {
            continue;
        };
        paths.push(path);
        if status.contains(['R', 'C']) {
            // Rust note: `extend` with an `Option` adds its value if there is one.
            paths.extend(entries.next());
        }
    }
    paths
}

/// Lines in `bytes`; a final line without a newline still counts.
fn count_lines(bytes: &[u8]) -> u32 {
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    let unterminated = usize::from(bytes.last().is_some_and(|byte| *byte != b'\n'));
    u32::try_from(newlines + unterminated).unwrap_or(u32::MAX)
}

/// Parses the old-side ranges from `git diff --unified=0` output (`@@ -a,b +c,d @@`).
fn parse_hunks(diff: &str) -> Vec<Hunk> {
    diff.lines()
        .filter_map(|line| line.strip_prefix("@@ -"))
        .filter_map(|rest| {
            let (old, rest) = rest.split_once(" +")?;
            let new = rest.split_once(' ').map_or(rest, |(range, _)| range);
            let (start, count) = parse_range(old)?;
            let (_, new_count) = parse_range(new)?;
            Some(Hunk {
                start,
                count,
                new_count,
            })
        })
        .collect()
}

/// `a,b` or `a` (count 1) from a hunk header.
fn parse_range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

impl RepoProbe for GitCli {
    fn head(&self) -> Result<String> {
        let output = self.run(&["rev-parse", "--verify", "HEAD"])?;
        Ok(checked_stdout("rev-parse --verify HEAD", &output)?
            .trim()
            .to_owned())
    }

    fn is_dirty(&self, paths: &[String]) -> Result<bool> {
        let mut args = vec!["status", "--porcelain", "-z", "--untracked-files=all", "--"];
        args.extend(paths.iter().map(String::as_str));
        let output = self.run(&args)?;
        let stdout = checked_stdout("status --porcelain", &output)?;
        Ok(status_paths(&stdout)
            .into_iter()
            .any(|path| !self.is_excluded(path)))
    }

    fn line_count(&self, revision: Revision<'_>, path: &str) -> Result<Option<u32>> {
        match revision {
            Revision::Commit(commit) => {
                let object = format!("{commit}:{path}");
                // `cat-file blob` fails for a missing path and for a directory (a tree).
                let output = self.run(&["cat-file", "blob", &object])?;
                Ok(output.status.success().then(|| count_lines(&output.stdout)))
            }
            Revision::WorkTree => {
                let full = self.root.join(path);
                match fs::read(&full) {
                    Ok(bytes) => Ok(Some(count_lines(&bytes))),
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::NotFound | ErrorKind::IsADirectory
                        ) =>
                    {
                        Ok(None)
                    }
                    Err(source) => Err(ClaimsError::Io { path: full, source }),
                }
            }
        }
    }

    fn path_exists(&self, commit: &str, path: &str) -> Result<bool> {
        let object = format!("{commit}:{path}");
        Ok(self.run(&["cat-file", "-e", &object])?.status.success())
    }

    fn grep_count(
        &self,
        commit: &str,
        pattern: &str,
        case_insensitive: bool,
        scope: &[String],
    ) -> Result<u32> {
        let mut args = vec!["grep", "-c", "-E", "--no-color"];
        if case_insensitive {
            args.push("-i");
        }
        args.extend(["-e", pattern, commit, "--"]);
        args.extend(scope.iter().map(String::as_str));
        let output = self.run(&args)?;
        // `git grep` exits 1 when nothing matches (errors exit 128), so 1 is a result even if
        // git also printed a warning.
        if output.status.code() == Some(1) {
            return Ok(0);
        }
        let stdout = checked_stdout("grep", &output)?;
        Ok(grep_counts(&stdout, commit)
            .filter(|(path, _)| !self.is_excluded(path))
            .map(|(_, count)| count)
            .sum())
    }

    fn commit_exists(&self, commit: &str) -> Result<bool> {
        let object = format!("{commit}^{{commit}}");
        Ok(self.run(&["cat-file", "-e", &object])?.status.success())
    }

    fn hunks(&self, from: &str, to: &str, path: &str) -> Result<Vec<Hunk>> {
        let output = self.run(&[
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=0",
            from,
            to,
            "--",
            path,
        ])?;
        Ok(parse_hunks(&checked_stdout("diff", &output)?))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;

    use super::*;

    pub(crate) const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

    /// In-memory [`RepoProbe`]: files have a line count per revision, and the search returns
    /// a fixed count.
    #[derive(Debug, Default)]
    pub(crate) struct FakeRepo {
        pub(crate) dirty: bool,
        /// `(commit or "worktree", path) -> lines`.
        pub(crate) files: HashMap<(String, String), u32>,
        pub(crate) grep_hits: u32,
        pub(crate) missing_commits: Vec<String>,
        pub(crate) hunks: Vec<Hunk>,
        pub(crate) head: Option<String>,
        /// Paths `path_exists` denies; every other path exists.
        pub(crate) absent_paths: Vec<String>,
        /// How many times `grep_count` ran.
        ///
        /// Rust note: `Cell` allows mutation through the `&self` the trait methods take.
        pub(crate) grep_calls: std::cell::Cell<u32>,
    }

    impl FakeRepo {
        pub(crate) fn with_file(mut self, commit: &str, path: &str, lines: u32) -> Self {
            self.files
                .insert((commit.to_owned(), path.to_owned()), lines);
            self
        }
    }

    impl RepoProbe for FakeRepo {
        fn head(&self) -> Result<String> {
            Ok(self.head.clone().unwrap_or_else(|| HEAD.to_owned()))
        }

        fn is_dirty(&self, _paths: &[String]) -> Result<bool> {
            Ok(self.dirty)
        }

        fn line_count(&self, revision: Revision<'_>, path: &str) -> Result<Option<u32>> {
            let key = match revision {
                Revision::Commit(commit) => commit.to_owned(),
                Revision::WorkTree => "worktree".to_owned(),
            };
            Ok(self.files.get(&(key, path.to_owned())).copied())
        }

        fn path_exists(&self, _: &str, path: &str) -> Result<bool> {
            Ok(!self.absent_paths.iter().any(|absent| absent == path))
        }

        fn grep_count(&self, _: &str, _: &str, _: bool, _: &[String]) -> Result<u32> {
            self.grep_calls.set(self.grep_calls.get() + 1);
            Ok(self.grep_hits)
        }

        fn commit_exists(&self, commit: &str) -> Result<bool> {
            Ok(!self.missing_commits.iter().any(|missing| missing == commit))
        }

        fn hunks(&self, _: &str, _: &str, _: &str) -> Result<Vec<Hunk>> {
            Ok(self.hunks.clone())
        }
    }

    pub(crate) fn file_input(path: &str, lines: Option<(u32, u32)>) -> NewFileAnchor {
        NewFileAnchor {
            path: path.to_owned(),
            lines: lines.map(|(start, end)| LineRange { start, end }),
        }
    }

    pub(crate) fn search_input(pattern: &str, hits: Option<u32>) -> NewSearchAnchor {
        NewSearchAnchor {
            pattern: pattern.to_owned(),
            case_insensitive: true,
            scope: vec!["src".to_owned()],
            hits,
        }
    }

    #[test]
    fn pin_file_records_head_and_normalized_path() {
        let repo = FakeRepo::default().with_file(HEAD, "src/a.rs", 50);
        let anchor = pin_file(&repo, file_input("./src//a.rs", Some((10, 20)))).expect("pin");
        assert_eq!(anchor.path, "src/a.rs");
        assert_eq!(anchor.commit, HEAD);
        assert!(!anchor.dirty);
        assert_eq!(anchor.to_string(), "src/a.rs:10-20 @ 0123456789ab");
    }

    #[test]
    fn pin_file_reads_the_work_tree_when_dirty() {
        let repo = FakeRepo {
            dirty: true,
            ..FakeRepo::default()
        }
        .with_file("worktree", "new.md", 3);
        let anchor = pin_file(&repo, file_input("new.md", Some((3, 3)))).expect("pin");
        assert!(anchor.dirty);
    }

    #[test]
    fn pin_file_rejects_bad_paths_and_ranges() {
        let repo = FakeRepo::default().with_file(HEAD, "a.rs", 5);
        for (path, lines) in [
            ("../etc/passwd", None),
            ("/etc/passwd", None),
            ("  ", None),
            (".", None),
            ("missing.rs", None),
            ("a.rs", Some((0, 1))),
            ("a.rs", Some((3, 2))),
            ("a.rs", Some((4, 6))),
        ] {
            let error = pin_file(&repo, file_input(path, lines)).expect_err(path);
            assert!(
                matches!(error, ClaimsError::Validation(_)),
                "{path}: {error}"
            );
        }
    }

    #[test]
    fn pin_search_counts_hits_itself() {
        let repo = FakeRepo::default();
        let anchor = pin_search(&repo, search_input("rate.?limit", None)).expect("pin");
        assert_eq!(anchor.hits, 0);
        assert_eq!(anchor.scope, vec!["src"]);
        assert_eq!(
            anchor.to_string(),
            "git grep -iE 'rate.?limit' -- src @ 0123456789ab (0 hits)"
        );
    }

    #[test]
    fn pin_search_rejects_a_wrong_hit_count() {
        let repo = FakeRepo {
            grep_hits: 2,
            ..FakeRepo::default()
        };
        let error = pin_search(&repo, search_input("x", Some(0))).expect_err("mismatch");
        assert!(error.to_string().contains("matches 2 lines at"), "{error}");
        assert!(pin_search(&repo, search_input("x", Some(2))).is_ok());
    }

    #[test]
    fn pin_search_rejects_empty_pattern_and_scope() {
        let repo = FakeRepo::default();
        assert!(pin_search(&repo, search_input(" ", None)).is_err());
        let mut no_scope = search_input("x", None);
        no_scope.scope.clear();
        assert!(pin_search(&repo, no_scope).is_err());
        let mut root_scope = search_input("x", None);
        root_scope.scope = vec!["./".to_owned(), ".".to_owned()];
        assert_eq!(pin_search(&repo, root_scope).expect("pin").scope, vec!["."]);
    }

    #[test]
    fn pin_search_rejects_a_scope_that_does_not_exist() {
        let repo = FakeRepo {
            absent_paths: vec!["src".to_owned()],
            ..FakeRepo::default()
        };
        let error = pin_search(&repo, search_input("x", None)).expect_err("missing scope");
        assert!(
            error.to_string().contains("scope path src does not exist"),
            "{error}"
        );
    }

    #[test]
    fn hunks_touch_only_ranges_they_change() {
        let range = LineRange { start: 10, end: 20 };
        let replace = |start, count| Hunk {
            start,
            count,
            new_count: 1,
        };
        assert!(replace(20, 1).touches(range));
        assert!(replace(5, 6).touches(range));
        assert!(!replace(21, 3).touches(range));
        assert!(!replace(1, 9).touches(range));
        // Insertions after line 10 split the range; after line 20 or 9 they do not.
        assert!(replace(10, 0).touches(range));
        assert!(!replace(20, 0).touches(range));
        assert!(!replace(9, 0).touches(range));
    }

    #[test]
    fn parses_unified_zero_hunk_headers() {
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -3 +3 @@ fn a()\n-x\n+y\n\
                    @@ -10,0 +11,2 @@\n+a\n+b\n@@ -20,4 +22 @@\n";
        assert_eq!(
            parse_hunks(diff),
            vec![
                Hunk {
                    start: 3,
                    count: 1,
                    new_count: 1
                },
                Hunk {
                    start: 10,
                    count: 0,
                    new_count: 2
                },
                Hunk {
                    start: 20,
                    count: 4,
                    new_count: 1
                },
            ]
        );
    }

    #[test]
    fn parses_grep_counts_with_colons_in_paths() {
        let stdout = format!("{HEAD}:src/a.rs:3\n{HEAD}:docs/a:b.md:2\n");
        assert_eq!(
            grep_counts(&stdout, HEAD).collect::<Vec<_>>(),
            vec![("src/a.rs", 3), ("docs/a:b.md", 2)]
        );
    }

    #[test]
    fn parses_status_entries_including_renames() {
        let stdout = " M claims.json\0?? .claims.json.lock\0R  new.rs\0old.rs\0";
        assert_eq!(
            status_paths(stdout),
            vec!["claims.json", ".claims.json.lock", "new.rs", "old.rs"]
        );
    }

    #[test]
    fn counts_lines_with_and_without_a_final_newline() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
    }
}
