//! The **Facet view**'s notes ([`super`]): its members, their merged facets,
//! and the plan of what it draws, addressed as one flat list of chunks.
//!
//! **Members.** A view shows a list of notes (CONTEXT.md, **Collection**); a
//! single-note view is a list of one. Each [`Member`] keeps its own text,
//! facets and shown block ranges. The filter bar reads the merged facets
//! ([`merge_entries`]): entries only, no blocks.
//!
//! **Addressing.** Core plans per member ([`plan_collection`], one cap over
//! all of them). The view numbers the drawn chunks globally, in list order:
//! chunk `i` is [`ChunkRef`] `refs[i]`, chunk `chunk` of member `member`.
//! Ids, tab stops and editors use the global number. Gaps are numbered
//! globally too ([`GapRef`]): each member in the plan has its own first gap
//! (above its first chunk) and last gap (below its last chunk), with the
//! gaps between its chunks in between. A gap never spans two members, so a
//! reveal never crosses a note boundary. For one member, gap `g` lies
//! before chunk `g`, and the last gap follows the last chunk.

use std::ops::Range;

use kimun_core::nfs::VaultPath;
use kimun_core::note::{
    ChunkPlan, Facet, NoteDetails, NoteFacets, NotePlan, Reveal, merge_entries, note_facets,
    plan_collection, reveal, reveal_gap, select_collection,
};

/// A note handed to a collection view: where it is, its title and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberInit {
    pub path: VaultPath,
    pub title: String,
    pub text: String,
}

/// One note of the view.
pub(crate) struct Member {
    pub(crate) path: VaultPath,
    /// The note's title: the first segment of each of its pieces'
    /// breadcrumb rows in a collection (not drawn for a single note).
    pub(crate) title: String,
    pub(crate) text: String,
    pub(crate) facets: NoteFacets,
    /// Block ranges shown: the picks' selection, plus revealed blocks.
    pub(crate) shown: Vec<Range<usize>>,
}

impl Member {
    /// `text`, the note at `path`, with nothing shown until selected.
    pub(crate) fn new(path: VaultPath, title: String, text: String) -> Self {
        let facets = note_facets(&text, &path);
        Self {
            path,
            title,
            text,
            facets,
            shown: Vec::new(),
        }
    }

    /// New text: facets re-derived, nothing shown until selected.
    pub(crate) fn set_text(&mut self, text: String) {
        self.facets = note_facets(&text, &self.path);
        self.text = text;
        self.shown.clear();
    }

    /// The title of `text`, the note at `path`, as the index derives it
    /// (its first non-empty line), else the note's name.
    fn title_of(path: &VaultPath, text: &str) -> String {
        Member::named(path, NoteDetails::get_title_from_text(text))
    }

    /// `title`, else (blank: a note with no text) the note's name, so a
    /// title segment is never drawn empty.
    fn named(path: &VaultPath, title: String) -> String {
        if title.trim().is_empty() {
            path.get_clean_name()
        } else {
            title
        }
    }

    /// The first source line of `plan`, one of this member's chunks,
    /// 0-based.
    fn first_line(&self, plan: &ChunkPlan) -> usize {
        let start = self
            .facets
            .blocks
            .get(plan.blocks.start)
            .map_or(0, |block| block.range.start);
        self.text[..start].matches('\n').count()
    }
}

/// Drawn chunk `i`: chunk `chunk` of `plans[plan]`, the plan of member
/// `member`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChunkRef {
    pub(crate) member: usize,
    /// Index into the plans (only members with a drawn chunk have one).
    pub(crate) plan: usize,
    /// Index among the member's chunks, which is its index in the member's
    /// `shown`.
    pub(crate) chunk: usize,
}

/// Gap `g`: between drawn chunks `above` and `below` (global numbers) of
/// one member. A member's first gap has no chunk above, its last gap none
/// below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GapRef {
    pub(crate) member: usize,
    pub(crate) above: Option<usize>,
    pub(crate) below: Option<usize>,
}

/// The view's notes and plan. See the module docs.
pub(crate) struct Members {
    pub(super) list: Vec<Member>,
    /// Every member's entries merged, for the filter bar. No blocks.
    pub(super) merged: NoteFacets,
    /// One plan per member with a drawn chunk, in list order.
    plans: Vec<NotePlan>,
    /// Per drawn chunk, where it lives.
    pub(crate) refs: Vec<ChunkRef>,
    /// Per gap, where it lies, in drawing order.
    pub(crate) gaps: Vec<GapRef>,
    /// Shown ranges past the cap, over all members, not drawn.
    pub(crate) cut: usize,
}

impl Members {
    pub(crate) fn new(list: Vec<Member>) -> Self {
        let mut members = Self {
            list,
            merged: NoteFacets {
                blocks: Vec::new(),
                entries: Vec::new(),
            },
            plans: Vec::new(),
            refs: Vec::new(),
            gaps: Vec::new(),
            cut: 0,
        };
        members.merge();
        members
    }

    /// The view's notes from `notes`, list order kept.
    pub(crate) fn from_inits(notes: Vec<MemberInit>) -> Self {
        Self::new(
            notes
                .into_iter()
                .map(|n| {
                    let title = Member::named(&n.path, n.title);
                    Member::new(n.path, title, n.text)
                })
                .collect(),
        )
    }

    /// The index of the member at `path` (compared as the vault compares
    /// paths: case aside).
    pub(crate) fn position(&self, path: &VaultPath) -> Option<usize> {
        self.list.iter().position(|m| m.path.is_like(path))
    }

    /// Member `idx` has new text: its facets and title are re-derived and
    /// the merged entries follow. Nothing is shown until selected again.
    pub(crate) fn set_text(&mut self, idx: usize, text: String) {
        let Some(member) = self.list.get_mut(idx) else {
            return;
        };
        member.title = Member::title_of(&member.path, &text);
        member.set_text(text);
        self.merge();
    }

    /// Member `idx` moved or was renamed to `to`: its facets are derived
    /// again from the new place (relative links), its title follows, and
    /// the merged entries follow. What it shows is kept.
    pub(crate) fn set_path(&mut self, idx: usize, to: VaultPath) {
        let Some(member) = self.list.get_mut(idx) else {
            return;
        };
        let shown = std::mem::take(&mut member.shown);
        member.path = to;
        member.title = Member::title_of(&member.path, &member.text);
        member.facets = note_facets(&member.text, &member.path);
        member.shown = shown;
        self.merge();
    }

    /// The members at `paths` (compared as [`Self::position`] does) leave
    /// the list; the merged entries follow, once. Returns how many went.
    /// Plan again after (the chunk numbering is stale).
    pub(crate) fn remove_paths(&mut self, paths: &[VaultPath]) -> usize {
        let before = self.list.len();
        self.list
            .retain(|m| !paths.iter().any(|path| m.path.is_like(path)));
        let removed = before - self.list.len();
        if removed > 0 {
            self.merge();
        }
        removed
    }

    /// How many members show something (a selected or revealed range),
    /// before the cap.
    pub(crate) fn notes_shown(&self) -> usize {
        self.list.iter().filter(|m| !m.shown.is_empty()).count()
    }

    /// Merge every member's entries again (a member's facets changed).
    pub(crate) fn merge(&mut self) {
        let notes: Vec<&NoteFacets> = self.list.iter().map(|m| &m.facets).collect();
        self.merged.entries = merge_entries(&notes);
    }

    /// Show what `picks` select in each member, nothing revealed.
    pub(crate) fn select(&mut self, picks: &[Facet]) {
        let notes: Vec<&NoteFacets> = self.list.iter().map(|m| &m.facets).collect();
        let shown = select_collection(&notes, picks);
        for (member, shown) in self.list.iter_mut().zip(shown) {
            member.shown = shown;
        }
    }

    /// Plan every member's `shown` under one cap of `limit` chunks, and
    /// number the chunks and gaps. Returns the previous plans and their
    /// chunk numbering, so the caller can tell which chunk texts changed
    /// ([`chunk_plan`]).
    pub(crate) fn plan(&mut self, limit: usize) -> (Vec<NotePlan>, Vec<ChunkRef>) {
        let notes: Vec<(&NoteFacets, &str)> = self
            .list
            .iter()
            .map(|m| (&m.facets, m.text.as_str()))
            .collect();
        let shown: Vec<Vec<Range<usize>>> = self.list.iter().map(|m| m.shown.clone()).collect();
        let (plans, cut) = plan_collection(&notes, &shown, limit);
        let mut refs = Vec::new();
        let mut gaps = Vec::new();
        for (at, plan) in plans.iter().enumerate() {
            let member = plan.note;
            let mut above = None;
            for chunk in 0..plan.chunks.len() {
                let i = refs.len();
                gaps.push(GapRef {
                    member,
                    above,
                    below: Some(i),
                });
                refs.push(ChunkRef {
                    member,
                    plan: at,
                    chunk,
                });
                above = Some(i);
            }
            gaps.push(GapRef {
                member,
                above,
                below: None,
            });
        }
        self.cut = cut;
        let old_refs = std::mem::replace(&mut self.refs, refs);
        self.gaps = gaps;
        (std::mem::replace(&mut self.plans, plans), old_refs)
    }

    /// How many chunks are drawn.
    pub(crate) fn len(&self) -> usize {
        self.refs.len()
    }

    /// Drawn chunk `i`'s plan.
    pub(crate) fn chunk(&self, i: usize) -> Option<&ChunkPlan> {
        chunk_plan(&self.plans, &self.refs, i)
    }

    /// The member drawn chunk `i` belongs to.
    pub(crate) fn member_of(&self, i: usize) -> Option<&Member> {
        self.refs.get(i).and_then(|r| self.list.get(r.member))
    }

    /// Drawn chunk `i`'s first source line in its own note, 0-based.
    pub(crate) fn first_line(&self, i: usize) -> usize {
        match (self.member_of(i), self.chunk(i)) {
            (Some(member), Some(plan)) => member.first_line(plan),
            _ => 0,
        }
    }

    /// The facet of crumb `index` of drawn chunk `i`.
    pub(crate) fn crumb_facet(&self, i: usize, index: usize) -> Option<Facet> {
        self.chunk(i)
            .and_then(|plan| plan.crumbs.get(index))
            .map(|crumb| crumb.facet.clone())
    }

    /// How many blocks gap `g` hides (0 for no such gap).
    pub(crate) fn gap_hidden(&self, g: usize) -> usize {
        let Some(gap) = self.gaps.get(g) else {
            return 0;
        };
        match (gap.above, gap.below) {
            (Some(above), _) => self.chunk(above).map_or(0, |p| p.hidden_below),
            (None, Some(below)) => self.chunk(below).map_or(0, |p| p.hidden_above),
            (None, None) => 0,
        }
    }

    /// Reveal one hidden block next to drawn chunk `i`, inside its member.
    /// Plan again after.
    pub(crate) fn reveal(&mut self, i: usize, dir: Reveal) {
        let Some(at) = self.refs.get(i).copied() else {
            return;
        };
        let member = &mut self.list[at.member];
        member.shown = reveal(&member.shown, at.chunk, dir, member.facets.blocks.len());
    }

    /// Reveal every block gap `g` hides, inside its member. Plan again after.
    pub(crate) fn reveal_gap(&mut self, g: usize) {
        let Some(gap) = self.gaps.get(g).copied() else {
            return;
        };
        // The gap's index among its member's gaps: before the chunk below
        // it, else after the chunk above it.
        let in_member = match (gap.below, gap.above) {
            (Some(below), _) => self.refs[below].chunk,
            (None, Some(above)) => self.refs[above].chunk + 1,
            (None, None) => return,
        };
        let member = &mut self.list[gap.member];
        member.shown = reveal_gap(&member.shown, in_member, member.facets.blocks.len());
    }
}

/// Chunk `i`'s plan in `plans` as numbered by `refs` (the current plan, or
/// one [`Members::plan`] handed back).
pub(crate) fn chunk_plan<'a>(
    plans: &'a [NotePlan],
    refs: &[ChunkRef],
    i: usize,
) -> Option<&'a ChunkPlan> {
    let at = refs.get(i)?;
    plans.get(at.plan)?.chunks.get(at.chunk)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use kimun_core::note::FacetKind;

    fn members(texts: &[&str]) -> Members {
        Members::new(
            texts
                .iter()
                .enumerate()
                .map(|(i, text)| {
                    let path = VaultPath::new(format!("/n{i}.md"));
                    Member::new(path, format!("n{i}"), text.to_string())
                })
                .collect(),
        )
    }

    fn tag(key: &str) -> Facet {
        Facet::new(FacetKind::Tag, key)
    }

    #[test]
    fn one_member_numbers_gaps_before_each_chunk_and_after_the_last() {
        let mut m = members(&["#x a\n\nb\n\n#x c\n\nd\n"]);
        m.select(&[tag("x")]);
        m.plan(usize::MAX);
        assert_eq!(m.len(), 2);
        let sides: Vec<_> = m.gaps.iter().map(|g| (g.above, g.below)).collect();
        assert_eq!(
            sides,
            [(None, Some(0)), (Some(0), Some(1)), (Some(1), None)]
        );
        assert_eq!(
            (0..3).map(|g| m.gap_hidden(g)).collect::<Vec<_>>(),
            [0, 1, 1]
        );
    }

    #[test]
    fn each_member_has_its_own_first_and_last_gap() {
        let mut m = members(&["a\n\n#x b\n", "#x c\n\nd\n"]);
        m.select(&[tag("x")]);
        m.plan(usize::MAX);
        let refs: Vec<_> = m.refs.iter().map(|r| (r.member, r.chunk)).collect();
        assert_eq!(refs, [(0, 0), (1, 0)]);
        let gaps: Vec<_> = m
            .gaps
            .iter()
            .map(|g| (g.member, g.above, g.below))
            .collect();
        assert_eq!(
            gaps,
            [
                (0, None, Some(0)),
                (0, Some(0), None),
                (1, None, Some(1)),
                (1, Some(1), None)
            ]
        );
        assert_eq!(
            (0..4).map(|g| m.gap_hidden(g)).collect::<Vec<_>>(),
            [1, 0, 0, 1]
        );
        // Filling member 1's last gap leaves member 0 alone.
        m.reveal_gap(3);
        assert_eq!(m.list[0].shown, [1..2]);
        assert_eq!(m.list[1].shown, [0..2]);
    }

    #[test]
    fn first_line_counts_in_the_chunks_own_note() {
        let mut m = members(&["#x a\n", "b\n\n#x c\n"]);
        m.select(&[tag("x")]);
        m.plan(usize::MAX);
        assert_eq!(m.first_line(0), 0);
        assert_eq!(m.first_line(1), 2);
    }
}
