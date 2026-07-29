//! The pure half of find-and-replace: compiling a **find pattern**, counting
//! its matches, expanding a replacement, and building the substituted lines a
//! **replace preview** draws (adr/0033, adr/0034, adr/0035).
//!
//! Nothing here touches a `TextArea`, a `Frame`, or the editor — it is
//! `&[String]` in, values out, so the semantics that are easy to get wrong
//! (smartcase, capture gating, span remapping) are testable with literals.
//!
//! Matching is per line, because that is what the textarea's search engine
//! does: a pattern can never span a newline, and `^`/`$` anchor per line.

use regex::Regex;

/// A compiled **find pattern**, plus the case decision that produced it.
#[derive(Debug, Clone)]
pub struct FindPattern {
    re: Regex,
    /// True when the pattern was compiled case-sensitively — i.e. the user
    /// typed at least one uppercase character. Surfaced in the find bar so
    /// smartcase is never a silent decision.
    case_sensitive: bool,
    /// True when the pattern captures, which is what gates `$1` expansion in
    /// the replacement (adr/0034).
    has_captures: bool,
}

impl FindPattern {
    /// Compile `query` under **smartcase**: an all-lowercase query matches any
    /// case, any uppercase makes it exact.
    ///
    /// The `(?i)` is prepended rather than set via `RegexBuilder` so a
    /// user-written inline flag (`(?-i)`, or a scoped `(?i:…)`) still wins —
    /// later inline flags override earlier ones.
    pub fn compile(query: &str) -> Result<Self, regex::Error> {
        let case_sensitive = query.chars().any(char::is_uppercase);
        let re = if case_sensitive {
            Regex::new(query)?
        } else {
            Regex::new(&format!("(?i){query}"))?
        };
        // `captures_len` counts the implicit whole-match group, so >1 means the
        // pattern has at least one real capture. Non-capturing `(?:…)` groups
        // do not count, which is the correct reading — they group, they do not
        // capture.
        let has_captures = re.captures_len() > 1;
        Ok(Self {
            re,
            case_sensitive,
            has_captures,
        })
    }

    pub fn as_regex(&self) -> &Regex {
        &self.re
    }

    pub fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    pub fn has_captures(&self) -> bool {
        self.has_captures
    }

    /// Total matches across every line. Cheap enough to run per keystroke:
    /// this is `find_iter` over strings the editor already holds, not the
    /// cell-by-cell row reconstruction that `paint_viewport_extras` avoids.
    pub fn count_matches(&self, lines: &[String]) -> usize {
        lines.iter().map(|l| self.re.find_iter(l).count()).sum()
    }

    /// Expand `replacement` for one match.
    ///
    /// Capture syntax (`$1`, `${name}`) is honoured **only when the pattern
    /// captures** (adr/0034). Otherwise the replacement is literal, so a `$`
    /// in ordinary note content — a price, inline LaTeX — survives instead of
    /// silently expanding to the empty string.
    pub fn expand(&self, caps: &regex::Captures<'_>, replacement: &str) -> String {
        if !self.has_captures {
            return replacement.to_string();
        }
        let mut out = String::new();
        caps.expand(replacement, &mut out);
        out
    }
}

/// One match rewritten inside a previewed line, in that line's **preview**
/// coordinates (char columns), not the buffer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewSpan {
    pub row: usize,
    /// Char column where the replacement text starts.
    pub start: usize,
    /// Char column just past the replacement text.
    pub end: usize,
    /// Whether this is the **current match** — the one `Enter` rewrites, as
    /// against the ones only `Ctrl+A` reaches.
    pub is_current: bool,
}

/// The result of substituting every match into a copy of the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub lines: Vec<String>,
    pub spans: Vec<PreviewSpan>,
}

/// Build the **replace preview**: every match replaced, plus where each
/// replacement landed so the renderer can colour it.
///
/// `current` is the buffer-coordinate `(row, char_col)` start of the current
/// match, so the span covering it can be flagged. It is passed in buffer
/// coordinates because that is what the editor knows; the remap to preview
/// coordinates happens here, where the length delta is being accumulated
/// anyway.
///
/// The line *count* never changes — the pattern cannot span a newline and the
/// replace field is single-line — which is why callers may keep their scroll
/// offset and row indices across a preview.
pub fn build_preview(
    pattern: &FindPattern,
    lines: &[String],
    replacement: &str,
    current: Option<(usize, usize)>,
) -> Preview {
    let mut out_lines = Vec::with_capacity(lines.len());
    let mut spans = Vec::new();

    for (row, line) in lines.iter().enumerate() {
        let mut rebuilt = String::with_capacity(line.len());
        // Byte cursor into the ORIGINAL line; char cursor into the REBUILT one.
        let mut last_byte = 0usize;
        let mut out_chars = 0usize;

        for caps in pattern.as_regex().captures_iter(line) {
            let m = caps.get(0).expect("group 0 always exists");
            let gap = &line[last_byte..m.start()];
            rebuilt.push_str(gap);
            out_chars += gap.chars().count();

            let expanded = pattern.expand(&caps, replacement);
            let expanded_chars = expanded.chars().count();
            let match_start_chars = line[..m.start()].chars().count();
            let is_current = current == Some((row, match_start_chars));

            rebuilt.push_str(&expanded);
            spans.push(PreviewSpan {
                row,
                start: out_chars,
                end: out_chars + expanded_chars,
                is_current,
            });
            out_chars += expanded_chars;
            last_byte = m.end();

            // A zero-width match (e.g. `\b`, `x*`) would otherwise loop
            // forever on the same offset; `captures_iter` already advances,
            // but the byte cursor must not go backwards.
            if m.start() == m.end() && m.end() == last_byte {
                continue;
            }
        }

        rebuilt.push_str(&line[last_byte..]);
        out_lines.push(rebuilt);
    }

    Preview {
        lines: out_lines,
        spans,
    }
}

/// Rewrite every match in `lines`, returning the new lines and how many
/// matches were rewritten. The **replace all** primitive.
///
/// Returns `None` when nothing matched, so callers can distinguish "no work"
/// from "rewrote zero characters" without re-counting.
pub fn replace_all(
    pattern: &FindPattern,
    lines: &[String],
    replacement: &str,
) -> Option<(Vec<String>, usize)> {
    let preview = build_preview(pattern, lines, replacement, None);
    let count = preview.spans.len();
    if count == 0 {
        return None;
    }
    Some((preview.lines, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── smartcase ──────────────────────────────────────────────────────────

    #[test]
    fn lowercase_pattern_matches_any_case() {
        let p = FindPattern::compile("todo").unwrap();
        assert!(!p.case_sensitive());
        assert_eq!(p.count_matches(&lines(&["todo Todo TODO"])), 3);
    }

    #[test]
    fn pattern_with_uppercase_is_exact() {
        let p = FindPattern::compile("Todo").unwrap();
        assert!(p.case_sensitive());
        assert_eq!(p.count_matches(&lines(&["todo Todo TODO"])), 1);
    }

    #[test]
    fn user_written_inline_flag_overrides_smartcase() {
        // Lowercase query, so smartcase prepends `(?i)` — the user's `(?-i)`
        // comes later in the pattern and must win.
        let p = FindPattern::compile("(?-i)todo").unwrap();
        assert_eq!(p.count_matches(&lines(&["todo Todo TODO"])), 1);
    }

    // ── capture gating (adr/0034) ──────────────────────────────────────────

    #[test]
    fn dollar_is_literal_when_pattern_does_not_capture() {
        let p = FindPattern::compile("price").unwrap();
        assert!(!p.has_captures());
        let (out, n) = replace_all(&p, &lines(&["the price here"]), "$5").unwrap();
        assert_eq!(n, 1);
        assert_eq!(out, lines(&["the $5 here"]));
    }

    #[test]
    fn dollar_expands_when_pattern_captures() {
        let p = FindPattern::compile(r"(\w+)-(\w+)").unwrap();
        assert!(p.has_captures());
        let (out, _) = replace_all(&p, &lines(&["alpha-beta"]), "$2 $1").unwrap();
        assert_eq!(out, lines(&["beta alpha"]));
    }

    #[test]
    fn non_capturing_group_does_not_enable_expansion() {
        let p = FindPattern::compile(r"(?:foo)").unwrap();
        assert!(!p.has_captures());
        let (out, _) = replace_all(&p, &lines(&["foo"]), "$1").unwrap();
        assert_eq!(out, lines(&["$1"]));
    }

    // ── replace all ────────────────────────────────────────────────────────

    #[test]
    fn replace_all_rewrites_every_line() {
        let p = FindPattern::compile("a").unwrap();
        let (out, n) = replace_all(&p, &lines(&["aa", "b", "a"]), "x").unwrap();
        assert_eq!(n, 3);
        assert_eq!(out, lines(&["xx", "b", "x"]));
    }

    #[test]
    fn replace_all_reports_none_when_nothing_matches() {
        let p = FindPattern::compile("zzz").unwrap();
        assert!(replace_all(&p, &lines(&["abc"]), "x").is_none());
    }

    #[test]
    fn empty_replacement_deletes_matches() {
        let p = FindPattern::compile("todo ").unwrap();
        let (out, n) = replace_all(&p, &lines(&["todo todo done"]), "").unwrap();
        assert_eq!(n, 2);
        assert_eq!(out, lines(&["done"]));
    }

    #[test]
    fn replace_all_never_changes_the_line_count() {
        let p = FindPattern::compile("x").unwrap();
        let src = lines(&["x", "", "xx", "y"]);
        let (out, _) = replace_all(&p, &src, "longer").unwrap();
        assert_eq!(out.len(), src.len());
    }

    // ── preview spans ──────────────────────────────────────────────────────

    #[test]
    fn preview_spans_are_in_preview_coordinates_not_buffer_ones() {
        // "ab" -> "XYZW" shifts everything after the first match right by 2,
        // so the second span must not be reported at its buffer column.
        let p = FindPattern::compile("ab").unwrap();
        let pv = build_preview(&p, &lines(&["ab-ab"]), "XYZW", None);
        assert_eq!(pv.lines, lines(&["XYZW-XYZW"]));
        assert_eq!(pv.spans[0].start, 0);
        assert_eq!(pv.spans[0].end, 4);
        assert_eq!(pv.spans[1].start, 5);
        assert_eq!(pv.spans[1].end, 9);
    }

    #[test]
    fn preview_flags_the_current_match_by_buffer_position() {
        let p = FindPattern::compile("ab").unwrap();
        // Second match starts at buffer char col 3 of "ab-ab".
        let pv = build_preview(&p, &lines(&["ab-ab"]), "X", Some((0, 3)));
        assert_eq!(
            pv.spans.iter().map(|s| s.is_current).collect::<Vec<_>>(),
            vec![false, true]
        );
    }

    #[test]
    fn preview_handles_multibyte_content() {
        let p = FindPattern::compile("é").unwrap();
        let pv = build_preview(&p, &lines(&["aéb"]), "ü", None);
        assert_eq!(pv.lines, lines(&["aüb"]));
        // Char columns, not byte offsets.
        assert_eq!(pv.spans[0].start, 1);
        assert_eq!(pv.spans[0].end, 2);
    }

    #[test]
    fn preview_with_captures_differs_per_match() {
        // The case that makes previewing only the current match misleading.
        let p = FindPattern::compile(r"(\w)(\d)").unwrap();
        let pv = build_preview(&p, &lines(&["a1 b2"]), "$2$1", None);
        assert_eq!(pv.lines, lines(&["1a 2b"]));
    }

    #[test]
    fn zero_width_pattern_terminates() {
        let p = FindPattern::compile(r"\b").unwrap();
        let pv = build_preview(&p, &lines(&["hi there"]), "|", None);
        assert_eq!(pv.lines, lines(&["|hi| |there|"]));
    }

    #[test]
    fn empty_lines_survive_preview() {
        let p = FindPattern::compile("x").unwrap();
        let pv = build_preview(&p, &lines(&["", "x", ""]), "y", None);
        assert_eq!(pv.lines, lines(&["", "y", ""]));
    }

    #[test]
    fn invalid_pattern_reports_error() {
        assert!(FindPattern::compile("[").is_err());
    }
}
