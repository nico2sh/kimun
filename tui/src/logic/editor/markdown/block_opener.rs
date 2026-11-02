//! Context-free CommonMark block-opener shape detection.
//!
//! These heuristics are NOT a parser — `parsed_buffer.rs` (pulldown) is
//! the real classifier. This module exists only for the incremental-splice
//! structural guard in `view.rs::try_incremental_parse`, which needs to ask
//! a single question about a single line WITHOUT parsing: "does the prefix
//! shape of this line look like it opens (or closes) a block construct?".
//!
//! The guard compares the shape of a line BEFORE and AFTER an edit and bails
//! to a full rebuild on any flip — because a line gaining or losing a
//! lazy-continuable opener (list, blockquote, indented code, HTML block) can
//! reshape the document beyond any window the widener could splice. The
//! detection is deliberately approximate and conservative: a false positive
//! costs one harmless full rebuild, and the post-slice verify is the
//! correctness backstop if the heuristic ever diverges from pulldown.

/// Maximum leading-space indent before a block marker. CommonMark §4: a
/// construct marker indented 4+ spaces is instead indented code.
const MAX_BLOCK_INDENT: usize = 3;

/// Minimum run length of `` ` `` or `~` that opens a fenced code block
/// (CommonMark §4.5).
const FENCE_MIN_RUN: usize = 3;

/// Maximum digits in an ordered-list marker (CommonMark §5.2: `123456789.`
/// is a list, a 10-digit number is not).
const MAX_ORDERED_LIST_DIGITS: usize = 9;

/// The context-free block-opener shapes a single line exhibits. Each field
/// is independent: a line can satisfy more than one (they are not mutually
/// exclusive), and the splice guard bails when ANY field differs across an
/// edit, so comparing two `OpenerShape`s for equality reproduces the prior
/// per-shape flip checks exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OpenerShape {
    /// Fenced code marker: ≤3 indent then 3+ backticks or 3+ tildes.
    pub fence: bool,
    /// Setext underline: a line of only `=` or only `-` (CommonMark §4.3).
    pub setext_underline: bool,
    /// Indented code opener: 4+ leading spaces (or a leading tab) followed
    /// by non-whitespace content (CommonMark §4.4).
    pub indented_code: bool,
    /// HTML block opener: ≤3 indent then `<` (conservative — catches both
    /// block-level and inline HTML; CommonMark §4.6).
    pub html_block: bool,
    /// List marker: ≤3 indent then `-`/`*`/`+` or `N.`/`N)`, followed by
    /// whitespace AND non-whitespace content (CommonMark §5.2).
    pub list_marker: bool,
    /// Blockquote marker: ≤3 indent then `>` (CommonMark §5.1).
    pub blockquote: bool,
}

/// Classify the context-free block-opener shape(s) of a single line.
pub(crate) fn opener_shape(line: &str) -> OpenerShape {
    OpenerShape {
        fence: is_fence_marker(line),
        setext_underline: is_setext_underline(line),
        indented_code: is_indented_code(line),
        html_block: is_html_block_opener(line),
        list_marker: is_list_marker(line),
        blockquote: is_blockquote_marker(line),
    }
}

/// Leading-space count and the line with that indent stripped. Used by the
/// ≤3-indent block markers so the indent literal lives in one place.
fn split_indent(line: &str) -> (usize, &str) {
    let trimmed = line.trim_start_matches(' ');
    (line.len() - trimmed.len(), trimmed)
}

/// ≤3 indent then a run of `FENCE_MIN_RUN`+ backticks or tildes (no mixing).
fn is_fence_marker(line: &str) -> bool {
    let (indent, trimmed) = split_indent(line);
    if indent > MAX_BLOCK_INDENT {
        return false;
    }
    let run = |c: char| {
        trimmed.starts_with(c) && trimmed.chars().take_while(|&x| x == c).count() >= FENCE_MIN_RUN
    };
    run('`') || run('~')
}

/// A non-empty line of only `=` or only `-` (ignoring surrounding space).
fn is_setext_underline(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && (trimmed.chars().all(|c| c == '=') || trimmed.chars().all(|c| c == '-'))
}

/// 4+ leading spaces (or a leading tab) followed by non-whitespace content.
/// A whitespace-only line does NOT open indented code in pulldown's
/// tokenization, so `"    "` → `"    !"` is a real flip we must detect.
fn is_indented_code(line: &str) -> bool {
    if let Some(rest) = line.strip_prefix('\t') {
        return !rest.trim().is_empty();
    }
    let leading_spaces = line.chars().take_while(|c| *c == ' ').count();
    if leading_spaces <= MAX_BLOCK_INDENT {
        return false;
    }
    !line[leading_spaces..].trim().is_empty()
}

/// ≤3 indent then `<`.
fn is_html_block_opener(line: &str) -> bool {
    let (indent, trimmed) = split_indent(line);
    indent <= MAX_BLOCK_INDENT && trimmed.starts_with('<')
}

/// ≤3 indent then `>`.
fn is_blockquote_marker(line: &str) -> bool {
    let (indent, trimmed) = split_indent(line);
    indent <= MAX_BLOCK_INDENT && trimmed.starts_with('>')
}

/// ≤3 indent then an unordered (`-`/`*`/`+`) or ordered (`N.`/`N)`, N ≤ 9
/// digits) marker, followed by whitespace AND non-whitespace content.
///
/// The "non-whitespace content after the marker" requirement mirrors
/// pulldown: a marker followed by only trailing whitespace (`"- "`,
/// `"*     "`) is a paragraph, not a list, so a flip from that to
/// `"- x"` promotes the row to a list and must be detected.
fn is_list_marker(line: &str) -> bool {
    let (indent, trimmed) = split_indent(line);
    if indent > MAX_BLOCK_INDENT {
        return false;
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if matches!(first, '-' | '*' | '+') {
        if !matches!(chars.next(), Some(' ' | '\t')) {
            return false;
        }
        return chars.any(|c| !c.is_whitespace());
    }
    if first.is_ascii_digit() {
        let mut digits = 1usize;
        let mut next = chars.next();
        while let Some(c) = next
            && c.is_ascii_digit()
        {
            digits += 1;
            if digits > MAX_ORDERED_LIST_DIGITS {
                return false;
            }
            next = chars.next();
        }
        if !matches!(next, Some('.' | ')')) {
            return false;
        }
        if !matches!(chars.next(), Some(' ' | '\t')) {
            return false;
        }
        return chars.any(|c| !c.is_whitespace());
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indented_code_detects_tab_and_4_spaces() {
        assert!(opener_shape("\tcode").indented_code);
        assert!(opener_shape("    code").indented_code);
        assert!(opener_shape("     code").indented_code);
        assert!(!opener_shape("   code").indented_code);
        assert!(!opener_shape("code").indented_code);
    }

    #[test]
    fn indented_code_rejects_whitespace_only() {
        // Whitespace-only lines never open indented code in pulldown's
        // tokenization, so the predicate requires non-whitespace content
        // after the 4+ space (or tab) prefix.
        assert!(!opener_shape("    ").indented_code);
        assert!(!opener_shape("     ").indented_code);
        assert!(!opener_shape("\t").indented_code);
        assert!(!opener_shape("\t   ").indented_code);
        assert!(opener_shape("    x").indented_code);
        assert!(opener_shape("\tx").indented_code);
    }

    #[test]
    fn list_marker_recognizes_pulldown_pattern() {
        // A marker followed by ONLY whitespace is a paragraph, not a list.
        assert!(opener_shape("* a").list_marker);
        assert!(opener_shape("- a").list_marker);
        assert!(opener_shape("+ a").list_marker);
        assert!(opener_shape("1. a").list_marker);
        assert!(opener_shape("12) a").list_marker);
        assert!(opener_shape("   * a").list_marker);
        assert!(!opener_shape("*").list_marker);
        assert!(!opener_shape("* ").list_marker);
        assert!(!opener_shape("*     ").list_marker);
        assert!(!opener_shape("1.").list_marker);
        assert!(!opener_shape("1. ").list_marker);
        assert!(!opener_shape("    * a").list_marker); // 4-space indent = code
        assert!(!opener_shape("1234567890. a").list_marker); // > 9 digits
    }

    #[test]
    fn blockquote_marker_recognizes_leading_gt() {
        assert!(opener_shape(">").blockquote);
        assert!(opener_shape("> a").blockquote);
        assert!(opener_shape("   > a").blockquote);
        assert!(!opener_shape("    > a").blockquote); // 4-space indent = code
        assert!(!opener_shape("a > b").blockquote);
        assert!(!opener_shape("").blockquote);
    }

    #[test]
    fn html_block_opener_detects_leading_lt() {
        assert!(opener_shape("<div>").html_block);
        assert!(opener_shape("  <div>").html_block);
        assert!(opener_shape("   <table>").html_block);
        assert!(!opener_shape("    <div>").html_block); // 4-space indent = code
        assert!(!opener_shape("text <span>").html_block);
    }

    #[test]
    fn fence_marker_requires_three_run() {
        assert!(opener_shape("```").fence);
        assert!(opener_shape("~~~rust").fence);
        assert!(opener_shape("   ```").fence);
        assert!(!opener_shape("``").fence);
        assert!(!opener_shape("    ```").fence); // 4-space indent = code
        assert!(!opener_shape("``~").fence); // no mixing / short run
    }

    #[test]
    fn setext_underline_all_same_char() {
        assert!(opener_shape("===").setext_underline);
        assert!(opener_shape("---").setext_underline);
        assert!(opener_shape("  ==  ").setext_underline);
        assert!(!opener_shape("=-=").setext_underline);
        assert!(!opener_shape("").setext_underline);
    }

    #[test]
    fn plain_prose_has_no_shape() {
        assert_eq!(opener_shape("just some words"), OpenerShape::default());
        assert_eq!(opener_shape(""), OpenerShape::default());
    }

    #[test]
    fn equality_detects_any_flip() {
        // The guard relies on `!=` catching a flip in any single field.
        assert_ne!(opener_shape("x"), opener_shape("> x")); // blockquote gained
        assert_ne!(opener_shape("x"), opener_shape("- x")); // list gained
        assert_ne!(opener_shape("text"), opener_shape("```")); // fence gained
        // Pure content edit inside a list item: shape unchanged.
        assert_eq!(opener_shape("- a"), opener_shape("- ab"));
    }
}
