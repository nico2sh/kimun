//! Terminal-free value types and pure helpers of the editor's public surface:
//! what following a link resolves to, which surface holds input, the clipboard
//! image, and the small text predicates the paste and autocomplete paths use.

use kimun_core::note::scan::LinkSpanKind;

/// What following resolves to — the **follow target** under the cursor.
///
/// Named for the action rather than the destination, because the destination is
/// not known here: a `Link` is still the raw string as written in the note,
/// with its kind, and only the follow (`logic::follow::resolve`) decides
/// whether it names a vault note, an external URL, or an attachment. Wider
/// than a **note link**, which is note→note only.
#[derive(Debug, Clone, PartialEq)]
pub enum FollowTarget {
    /// A wiki-link or markdown link (or image): the raw target string and
    /// the link's kind, which decides how the target reads — a wikilink
    /// names a note, a markdown link is a path.
    Link { target: String, kind: LinkSpanKind },
    /// A hashtag label with the name **without** the leading `#`.
    Label(String),
}

/// Which editor-internal surface currently holds input — the **editor claim**.
///
/// Read into the `Intent` classifier's snapshot so ownership is decided once,
/// there, instead of being re-asserted per event kind further down. The holder
/// is named rather than merely counted because what a claim blocks differs by
/// holder: the find bar blocks a paste, a click and a bare Space; the popup
/// wants all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorClaim {
    #[default]
    None,
    FindBar,
    Autocomplete,
}

/// The link or label target at char column `col` of `line`, if the column is
/// inside a wikilink, markdown link, or hashtag span.
pub(crate) fn follow_target_in_line(line: &str, col: usize) -> Option<FollowTarget> {
    // F5: Check wiki-link / markdown-link spans first; Link wins over Label
    // even if a future edit accidentally lets a Label slip through a Link range.
    if let Some(span) = kimun_core::note::scan::link_char_spans(line)
        .into_iter()
        .find(|s| s.start <= col && col < s.end)
    {
        return Some(FollowTarget::Link {
            target: span.target,
            kind: span.kind,
        });
    }

    // Fallback: check for a hashtag label (via the markdown parser).
    let parsed = crate::logic::editor::markdown::ParsedLine::parse(line);
    parsed
        .elements
        .iter()
        .find(|e| {
            e.kind == crate::logic::editor::markdown::ElementKind::Label
                && col >= e.start_char
                && col < e.end_char
        })
        .map(|e| {
            let span: String = line
                .chars()
                .skip(e.start_char)
                .take(e.end_char - e.start_char)
                .collect();
            let name = span.trim_start_matches('#').to_string();
            FollowTarget::Label(name)
        })
}

/// Returns true if any autocomplete trigger char (`[` for `[[wikilink`,
/// `#` for `#hashtag`) appears between the start of `line` and the
/// cursor's char column. Walks backwards from the cursor so the common
/// "user just typed inside a trigger" case short-circuits quickly. The
/// scan stays within one row because triggers can't cross a newline.
///
/// UTF-8 safe: takes a char column and never slices on a byte that is
/// not a codepoint boundary. Wikilinks can contain spaces
/// (`[[my note title`), so the walk does NOT stop at whitespace — only
/// the trigger char or start-of-row halts it.
pub fn has_trigger_before_cursor(line: &str, col: usize) -> bool {
    let cursor_byte = line
        .char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line.len());
    line[..cursor_byte]
        .chars()
        .rev()
        .any(|c| c == '[' || c == '#')
}

/// Convert a 0-based character column into a byte offset within `line`.
/// Out-of-range columns return `line.len()`.
pub(crate) fn char_col_to_byte(line: &str, char_col: usize) -> usize {
    line.char_indices()
        .nth(char_col)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
}

/// Owned RGBA image data lifted from the system clipboard. Returned by
/// `TextEditorComponent::take_clipboard_image` so the screen layer can
/// encode + persist without holding the editor's clipboard borrow.
#[derive(Debug, Clone)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Schemes the paste-over-selection flow recognises as "linkable" — broader
/// than `core::note::scan::is_remote_url` (http/https only) because users routinely paste
/// `mailto:` and FTP links and expect them wrapped as markdown links too.
const LINKABLE_PASTE_SCHEMES: &[&str] = &["http", "https", "ftp", "ftps", "mailto"];

/// Visual rows one mouse-wheel notch scrolls the editor.
pub(crate) const WHEEL_SCROLL_ROWS: isize = 1;

pub(crate) fn linkable_url(s: &str) -> Option<&str> {
    kimun_core::note::scan::url_with_allowed_scheme(s, LINKABLE_PASTE_SCHEMES)
}

/// If `clip` is a linkable URL and `selection` is non-empty, returns
/// `Some("[escaped_selection](url)")`. Otherwise returns `None`, signalling the
/// caller to insert `clip` verbatim.
pub(crate) fn try_build_markdown_link(clip: &str, selection: Option<&str>) -> Option<String> {
    let url = linkable_url(clip)?;
    let sel = selection.filter(|s| !s.is_empty())?;
    let escaped = sel.replace('\\', r"\\").replace(']', r"\]");
    Some(format!("[{escaped}]({url})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_trigger_before_cursor_finds_bracket() {
        assert!(has_trigger_before_cursor("hello [[foo", 11));
        assert!(has_trigger_before_cursor("[[a b c", 7));
    }

    #[test]
    fn has_trigger_before_cursor_finds_hashtag() {
        assert!(has_trigger_before_cursor("text #tag", 9));
    }

    #[test]
    fn has_trigger_before_cursor_no_trigger_bails() {
        assert!(!has_trigger_before_cursor("plain prose here", 16));
        assert!(!has_trigger_before_cursor("", 0));
    }

    #[test]
    fn has_trigger_before_cursor_handles_multibyte_no_panic() {
        // Regression: the previous 64-byte saturating_sub slice could
        // land mid-codepoint and panic on CJK / emoji / accented lines.
        let line = "你好世界".to_string() + &"a".repeat(80);
        let col = line.chars().count();
        assert!(!has_trigger_before_cursor(&line, col));

        let with_emoji = "🦀".repeat(20) + "[[note";
        let col = with_emoji.chars().count();
        assert!(has_trigger_before_cursor(&with_emoji, col));

        let accented = "é".repeat(100);
        let col = accented.chars().count();
        assert!(!has_trigger_before_cursor(&accented, col));
    }

    #[test]
    fn has_trigger_before_cursor_ignores_chars_after_cursor() {
        // Trigger AFTER cursor must not match.
        assert!(!has_trigger_before_cursor("foo [[bar", 3));
    }

    #[test]
    fn has_trigger_before_cursor_wikilink_with_spaces() {
        // Wikilink contents can contain spaces; we must still detect the
        // opening bracket far back on the line.
        assert!(has_trigger_before_cursor("[[my note title", 15));
    }

    #[test]
    fn linkable_url_accepts_supported_schemes() {
        assert_eq!(
            linkable_url("https://example.com"),
            Some("https://example.com")
        );
        assert_eq!(
            linkable_url("http://example.com/path?q=1#frag"),
            Some("http://example.com/path?q=1#frag"),
        );
        assert_eq!(
            linkable_url("  https://example.com  "),
            Some("https://example.com")
        );
        assert_eq!(
            linkable_url("ftp://files.example.com/x"),
            Some("ftp://files.example.com/x"),
        );
        assert_eq!(
            linkable_url("ftps://files.example.com/x"),
            Some("ftps://files.example.com/x"),
        );
        assert_eq!(
            linkable_url("mailto:user@example.com"),
            Some("mailto:user@example.com"),
        );
        assert_eq!(
            linkable_url("mailto:user@example.com?subject=hi"),
            Some("mailto:user@example.com?subject=hi"),
        );
    }

    #[test]
    fn linkable_url_rejects_other_schemes_and_plain_text() {
        assert_eq!(linkable_url("file:///etc/passwd"), None);
        assert_eq!(linkable_url("ssh://host"), None);
        assert_eq!(linkable_url("javascript:alert(1)"), None);
        assert_eq!(linkable_url("example.com"), None);
        assert_eq!(linkable_url("not a url"), None);
        assert_eq!(linkable_url(""), None);
        assert_eq!(linkable_url("https://example.com\nmore"), None);
    }

    #[test]
    fn try_build_markdown_link_wraps_selection_when_clip_is_url() {
        assert_eq!(
            try_build_markdown_link("https://example.com", Some("click here")).as_deref(),
            Some("[click here](https://example.com)"),
        );
    }

    #[test]
    fn try_build_markdown_link_trims_url_whitespace() {
        assert_eq!(
            try_build_markdown_link("  https://example.com\n", Some("link")).as_deref(),
            Some("[link](https://example.com)"),
        );
    }

    #[test]
    fn try_build_markdown_link_returns_none_when_no_selection() {
        assert_eq!(try_build_markdown_link("https://example.com", None), None);
    }

    #[test]
    fn try_build_markdown_link_returns_none_when_not_url() {
        assert_eq!(try_build_markdown_link("plain text", Some("sel")), None);
    }

    #[test]
    fn try_build_markdown_link_returns_none_when_selection_empty() {
        assert_eq!(
            try_build_markdown_link("https://example.com", Some("")),
            None
        );
    }

    #[test]
    fn try_build_markdown_link_escapes_close_bracket_in_selection() {
        assert_eq!(
            try_build_markdown_link("https://example.com", Some("a]b")).as_deref(),
            Some(r"[a\]b](https://example.com)"),
        );
    }

    #[test]
    fn try_build_markdown_link_wraps_ftp_url() {
        assert_eq!(
            try_build_markdown_link("ftp://files.example.com/x", Some("download")).as_deref(),
            Some("[download](ftp://files.example.com/x)"),
        );
    }

    #[test]
    fn try_build_markdown_link_wraps_mailto_url() {
        assert_eq!(
            try_build_markdown_link("mailto:user@example.com", Some("email me")).as_deref(),
            Some("[email me](mailto:user@example.com)"),
        );
    }
}
