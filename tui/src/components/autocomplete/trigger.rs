use std::ops::Range;

use kimun_core::note::is_inside_exclusion_zone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    Wikilink,
    Hashtag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerContext {
    pub kind: TriggerKind,
    /// The text already typed between the trigger sigil (`[[` or `#`) and
    /// the cursor — used as the prefix for the core suggestion query.
    pub query: String,
    /// Byte range that will be replaced when the user accepts a suggestion.
    /// Starts immediately after the sigil and ends at the cursor.
    pub replace_range: Range<usize>,
    /// Byte offset of `replace_range.start`, kept as a separate field so the
    /// host can map it to a screen anchor without re-parsing.
    pub anchor_col: usize,
}

/// Per-call knobs for `detect_trigger_with`.
#[derive(Debug, Clone, Copy)]
pub struct TriggerOptions {
    /// When `true`, a `#` at the start of a line defers (and is
    /// suppressed when followed by a space) so Markdown headers don't
    /// inadvertently open the hashtag popup. Editor uses `true`; the
    /// search box uses `false` because its input has no Markdown
    /// headers.
    pub disambiguate_header: bool,
    /// When `true`, suppress hashtag triggers inside code spans,
    /// fenced blocks, frontmatter, link bodies, or closed wikilinks
    /// (via `core::note::is_inside_exclusion_zone`). Editor uses
    /// `true`; the search box uses `false` because its input is plain
    /// text and the markdown parser would falsely classify literal
    /// backticks / brackets as code or link spans.
    pub apply_exclusion_zone: bool,
}

impl Default for TriggerOptions {
    fn default() -> Self {
        Self {
            disambiguate_header: true,
            apply_exclusion_zone: true,
        }
    }
}

/// Inspect `text` at `cursor` (a byte offset) and decide whether an
/// autocomplete popup should be active.
///
/// Returns `Some(TriggerContext)` when the cursor sits inside an open
/// wikilink target (`[[…|`) or an open hashtag word (`#…`). Returns `None`
/// otherwise — including when the cursor is inside a code span, fenced
/// block, frontmatter, or already-closed wikilink/markdown link (delegated
/// to `kimun_core::note::content_extractor::is_inside_exclusion_zone`).
///
/// Disambiguation rules in play:
/// - **Hashtag vs. Markdown header**: a `#` at the start of a line only
///   triggers the popup once the user has typed the next character AND
///   that character is not a space (a space means `# Heading`).
/// - **Wikilink target vs. alias**: in `[[target|alias]]`, only the
///   `target` portion triggers; the cursor crossing the `|` deactivates
///   the popup.
pub fn detect_trigger(text: &str, cursor: usize) -> Option<TriggerContext> {
    detect_trigger_with(text, cursor, TriggerOptions::default())
}

/// Variant of [`detect_trigger`] that takes explicit options. Used by
/// the search-box controller to suppress the column-0 `#` header
/// disambiguation, which only matters in the Markdown editor.
pub fn detect_trigger_with(
    text: &str,
    cursor: usize,
    opts: TriggerOptions,
) -> Option<TriggerContext> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    // The exclusion-zone check is applied selectively below — only for
    // hashtags. A wikilink trigger inside an already-closed `[[foo]]`
    // means the user is editing the target portion, which the spec
    // explicitly supports (see "Suggestion acceptance" — alias-suffix
    // preservation). Applying exclusion up-front here would block that
    // reopen-mid-edit flow.

    // Walk backwards from the cursor, tracking the two possible trigger
    // contexts in parallel:
    //
    // - **hashtag**: only word chars `[A-Za-z0-9_]` may sit between the
    //   `#` and the cursor (matches the hashtag regex in
    //   `core::note::content_extractor`). Any other char before we hit
    //   `#` makes a hashtag impossible.
    // - **wikilink**: any char except `]`, `\n`, `\r`, or a `|` already
    //   seen on the way back. A `]` closes a prior wikilink so we are not
    //   inside one; a `|` means the cursor is in the alias portion, which
    //   we don't autocomplete.
    //
    // The first context that hits its opener wins. Wikilink opener is
    // `[[`; when both `#` and `[[` are present, we keep scanning past `#`
    // and prefer `[[` (the outer context).
    let mut hash_pos: Option<usize> = None;
    let mut hash_possible = true;
    let mut wikilink_pos: Option<usize> = None;
    let mut wikilink_possible = true;
    let mut pipe_seen = false;
    let mut prev_was_bracket = false;

    let mut i = cursor;
    while i > 0 && (hash_possible || wikilink_possible) {
        let prev = prev_char_boundary(text, i);
        let c = text[prev..i].chars().next()?;

        if c == '\n' || c == '\r' {
            break;
        }

        if wikilink_possible {
            match c {
                ']' => wikilink_possible = false,
                '|' => pipe_seen = true,
                '[' => {
                    if prev_was_bracket {
                        wikilink_pos = Some(prev);
                        break;
                    }
                }
                _ => {}
            }
        }

        if hash_possible && hash_pos.is_none() {
            if c == '#' {
                hash_pos = Some(prev);
            } else if !(c.is_ascii_alphanumeric() || c == '_') {
                hash_possible = false;
            }
        }

        prev_was_bracket = c == '[';
        i = prev;
    }

    // Wikilink takes precedence when both are detected — it is the outer
    // context. A wikilink with a `|` between the opener and the cursor
    // means we are in the alias portion; bail.
    if let Some(open) = wikilink_pos {
        if pipe_seen {
            return None;
        }
        let inner_start = open + 2;
        if inner_start > cursor {
            return None;
        }
        let query = text[inner_start..cursor].to_string();
        return Some(TriggerContext {
            kind: TriggerKind::Wikilink,
            query,
            replace_range: inner_start..cursor,
            anchor_col: inner_start,
        });
    }

    if let Some(hash) = hash_pos {
        let inner_start = hash + 1;
        if inner_start > cursor {
            return None;
        }

        // Hashtag-only: suppress inside code spans, fenced blocks,
        // frontmatter, markdown links, or already-closed wikilinks /
        // markdown link bodies — but only when the caller is editing
        // Markdown. The search box turns this off because its input is
        // plain text.
        if opts.apply_exclusion_zone && is_inside_exclusion_zone(text, cursor) {
            return None;
        }

        // Column-0 disambiguation: defer the trigger when the user has
        // just typed `#` at the start of a line, since the next keystroke
        // tells us whether this is a hashtag (anything non-space) or a
        // Markdown header (space). Only active in contexts that actually
        // support Markdown headers (the editor); the search box turns
        // this off via `TriggerOptions`.
        if opts.disambiguate_header {
            let at_line_start =
                hash == 0 || text.as_bytes().get(hash - 1) == Some(&b'\n');
            if at_line_start {
                if cursor == inner_start {
                    return None;
                }
                let next_char = text[inner_start..].chars().next();
                if next_char == Some(' ') {
                    return None;
                }
            }
        }

        let query = text[inner_start..cursor].to_string();
        return Some(TriggerContext {
            kind: TriggerKind::Hashtag,
            query,
            replace_range: inner_start..cursor,
            anchor_col: inner_start,
        });
    }

    None
}

fn prev_char_boundary(text: &str, i: usize) -> usize {
    (0..i)
        .rev()
        .find(|&p| text.is_char_boundary(p))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(text: &str, cursor: usize) -> Option<TriggerContext> {
        detect_trigger(text, cursor)
    }

    // ---- Wikilink trigger ----

    #[test]
    fn wikilink_opens_with_empty_query() {
        let t = ctx("[[", 2).unwrap();
        assert_eq!(t.kind, TriggerKind::Wikilink);
        assert_eq!(t.query, "");
        assert_eq!(t.replace_range, 2..2);
        assert_eq!(t.anchor_col, 2);
    }

    #[test]
    fn wikilink_filters_by_typed_prefix() {
        let t = ctx("see [[foo", 9).unwrap();
        assert_eq!(t.kind, TriggerKind::Wikilink);
        assert_eq!(t.query, "foo");
        assert_eq!(t.replace_range, 6..9);
    }

    #[test]
    fn wikilink_with_pipe_alias_does_not_trigger() {
        // Cursor inside alias portion.
        assert!(ctx("[[target|al", 11).is_none());
    }

    #[test]
    fn wikilink_after_closing_brackets_is_not_a_trigger() {
        assert!(ctx("[[done]] more", 13).is_none());
    }

    #[test]
    fn wikilink_with_newline_inside_does_not_trigger() {
        assert!(ctx("[[foo\nbar", 9).is_none());
    }

    #[test]
    fn lone_single_bracket_does_not_trigger() {
        assert!(ctx("[foo", 4).is_none());
    }

    // ---- Hashtag trigger (mid-line) ----

    #[test]
    fn hashtag_mid_line_opens_immediately() {
        let t = ctx("some note #", 11).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "");
        assert_eq!(t.replace_range, 11..11);
    }

    #[test]
    fn hashtag_with_typed_query() {
        let t = ctx("about #pro", 10).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "pro");
        assert_eq!(t.replace_range, 7..10);
        assert_eq!(t.anchor_col, 7);
    }

    #[test]
    fn hashtag_closes_when_word_char_boundary_passes() {
        // A space after `#proj` breaks the hashtag context.
        assert!(ctx("about #proj here", 16).is_none());
    }

    // ---- Hashtag vs. header disambiguation at start of line ----

    #[test]
    fn hash_alone_at_start_of_line_does_not_trigger() {
        assert!(ctx("#", 1).is_none());
    }

    #[test]
    fn hash_then_space_at_start_of_line_is_header() {
        assert!(ctx("# ", 2).is_none());
    }

    #[test]
    fn hash_then_letter_at_start_of_line_opens_popup() {
        let t = ctx("#p", 2).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "p");
        assert_eq!(t.replace_range, 1..2);
    }

    #[test]
    fn hash_then_letter_after_newline_opens_popup() {
        let t = ctx("para\n#p", 7).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "p");
    }

    #[test]
    fn hash_then_space_after_newline_is_header() {
        assert!(ctx("para\n# ", 7).is_none());
    }

    // ---- Wikilink wins over hashtag when both present ----

    #[test]
    fn wikilink_outer_wins_over_inner_hash() {
        // User typed `[[#foo`; we are inside the wikilink, so the popup is
        // wikilink-flavoured with `#foo` as the query.
        let t = ctx("[[#foo", 6).unwrap();
        assert_eq!(t.kind, TriggerKind::Wikilink);
        assert_eq!(t.query, "#foo");
    }

    // ---- Exclusion zones (delegate to core) ----

    #[test]
    fn hash_inside_inline_code_does_not_trigger() {
        // `#tag` is inside the backticks — exclusion zone.
        assert!(ctx("here `#tag`", 9).is_none());
    }

    #[test]
    fn hash_inside_fenced_code_does_not_trigger() {
        let text = "para\n\n```\n#tag\n```\nafter";
        let cursor = text.find("#tag").unwrap() + 4;
        assert!(ctx(text, cursor).is_none());
    }

    #[test]
    fn hash_inside_frontmatter_does_not_trigger() {
        let text = "---\ntitle: Hi #tag\n---\nbody";
        let cursor = text.find("#tag").unwrap() + 4;
        assert!(ctx(text, cursor).is_none());
    }

    // ---- Cursor edge cases ----

    #[test]
    fn cursor_at_zero_never_triggers() {
        assert!(ctx("", 0).is_none());
        assert!(ctx("anything", 0).is_none());
    }

    #[test]
    fn cursor_past_end_returns_none() {
        assert!(ctx("short", 100).is_none());
    }

    #[test]
    fn cursor_not_on_char_boundary_returns_none() {
        // "é" is 2 bytes (0xc3 0xa9); cursor=1 is not a char boundary.
        assert!(ctx("é", 1).is_none());
    }

    // ---- Trigger preserved across cursor moves that stay in range ----

    #[test]
    fn trigger_active_at_every_cursor_position_inside_target() {
        let text = "see [[foo";
        // From just-after-`[[` through end of typed text, every position
        // yields a valid wikilink trigger with the appropriate query.
        for cursor in 6..=9 {
            let t = ctx(text, cursor).unwrap();
            assert_eq!(t.kind, TriggerKind::Wikilink);
            assert_eq!(t.query, &text[6..cursor]);
        }
    }

    #[test]
    fn trigger_cleared_when_cursor_moves_before_opener() {
        // Cursor at 5 sits on the first `[`; the user is now outside.
        assert!(ctx("see [[foo", 5).is_none());
    }

    // ---- CRLF handling ----

    #[test]
    fn crlf_line_treated_like_lf_for_column_0() {
        // `\r\n` before `#`: the line starts at the byte right after `\n`,
        // matching how `at_line_start` is computed.
        let text = "para\r\n#p";
        let cursor = text.len();
        let t = ctx(text, cursor).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "p");
    }

    #[test]
    fn crlf_just_after_hash_at_start_of_line_defers() {
        let text = "para\r\n#";
        assert!(ctx(text, text.len()).is_none());
    }

    // ---- TriggerOptions: header disambiguation disabled (search-box) ----

    #[test]
    fn search_box_opts_hash_alone_at_start_opens_immediately() {
        let opts = TriggerOptions {
            disambiguate_header: false,
            apply_exclusion_zone: true,
        };
        let t = detect_trigger_with("#", 1, opts).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "");
    }

    #[test]
    fn search_box_opts_hash_then_space_at_start_still_opens() {
        // No Markdown headers in the search input, so `# ` is a no-op
        // hashtag-with-empty-query — but the rule lets it through, and
        // the popup will close on the next typed char if no match.
        let opts = TriggerOptions {
            disambiguate_header: false,
            apply_exclusion_zone: true,
        };
        let t = detect_trigger_with("#", 1, opts);
        assert!(t.is_some());
    }

    #[test]
    fn search_box_opts_mid_line_unchanged() {
        // The disambiguation flag has no effect on mid-line `#`.
        let opts = TriggerOptions {
            disambiguate_header: false,
            apply_exclusion_zone: true,
        };
        let t = detect_trigger_with("foo #pro", 8, opts).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "pro");
    }

    #[test]
    fn search_box_opts_backtick_does_not_suppress_hashtag() {
        // With apply_exclusion_zone=false (search-box mode), a literal
        // backtick in the query does not falsely classify the cursor
        // as being inside a code span.
        let opts = TriggerOptions {
            disambiguate_header: false,
            apply_exclusion_zone: false,
        };
        let t = detect_trigger_with("`#abc", 5, opts).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "abc");
    }
}
