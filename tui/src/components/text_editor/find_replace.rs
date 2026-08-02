//! The pure half of find-and-replace: compiling a **find pattern**, counting
//! its matches, expanding a replacement, and building the substituted lines a
//! **replace preview** draws.
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
    /// the replacement.
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
    pub fn count_matches<S: AsRef<str>>(&self, rows: impl Iterator<Item = S>) -> usize {
        rows.map(|row| self.re.find_iter(row.as_ref()).count())
            .sum()
    }

    /// Every match as a `(row, start_char, end_char)` span in **logical**
    /// buffer coordinates.
    ///
    /// The same coordinates `count_matches` and the textarea's stepping use, so
    /// highlighting built from these cannot disagree with them about what
    /// matched — the way matching against rendered cell text does the moment a
    /// row contains concealed markdown.
    pub fn match_spans<S: AsRef<str>>(
        &self,
        rows: impl Iterator<Item = S>,
    ) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        for (row, line) in rows.enumerate() {
            let line = line.as_ref();
            for m in self.re.find_iter(line) {
                let start = line[..m.start()].chars().count();
                let end = start + line[m.range()].chars().count();
                out.push((row, start, end));
            }
        }
        out
    }

    /// Expand `replacement` for one match.
    ///
    /// Capture syntax (`$1`, `${name}`) is honoured **only when the pattern
    /// captures**. Otherwise the replacement is literal, so a `$`
    /// in ordinary note content — a price, inline LaTeX — survives instead of
    /// silently expanding to the empty string.
    ///
    /// Even when the pattern captures, a `$` that names a group which does not
    /// exist stays literal. `Captures::expand` would erase it: in a note
    /// `$100` parses as group 100 and `$x^2$` as a group named `x`, and both
    /// expand to nothing. Gating on whether the pattern captures at all is not
    /// enough — a user who writes a capture group is exactly the user who then
    /// writes `$1 costs $100`.
    pub fn expand(&self, caps: &regex::Captures<'_>, replacement: &str) -> String {
        if !self.has_captures {
            return replacement.to_string();
        }
        let mut out = String::new();
        let mut rest = replacement;
        while let Some(dollar) = rest.find('$') {
            out.push_str(&rest[..dollar]);
            let tail = &rest[dollar..];
            // `$$` is regex-crate syntax for a literal `$`; keep honouring it.
            if let Some(after) = tail.strip_prefix("$$") {
                out.push('$');
                rest = after;
                continue;
            }
            // Ask the crate to expand this one reference in isolation. If it
            // resolves to nothing AND the group does not exist, the reference
            // was never a reference — emit it as typed.
            let end = reference_end(tail);
            let reference = &tail[..end];
            let mut expanded = String::new();
            caps.expand(reference, &mut expanded);
            if expanded.is_empty() && !group_exists(caps, reference) {
                out.push_str(reference);
            } else {
                out.push_str(&expanded);
            }
            rest = &tail[end..];
        }
        out.push_str(rest);
        out
    }
}

/// Byte length of the capture reference starting at `s[0] == '$'`, mirroring
/// the `regex` crate's own grammar: `${...}` up to the brace, otherwise the
/// run of `[0-9A-Za-z_]` after the sigil. A bare `$` with nothing referenceable
/// after it is length 1.
fn reference_end(s: &str) -> usize {
    debug_assert!(s.starts_with('$'));
    if let Some(rest) = s.strip_prefix("${") {
        return match rest.find('}') {
            Some(close) => 2 + close + 1,
            None => s.len(),
        };
    }
    let name_len = s[1..]
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(s.len() - 1);
    1 + name_len
}

/// Whether `reference` (a single `$…` capture reference) names a group that
/// actually exists in `caps` — the difference between "this group matched
/// nothing" and "this was never a group".
fn group_exists(caps: &regex::Captures<'_>, reference: &str) -> bool {
    let name = reference
        .trim_start_matches('$')
        .trim_start_matches('{')
        .trim_end_matches('}');
    if name.is_empty() {
        return false;
    }
    match name.parse::<usize>() {
        Ok(index) => index < caps.len(),
        Err(_) => caps.name(name).is_some(),
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
pub fn build_preview<S: AsRef<str>>(
    pattern: &FindPattern,
    rows: impl Iterator<Item = S>,
    replacement: &str,
    current: Option<(usize, usize)>,
) -> Preview {
    let mut out_lines = Vec::new();
    let mut spans = Vec::new();

    for (row, line) in rows.enumerate() {
        let line = line.as_ref();
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
    let preview = build_preview(pattern, lines.iter(), replacement, None);
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
        assert_eq!(p.count_matches(lines(&["todo Todo TODO"]).iter()), 3);
    }

    #[test]
    fn pattern_with_uppercase_is_exact() {
        let p = FindPattern::compile("Todo").unwrap();
        assert!(p.case_sensitive());
        assert_eq!(p.count_matches(lines(&["todo Todo TODO"]).iter()), 1);
    }

    #[test]
    fn user_written_inline_flag_overrides_smartcase() {
        // Lowercase query, so smartcase prepends `(?i)` — the user's `(?-i)`
        // comes later in the pattern and must win.
        let p = FindPattern::compile("(?-i)todo").unwrap();
        assert_eq!(p.count_matches(lines(&["todo Todo TODO"]).iter()), 1);
    }

    // ── capture gating ──────────────────────────────────────────

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
    fn a_dollar_naming_no_group_stays_literal_even_when_the_pattern_captures() {
        // The capture gate alone protects only patterns with no groups. A user
        // who writes a group is exactly the user who then writes a price.
        let p = FindPattern::compile(r"(Total)").unwrap();
        let (out, _) = replace_all(&p, &lines(&["Total: 5 due"]), "$1 cost $100").unwrap();
        assert_eq!(out, lines(&["Total cost $100: 5 due"]));
    }

    #[test]
    fn inline_latex_survives_a_capturing_pattern() {
        let p = FindPattern::compile(r"(area)").unwrap();
        let (out, _) = replace_all(&p, &lines(&["the area"]), "$1 $x^2$").unwrap();
        assert_eq!(out, lines(&["the area $x^2$"]));
    }

    #[test]
    fn braced_and_named_references_still_expand() {
        let p = FindPattern::compile(r"(?<word>\w+)-(\d+)").unwrap();
        let (out, _) = replace_all(&p, &lines(&["ab-12"]), "${word}/$2").unwrap();
        assert_eq!(out, lines(&["ab/12"]));
    }

    #[test]
    fn double_dollar_is_still_an_escape() {
        let p = FindPattern::compile(r"(x)").unwrap();
        let (out, _) = replace_all(&p, &lines(&["x"]), "$$1").unwrap();
        assert_eq!(out, lines(&["$1"]));
    }

    #[test]
    fn a_group_that_matched_nothing_expands_to_nothing() {
        // Distinct from a group that does not exist: this one is real and
        // simply matched the empty string, so erasing it is correct.
        let p = FindPattern::compile(r"a(z*)").unwrap();
        let (out, _) = replace_all(&p, &lines(&["a"]), "[$1]").unwrap();
        assert_eq!(out, lines(&["[]"]));
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
        let pv = build_preview(&p, lines(&["ab-ab"]).iter(), "XYZW", None);
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
        let pv = build_preview(&p, lines(&["ab-ab"]).iter(), "X", Some((0, 3)));
        assert_eq!(
            pv.spans.iter().map(|s| s.is_current).collect::<Vec<_>>(),
            vec![false, true]
        );
    }

    #[test]
    fn preview_handles_multibyte_content() {
        let p = FindPattern::compile("é").unwrap();
        let pv = build_preview(&p, lines(&["aéb"]).iter(), "ü", None);
        assert_eq!(pv.lines, lines(&["aüb"]));
        // Char columns, not byte offsets.
        assert_eq!(pv.spans[0].start, 1);
        assert_eq!(pv.spans[0].end, 2);
    }

    #[test]
    fn preview_with_captures_differs_per_match() {
        // The case that makes previewing only the current match misleading.
        let p = FindPattern::compile(r"(\w)(\d)").unwrap();
        let pv = build_preview(&p, lines(&["a1 b2"]).iter(), "$2$1", None);
        assert_eq!(pv.lines, lines(&["1a 2b"]));
    }

    #[test]
    fn zero_width_pattern_terminates() {
        let p = FindPattern::compile(r"\b").unwrap();
        let pv = build_preview(&p, lines(&["hi there"]).iter(), "|", None);
        assert_eq!(pv.lines, lines(&["|hi| |there|"]));
    }

    #[test]
    fn empty_lines_survive_preview() {
        let p = FindPattern::compile("x").unwrap();
        let pv = build_preview(&p, lines(&["", "x", ""]).iter(), "y", None);
        assert_eq!(pv.lines, lines(&["", "y", ""]));
    }

    #[test]
    fn invalid_pattern_reports_error() {
        assert!(FindPattern::compile("[").is_err());
    }
}
