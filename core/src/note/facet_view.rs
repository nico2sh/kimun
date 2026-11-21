//! Planning what a facet view draws: the chunks of source text for the shown
//! block ranges, and the reveal of one hidden block next to a chunk. See
//! `CONTEXT.md`, **Facet view**.

use std::ops::Range;

use super::blocks::{BlockKind, NoteBlock};
use super::facets::{Facet, FacetKind, NoteFacets};

/// The default most chunks a caller asks `plan_chunks` for.
pub const MAX_RANGES: usize = 100;

/// The most hidden blocks a gap can have for one click to reveal all of it.
pub const GAP_SHOW_ALL_MAX: usize = 3;

/// A heading that contains a chunk: one breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub facet: Facet,
}

/// One run of shown blocks, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPlan {
    /// Block indexes this chunk draws.
    pub blocks: Range<usize>,
    /// Source text of the chunk: the blocks' bytes from the first block's start
    /// to the last block's end. A table row's `lead` (header and delimiter rows)
    /// is put in front when the first block has one and the range does not
    /// start at the table's first row.
    pub text: String,
    /// Blocks hidden between this chunk and the one before it (or the note start).
    pub hidden_above: usize,
    /// Blocks hidden between this chunk and the next (or the note end).
    pub hidden_below: usize,
    /// Headings that strictly contain the chunk's first block, outermost
    /// first. The chunk's own heading (a scope starting at the first block) is
    /// not listed.
    pub crumbs: Vec<Crumb>,
}

/// The headings whose scope strictly contains block `first`, outermost first.
fn crumbs_for(facets: &NoteFacets, first: usize) -> Vec<Crumb> {
    let mut found: Vec<(usize, Crumb)> = Vec::new();
    for entry in &facets.entries {
        if entry.facet.kind != FacetKind::Heading {
            continue;
        }
        if let Some(scope) = entry
            .scopes
            .iter()
            .find(|s| s.start < first && first < s.end)
        {
            found.push((
                scope.start,
                Crumb {
                    label: entry.label.clone(),
                    facet: entry.facet.clone(),
                },
            ));
        }
    }
    found.sort_by_key(|(start, _)| *start);
    found.into_iter().map(|(_, c)| c).collect()
}

/// Where a top-level indented code block's first line starts. The block's
/// range starts after the indent, but the indent is what makes it code, so
/// the chunk keeps it. `None` for any other block, a fenced one included.
fn code_line_start(text: &str, block: &NoteBlock) -> Option<usize> {
    if block.kind != BlockKind::Code || block.depth != 0 {
        return None;
    }
    let rest = &text[block.range.start..];
    if rest.starts_with("```") || rest.starts_with("~~~") {
        return None;
    }
    let line_start = text[..block.range.start].rfind('\n').map_or(0, |i| i + 1);
    let indent = &text[line_start..block.range.start];
    (!indent.is_empty() && indent.chars().all(|c| c == ' ' || c == '\t')).then_some(line_start)
}

/// `text[span]` with the indent of the span's first line taken off the
/// lines after it. A block starts at its marker, so the first line has no
/// indent, but a nested list item's later lines keep the indent of its
/// level. Drawn alone, those lines read as an indented code block. Only
/// whitespace goes, and never more than the first line's indent. A span in a
/// quote (`> - x`) has no whitespace-only indent and stays as it is.
fn dedented(text: &str, span: Range<usize>) -> String {
    let line_start = text[..span.start].rfind('\n').map_or(0, |i| i + 1);
    let indent = &text[line_start..span.start];
    let source = &text[span];
    if indent.is_empty() || !indent.chars().all(|c| c == ' ' || c == '\t') {
        return source.to_string();
    }
    let mut out = String::with_capacity(source.len());
    for (i, line) in source.split_inclusive('\n').enumerate() {
        if i == 0 {
            out.push_str(line);
            continue;
        }
        let strip = line
            .bytes()
            .take(indent.len())
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count();
        out.push_str(&line[strip..]);
    }
    out
}

/// Chunks for `shown` (ascending, non-overlapping). At most `limit` chunks;
/// the second value counts the ranges left out.
pub fn plan_chunks(
    facets: &NoteFacets,
    text: &str,
    shown: &[Range<usize>],
    limit: usize,
) -> (Vec<ChunkPlan>, usize) {
    let blocks = &facets.blocks;
    let mut plans = Vec::new();
    for (at, range) in shown.iter().enumerate().take(limit) {
        if range.is_empty() || range.end > blocks.len() {
            continue;
        }
        let first = &blocks[range.start];
        let last = &blocks[range.end - 1];
        let mut chunk = String::new();
        if let Some(lead) = &first.lead {
            let first_row = range.start > 0 && blocks[range.start - 1].lead.as_ref() == Some(lead);
            if first_row {
                chunk.push_str(&text[lead.clone()]);
            }
        }
        let start = code_line_start(text, first).unwrap_or(first.range.start);
        chunk.push_str(&dedented(text, start..last.range.end));
        let above_from = if at == 0 { 0 } else { shown[at - 1].end };
        let below_to = shown.get(at + 1).map_or(blocks.len(), |next| next.start);
        plans.push(ChunkPlan {
            blocks: range.clone(),
            text: chunk,
            hidden_above: range.start.saturating_sub(above_from),
            hidden_below: below_to.saturating_sub(range.end),
            crumbs: crumbs_for(facets, range.start),
        });
    }
    (plans, shown.len().saturating_sub(limit))
}

/// `ranges` (ascending) with touching or overlapping neighbours merged.
fn merge_touching(ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// Which side of a chunk to reveal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reveal {
    Above,
    Below,
}

/// `shown` with the one block next to chunk `chunk` revealed in `dir`.
/// Ranges that now touch merge. `block_count` bounds the result.
pub fn reveal(
    shown: &[Range<usize>],
    chunk: usize,
    dir: Reveal,
    block_count: usize,
) -> Vec<Range<usize>> {
    let mut out = shown.to_vec();
    let Some(range) = out.get_mut(chunk) else {
        return out;
    };
    match dir {
        Reveal::Above => range.start = range.start.saturating_sub(1),
        Reveal::Below => range.end = (range.end + 1).min(block_count),
    }
    merge_touching(out)
}

/// `shown` with the whole gap `gap` filled. Gap `g` lies before chunk `g`
/// (gap 0 is above the first chunk, gap `shown.len()` below the last).
/// Touching ranges merge. A gap that hides nothing, or `gap > shown.len()`,
/// changes nothing.
pub fn reveal_gap(shown: &[Range<usize>], gap: usize, block_count: usize) -> Vec<Range<usize>> {
    if gap > shown.len() {
        return shown.to_vec();
    }
    let from = if gap == 0 { 0 } else { shown[gap - 1].end };
    let to = shown.get(gap).map_or(block_count, |next| next.start);
    if from >= to {
        return shown.to_vec();
    }
    let mut out = shown.to_vec();
    out.insert(gap, from..to);
    merge_touching(out)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use crate::nfs::VaultPath;
    use crate::note::note_facets;

    fn facets(text: &str) -> NoteFacets {
        note_facets(text, &VaultPath::new("/n.md"))
    }

    const NINE: &str = "p0\n\np1\n\np2\n\np3\n\np4\n\np5\n\np6\n\np7\n\np8\n";

    #[test]
    fn a_nested_item_loses_its_levels_indent_on_later_lines() {
        let text = "- one\n  - two\n    - three\n      more of three\n\n      para of three\n";
        let f = facets(text);
        let last = f.blocks.len() - 1;
        let (plans, _) = plan_chunks(&f, text, &[2..last + 1], MAX_RANGES);
        assert_eq!(plans[0].text, "- three\n  more of three\n\n  para of three");
    }

    #[test]
    fn real_code_blocks_keep_their_shape() {
        let nested = "- one\n  - two\n    - three\n      ```\n        code  \n      ```\n";
        let f = facets(nested);
        let (plans, _) = plan_chunks(&f, nested, &[2..3], MAX_RANGES);
        assert_eq!(plans[0].text, "- three\n  ```\n    code  \n  ```");
        let indented = "para\n\n    a\n\n    b\n      c\n";
        let f = facets(indented);
        let (plans, _) = plan_chunks(&f, indented, &[1..2], MAX_RANGES);
        assert_eq!(plans[0].text, "    a\n\n    b\n      c");
        let top = "para\n\n```\n  code\n```\n";
        let f = facets(top);
        let (plans, _) = plan_chunks(&f, top, &[1..2], MAX_RANGES);
        assert_eq!(plans[0].text, "```\n  code\n```");
    }

    #[test]
    fn a_chunk_in_a_quote_keeps_its_text() {
        let text = "> - one\n>   more\n";
        let f = facets(text);
        let (plans, _) = plan_chunks(&f, text, &[0..1], MAX_RANGES);
        assert!(plans[0].text.contains(">   more"));
    }

    #[test]
    fn a_chunk_is_the_source_text_of_its_blocks() {
        let text = "one\n\ntwo\n\nthree\n";
        let f = facets(text);
        let (plans, cut) = plan_chunks(&f, text, &[1..2], MAX_RANGES);
        assert_eq!(cut, 0);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].text, "two");
        assert_eq!(plans[0].blocks, 1..2);
    }

    #[test]
    fn hidden_counts_are_the_gaps() {
        let f = facets(NINE);
        assert_eq!(f.blocks.len(), 9);
        let (plans, _) = plan_chunks(&f, NINE, &[2..3, 6..7], MAX_RANGES);
        assert_eq!((plans[0].hidden_above, plans[0].hidden_below), (2, 3));
        assert_eq!((plans[1].hidden_above, plans[1].hidden_below), (3, 2));
    }

    #[test]
    fn a_table_row_chunk_carries_the_header() {
        let text = "| h1 | h2 |\n|----|----|\n| a | b |\n| c | d |\n| e | f |\n";
        let f = facets(text);
        let (plans, _) = plan_chunks(&f, text, &[1..2], MAX_RANGES);
        assert_eq!(plans[0].text, "| h1 | h2 |\n|----|----|\n| c | d |");
        let (first, _) = plan_chunks(&f, text, &[0..1], MAX_RANGES);
        assert_eq!(first[0].text, "| a | b |");
    }

    #[test]
    fn no_picks_whole_document_is_one_chunk() {
        let f = facets(NINE);
        let n = f.blocks.len();
        let (plans, cut) = plan_chunks(&f, NINE, &[0..n], MAX_RANGES);
        assert_eq!(cut, 0);
        assert_eq!(plans.len(), 1);
        assert_eq!((plans[0].hidden_above, plans[0].hidden_below), (0, 0));
    }

    #[test]
    fn more_than_max_ranges_are_cut_and_counted() {
        let text: String = (0..300).map(|i| format!("p{i}\n\n")).collect();
        let f = facets(&text);
        let shown: Vec<Range<usize>> = (0..150).map(|i| i * 2..i * 2 + 1).collect();
        let (plans, cut) = plan_chunks(&f, &text, &shown, MAX_RANGES);
        assert_eq!(plans.len(), 100);
        assert_eq!(cut, 50);
        let (few, cut) = plan_chunks(&f, &text, &shown, 10);
        assert_eq!((few.len(), cut), (10, 140));
    }

    #[test]
    fn reveal_above_and_below_extend_by_one_block() {
        assert_eq!(reveal(&[3..5], 0, Reveal::Above, 9), [2..5]);
        assert_eq!(reveal(&[3..5], 0, Reveal::Below, 9), [3..6]);
    }

    #[test]
    fn reveal_merges_ranges_that_touch() {
        assert_eq!(reveal(&[1..3, 4..6], 0, Reveal::Below, 9), [1..6]);
        assert_eq!(reveal(&[1..3, 4..6], 1, Reveal::Above, 9), [1..6]);
    }

    #[test]
    fn reveal_stops_at_the_note_edges() {
        assert_eq!(reveal(&[0..2], 0, Reveal::Above, 5), [0..2]);
        assert_eq!(reveal(&[3..5], 0, Reveal::Below, 5), [3..5]);
        assert_eq!(reveal(&[3..5], 7, Reveal::Below, 5), [3..5]);
    }

    #[test]
    fn crlf_text_gives_the_same_chunk_boundaries() {
        let lf = "# A\n\none\n\n| h |\n|---|\n| x |\n| y |\n\nlast\n";
        let crlf = lf.replace('\n', "\r\n");
        let (lf_f, crlf_f) = (facets(lf), facets(&crlf));
        let shown = [1..2, 3..4];
        let (a, _) = plan_chunks(&lf_f, lf, &shown, MAX_RANGES);
        let (b, _) = plan_chunks(&crlf_f, &crlf, &shown, MAX_RANGES);
        let key = |p: &[ChunkPlan]| -> Vec<(Range<usize>, usize, usize)> {
            p.iter()
                .map(|c| (c.blocks.clone(), c.hidden_above, c.hidden_below))
                .collect()
        };
        assert_eq!(key(&a), key(&b));
        assert_eq!(b[1].text, "| h |\r\n|---|\r\n| y |");
    }

    fn labels(p: &ChunkPlan) -> Vec<&str> {
        p.crumbs.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn crumbs_name_the_headings_that_contain_the_chunk() {
        let text = "# A\n\n## A1\n\npara\n";
        let f = facets(text);
        let (plans, _) = plan_chunks(&f, text, &[2..3], MAX_RANGES);
        assert_eq!(f.blocks.len(), 3);
        assert_eq!(labels(&plans[0]), ["A", "A1"]);
    }

    #[test]
    fn a_chunk_that_starts_at_a_heading_does_not_list_that_heading() {
        let text = "# A\n\n## A1\n\npara\n";
        let f = facets(text);
        let (plans, _) = plan_chunks(&f, text, &[1..2], MAX_RANGES);
        assert_eq!(labels(&plans[0]), ["A"]);
    }

    #[test]
    fn a_note_without_headings_has_no_crumbs() {
        let f = facets(NINE);
        let (plans, _) = plan_chunks(&f, NINE, &[2..3], MAX_RANGES);
        assert!(plans[0].crumbs.is_empty());
    }

    #[test]
    fn crumbs_for_repeated_heading_text_follow_each_section() {
        let text = "# Mon\n\n### Morning\n\nm1\n\n# Tue\n\n### Morning\n\nm2\n";
        let f = facets(text);
        assert_eq!(f.blocks.len(), 6);
        let (plans, _) = plan_chunks(&f, text, &[2..3, 5..6], MAX_RANGES);
        assert_eq!(labels(&plans[0]), ["Mon", "Morning"]);
        assert_eq!(labels(&plans[1]), ["Tue", "Morning"]);
    }

    #[test]
    fn crumb_facets_are_the_heading_facets() {
        let text = "# Älpha\n\npara\n";
        let f = facets(text);
        let (plans, _) = plan_chunks(&f, text, &[1..2], MAX_RANGES);
        assert_eq!(
            plans[0].crumbs,
            [Crumb {
                label: "Älpha".to_string(),
                facet: Facet::new(FacetKind::Heading, "Älpha"),
            }]
        );
    }

    #[test]
    fn reveal_gap_fills_the_gap_above_between_and_below() {
        let shown = [2..3, 6..7];
        assert_eq!(reveal_gap(&shown, 0, 9), [0..3, 6..7]);
        assert_eq!(reveal_gap(&shown, 1, 9), [2..7]);
        assert_eq!(reveal_gap(&shown, 2, 9), [2..3, 6..9]);
    }

    #[test]
    fn reveal_gap_merges_touching_ranges() {
        assert_eq!(reveal_gap(&[0..2, 4..6, 7..9], 1, 9), [0..6, 7..9]);
        assert_eq!(reveal_gap(&[0..2, 3..5], 1, 5), [0..5]);
    }

    #[test]
    fn reveal_gap_ignores_an_empty_gap_and_a_bad_index() {
        let shown = [0..2, 2..4];
        assert_eq!(reveal_gap(&shown, 0, 9), shown);
        assert_eq!(reveal_gap(&shown, 1, 9), shown);
        assert_eq!(reveal_gap(&[0..9], 1, 9), [0..9]);
        assert_eq!(reveal_gap(&shown, 3, 9), shown);
    }

    #[test]
    fn crlf_gives_the_same_crumbs() {
        let lf = "# A\n\n## A1\n\npara\n\n# B\n\nlast\n";
        let crlf = lf.replace('\n', "\r\n");
        let shown = [2..3, 4..5];
        let (a, _) = plan_chunks(&facets(lf), lf, &shown, MAX_RANGES);
        let (b, _) = plan_chunks(&facets(&crlf), &crlf, &shown, MAX_RANGES);
        assert_eq!(a[0].crumbs, b[0].crumbs);
        assert_eq!(a[1].crumbs, b[1].crumbs);
        assert_eq!(labels(&b[0]), ["A", "A1"]);
        assert_eq!(labels(&b[1]), ["B"]);
    }

    #[test]
    fn reveal_gap_with_no_shown_ranges_fills_everything() {
        assert_eq!(reveal_gap(&[], 0, 5), [0..5]);
        assert!(reveal_gap(&[], 0, 0).is_empty());
    }

    #[test]
    fn crumbs_skip_levels_correctly() {
        let text = "# A\n\n### C\n\npara\n\n## B\n\nafter\n";
        let f = facets(text);
        assert_eq!(f.blocks.len(), 5);
        let (plans, _) = plan_chunks(&f, text, &[2..3, 4..5], MAX_RANGES);
        assert_eq!(labels(&plans[0]), ["A", "C"]);
        assert_eq!(labels(&plans[1]), ["A", "B"]);
    }
}
