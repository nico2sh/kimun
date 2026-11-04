//! OUTLINE: the open note's headings as an indented tree, read from the
//! editor buffer (not the file), and the rule that maps a listed heading back
//! to a buffer row after edits.

use std::collections::HashMap;
use std::num::NonZeroU64;

use kimun_core::nfs::VaultPath;

use crate::logic::list::{ListRow, YankTarget};
use crate::logic::nav::HeadingTarget;

/// One OUTLINE row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutlineEntry {
    /// 1-based nesting depth: how many headings enclose this one, plus one.
    /// Not the markdown level — `## A` over `#### B` is depth 1 and 2.
    pub depth: usize,
    pub target: HeadingTarget,
}

impl OutlineEntry {
    /// The OUTLINE rows of a note body, in document order.
    pub fn all_of(text: &str) -> Vec<Self> {
        let headings = kimun_core::note::note_headings(text);
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for heading in &headings {
            *counts.entry(heading.text.as_str()).or_default() += 1;
        }
        // Depth counts the open ancestors with a lower level, as a breadcrumb
        // does.
        let mut open_levels: Vec<u8> = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::new();
        headings
            .iter()
            .map(|heading| {
                open_levels.retain(|lvl| *lvl < heading.level);
                open_levels.push(heading.level);
                let seen = seen.entry(heading.text.as_str()).or_default();
                let occurrence = *seen;
                *seen += 1;
                OutlineEntry {
                    depth: open_levels.len(),
                    target: HeadingTarget {
                        text: heading.text.clone(),
                        occurrence,
                        count: counts[heading.text.as_str()],
                        line: heading.line,
                    },
                }
            })
            .collect()
    }

    pub fn heading(&self) -> &str {
        &self.target.text
    }

    /// Everything the row shows or jumps by, but its line: a body edit that
    /// only moves headings keeps the list (and its filter) as it is.
    pub fn same_row(&self, other: &Self) -> bool {
        fn key(e: &OutlineEntry) -> (usize, &str, usize, usize) {
            (e.depth, &e.target.text, e.target.occurrence, e.target.count)
        }
        key(self) == key(other)
    }
}

impl ListRow for OutlineEntry {
    fn match_text(&self) -> Option<&str> {
        Some(&self.target.text)
    }

    fn yank_target(&self) -> Option<YankTarget> {
        // The heading text alone — the row carries no note path, and the depth
        // is presentation, not content.
        Some(YankTarget::new(self.target.text.clone(), "heading"))
    }
}

/// What an [`OutlineModel::sync`] means for the listed rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Changed {
    /// Same rows (the revision was unchanged, or the headings only moved):
    /// keep the list as it is — its filter and selection with it.
    Kept,
    /// The rows differ (another note, a heading added/removed/renamed, or the
    /// first sync): rebuild the list from [`OutlineModel::entries`].
    Rebuilt,
}

/// The OUTLINE state of one view: the note and buffer revision the headings
/// were read at, and the headings. Rows are listed by the UI; this decides
/// when they change and maps a listed row back to a buffer line.
pub struct OutlineModel {
    note: VaultPath,
    /// Buffer revision `entries` was computed at; `None` = unknown, re-parse.
    revision: Option<NonZeroU64>,
    /// Kept current even when the list is not rebuilt: `resolve` reads lines
    /// from here, the listed rows' lines being hints only. `pub(crate)` for
    /// the TUI panel's tests, which index it directly.
    pub(crate) entries: Vec<OutlineEntry>,
    /// Whether a list was ever handed out (a [`Changed::Rebuilt`]).
    listed: bool,
}

impl Default for OutlineModel {
    fn default() -> Self {
        Self::new()
    }
}

impl OutlineModel {
    pub fn new() -> Self {
        Self {
            note: VaultPath::empty(),
            revision: None,
            entries: Vec::new(),
            listed: false,
        }
    }

    /// Read `note`'s headings from its buffer `text` (asked for only when
    /// needed) at `revision` (`None` when unknown, e.g. a buffer just
    /// replaced). Cheap to call often: an unchanged revision of the same note
    /// skips the parse, and unchanged headings answer [`Changed::Kept`].
    pub fn sync(
        &mut self,
        note: &VaultPath,
        revision: Option<NonZeroU64>,
        text: impl FnOnce() -> String,
    ) -> Changed {
        let same_note = *note == self.note && self.listed;
        if same_note && revision.is_some() && revision == self.revision {
            return Changed::Kept;
        }
        let entries = OutlineEntry::all_of(&text());
        self.revision = revision;
        let unchanged = same_note
            && entries.len() == self.entries.len()
            && entries
                .iter()
                .zip(&self.entries)
                .all(|(a, b)| a.same_row(b));
        self.entries = entries;
        if unchanged {
            return Changed::Kept;
        }
        self.note = note.clone();
        self.listed = true;
        Changed::Rebuilt
    }

    /// The headings as last synced, in document order.
    pub fn entries(&self) -> &[OutlineEntry] {
        &self.entries
    }

    /// The note the listed rows belong to (empty before the first sync).
    pub fn note(&self) -> &VaultPath {
        &self.note
    }

    /// The buffer revision the headings were read at (`None` = unknown).
    pub fn revision(&self) -> Option<NonZeroU64> {
        self.revision
    }

    /// The buffer row `target` stands for, against the headings as last
    /// synced — so sync first. The same `occurrence`-th heading when as many
    /// headings read `target.text` as when the row was listed (lines may have
    /// moved; occurrence order has not); otherwise one was added or removed,
    /// and the one nearest `target.line` (the earlier on a tie). `None` when
    /// no heading reads that text any more.
    pub fn resolve(&self, target: &HeadingTarget) -> Option<usize> {
        let lines: Vec<usize> = self
            .entries
            .iter()
            .filter(|e| e.target.text == target.text)
            .map(|e| e.target.line)
            .collect();
        if lines.len() == target.count {
            return lines.get(target.occurrence).copied();
        }
        lines
            .into_iter()
            .min_by_key(|line| (line.abs_diff(target.line), *line))
    }
}
