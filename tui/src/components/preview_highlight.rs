//! Shared text helpers for the note-preview surfaces (the Query panel's context
//! preview and the note browser's preview pane). Both highlight query needles in
//! a note's body and wrap long lines; the case-insensitive matching is the one
//! piece that is both subtle (must stay on character boundaries for every
//! non-ASCII case fold) and was previously implemented twice — once correctly,
//! once dropping highlights for length-changing folds. It lives here now; each
//! surface keeps its own span styling.

/// Non-overlapping byte ranges in `haystack` where any of `needles` matches,
/// case-insensitively, earliest- and longest-first.
///
/// Matching is character-based, so every returned offset is a real `char`
/// boundary of `haystack` — slicing `haystack[start..end]` never panics, even
/// when a case fold changes byte length (`İ`, `ẞ`) or shifts boundaries. Empty
/// needles contribute nothing. Per needle, matches are non-overlapping (like
/// `str::match_indices`); across needles, a later range that overlaps one
/// already kept is dropped, with the longest range at each start winning.
pub fn match_ranges(haystack: &str, needles: &[String]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for needle in needles {
        collect_needle(haystack, needle, &mut ranges);
    }
    // Longest match first at each start, so an overlapping shorter needle never
    // truncates a longer one.
    ranges.sort_unstable_by_key(|(s, e)| (*s, std::cmp::Reverse(*e)));
    ranges.dedup();

    let mut kept: Vec<(usize, usize)> = Vec::new();
    let mut pos = 0;
    for (start, end) in ranges {
        if start < pos {
            continue; // overlaps a range already kept
        }
        kept.push((start, end));
        pos = end;
    }
    kept
}

/// Append every non-overlapping case-insensitive occurrence of `needle` in
/// `haystack` to `out`, as byte ranges derived from character indices.
fn collect_needle(haystack: &str, needle: &str, out: &mut Vec<(usize, usize)>) {
    let needle_chars: Vec<char> = needle.chars().collect();
    if needle_chars.is_empty() {
        return;
    }
    let hay: Vec<(usize, char)> = haystack.char_indices().collect();
    let n = needle_chars.len();
    let mut i = 0;
    while i + n <= hay.len() {
        if (0..n).all(|j| chars_eq_ignore_case(hay[i + j].1, needle_chars[j])) {
            let start = hay[i].0;
            let end = hay
                .get(i + n)
                .map(|(b, _)| *b)
                .unwrap_or_else(|| haystack.len());
            out.push((start, end));
            i += n; // non-overlapping, matching `str::match_indices`
        } else {
            i += 1;
        }
    }
}

/// Case-insensitive single-character compare that handles multi-character folds
/// (e.g. `ẞ`/`ß`) by comparing the full lowercase mappings.
fn chars_eq_ignore_case(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
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

    fn needles(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn matches_are_case_insensitive() {
        assert_eq!(match_ranges("Hello World", &needles(&["world"])), [(6, 11)]);
        assert_eq!(match_ranges("HELLO", &needles(&["hell"])), [(0, 4)]);
    }

    #[test]
    fn all_occurrences_per_needle() {
        // `aa` appears at byte 0 and 3 (non-overlapping, like match_indices).
        assert_eq!(match_ranges("aa aa", &needles(&["aa"])), [(0, 2), (3, 5)]);
    }

    #[test]
    fn overlapping_needles_keep_longest() {
        // "foobar" and "foo" both start at 0; the longer wins, the shorter is
        // dropped as overlapping.
        let r = match_ranges("foobar", &needles(&["foo", "foobar"]));
        assert_eq!(r, [(0, 6)]);
    }

    #[test]
    fn empty_needles_contribute_nothing() {
        assert!(match_ranges("anything", &needles(&[""])).is_empty());
        assert!(match_ranges("anything", &[]).is_empty());
    }

    #[test]
    fn ascii_needle_matches_on_line_containing_a_length_changing_fold() {
        // `İ` (U+0130) lowercases to a longer string, so the old
        // `lower.len() != line.len()` bail dropped highlighting for the WHOLE
        // line — including the plain ASCII "there". Char-based matching finds
        // it regardless, on real char boundaries (no panic).
        let hay = "Hİ there";
        let r = match_ranges(hay, &needles(&["there"]));
        assert_eq!(r.len(), 1, "ascii needle must still match: {r:?}");
        let (s, e) = r[0];
        assert!(hay.is_char_boundary(s) && hay.is_char_boundary(e));
        assert_eq!(&hay[s..e], "there");
    }

    #[test]
    fn multibyte_haystack_offsets_are_valid() {
        let hay = "日本語テスト";
        let r = match_ranges(hay, &needles(&["テスト"]));
        assert_eq!(r.len(), 1);
        let (s, e) = r[0];
        assert_eq!(&hay[s..e], "テスト");
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
