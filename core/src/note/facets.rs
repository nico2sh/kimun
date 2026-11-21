//! Facets of a note: the headings, tags and note links a user can pick to
//! narrow the note, each with the blocks it owns. See `CONTEXT.md`, **Facet**,
//! **Facet kind** and **Facet scope**.

use std::collections::HashMap;
use std::ops::Range;

use crate::nfs::VaultPath;

use super::blocks::{note_blocks, BlockKind, NoteBlock};
use super::diacritics::remove_diacritics;
use super::walk;
use super::NoteHeading;

/// The kind of a [`Facet`]. Facets of one kind combine with OR, facets of
/// different kinds with AND.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FacetKind {
    Heading,
    Tag,
    NoteLink,
}

/// The identity of a facet: its kind and a lowercase merge key (a heading
/// key also drops diacritics). Equal facets in different notes are equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Facet {
    pub kind: FacetKind,
    pub key: String,
}

impl Facet {
    /// The facet of `kind` that `text` names, its key normalized the one way
    /// every call site shares: lowercase, and for a heading also without
    /// diacritics (as [`super::scan::heading_section_range`] compares them).
    /// A tag stays diacritic-sensitive, as the index stores it.
    pub fn new(kind: FacetKind, text: &str) -> Self {
        let key = match kind {
            FacetKind::Heading => remove_diacritics(text).to_lowercase(),
            FacetKind::Tag | FacetKind::NoteLink => text.to_lowercase(),
        };
        Self { kind, key }
    }
}

/// A facet found in a note, with the blocks it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetEntry {
    pub facet: Facet,
    /// The first occurrence as written, for the pill.
    pub label: String,
    /// One range of block indexes per occurrence, ascending, overlaps merged.
    pub scopes: Vec<Range<usize>>,
    /// Number of occurrences in the note.
    pub count: usize,
}

/// A note's blocks and facets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFacets {
    pub blocks: Vec<NoteBlock>,
    /// In document order of first occurrence.
    pub entries: Vec<FacetEntry>,
}

/// The blocks and facets of `text`, a note at `ref_path` (which resolves
/// relative note links).
///
/// ```
/// use kimun_core::nfs::VaultPath;
/// use kimun_core::note::{note_facets, select_blocks, Facet, FacetKind};
///
/// let text = "## Trip\n- Book hotel #travel\n  - check dates\n- Buy tickets\n  - #travel flight\n";
/// let facets = note_facets(text, &VaultPath::new("/trip.md"));
/// let travel = facets
///     .entries
///     .iter()
///     .find(|e| e.facet.key == "travel")
///     .unwrap();
/// // The hotel item with its child, then the flight item alone: no parents.
/// assert_eq!(travel.scopes, [1..3, 4..5]);
///
/// let pick = Facet { kind: FacetKind::Tag, key: "travel".into() };
/// assert_eq!(select_blocks(&facets, &[pick]), [1..3, 4..5]);
/// ```
pub fn note_facets(text: &str, ref_path: &VaultPath) -> NoteFacets {
    let blocks = note_blocks(text);
    let walked = walk::walk(text);

    // (byte position, facet, label, scope) for every occurrence.
    let mut found: Vec<(usize, Facet, String, Range<usize>)> = Vec::new();

    // Headings: a heading block the outline lists, matched by line number.
    let outline: HashMap<usize, NoteHeading> = walked
        .headings(text)
        .into_iter()
        .map(|h| (h.line, h))
        .collect();
    let (mut line, mut seen) = (0, 0);
    for (at, block) in blocks.iter().enumerate() {
        let BlockKind::Heading(level) = block.kind else {
            continue;
        };
        line += text[seen..block.range.start].matches('\n').count();
        seen = block.range.start;
        if let Some(heading) = outline.get(&line) {
            found.push((
                block.range.start,
                Facet::new(FacetKind::Heading, &heading.text),
                heading.text.clone(),
                section_of(&blocks, at, level),
            ));
        }
    }

    let mut place = |byte: usize, facet: Facet, label: String| {
        if let Some(at) = block_at(&blocks, byte) {
            found.push((byte, facet, label, scope_of(&blocks, at)));
        } else if let Some(rows) = table_rows_led_by(&blocks, byte) {
            found.push((byte, facet, label, rows));
        }
    };
    for tag in &walked.tags {
        place(
            tag.range.start,
            Facet::new(FacetKind::Tag, &tag.name),
            tag.name.clone(),
        );
    }
    for (link, target) in walked.note_links(ref_path) {
        let name = target.path().get_clean_name();
        place(
            link.range.start,
            Facet::new(FacetKind::NoteLink, &name),
            name,
        );
    }
    found.sort_by_key(|(byte, ..)| *byte);

    let mut entries: Vec<FacetEntry> = Vec::new();
    let mut index: HashMap<Facet, usize> = HashMap::new();
    for (_, facet, label, scope) in found {
        match index.get(&facet) {
            Some(&at) => {
                entries[at].count += 1;
                entries[at].scopes.push(scope);
            }
            None => {
                index.insert(facet.clone(), entries.len());
                entries.push(FacetEntry {
                    facet,
                    label,
                    scopes: vec![scope],
                    count: 1,
                });
            }
        }
    }
    for entry in &mut entries {
        entry.scopes = merge_overlapping(std::mem::take(&mut entry.scopes));
    }
    NoteFacets { blocks, entries }
}

/// Block-index ranges to show for `picks`: ascending, non-overlapping, with
/// touching ranges joined. No picks shows every block.
///
/// Picks of one kind combine with OR (the union of their scopes), and a pick
/// the note does not hold adds nothing. The kinds that have picks combine
/// with AND (the intersection). A kind whose picks are all absent from the
/// note leaves nothing to show.
pub fn select_blocks(facets: &NoteFacets, picks: &[Facet]) -> Vec<Range<usize>> {
    let len = facets.blocks.len();
    let mut shown = vec![true; len];
    for kind in [FacetKind::Heading, FacetKind::Tag, FacetKind::NoteLink] {
        let of_kind: Vec<&Facet> = picks.iter().filter(|p| p.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }
        let mut owned = vec![false; len];
        for entry in facets
            .entries
            .iter()
            .filter(|e| of_kind.contains(&&e.facet))
        {
            for scope in &entry.scopes {
                owned[scope.clone()].fill(true);
            }
        }
        for (show, own) in shown.iter_mut().zip(owned) {
            *show &= own;
        }
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (at, show) in shown.into_iter().enumerate() {
        if !show {
            continue;
        }
        match ranges.last_mut() {
            Some(last) if last.end == at => last.end += 1,
            _ => ranges.push(at..at + 1),
        }
    }
    ranges
}

/// Index of the block whose own text holds `byte`.
fn block_at(blocks: &[NoteBlock], byte: usize) -> Option<usize> {
    let after = blocks.partition_point(|b| b.range.start <= byte);
    let at = after.checked_sub(1)?;
    (byte < blocks[at].range.end).then_some(at)
}

/// The rows of the table whose header holds `byte`: a facet in a header owns
/// every row under it.
fn table_rows_led_by(blocks: &[NoteBlock], byte: usize) -> Option<Range<usize>> {
    let first = blocks
        .iter()
        .position(|b| b.lead.as_ref().is_some_and(|l| l.contains(&byte)))?;
    let lead = blocks[first].lead.clone();
    let end = blocks[first..]
        .iter()
        .position(|b| b.lead != lead)
        .map_or(blocks.len(), |n| first + n);
    Some(first..end)
}

/// The blocks a facet found in block `at` owns.
fn scope_of(blocks: &[NoteBlock], at: usize) -> Range<usize> {
    match blocks[at].kind {
        BlockKind::Heading(level) => section_of(blocks, at, level),
        BlockKind::ListItem => {
            let depth = blocks[at].depth;
            let end = blocks[at + 1..]
                .iter()
                .position(|b| b.depth <= depth)
                .map_or(blocks.len(), |n| at + 1 + n);
            at..end
        }
        _ => at..at + 1,
    }
}

/// A heading's section: up to the next heading of its level or higher.
fn section_of(blocks: &[NoteBlock], at: usize, level: u8) -> Range<usize> {
    let end = blocks[at + 1..]
        .iter()
        .position(|b| matches!(b.kind, BlockKind::Heading(l) if l <= level))
        .map_or(blocks.len(), |n| at + 1 + n);
    at..end
}

/// `ranges` sorted, with equal and overlapping ranges joined. Ranges that
/// only touch stay apart.
fn merge_overlapping(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start < last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn facets(text: &str) -> NoteFacets {
        note_facets(text, &VaultPath::new("/n.md"))
    }

    fn entry<'a>(f: &'a NoteFacets, kind: FacetKind, key: &str) -> &'a FacetEntry {
        f.entries
            .iter()
            .find(|e| e.facet.kind == kind && e.facet.key == key)
            .unwrap_or_else(|| panic!("no {kind:?} facet {key:?} in {:?}", f.entries))
    }

    fn tag<'a>(f: &'a NoteFacets, key: &str) -> &'a FacetEntry {
        entry(f, FacetKind::Tag, key)
    }

    fn has(f: &NoteFacets, kind: FacetKind, key: &str) -> bool {
        f.entries
            .iter()
            .any(|e| e.facet.kind == kind && e.facet.key == key)
    }

    /// The text of the first line of each block of `range`, to name scopes.
    fn lines(text: &str, f: &NoteFacets, range: &Range<usize>) -> Vec<String> {
        f.blocks[range.clone()]
            .iter()
            .map(|b| {
                text[b.range.clone()]
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    const LISBON: &str = "## Trip to Lisbon\n- Book hotel #travel\n  - check dates\n  - ask about breakfast\n- Buy tickets\n  - #travel flight in the morning\n";

    #[test]
    fn tag_in_paragraph_owns_the_paragraph() {
        let text = "Intro\n\nSome #idea here.\n\nOutro\n";
        let f = facets(text);
        let e = tag(&f, "idea");
        assert_eq!(e.scopes, [1..2]);
        assert_eq!(e.count, 1);
    }

    #[test]
    fn tag_in_list_item_owns_item_and_children_not_parents() {
        let f = facets(LISBON);
        let e = tag(&f, "travel");
        assert_eq!(e.scopes.len(), 2);
        assert_eq!(
            lines(LISBON, &f, &e.scopes[0]),
            [
                "- Book hotel #travel",
                "- check dates",
                "- ask about breakfast"
            ]
        );
        assert_eq!(
            lines(LISBON, &f, &e.scopes[1]),
            ["- #travel flight in the morning"]
        );
        assert_eq!(e.count, 2);
    }

    #[test]
    fn tag_in_deep_nested_item_owns_only_that_subtree() {
        let text = "- a\n  - b\n    - c #deep\n    - d\n  - e\n";
        let f = facets(text);
        assert_eq!(lines(text, &f, &tag(&f, "deep").scopes[0]), ["- c #deep"]);
    }

    #[test]
    fn tag_in_heading_line_owns_the_section() {
        let text = "# Day 3 #travel\n\nwalk\n\n# Day 4\n\nrest\n";
        let f = facets(text);
        assert_eq!(tag(&f, "travel").scopes, [0..2]);
    }

    #[test]
    fn heading_scope_includes_subheadings_and_stops_at_a_peer() {
        let text = "# A\n\none\n\n## A1\n\ntwo\n\n# B\n\nthree\n";
        let f = facets(text);
        assert_eq!(entry(&f, FacetKind::Heading, "a").scopes, [0..4]);
        assert_eq!(entry(&f, FacetKind::Heading, "a1").scopes, [2..4]);
        assert_eq!(entry(&f, FacetKind::Heading, "b").scopes, [4..6]);
    }

    #[test]
    fn skipped_heading_levels_end_sections_correctly() {
        let text = "# A\n\n### C\n\n## B\n";
        let f = facets(text);
        assert_eq!(f.blocks.len(), 3);
        assert_eq!(entry(&f, FacetKind::Heading, "a").scopes, [0..3]);
        assert_eq!(entry(&f, FacetKind::Heading, "c").scopes, [1..2]);
        assert_eq!(entry(&f, FacetKind::Heading, "b").scopes, [2..3]);
    }

    #[test]
    fn table_tag_owns_one_row() {
        let text = "| h |\n|---|\n| a #x |\n| b |\n";
        let f = facets(text);
        let e = tag(&f, "x");
        assert_eq!(e.scopes, [0..1]);
        assert!(f.blocks[0].lead.is_some());
    }

    #[test]
    fn blockquote_tag_owns_inner_paragraph() {
        let text = "> one #q\n>\n> two\n";
        let f = facets(text);
        assert_eq!(tag(&f, "q").scopes, [0..1]);
    }

    #[test]
    fn note_link_wikilink_and_markdown_merge() {
        let text = "See [[plan]].\n\nAlso [p](work/plan.md).\n";
        let f = facets(text);
        let e = entry(&f, FacetKind::NoteLink, "plan");
        assert_eq!(e.count, 2);
        assert_eq!(e.scopes, [0..1, 1..2]);
    }

    #[test]
    fn tag_twice_in_one_block_is_one_scope_two_count() {
        let f = facets("#a and #a again\n");
        let e = tag(&f, "a");
        assert_eq!(e.count, 2);
        assert_eq!(e.scopes, [0..1]);
    }

    #[test]
    fn tag_in_code_span_fence_and_frontmatter_is_no_facet() {
        let text = "---\ntags: [fm]\n---\nA `#span` here\n\n```\n#fence\n```\n";
        let f = facets(text);
        assert!(f.entries.iter().all(|e| e.facet.kind != FacetKind::Tag));
    }

    #[test]
    fn external_url_and_image_are_no_facets() {
        let f = facets("[x](https://example.com) ![pic](pic.png) <https://a.org>\n");
        assert!(f.entries.is_empty(), "{:?}", f.entries);
    }

    #[test]
    fn heading_keys_ignore_case_level_and_diacritics() {
        let f = facets("# Café\n\none\n\n## cafe\n\ntwo\n");
        let e = entry(&f, FacetKind::Heading, "cafe");
        assert_eq!(e.count, 2);
        assert_eq!(e.label, "Café");
    }

    #[test]
    fn crlf_note_matches_lf_note() {
        let lf = "# A #t\n\n- x [[plan]]\n  - y\n\n| h |\n|---|\n| c #t |\n";
        let crlf = lf.replace('\n', "\r\n");
        let key = |f: &NoteFacets| -> Vec<(Facet, Vec<Range<usize>>, usize)> {
            f.entries
                .iter()
                .map(|e| (e.facet.clone(), e.scopes.clone(), e.count))
                .collect()
        };
        assert_eq!(key(&facets(lf)), key(&facets(&crlf)));
    }

    #[test]
    fn no_headings_and_empty_note() {
        let f = facets("plain #only text\n");
        assert!(has(&f, FacetKind::Tag, "only"));
        assert!(!f.entries.iter().any(|e| e.facet.kind == FacetKind::Heading));
        let empty = facets("");
        assert!(empty.blocks.is_empty() && empty.entries.is_empty());
    }

    fn pick(kind: FacetKind, key: &str) -> Facet {
        Facet {
            kind,
            key: key.to_string(),
        }
    }

    fn shown(text: &str, picks: &[Facet]) -> Vec<Vec<String>> {
        let f = facets(text);
        select_blocks(&f, picks)
            .iter()
            .map(|r| lines(text, &f, r))
            .collect()
    }

    const JOURNAL: &str = "# 2026-11-01\n\nmorning #idea one\n\nplain\n\n# 2026-11-02\n\nnoon #idea two\n\n#todo call\n";

    #[test]
    fn no_picks_show_every_block() {
        let f = facets(JOURNAL);
        assert_eq!(select_blocks(&f, &[]), [0..f.blocks.len()]);
        assert!(select_blocks(&facets(""), &[]).is_empty());
    }

    #[test]
    fn two_tags_union() {
        let out = shown(
            JOURNAL,
            &[pick(FacetKind::Tag, "idea"), pick(FacetKind::Tag, "todo")],
        );
        assert_eq!(
            out,
            [
                vec!["morning #idea one"],
                vec!["noon #idea two", "#todo call"]
            ]
        );
    }

    #[test]
    fn heading_and_tag_intersect() {
        let out = shown(
            JOURNAL,
            &[
                pick(FacetKind::Heading, "2026-11-01"),
                pick(FacetKind::Tag, "idea"),
            ],
        );
        assert_eq!(out, [vec!["morning #idea one"]]);
    }

    #[test]
    fn overlapping_headings_dedupe() {
        let text = "# A\n\none\n\n## A1\n\ntwo\n";
        let f = facets(text);
        let out = select_blocks(
            &f,
            &[
                pick(FacetKind::Heading, "a"),
                pick(FacetKind::Heading, "a1"),
            ],
        );
        assert_eq!(out, [0..4]);
    }

    #[test]
    fn absent_pick_in_a_kind_gives_nothing() {
        assert!(shown(JOURNAL, &[pick(FacetKind::Tag, "zzz")]).is_empty());
    }

    #[test]
    fn absent_pick_among_present_picks_of_the_same_kind_is_ignored() {
        let out = shown(
            JOURNAL,
            &[pick(FacetKind::Tag, "idea"), pick(FacetKind::Tag, "zzz")],
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn touching_ranges_merge() {
        let text = "a #x\n\nb #x\n\nc\n";
        let f = facets(text);
        assert_eq!(select_blocks(&f, &[pick(FacetKind::Tag, "x")]), [0..2]);
    }

    #[test]
    fn tag_and_note_link_are_different_kinds() {
        let text = "[[plan]] #a\n\n[[plan]] only\n\n#a only\n";
        let f = facets(text);
        let out = select_blocks(
            &f,
            &[pick(FacetKind::Tag, "a"), pick(FacetKind::NoteLink, "plan")],
        );
        assert_eq!(out, [0..1]);
    }

    #[test]
    fn tag_after_a_code_block_inside_an_item_is_kept() {
        let text = "- a\n\n  ```\n  x\n  ```\n\n  b #t\n";
        let f = facets(text);
        assert_eq!(tag(&f, "t").scopes, [0..1]);
    }

    #[test]
    fn tag_in_a_table_header_owns_every_row() {
        let text = "intro\n\n| h #t |\n|---|\n| a |\n| b |\n\nafter\n";
        let f = facets(text);
        assert_eq!(tag(&f, "t").scopes, [1..3]);
    }

    #[test]
    fn facet_new_normalizes_the_key_by_kind() {
        assert_eq!(
            Facet::new(FacetKind::Heading, "Café"),
            Facet::new(FacetKind::Heading, "cafe")
        );
        assert_eq!(
            Facet::new(FacetKind::Tag, "Idea"),
            Facet::new(FacetKind::Tag, "idea")
        );
        assert_ne!(
            Facet::new(FacetKind::Tag, "café"),
            Facet::new(FacetKind::Tag, "cafe")
        );
        assert_ne!(
            Facet::new(FacetKind::Tag, "plan"),
            Facet::new(FacetKind::NoteLink, "plan")
        );
    }
}
