//! Shared text helpers for the note-preview surfaces (the Query panel's context
//! preview and the note browser's preview pane). Both highlight query needles in
//! a note's body and wrap long lines; the case-insensitive matching is the one
//! piece that is both subtle (must stay on character boundaries for every
//! non-ASCII case fold) and was previously implemented twice — once correctly,
//! once dropping highlights for length-changing folds. It now lives in
//! `logic::text_match`; each surface keeps its own span styling.

/// Walk `line` against precomputed non-overlapping `ranges` (from
/// `crate::logic::text_match::match_ranges`, ascending) and emit one item per segment via
/// `mk(slice, is_match)` — alternating non-match gaps and matched spans, plus
/// the trailing gap. Empty `ranges` yields the whole line as a single non-match
/// segment. Shared so the gap/match/tail splitting lives in one place; each
/// caller supplies its own constructor (owned `'static` spans vs borrowed `'a`
/// spans, with its own styles).
pub fn style_ranges<'a, T>(
    line: &'a str,
    ranges: &[(usize, usize)],
    mut mk: impl FnMut(&'a str, bool) -> T,
) -> Vec<T> {
    let mut out = Vec::new();
    let mut pos = 0;
    for &(start, end) in ranges {
        if start > pos {
            out.push(mk(&line[pos..start], false));
        }
        out.push(mk(&line[start..end], true));
        pos = end;
    }
    if pos < line.len() {
        out.push(mk(&line[pos..], false));
    }
    out
}

/// Wrap `line` into pieces that each fit within `max_width` *characters* (not
/// bytes). Breaks at word boundaries when possible, hard-breaks an
/// over-long word otherwise. A `max_width` of 0 returns the line unchanged.
pub fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || line.chars().count() <= max_width {
        return vec![line.to_string()];
    }

    let mut result = Vec::new();
    let mut remaining = line;

    while remaining.chars().count() > max_width {
        // Byte index of the `max_width`-th character.
        let byte_limit = remaining
            .char_indices()
            .nth(max_width)
            .map(|(i, _)| i)
            .unwrap_or(remaining.len());

        // Prefer a space within the allowed range; hard-break if none.
        let break_at = remaining[..byte_limit]
            .rfind(' ')
            .map(|i| i + 1) // keep the space on the current line
            .unwrap_or(byte_limit);
        result.push(remaining[..break_at].trim_end().to_string());
        remaining = &remaining[break_at..];
    }
    if !remaining.is_empty() {
        result.push(remaining.to_string());
    }
    result
}

/// Soft-wrap `line` for a monospace view `columns` cells wide: the byte
/// ranges of its rows, in order, together covering the whole line (so a
/// match's byte range can be cut to the rows it spans, see
/// [`split_ranges`]).
///
/// Widths are display cells (`unicode-width`): a wide character (CJK) takes
/// two, a combining mark none; a grapheme cluster is never split. A row
/// breaks after the last whitespace that lets the next word fit; a word wider
/// than `columns` breaks by character. Whitespace never forces a break: it
/// hangs at the end of its row (invisible), so a row's visible text fits
/// `columns`; the one exception is a single grapheme wider than the whole
/// row, which gets a row of its own. An empty line, or `columns` 0, is one
/// row.
pub fn wrap_ranges(line: &str, columns: usize) -> Vec<std::ops::Range<usize>> {
    use unicode_segmentation::UnicodeSegmentation as _;
    use unicode_width::UnicodeWidthStr as _;

    if columns == 0 || line.is_empty() {
        return std::iter::once(0..line.len()).collect();
    }
    let mut rows = Vec::new();
    let mut start = 0;
    let mut width = 0;
    // Just after the last whitespace in the current row, and the row's width
    // up to there: where a word that does not fit moves to the next row.
    let mut space: Option<(usize, usize)> = None;
    for (at, g) in line.grapheme_indices(true) {
        let w = g.width();
        if g.chars().all(char::is_whitespace) {
            width += w;
            space = Some((at + g.len(), width));
            continue;
        }
        while width + w > columns && at > start {
            match space.take() {
                Some((after, upto)) if after > start => {
                    rows.push(start..after);
                    start = after;
                    width -= upto;
                }
                _ => {
                    rows.push(start..at);
                    start = at;
                    width = 0;
                }
            }
        }
        width += w;
    }
    rows.push(start..line.len());
    rows
}

/// The parts of `ranges` (byte ranges into a line, ascending) that fall in
/// `row` (a byte range of the same line, e.g. from [`wrap_ranges`]),
/// relative to the row's start. A range spanning several rows is cut at each
/// row edge; an empty row has none.
pub fn split_ranges(ranges: &[(usize, usize)], row: std::ops::Range<usize>) -> Vec<(usize, usize)> {
    ranges
        .iter()
        .filter(|&&(s, e)| s < row.end && e > row.start)
        .map(|&(s, e)| (s.max(row.start) - row.start, e.min(row.end) - row.start))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::text_match::match_ranges;

    fn needles(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn style_ranges_alternates_gaps_and_matches() {
        let line = "see widget and gadget";
        let ranges = match_ranges(line, &needles(&["widget", "gadget"]));
        let segs: Vec<(String, bool)> = style_ranges(line, &ranges, |s, hit| (s.to_string(), hit));
        // Every match becomes a `true` segment, gaps `false` — not just the first.
        assert_eq!(
            segs,
            vec![
                ("see ".to_string(), false),
                ("widget".to_string(), true),
                (" and ".to_string(), false),
                ("gadget".to_string(), true),
            ]
        );
    }

    #[test]
    fn style_ranges_empty_is_one_non_match_segment() {
        let segs: Vec<(String, bool)> =
            style_ranges("no matches here", &[], |s, hit| (s.to_string(), hit));
        assert_eq!(segs, vec![("no matches here".to_string(), false)]);
    }

    #[test]
    fn wrap_line_fits_within_width() {
        assert_eq!(wrap_line("short", 20), vec!["short"]);
    }

    #[test]
    fn wrap_line_breaks_at_word_boundary() {
        assert_eq!(
            wrap_line("hello world foo bar", 12),
            vec!["hello world", "foo bar"]
        );
    }

    #[test]
    fn wrap_line_hard_breaks_long_word() {
        assert_eq!(wrap_line("abcdefghij", 5), vec!["abcde", "fghij"]);
    }

    #[test]
    fn wrap_line_handles_multibyte_chars() {
        assert_eq!(wrap_line("日本語テスト", 3), vec!["日本語", "テスト"]);
    }

    #[test]
    fn wrap_line_empty_string() {
        assert_eq!(wrap_line("", 10), vec![""]);
    }

    /// Each row of `wrap_ranges` as text.
    fn rows(line: &str, columns: usize) -> Vec<&str> {
        wrap_ranges(line, columns)
            .into_iter()
            .map(|r| &line[r])
            .collect()
    }

    #[test]
    fn wrap_ranges_breaks_after_the_space_and_keeps_it() {
        assert_eq!(rows("hello world foo bar", 12), ["hello world ", "foo bar"]);
        assert_eq!(rows("short", 20), ["short"]);
    }

    #[test]
    fn wrap_ranges_cover_the_whole_line_in_order() {
        let line = "the quick  brown fox jumps over   the lazy dog";
        for columns in 1..=line.len() + 1 {
            let ranges = wrap_ranges(line, columns);
            assert_eq!(ranges.first().unwrap().start, 0);
            assert_eq!(ranges.last().unwrap().end, line.len());
            assert!(ranges.windows(2).all(|w| w[0].end == w[1].start));
            for r in &ranges {
                assert!(
                    line[r.clone()].trim_end().len() <= columns,
                    "{columns}: {:?}",
                    &line[r.clone()]
                );
            }
        }
    }

    #[test]
    fn wrap_ranges_exact_width_is_one_row() {
        assert_eq!(rows("abcde", 5), ["abcde"]);
        // A space after a full row hangs; the next word starts the next row.
        assert_eq!(rows("abcde f", 5), ["abcde ", "f"]);
        assert_eq!(rows("abcdef", 5), ["abcde", "f"]);
    }

    #[test]
    fn wrap_ranges_hard_break_an_unbroken_line() {
        assert_eq!(rows("abcdefghijk", 4), ["abcd", "efgh", "ijk"]);
        // A long word after a short one: the short one keeps its row.
        assert_eq!(rows("ab cdefghij", 4), ["ab ", "cdef", "ghij"]);
    }

    #[test]
    fn wrap_ranges_count_wide_characters_as_two_cells() {
        assert_eq!(rows("日本語テスト", 4), ["日本", "語テ", "スト"]);
        assert_eq!(rows("日本語テスト", 5), ["日本", "語テ", "スト"]);
        // A wide character in a row too narrow for it still gets a row.
        assert_eq!(rows("日本", 1), ["日", "本"]);
        assert_eq!(rows("a日b", 2), ["a", "日", "b"]);
    }

    #[test]
    fn wrap_ranges_keep_combining_marks_with_their_letter() {
        // `e` + U+0301 is one cell, one grapheme: never split.
        let line = "cafe\u{301}s";
        assert_eq!(rows(line, 4), ["cafe\u{301}", "s"]);
        assert_eq!(rows(line, 5), [line]);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // one row, as a range
    fn wrap_ranges_empty_line_and_zero_columns() {
        assert_eq!(wrap_ranges("", 10), [0..0]);
        assert_eq!(wrap_ranges("abc", 0), [0..3]);
        assert_eq!(rows("   ", 1), ["   "], "blanks never force a break");
    }

    #[test]
    fn split_ranges_cuts_a_match_across_rows() {
        let line = "abcd efgh";
        let hits = match_ranges(line, &needles(&["cd ef", "h"]));
        let parts: Vec<Vec<&str>> = wrap_ranges(line, 4)
            .into_iter()
            .map(|row| {
                let text = &line[row.clone()];
                split_ranges(&hits, row)
                    .into_iter()
                    .map(|(s, e)| &text[s..e])
                    .collect()
            })
            .collect();
        assert_eq!(parts, vec![vec!["cd "], vec!["ef", "h"]]);
        assert!(split_ranges(&hits, 0..0).is_empty());
    }
}
