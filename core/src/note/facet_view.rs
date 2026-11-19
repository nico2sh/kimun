//! Planning what a facet view draws: the chunks of source text for the shown
//! block ranges, and the reveal of one hidden block next to a chunk. See
//! `CONTEXT.md`, **Facet view**.

use std::ops::Range;

use super::facets::NoteFacets;

/// The default most chunks a caller asks `plan_chunks` for.
pub const MAX_RANGES: usize = 100;

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
        chunk.push_str(&text[first.range.start..last.range.end]);
        let above_from = if at == 0 { 0 } else { shown[at - 1].end };
        let below_to = shown.get(at + 1).map_or(blocks.len(), |next| next.start);
        plans.push(ChunkPlan {
            blocks: range.clone(),
            text: chunk,
            hidden_above: range.start.saturating_sub(above_from),
            hidden_below: below_to.saturating_sub(range.end),
        });
    }
    (plans, shown.len().saturating_sub(limit))
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
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(out.len());
    for r in out {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
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
}
