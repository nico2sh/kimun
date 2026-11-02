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
}
