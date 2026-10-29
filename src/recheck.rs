//! `recheck`: compares every anchor on an active claim with the repository's `HEAD`, so stale
//! line references and overturned absence searches are reported instead of trusted.

use std::collections::HashMap;

use serde::Serialize;

use crate::anchor::{Hunk, RepoProbe, Revision};
use crate::catalog::Catalog;
use crate::error::Result;
use crate::model::{
    line_count_text, short_commit, Anchor, ClaimId, EvidenceId, FileAnchor, LineRange,
    SearchAnchor, Stance,
};

/// How an anchor compares with `HEAD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorStatus {
    /// Nothing the anchor covers changed.
    Current,
    /// The anchored lines are unchanged but at different line numbers now.
    Moved,
    /// The anchored lines (or file) changed, or a search's hit count changed.
    Changed,
    /// An absence search now matches, or a search that matched now finds nothing.
    Overturned,
    /// The anchored file no longer exists at `HEAD`.
    Missing,
    /// The pinned commit is gone from the repository (history was rewritten).
    CommitMissing,
    /// The anchor was recorded with uncommitted changes, so its commit may not show what the
    /// auditor saw.
    Unpinned,
}

impl AnchorStatus {
    /// The status's snake_case name, as it appears in JSON.
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Moved => "moved",
            Self::Changed => "changed",
            Self::Overturned => "overturned",
            Self::Missing => "missing",
            Self::CommitMissing => "commit_missing",
            Self::Unpinned => "unpinned",
        }
    }
}

/// Where an anchor sits in the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorRole {
    /// A claim's `source_anchor`.
    Source,
    /// An evidence item's anchor.
    Evidence,
}

/// One anchor that is not [`AnchorStatus::Current`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecheckItem {
    /// Claim the anchor belongs to.
    pub claim: ClaimId,
    /// Source or evidence anchor.
    pub role: AnchorRole,
    /// Evidence id, for evidence anchors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceId>,
    /// Evidence stance, for evidence anchors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stance: Option<Stance>,
    /// The anchor as recorded, in short form.
    pub anchor: String,
    /// How it compares with `HEAD`.
    pub status: AnchorStatus,
    /// What changed, for a person to read.
    pub detail: String,
}

/// Result of `recheck`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecheckReport {
    /// Commit the anchors were compared with.
    pub head: String,
    /// Anchors checked (on active claims only).
    pub checked: usize,
    /// Anchors that are current.
    pub current: usize,
    /// Every other anchor, in catalog order.
    pub items: Vec<RecheckItem>,
}

impl RecheckReport {
    /// `true` when every anchor is current.
    pub fn all_current(&self) -> bool {
        self.items.is_empty()
    }
}

/// Rechecks every source and evidence anchor on active claims against `HEAD`.
///
/// # Errors
///
/// [`crate::error::ClaimsError::Git`] if the repository cannot be read.
pub fn recheck(catalog: &Catalog, probe: &impl RepoProbe) -> Result<RecheckReport> {
    let mut checker = Checker {
        probe,
        head: probe.head()?,
        head_lines: HashMap::new(),
        commits: HashMap::new(),
        hunks: HashMap::new(),
    };
    let mut checked = 0;
    let mut items = Vec::new();
    for claim in catalog.claims().iter().filter(|claim| claim.is_active()) {
        let source = claim
            .source_anchor
            .as_ref()
            .map(|anchor| (AnchorRole::Source, None, None, Anchor::File(anchor.clone())));
        let evidence = claim.evidence.iter().filter_map(|item| {
            let anchor = item.anchor.clone()?;
            Some((
                AnchorRole::Evidence,
                Some(item.id),
                Some(item.stance),
                anchor,
            ))
        });
        for (role, evidence, stance, anchor) in source.into_iter().chain(evidence) {
            checked += 1;
            let (status, detail) = checker.check(&anchor)?;
            if status != AnchorStatus::Current {
                items.push(RecheckItem {
                    claim: claim.id,
                    role,
                    evidence,
                    stance,
                    anchor: anchor.to_string(),
                    status,
                    detail,
                });
            }
        }
    }
    Ok(RecheckReport {
        current: checked - items.len(),
        head: checker.head,
        checked,
        items,
    })
}

/// Runs the checks, caching git answers that several anchors share.
struct Checker<'probe, P> {
    probe: &'probe P,
    head: String,
    /// Line count of each path at `HEAD` (`None`: not a file there).
    head_lines: HashMap<String, Option<u32>>,
    /// Whether each pinned commit still exists.
    commits: HashMap<String, bool>,
    /// Hunks from `(commit, path)` to `HEAD`.
    hunks: HashMap<(String, String), Vec<Hunk>>,
}

impl<P: RepoProbe> Checker<'_, P> {
    fn check(&mut self, anchor: &Anchor) -> Result<(AnchorStatus, String)> {
        if anchor.dirty() {
            return Ok((
                AnchorStatus::Unpinned,
                "recorded with uncommitted changes; re-record it on a clean tree".to_owned(),
            ));
        }
        match anchor {
            Anchor::File(file) => self.check_file(file),
            Anchor::Search(search) => self.check_search(search),
        }
    }

    fn check_file(&mut self, anchor: &FileAnchor) -> Result<(AnchorStatus, String)> {
        let head_lines = match self.head_lines.get(&anchor.path) {
            Some(lines) => *lines,
            None => {
                let lines = self
                    .probe
                    .line_count(Revision::Commit(&self.head), &anchor.path)?;
                self.head_lines.insert(anchor.path.clone(), lines);
                lines
            }
        };
        if head_lines.is_none() {
            return Ok((
                AnchorStatus::Missing,
                format!(
                    "{} does not exist at {}",
                    anchor.path,
                    short_commit(&self.head)
                ),
            ));
        }
        if anchor.commit == self.head {
            return Ok((AnchorStatus::Current, String::new()));
        }
        let exists = match self.commits.get(&anchor.commit) {
            Some(exists) => *exists,
            None => {
                let exists = self.probe.commit_exists(&anchor.commit)?;
                self.commits.insert(anchor.commit.clone(), exists);
                exists
            }
        };
        if !exists {
            return Ok((
                AnchorStatus::CommitMissing,
                format!(
                    "commit {} is no longer in the repository",
                    short_commit(&anchor.commit)
                ),
            ));
        }
        let key = (anchor.commit.clone(), anchor.path.clone());
        if !self.hunks.contains_key(&key) {
            let hunks = self.probe.hunks(&anchor.commit, &self.head, &anchor.path)?;
            self.hunks.insert(key.clone(), hunks);
        }
        let hunks = &self.hunks[&key];
        Ok(compare_lines(anchor, hunks))
    }

    fn check_search(&self, anchor: &SearchAnchor) -> Result<(AnchorStatus, String)> {
        let hits = self.probe.grep_count(
            &self.head,
            &anchor.pattern,
            anchor.case_insensitive,
            &anchor.scope,
        )?;
        let head = short_commit(&self.head);
        Ok(match (anchor.hits, hits) {
            (before, now) if before == now => (AnchorStatus::Current, String::new()),
            (0, now) => (
                AnchorStatus::Overturned,
                format!(
                    "the search found nothing when recorded but matches {} at {head}",
                    line_count_text(now)
                ),
            ),
            (before, 0) => (
                AnchorStatus::Overturned,
                format!(
                    "the search matched {} when recorded but none at {head}",
                    line_count_text(before)
                ),
            ),
            (before, now) => (
                AnchorStatus::Changed,
                format!(
                    "the search matched {} when recorded and {now} at {head}",
                    line_count_text(before)
                ),
            ),
        })
    }
}

/// Compares a file anchor with the hunks between its commit and `HEAD`.
fn compare_lines(anchor: &FileAnchor, hunks: &[Hunk]) -> (AnchorStatus, String) {
    let Some(range) = anchor.lines else {
        return if hunks.is_empty() {
            (AnchorStatus::Current, String::new())
        } else {
            (
                AnchorStatus::Changed,
                format!(
                    "{} changed in {} places since it was recorded",
                    anchor.path,
                    hunks.len()
                ),
            )
        };
    };
    if hunks.iter().any(|hunk| hunk.touches(range)) {
        return (
            AnchorStatus::Changed,
            format!(
                "lines {range} of {} changed since they were recorded",
                anchor.path
            ),
        );
    }
    // Hunks wholly before the range shift it by the lines they add minus the lines they remove.
    let shift: i64 = hunks
        .iter()
        .filter(|hunk| {
            if hunk.count == 0 {
                hunk.start < range.start
            } else {
                hunk.start + hunk.count <= range.start
            }
        })
        .map(|hunk| i64::from(hunk.new_count) - i64::from(hunk.count))
        .sum();
    if shift == 0 {
        return (AnchorStatus::Current, String::new());
    }
    let moved = |line: u32| u32::try_from(i64::from(line) + shift).unwrap_or(line);
    let now = LineRange {
        start: moved(range.start),
        end: moved(range.end),
    };
    (
        AnchorStatus::Moved,
        format!("unchanged, now at {}:{now}", anchor.path),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anchor::tests::{file_input, search_input, FakeRepo, HEAD};
    use crate::anchor::{pin, pin_file, NewAnchor};
    use crate::catalog::tests::{at, evidence, new_claim};

    const OLD: &str = "1111111111111111111111111111111111111111";

    fn file_anchor(lines: Option<(u32, u32)>) -> FileAnchor {
        FileAnchor {
            path: "src/a.rs".to_owned(),
            lines: lines.map(|(start, end)| LineRange { start, end }),
            commit: OLD.to_owned(),
            dirty: false,
        }
    }

    fn hunk(start: u32, count: u32, new_count: u32) -> Hunk {
        Hunk {
            start,
            count,
            new_count,
        }
    }

    #[test]
    fn line_anchors_are_current_moved_or_changed() {
        let anchor = file_anchor(Some((10, 12)));
        assert_eq!(compare_lines(&anchor, &[]).0, AnchorStatus::Current);
        assert_eq!(
            compare_lines(&anchor, &[hunk(30, 2, 0)]).0,
            AnchorStatus::Current
        );
        let (status, detail) = compare_lines(&anchor, &[hunk(2, 1, 4), hunk(5, 0, 1)]);
        assert_eq!(status, AnchorStatus::Moved);
        assert_eq!(detail, "unchanged, now at src/a.rs:14-16");
        assert_eq!(
            compare_lines(&anchor, &[hunk(11, 1, 1)]).0,
            AnchorStatus::Changed
        );
    }

    #[test]
    fn labels_match_the_json_names() {
        for status in [
            AnchorStatus::Current,
            AnchorStatus::Moved,
            AnchorStatus::Changed,
            AnchorStatus::Overturned,
            AnchorStatus::Missing,
            AnchorStatus::CommitMissing,
            AnchorStatus::Unpinned,
        ] {
            assert_eq!(
                serde_json::to_value(status).expect("serialize"),
                status.label()
            );
        }
    }

    #[test]
    fn whole_file_anchors_change_on_any_hunk() {
        let anchor = file_anchor(None);
        assert_eq!(compare_lines(&anchor, &[]).0, AnchorStatus::Current);
        assert_eq!(
            compare_lines(&anchor, &[hunk(1, 0, 1)]).0,
            AnchorStatus::Changed
        );
    }

    /// One claim with a source anchor and two evidence anchors, pinned against `repo`.
    fn anchored_catalog(repo: &FakeRepo) -> Catalog {
        let mut catalog = Catalog::default();
        let mut input = new_claim("the README promises rate limiting");
        input.source_anchor =
            Some(pin_file(repo, file_input("README.md", Some((4, 4)))).expect("pin"));
        let id = catalog.add_claim(input, at(0)).expect("claim").id;
        for (stance, anchor) in [
            (
                Stance::Refutes,
                NewAnchor::Search(search_input("rate.?limit", None)),
            ),
            (
                Stance::Neutral,
                NewAnchor::File(file_input("src/a.rs", Some((10, 12)))),
            ),
        ] {
            let mut item = evidence(stance);
            item.anchor = Some(pin(repo, anchor).expect("pin"));
            catalog.add_evidence(id, item, at(1)).expect("evidence");
        }
        catalog
    }

    #[test]
    fn recheck_reports_only_anchors_that_are_not_current() {
        let recorded = FakeRepo {
            head: Some(OLD.to_owned()),
            ..FakeRepo::default()
        }
        .with_file(OLD, "README.md", 10)
        .with_file(OLD, "src/a.rs", 40);
        let catalog = anchored_catalog(&recorded);

        // At HEAD, src/a.rs gained a line above the range, README.md is gone, and the absence
        // search now matches.
        let now = FakeRepo {
            grep_hits: 3,
            hunks: vec![hunk(1, 0, 1)],
            ..FakeRepo::default()
        }
        .with_file(HEAD, "src/a.rs", 41);
        let report = recheck(&catalog, &now).expect("recheck");

        assert_eq!(report.head, HEAD);
        assert_eq!((report.checked, report.current), (3, 0));
        let statuses: Vec<_> = report
            .items
            .iter()
            .map(|item| (item.role, item.stance, item.status))
            .collect();
        assert_eq!(
            statuses,
            vec![
                (AnchorRole::Source, None, AnchorStatus::Missing),
                (
                    AnchorRole::Evidence,
                    Some(Stance::Refutes),
                    AnchorStatus::Overturned
                ),
                (
                    AnchorRole::Evidence,
                    Some(Stance::Neutral),
                    AnchorStatus::Moved
                ),
            ]
        );
        assert!(report.items[1].detail.contains("matches 3 lines"));
    }

    #[test]
    fn recheck_flags_dirty_anchors_and_rewritten_history() {
        let dirty = FakeRepo {
            dirty: true,
            head: Some(OLD.to_owned()),
            ..FakeRepo::default()
        }
        .with_file("worktree", "README.md", 10)
        .with_file("worktree", "src/a.rs", 40);
        let catalog = anchored_catalog(&dirty);
        let now = FakeRepo {
            missing_commits: vec![OLD.to_owned()],
            ..FakeRepo::default()
        }
        .with_file(HEAD, "README.md", 10)
        .with_file(HEAD, "src/a.rs", 40);
        let report = recheck(&catalog, &now).expect("recheck");
        assert!(report
            .items
            .iter()
            .all(|item| item.status == AnchorStatus::Unpinned));

        let clean = FakeRepo {
            head: Some(OLD.to_owned()),
            ..FakeRepo::default()
        }
        .with_file(OLD, "README.md", 10)
        .with_file(OLD, "src/a.rs", 40);
        let report = recheck(&anchored_catalog(&clean), &now).expect("recheck");
        let statuses: Vec<_> = report.items.iter().map(|item| item.status).collect();
        assert_eq!(
            statuses,
            vec![AnchorStatus::CommitMissing, AnchorStatus::CommitMissing]
        );
        assert_eq!(report.current, 1, "the search still finds nothing");
    }

    #[test]
    fn recheck_skips_retired_claims() {
        let repo = FakeRepo::default()
            .with_file(HEAD, "README.md", 10)
            .with_file(HEAD, "src/a.rs", 40);
        let mut catalog = anchored_catalog(&repo);
        catalog
            .retire_claim(ClaimId::new(1), "withdrawn", at(2))
            .expect("retire");
        let report = recheck(&catalog, &repo).expect("recheck");
        assert_eq!(report.checked, 0);
        assert!(report.all_current());
    }
}
