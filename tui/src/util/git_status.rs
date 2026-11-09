//! Workspace git-status summary for the status bar (display only — every other git
//! interaction is out of scope).
//!
//! This shells out to the user's `git`; it is repo metadata, not a vault
//! file operation, so it lives in the TUI rather than core.

use std::path::PathBuf;

/// The footer's git summary: how many paths `git status` lists as changed.
/// Structured so consumers (TUI footer, GUI footer and its spoken form) word
/// it from the count, never by parsing the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitSummary {
    pub changed: usize,
}

impl GitSummary {
    /// The footer segment: `git ✓` (clean) or `git ●N`.
    pub fn label(&self) -> String {
        if self.changed == 0 {
            "git ✓".to_string()
        } else {
            format!("git ●{}", self.changed)
        }
    }

    /// The segment as a screen reader should say it.
    pub fn spoken(&self) -> String {
        match self.changed {
            0 => "Git: clean".to_string(),
            1 => "Git: 1 changed path".to_string(),
            n => format!("Git: {n} changed paths"),
        }
    }
}

impl From<&GitDetail> for GitSummary {
    fn from(detail: &GitDetail) -> Self {
        Self {
            changed: detail.changed(),
        }
    }
}

/// The footer summary, or `None` when `git` is missing or the workspace is
/// not a repository. One `git status` run, shared with [`detail`].
pub async fn fetch(root: PathBuf) -> Option<GitSummary> {
    detail(root).await.as_ref().map(GitSummary::from)
}

/// The workspace's git state in full: where HEAD is and what the changed
/// paths are, by kind. What the leader's `git status` toast words; the
/// status bar's one-segment summary is [`fetch`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GitDetail {
    /// The branch HEAD is on; `None` when detached.
    pub branch: Option<String>,
    /// Commits ahead of / behind the upstream, when there is one.
    pub ahead: usize,
    pub behind: usize,
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub untracked: usize,
    /// Paths with a merge conflict.
    pub conflicted: usize,
}

impl GitDetail {
    /// Parse `git status --porcelain=v1 --branch` output.
    pub fn parse(out: &str) -> Self {
        let mut detail = Self::default();
        for line in out.lines() {
            if let Some(head) = line.strip_prefix("## ") {
                detail.parse_branch(head);
                continue;
            }
            let mut chars = line.chars();
            let (Some(x), Some(y)) = (chars.next(), chars.next()) else {
                continue;
            };
            match (x, y) {
                ('?', '?') => detail.untracked += 1,
                ('!', '!') => {}
                ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => detail.conflicted += 1,
                // A copy is told with the renames. `AD` (staged new, gone from
                // the tree) is a new path first.
                _ if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') => detail.renamed += 1,
                _ if x == 'A' || y == 'A' => detail.added += 1,
                _ if x == 'D' || y == 'D' => detail.deleted += 1,
                _ => detail.modified += 1,
            }
        }
        detail
    }

    /// `main...origin/main [ahead 2, behind 1]`, `HEAD (no branch)`,
    /// `No commits yet on main`.
    fn parse_branch(&mut self, head: &str) {
        let (names, tracking) = match head.split_once(" [") {
            Some((names, rest)) => (names, rest.trim_end_matches(']')),
            None => (head, ""),
        };
        for part in tracking.split(", ") {
            if let Some(n) = part.strip_prefix("ahead ") {
                self.ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                self.behind = n.parse().unwrap_or(0);
            }
        }
        self.branch = if let Some(name) = names
            .strip_prefix("No commits yet on ")
            .or_else(|| names.strip_prefix("Initial commit on "))
        {
            Some(name.to_string())
        } else if names.starts_with("HEAD (no branch)") {
            None
        } else {
            Some(names.split("...").next().unwrap_or(names).to_string())
        };
    }

    /// Changed paths, all kinds.
    pub fn changed(&self) -> usize {
        self.modified + self.added + self.deleted + self.renamed + self.untracked + self.conflicted
    }

    /// One sentence for a toast: `On main, 2 ahead · 3 modified · 8 untracked`,
    /// or `On main · clean`.
    pub fn describe(&self) -> String {
        let mut parts = vec![match &self.branch {
            Some(name) => format!("On {name}"),
            None => "Detached HEAD".to_string(),
        }];
        if self.ahead > 0 {
            parts[0].push_str(&format!(", {} ahead", self.ahead));
        }
        if self.behind > 0 {
            parts[0].push_str(&format!(", {} behind", self.behind));
        }
        let kinds = [
            (self.conflicted, "conflicted"),
            (self.modified, "modified"),
            (self.added, "added"),
            (self.deleted, "deleted"),
            (self.renamed, "renamed"),
            (self.untracked, "untracked"),
        ];
        let changes: Vec<String> = kinds
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, what)| format!("{n} {what}"))
            .collect();
        if changes.is_empty() {
            parts.push("clean".to_string());
        } else {
            parts.extend(changes);
        }
        parts.join(" · ")
    }
}

/// The workspace's git state in full, or `None` when `git` is missing or the
/// workspace is not a repository.
pub async fn detail(root: PathBuf) -> Option<GitDetail> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["status", "--porcelain=v1", "--branch"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(GitDetail::parse(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_tree_on_a_branch() {
        let d = GitDetail::parse("## main...origin/main\n");
        assert_eq!(d.branch.as_deref(), Some("main"));
        assert_eq!(d.changed(), 0);
        assert_eq!(d.describe(), "On main · clean");
    }

    #[test]
    fn changes_are_told_by_kind() {
        let out = "## main...origin/main [ahead 2, behind 1]\n M a.md\nMM b.md\nA  c.md\n D d.md\nR  e.md -> f.md\n?? g.md\n?? h.md\nUU i.md\n";
        let d = GitDetail::parse(out);
        assert_eq!((d.ahead, d.behind), (2, 1));
        assert_eq!(
            (
                d.modified,
                d.added,
                d.deleted,
                d.renamed,
                d.untracked,
                d.conflicted
            ),
            (2, 1, 1, 1, 2, 1)
        );
        assert_eq!(d.changed(), 8);
        assert_eq!(
            d.describe(),
            "On main, 2 ahead, 1 behind · 1 conflicted · 2 modified · 1 added · 1 deleted · 1 renamed · 2 untracked"
        );
    }

    #[test]
    fn a_detached_head_and_a_fresh_repository() {
        let d = GitDetail::parse("## HEAD (no branch)\n?? a.md\n");
        assert_eq!(d.branch, None);
        assert_eq!(d.describe(), "Detached HEAD · 1 untracked");
        let d = GitDetail::parse("## No commits yet on trunk\n?? a.md\n");
        assert_eq!(d.branch.as_deref(), Some("trunk"));
        assert_eq!(d.describe(), "On trunk · 1 untracked");
    }

    #[test]
    fn mixed_states_and_older_git_headers() {
        let d = GitDetail::parse("## Initial commit on master\nAD a.md\nC  b.md -> c.md\n");
        assert_eq!(d.branch.as_deref(), Some("master"));
        assert_eq!((d.added, d.deleted, d.renamed), (1, 0, 1));
        let d = GitDetail::parse("## main...origin/main [gone]\n");
        assert_eq!(
            (d.branch.as_deref(), d.ahead, d.behind),
            (Some("main"), 0, 0)
        );
    }

    #[test]
    fn the_summary_is_worded_from_the_count() {
        assert_eq!(GitSummary { changed: 0 }.label(), "git ✓");
        assert_eq!(GitSummary { changed: 3 }.label(), "git ●3");
        assert_eq!(GitSummary { changed: 0 }.spoken(), "Git: clean");
        assert_eq!(GitSummary { changed: 1 }.spoken(), "Git: 1 changed path");
        assert_eq!(GitSummary { changed: 3 }.spoken(), "Git: 3 changed paths");
    }

    #[test]
    fn ignored_paths_do_not_count() {
        assert_eq!(GitDetail::parse("## main\n!! target/\n").changed(), 0);
    }
}
