use std::ops::Range;

use kimun_core::note::scan::{
    ExclusionZones, is_inside_code_link_or_frontmatter, is_inside_exclusion_zone,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    Wikilink,
    Hashtag,
    LinkFilter,
    /// A leading `?` in a query input: autocompletes saved-search names.
    /// Accepting expands to the search's stored query (see `adr/0006`).
    SavedSearch,
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
    /// For a `LinkFilter` trigger, the operator char that opened it (`<`,
    /// `>`, or `=`) so the popup can render the correct sigil. `None` for
    /// `Wikilink` and `Hashtag` triggers, which have fixed sigils.
    pub opener: Option<char>,
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
    /// (via `core::note::scan::is_inside_exclusion_zone`). Editor uses
    /// `true`; the search box uses `false` because its input is plain
    /// text and the markdown parser would falsely classify literal
    /// backticks / brackets as code or link spans.
    pub apply_exclusion_zone: bool,
    /// When `true`, a leading `?` opens a [`TriggerKind::SavedSearch`]
    /// trigger. Only the search-query box enables it; the editor leaves it
    /// `false` so a note that opens with `?` cannot shadow the `#`/`[[`
    /// triggers the backward scan would otherwise find. Gating here (rather
    /// than dropping the trigger after the fact) keeps detection the single
    /// authority on whether `?` is special. See `adr/0006`.
    pub allow_saved_search: bool,
}

impl Default for TriggerOptions {
    fn default() -> Self {
        Self {
            disambiguate_header: true,
            apply_exclusion_zone: true,
            allow_saved_search: false,
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
/// to `kimun_core::note::scan::is_inside_exclusion_zone`).
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
    detect_trigger_with_zones(text, cursor, opts, None)
}

/// Lazily answers the two exclusion-zone queries the trigger veto
/// needs. Consulted ONLY after a wikilink/hashtag opener is found in the
/// local backward scan, so a keystroke with no trigger candidate never
/// pays for the full-buffer `ExclusionZones` scan (pulldown + regex).
/// Implementors decide whether to precompute, recompute, or memoize.
pub trait ZoneOracle {
    /// Mirror of `ExclusionZones::contains` — the hashtag veto.
    fn contains(&mut self, cursor: usize) -> bool;
    /// Mirror of `ExclusionZones::contains_code_link_or_frontmatter` —
    /// the wikilink veto.
    fn contains_code_link_or_frontmatter(&mut self, cursor: usize) -> bool;
}

/// Oracle backed by a precomputed `ExclusionZones`.
struct PrecomputedOracle<'a>(&'a ExclusionZones);
impl ZoneOracle for PrecomputedOracle<'_> {
    fn contains(&mut self, cursor: usize) -> bool {
        self.0.contains(cursor)
    }
    fn contains_code_link_or_frontmatter(&mut self, cursor: usize) -> bool {
        self.0.contains_code_link_or_frontmatter(cursor)
    }
}

/// Oracle that recomputes per query from `text`. Used by the no-cache
/// convenience paths (`detect_trigger`, `detect_trigger_with`) and
/// tests; at most one veto query runs per call, so the recompute is
/// paid only when a candidate opener is actually present.
struct RecomputeOracle<'t>(&'t str);
impl ZoneOracle for RecomputeOracle<'_> {
    fn contains(&mut self, cursor: usize) -> bool {
        is_inside_exclusion_zone(self.0, cursor)
    }
    fn contains_code_link_or_frontmatter(&mut self, cursor: usize) -> bool {
        is_inside_code_link_or_frontmatter(self.0, cursor)
    }
}

/// Variant of [`detect_trigger_with`] that accepts a precomputed
/// `ExclusionZones` for the same `text`. When `zones` is `None`, the
/// exclusion check recomputes per call (used by tests and the no-cache
/// convenience function). Thin wrapper over [`detect_trigger_with_oracle`].
pub fn detect_trigger_with_zones(
    text: &str,
    cursor: usize,
    opts: TriggerOptions,
    zones: Option<&ExclusionZones>,
) -> Option<TriggerContext> {
    match zones {
        Some(z) => detect_trigger_with_oracle(text, cursor, opts, &mut PrecomputedOracle(z)),
        None => detect_trigger_with_oracle(text, cursor, opts, &mut RecomputeOracle(text)),
    }
}

/// Core trigger detection. The local backward scan from `cursor` runs
/// unconditionally; `oracle` is consulted only at the exclusion veto,
/// reached only once a `[[`/`#` opener has been found — so the caller's
/// zone computation can stay lazy.
pub fn detect_trigger_with_oracle(
    text: &str,
    cursor: usize,
    opts: TriggerOptions,
    oracle: &mut dyn ZoneOracle,
) -> Option<TriggerContext> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }

    // SavedSearch: a leading `?` (only blanks before it, on the first line)
    // owns the whole query. Checked before the backward scan because `?` is
    // not an opener for any other trigger. A saved-search name may contain
    // spaces, so the prefix runs from just after `?` to the cursor. Only the
    // search-query box opts in (`allow_saved_search`); see `adr/0006`.
    if opts.allow_saved_search
        && let Some(q_pos) = text.find('?')
        && text[..q_pos].bytes().all(|b| b == b' ' || b == b'\t')
    {
        let inner_start = q_pos + 1;
        // `inner_start > cursor` when the caret sits on/before the `?`
        // (e.g. text "?x", cursor 0) — no prefix yet, so don't trigger.
        if inner_start <= cursor {
            return Some(TriggerContext {
                kind: TriggerKind::SavedSearch,
                query: text[inner_start..cursor].to_string(),
                replace_range: inner_start..cursor,
                anchor_col: inner_start,
                opener: None,
            });
        }
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
    //   `core::note::scan`). Any other char before we hit
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
    let mut link_filter_pos: Option<usize> = None;
    let mut link_filter_possible = true;

    let mut i = cursor;
    while i > 0 && (hash_possible || wikilink_possible || link_filter_possible) {
        let prev = prev_char_boundary(text, i);
        let c = text[prev..i].chars().next()?;

        if c == '\n' || c == '\r' {
            break;
        }

        if wikilink_possible {
            match c {
                ']' => wikilink_possible = false,
                '|' => pipe_seen = true,
                '[' if prev_was_bracket => {
                    wikilink_pos = Some(prev);
                    break;
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

        if link_filter_possible && link_filter_pos.is_none() {
            if is_link_filter_opener(c) {
                link_filter_pos = Some(prev);
                // Opener found: the link-filter state is resolved, so stop
                // scanning for it. The loop continues only if another state
                // machine (wikilink/hashtag) is still live — mirroring how
                // `wikilink_possible` / `hash_possible` stop driving the loop
                // once their openers/stoppers are hit.
                link_filter_possible = false;
            } else if !is_link_filter_target_char(c) {
                link_filter_possible = false;
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
        // Suppress inside code, markdown link bodies, frontmatter —
        // but NOT inside an already-closed `[[…]]` (that is the
        // reopen-mid-target case the spec wants to support). Only
        // applied when the caller is editing Markdown (search box
        // disables this).
        if opts.apply_exclusion_zone && oracle.contains_code_link_or_frontmatter(cursor) {
            return None;
        }
        let query = text[inner_start..cursor].to_string();
        return Some(TriggerContext {
            kind: TriggerKind::Wikilink,
            query,
            replace_range: inner_start..cursor,
            anchor_col: inner_start,
            opener: None,
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
        // plain text. Checked before the word-boundary guard so a future
        // relaxation of the boundary rule cannot accidentally let popups
        // leak into excluded regions.
        if opts.apply_exclusion_zone && oracle.contains(cursor) {
            return None;
        }

        // Word-boundary guard — mirrors `core::note::scan::
        // label_matches_inner`. The tag region runs from `#` through the
        // contiguous `[A-Za-z0-9_]+` word that follows it; reject if the
        // character on EITHER side of that region is alphanumeric, `_`, or
        // another `#`. Both sides are required because the popup may open
        // when the cursor is inside an existing tag (e.g. `#tag#more`
        // cursor between `g` and the second `#`) — checking only the
        // preceding char would suggest a label the indexer then rejects.
        if hash > 0 {
            let preceding_blocks_label = text[..hash]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric() || c == '_' || c == '#')
                .unwrap_or(false);
            if preceding_blocks_label {
                return None;
            }
        }
        let bytes = text.as_bytes();
        let mut word_end = inner_start;
        while word_end < bytes.len() {
            let b = bytes[word_end];
            if b.is_ascii_alphanumeric() || b == b'_' {
                word_end += 1;
            } else {
                break;
            }
        }
        let following_blocks_label = text[word_end..]
            .chars()
            .next()
            .map(|c| c.is_alphanumeric() || c == '_' || c == '#')
            .unwrap_or(false);
        if following_blocks_label {
            return None;
        }

        // Column-0 disambiguation: defer the trigger when the user has
        // just typed `#` at the start of a line, since the next keystroke
        // tells us whether this is a hashtag (anything non-space) or a
        // Markdown header (space). Only active in contexts that actually
        // support Markdown headers (the editor); the search box turns
        // this off via `TriggerOptions`.
        if opts.disambiguate_header {
            let at_line_start = hash == 0 || text.as_bytes().get(hash - 1) == Some(&b'\n');
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
            opener: None,
        });
    }

    // LinkFilter trigger: a note-name operator (`<`, `>`, `=`) followed by
    // a target, optionally in its exclusion form (`-<`, `->`, `-=`). All
    // three operators take a note-name argument (backlinks / forward links
    // / note name — ADR-0005 alphabet) and share the same suggestion list.
    // The opener must be at a token start — i.e. preceded by nothing,
    // whitespace, or a `-` that is itself at string-start or preceded by
    // whitespace (the exclusion form).
    if let Some(gt) = link_filter_pos {
        let inner_start = gt + 1; // byte just after the opener
        if inner_start > cursor {
            return None;
        }

        // Token-start guard: what sits immediately before the opener?
        let token_start = if gt == 0 {
            true // `>` at string start
        } else {
            let before_gt = text[..gt].chars().next_back().unwrap();
            if before_gt.is_whitespace() {
                true // `>` after whitespace
            } else if before_gt == '-' {
                // Allow only `->` where `-` itself is at string-start or
                // preceded by whitespace.
                let dash_pos = gt - before_gt.len_utf8();
                dash_pos == 0
                    || text[..dash_pos]
                        .chars()
                        .next_back()
                        .map(|c| c.is_whitespace())
                        .unwrap_or(false)
            } else {
                false
            }
        };
        if !token_start {
            return None;
        }

        if opts.apply_exclusion_zone && oracle.contains(cursor) {
            return None;
        }

        // The opener char sits at byte `gt`; capture it so the popup can
        // render the matching sigil (`<`, `>`, or `=`).
        let opener = text[gt..inner_start].chars().next();

        let query = text[inner_start..cursor].to_string();
        return Some(TriggerContext {
            kind: TriggerKind::LinkFilter,
            query,
            replace_range: inner_start..cursor,
            anchor_col: inner_start,
            opener,
        });
    }

    None
}

/// Returns `true` for the note-name operators that open a link-filter
/// trigger: `<` (backlinks), `>` (forward links), `=` (note name) — the
/// ADR-0005 alphabet operators that take a note-name argument. `@`
/// (section), `#` (label) and `/` (path) are intentionally excluded.
fn is_link_filter_opener(c: char) -> bool {
    matches!(c, '<' | '>' | '=')
}

/// Returns `true` for characters that may appear in a link-filter target
/// prefix (everything between the opener and the cursor). Mirrors the valid
/// characters for note names / paths: letters, digits, `_`, `-`, `.`, `/`,
/// `*`, `{`, `}`. Spaces are intentionally excluded because they are
/// query-token separators — the scan stops at a space, so a `>` preceded
/// only by a space is considered a valid token-start.
fn is_link_filter_target_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '*' | '{' | '}')
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

    /// `ctx` with the search-box opt-in for the leading-`?` SavedSearch trigger.
    fn ctx_ss(text: &str, cursor: usize) -> Option<TriggerContext> {
        detect_trigger_with(
            text,
            cursor,
            TriggerOptions {
                allow_saved_search: true,
                ..TriggerOptions::default()
            },
        )
    }

    /// Records how many times the exclusion veto consulted the oracle.
    struct CountingOracle {
        calls: usize,
    }
    impl ZoneOracle for CountingOracle {
        fn contains(&mut self, _: usize) -> bool {
            self.calls += 1;
            false
        }
        fn contains_code_link_or_frontmatter(&mut self, _: usize) -> bool {
            self.calls += 1;
            false
        }
    }

    // ---- Lazy exclusion oracle ----

    #[test]
    fn oracle_untouched_without_trigger_candidate() {
        // Plain prose, caret at end: the local backward scan finds no
        // `[[`/`#` opener, so the veto is never reached and the
        // (expensive in the real impl) zone query must not run.
        let mut o = CountingOracle { calls: 0 };
        let r = detect_trigger_with_oracle("hello world", 11, TriggerOptions::default(), &mut o);
        assert!(r.is_none());
        assert_eq!(o.calls, 0, "no opener must not consult the zone oracle");
    }

    #[test]
    fn oracle_consulted_for_hashtag_candidate() {
        let mut o = CountingOracle { calls: 0 };
        let _ = detect_trigger_with_oracle("#tag", 4, TriggerOptions::default(), &mut o);
        assert!(o.calls >= 1, "a # candidate must consult the veto oracle");
    }

    #[test]
    fn oracle_consulted_for_wikilink_candidate() {
        let mut o = CountingOracle { calls: 0 };
        let _ = detect_trigger_with_oracle("[[me", 4, TriggerOptions::default(), &mut o);
        assert!(o.calls >= 1, "a [[ candidate must consult the veto oracle");
    }

    #[test]
    fn oracle_untouched_when_exclusion_disabled() {
        // Search box (apply_exclusion_zone = false) must never consult
        // the oracle even with an opener present.
        let opts = TriggerOptions {
            apply_exclusion_zone: false,
            ..TriggerOptions::default()
        };
        let mut o = CountingOracle { calls: 0 };
        let _ = detect_trigger_with_oracle("#tag", 4, opts, &mut o);
        assert_eq!(
            o.calls, 0,
            "apply_exclusion_zone=false must skip the oracle entirely"
        );
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

    // ---- SavedSearch trigger ----

    #[test]
    fn saved_search_opens_on_leading_question_mark() {
        let t = ctx_ss("?to", 3).unwrap();
        assert_eq!(t.kind, TriggerKind::SavedSearch);
        assert_eq!(t.query, "to");
        assert_eq!(t.replace_range, 1..3);
        assert_eq!(t.anchor_col, 1);
    }

    #[test]
    fn saved_search_opens_with_empty_query() {
        let t = ctx_ss("?", 1).unwrap();
        assert_eq!(t.kind, TriggerKind::SavedSearch);
        assert_eq!(t.query, "");
        assert_eq!(t.replace_range, 1..1);
    }

    #[test]
    fn saved_search_not_triggered_when_not_leading() {
        // `?` is only special as the leading (blank-prefixed) char.
        assert_ne!(
            ctx_ss("#a ?to", 6).map(|t| t.kind),
            Some(TriggerKind::SavedSearch)
        );
        assert_ne!(
            ctx_ss("note ?x", 7).map(|t| t.kind),
            Some(TriggerKind::SavedSearch)
        );
    }

    #[test]
    fn saved_search_off_by_default() {
        // Without the opt-in (the editor's default), a leading `?` is inert,
        // so it can't shadow the hashtag/wikilink scan.
        assert_eq!(ctx("?to", 3), None);
    }

    #[test]
    fn hashtag_closes_when_word_char_boundary_passes() {
        // A space after `#proj` breaks the hashtag context.
        assert!(ctx("about #proj here", 16).is_none());
    }

    #[test]
    fn hash_mid_word_does_not_trigger() {
        // `hello#` — `#` immediately follows a letter, so it is not a label.
        assert!(ctx("hello#", 6).is_none());
    }

    #[test]
    fn hash_mid_word_with_query_does_not_trigger() {
        // `hello#tag` — still mid-word, popup must not open.
        assert!(ctx("hello#tag", 9).is_none());
    }

    #[test]
    fn hash_after_digit_does_not_trigger() {
        assert!(ctx("abc123#tag", 10).is_none());
    }

    #[test]
    fn hash_after_underscore_does_not_trigger() {
        assert!(ctx("foo_#tag", 8).is_none());
    }

    #[test]
    fn double_hash_does_not_trigger() {
        // `##tag` — second `#` immediately follows first `#`, not a label.
        assert!(ctx("##tag", 5).is_none());
    }

    #[test]
    fn triple_hash_does_not_trigger() {
        assert!(ctx("###tag", 6).is_none());
    }

    #[test]
    fn double_hash_mid_line_does_not_trigger() {
        assert!(ctx("hello ##tag", 11).is_none());
    }

    #[test]
    fn hash_between_double_hash_at_start_does_not_trigger() {
        // `##tag` with cursor between the two `#`s — the column-0 case the
        // earlier `if hash > 0` gate let through.
        assert!(ctx("##tag", 1).is_none());
    }

    #[test]
    fn adjacent_hash_at_cursor_does_not_trigger() {
        // `#tag#more` with cursor right after `g` — popup must not open
        // because the indexer will reject both `#tag` and `#more`.
        assert!(ctx("#tag#more", 4).is_none());
    }

    #[test]
    fn adjacent_hash_with_cursor_inside_tag_does_not_trigger() {
        // Cursor mid-tag (`#ta|g#more`) — the following `#` still
        // invalidates the tag region.
        assert!(ctx("#tag#more", 3).is_none());
    }

    #[test]
    fn trailing_hash_after_tag_does_not_trigger() {
        // `#draft#` cursor between `t` and trailing `#`.
        assert!(ctx("#draft#", 6).is_none());
    }

    #[test]
    fn search_box_double_hash_at_start_does_not_trigger() {
        // Same column-0 `##` case under search-box opts — the guard now
        // catches it via the following-char check (the original gate
        // skipped it when `disambiguate_header=false`).
        let opts = TriggerOptions {
            disambiguate_header: false,
            apply_exclusion_zone: false,
            allow_saved_search: false,
        };
        assert!(detect_trigger_with("##tag", 1, opts).is_none());
        assert!(detect_trigger_with("##", 1, opts).is_none());
    }

    #[test]
    fn hash_after_space_then_hash_triggers() {
        // `# #tag` — space breaks the `##` run, second `#` is a valid label start.
        let t = ctx("# #tag", 6).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "tag");
    }

    #[test]
    fn hash_after_punctuation_triggers() {
        // Punctuation is not a label char, so `#tag` after `,` is a valid hashtag.
        let t = ctx("hi,#tag", 7).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "tag");
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
            allow_saved_search: false,
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
            allow_saved_search: false,
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
            allow_saved_search: false,
        };
        let t = detect_trigger_with("foo #pro", 8, opts).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "pro");
    }

    #[test]
    fn wikilink_inside_fenced_code_does_not_trigger() {
        let text = "para\n\n```\n[[note\n```\nafter";
        let cursor = text.find("[[note").unwrap() + 6;
        assert!(ctx(text, cursor).is_none());
    }

    #[test]
    fn wikilink_inside_frontmatter_does_not_trigger() {
        let text = "---\ntitle: see [[me\n---\nbody";
        let cursor = text.find("[[me").unwrap() + 4;
        assert!(ctx(text, cursor).is_none());
    }

    #[test]
    fn wikilink_reopen_mid_existing_target_still_works() {
        // The spec carve-out: cursor inside an already-closed `[[foo]]`
        // STILL triggers (so the user can edit the target). The new
        // exclusion-zone check excludes only code/link/frontmatter,
        // NOT closed wikilinks.
        let text = "see [[foo]]";
        let t = ctx(text, 7).unwrap(); // cursor between `o` and `o`
        assert_eq!(t.kind, TriggerKind::Wikilink);
    }

    #[test]
    fn search_box_opts_backtick_does_not_suppress_hashtag() {
        // With apply_exclusion_zone=false (search-box mode), a literal
        // backtick in the query does not falsely classify the cursor
        // as being inside a code span.
        let opts = TriggerOptions {
            disambiguate_header: false,
            apply_exclusion_zone: false,
            allow_saved_search: false,
        };
        let t = detect_trigger_with("`#abc", 5, opts).unwrap();
        assert_eq!(t.kind, TriggerKind::Hashtag);
        assert_eq!(t.query, "abc");
    }

    // ---- LinkFilter trigger (`>` / `->`) ----

    #[test]
    fn detects_link_filter_trigger() {
        // `>` at the start of a query token opens a LinkFilter trigger.
        let t = detect_trigger(">pro", 4).expect("should detect");
        assert_eq!(t.kind, TriggerKind::LinkFilter);
        assert_eq!(t.query, "pro");
    }

    #[test]
    fn detects_excluded_link_filter_trigger() {
        let t = detect_trigger("->dra", 5).expect("should detect");
        assert_eq!(t.kind, TriggerKind::LinkFilter);
        assert_eq!(t.query, "dra");
    }

    #[test]
    fn link_filter_only_at_token_start() {
        // A `>` that is not at a token boundary (e.g. inside a word) must NOT
        // trigger. Here `a>b` — the `>` is preceded by a non-space word char.
        let t = detect_trigger("a>b", 3);
        assert!(t.is_none() || t.unwrap().kind != TriggerKind::LinkFilter);
    }

    #[test]
    fn detects_backlink_filter_trigger() {
        let t = detect_trigger("<pro", 4).expect("should detect");
        assert_eq!(t.kind, TriggerKind::LinkFilter);
        assert_eq!(t.query, "pro");
        assert_eq!(t.opener, Some('<'));
    }

    #[test]
    fn detects_forward_link_filter_trigger() {
        let t = detect_trigger(">pro", 4).expect("should detect");
        assert_eq!(t.kind, TriggerKind::LinkFilter);
        assert_eq!(t.query, "pro");
        assert_eq!(t.opener, Some('>'));
    }

    #[test]
    fn detects_note_name_filter_trigger() {
        let t = detect_trigger("=pro", 4).expect("should detect");
        assert_eq!(t.kind, TriggerKind::LinkFilter);
        assert_eq!(t.query, "pro");
        assert_eq!(t.opener, Some('='));
    }

    #[test]
    fn excluded_link_filter_captures_inner_opener() {
        // The exclusion form `-<` / `->` / `-=` must capture the operator
        // char (not the `-`) so the popup renders the right sigil.
        for (q, op) in [("-<dra", '<'), ("->dra", '>'), ("-=dra", '=')] {
            let t = detect_trigger(q, q.len()).expect("should detect");
            assert_eq!(t.kind, TriggerKind::LinkFilter, "{q}");
            assert_eq!(t.opener, Some(op), "{q}");
        }
    }

    #[test]
    fn wikilink_and_hashtag_have_no_opener() {
        assert_eq!(ctx("[[foo", 5).unwrap().opener, None);
        assert_eq!(ctx("a #foo", 6).unwrap().opener, None);
    }

    #[test]
    fn detects_excluded_forms() {
        for q in ["-<dra", "->dra", "-=dra"] {
            let t = detect_trigger(q, q.len()).expect("should detect");
            assert_eq!(t.kind, TriggerKind::LinkFilter, "{q}");
            assert_eq!(t.query, "dra", "{q}");
        }
    }

    #[test]
    fn link_filter_new_openers_only_at_token_start() {
        // not at a token boundary -> no LinkFilter trigger
        let t = detect_trigger("a<b", 3);
        assert!(t.is_none() || t.unwrap().kind != TriggerKind::LinkFilter);
    }
}
