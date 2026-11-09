//! The TUI's styles over an answer's markdown: a thin mapper from the
//! shared, toolkit-free rules ([`crate::ask::markdown`], which the GUI's
//! Thread draws by too) onto ratatui [`Style`]s, one wrapped visual slice of
//! a line at a time.
//!
//! - **The rules are not ours.** Block kinds, emphasis (bold, italic,
//!   strikethrough, CommonMark's flanking and intraword `_`), inline code
//!   and citations all come from [`parse_answer`](crate::ask::markdown::parse_answer),
//!   over the whole answer at once (so emphasis pairs across a wrap, and
//!   the editor's model decides what a line is).
//! - **Hidden sigils need a column map.** Emphasis sigils are left out of the
//!   rendered text, which breaks the 1:1 byte↔column identity, so
//!   [`style_slice_mapped`] returns, alongside the styled line, a **column
//!   map** (`rendered char index → source byte offset in the slice`) for the
//!   citation hit-test.
//! - **One style per run.** The terminal draws one style where the shared
//!   rules may stack several: code wins, then a citation, then bold, then
//!   italic; strikethrough adds the crossed-out modifier to any of them but
//!   code.

use std::ops::Range;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

pub use crate::ask::markdown::BlockKind as LineKind;
use crate::ask::markdown::{AnswerLine, SpanStyle};
use crate::settings::themes::Theme;

/// The semantic styles the answer body renders with, resolved from the theme
/// once per render and reused across every line.
#[derive(Clone, Copy)]
pub struct MdStyles {
    pub base: Style,
    pub heading: Style,
    pub quote: Style,
    pub code: Style,
    pub bold: Style,
    pub italic: Style,
    pub citation: Style,
}

impl MdStyles {
    /// Build from the theme, mirroring the editor's markdown color conventions
    /// (`text_editor::markdown_style::role_style`): headings bright+bold, inline/code
    /// aqua on a soft background, bold accent+bold, italic secondary+italic,
    /// blockquote secondary. Citations keep the answer's accent marker color.
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            base: Style::default().fg(theme.fg.to_ratatui()),
            heading: Style::default()
                .fg(theme.fg_bright.to_ratatui())
                .add_modifier(Modifier::BOLD),
            quote: Style::default().fg(theme.fg_secondary.to_ratatui()),
            code: Style::default()
                .fg(theme.aqua.to_ratatui())
                .bg(theme.bg_soft.to_ratatui()),
            bold: Style::default()
                .fg(theme.accent.to_ratatui())
                .add_modifier(Modifier::BOLD),
            italic: Style::default()
                .fg(theme.fg_secondary.to_ratatui())
                .add_modifier(Modifier::ITALIC),
            citation: Style::default().fg(theme.accent.to_ratatui()),
        }
    }

    /// The one style a run with the shared `style` draws in.
    fn of(&self, style: SpanStyle) -> Style {
        let run = if style.code {
            self.code
        } else if style.citation {
            self.citation
        } else if style.bold {
            self.bold
        } else if style.italic {
            self.italic
        } else {
            self.base
        };
        if style.strike && !style.code {
            run.add_modifier(Modifier::CROSSED_OUT)
        } else {
            run
        }
    }
}

/// Style the wrapped visual `slice` (a byte range into `text`) of the answer
/// line `line`, whose source text is `text`, returning the styled [`Line`]
/// and its **column map**: `map[k]` is the source byte offset (into the
/// slice) of the `k`-th *rendered* character.
///
/// Code, heading and quote lines are verbatim in their kind's style, with the
/// identity map. An ordinary line drops its hidden emphasis sigils, so `map`
/// skips their bytes — a caller resolving a rendered column back to a source
/// byte walks `map`.
pub fn style_slice_mapped(
    text: &str,
    line: &AnswerLine,
    slice: Range<usize>,
    styles: &MdStyles,
) -> (Line<'static>, Vec<usize>) {
    match line.kind {
        LineKind::Code => whole_slice(&text[slice], styles.code),
        LineKind::Heading => whole_slice(&text[slice], styles.heading),
        LineKind::Quote => whole_slice(&text[slice], styles.quote),
        LineKind::Normal => inline_spans(text, line, slice, styles),
    }
}

/// Style `slice` as a single verbatim span (no hiding) with the identity map.
fn whole_slice(slice: &str, style: Style) -> (Line<'static>, Vec<usize>) {
    let map: Vec<usize> = slice.char_indices().map(|(i, _)| i).collect();
    (Line::from(Span::styled(slice.to_string(), style)), map)
}

/// The shown spans of an ordinary line that fall in `slice`, one ratatui
/// span per run of one style; the concatenation is the slice with exactly
/// its hidden sigils removed.
fn inline_spans(
    text: &str,
    line: &AnswerLine,
    slice: Range<usize>,
    styles: &MdStyles,
) -> (Line<'static>, Vec<usize>) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut buf_style = styles.base;
    let mut map: Vec<usize> = Vec::new();
    for span in line.shown_spans() {
        let start = span.range.start.max(slice.start);
        let end = span.range.end.min(slice.end);
        if start >= end {
            continue;
        }
        let style = styles.of(span.style);
        if style != buf_style && !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut buf), buf_style));
        }
        buf_style = style;
        let part = &text[start..end];
        buf.push_str(part);
        map.extend(part.char_indices().map(|(i, _)| start - slice.start + i));
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, buf_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), styles.base));
    }
    (Line::from(spans), map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::markdown::parse_answer;

    fn styles() -> MdStyles {
        MdStyles::from_theme(&Theme::default())
    }

    /// The styled spans, concatenated — the rendered text of a line.
    fn rendered(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Style the whole of line `row` of `answer` (its kind is the shared
    /// parser's), returning the styled line and its column map.
    fn style_row(answer: &str, row: usize, styles: &MdStyles) -> (Line<'static>, Vec<usize>) {
        let lines = parse_answer(answer);
        let line = &lines[row];
        let text = line.text(answer);
        style_slice_mapped(text, line, 0..text.len(), styles)
    }

    /// A one-line answer, styled whole.
    fn style_line_mapped(raw: &str, styles: &MdStyles) -> (Line<'static>, Vec<usize>) {
        style_row(raw, 0, styles)
    }

    /// Just the styled line (the common case; the map is asserted separately).
    fn style_slice(raw: &str, styles: &MdStyles) -> Line<'static> {
        style_line_mapped(raw, styles).0
    }

    #[test]
    fn code_slice_is_never_citation_styled() {
        let s = styles();
        // A `[1]` sitting inside a code line keeps the code style — no accent.
        // (The kind is the parser's now, so the line sits in a fence.)
        let line = style_row("```\nlet n = arr[1];", 1, &s).0;
        assert_eq!(line.spans.len(), 1, "code renders as one verbatim span");
        assert_eq!(line.spans[0].style, s.code);
        assert!(line.spans[0].style != s.citation);
        assert_eq!(rendered(&line), "let n = arr[1];");
    }

    #[test]
    fn heading_slice_gets_heading_styling() {
        let s = styles();
        let line = style_slice("## Overview", &s);
        assert_eq!(line.spans[0].style, s.heading);
        assert_eq!(rendered(&line), "## Overview");
    }

    #[test]
    fn prose_citation_gets_citation_style_and_preserves_bytes() {
        let s = styles();
        let line = style_slice("See [1] and [2].", &s);
        assert_eq!(rendered(&line), "See [1] and [2].", "1:1 with the source");
        // The `[1]`/`[2]` markers carry the citation style.
        let cited: String = line
            .spans
            .iter()
            .filter(|sp| sp.style == s.citation)
            .map(|sp| sp.content.as_ref())
            .collect();
        assert_eq!(cited, "[1][2]");
    }

    #[test]
    fn bold_sigils_are_hidden_and_the_run_is_styled() {
        let s = styles();
        let line = style_slice("a **b** `c` d", &s);
        // The `**` pair is dropped; the code span's backticks stay literal.
        assert_eq!(rendered(&line), "a b `c` d");
        let bold: String = line
            .spans
            .iter()
            .filter(|sp| sp.style == s.bold)
            .map(|sp| sp.content.as_ref())
            .collect();
        assert_eq!(bold, "b", "only the run between the sigils is bold");
        assert!(
            line.spans.iter().any(|sp| sp.style == s.code),
            "inline code run is styled"
        );
    }

    #[test]
    fn italic_sigils_are_hidden_for_both_star_and_underscore() {
        let s = styles();
        for (src, want) in [
            ("an *em* word", "an em word"),
            ("an _em_ word", "an em word"),
        ] {
            let line = style_slice(src, &s);
            assert_eq!(rendered(&line), want);
            let italic: String = line
                .spans
                .iter()
                .filter(|sp| sp.style == s.italic)
                .map(|sp| sp.content.as_ref())
                .collect();
            assert_eq!(italic, "em");
        }
    }

    #[test]
    fn a_lone_sigil_stays_visible_and_emphasizes_nothing() {
        let s = styles();
        // An unbalanced `*` (a stray bullet / arithmetic) must not be eaten and
        // must not italicize the tail of the line.
        let line = style_slice("2 * 3 = 6 and rest", &s);
        assert_eq!(rendered(&line), "2 * 3 = 6 and rest", "lone sigil kept");
        assert!(
            line.spans.iter().all(|sp| sp.style != s.italic),
            "no run is italicized by an unmatched sigil"
        );
    }

    #[test]
    fn space_flanked_stars_are_not_emphasis() {
        let s = styles();
        // Whitespace on the inner side means neither run can open/close, so the
        // asterisks stay literal and nothing between them is italicized.
        for src in ["width * height * depth = volume", "2 * 3 * 4"] {
            let line = style_slice(src, &s);
            assert_eq!(rendered(&line), src, "{src} stays verbatim");
            assert!(
                line.spans
                    .iter()
                    .all(|sp| sp.style != s.italic && sp.style != s.bold),
                "{src} gets no emphasis styling"
            );
        }
    }

    #[test]
    fn glob_stars_stay_visible_and_emphasize_nothing() {
        let s = styles();
        // `*.rs`/`*.md`: each `*` can open (followed by `.`) but neither can
        // close (preceded by a space), so no pair forms — both stars survive.
        let line = style_slice("match *.rs and *.md files", &s);
        assert_eq!(rendered(&line), "match *.rs and *.md files");
        assert!(
            line.spans
                .iter()
                .all(|sp| sp.style != s.italic && sp.style != s.bold),
            "glob stars italicize nothing"
        );
    }

    #[test]
    fn real_star_emphasis_still_works() {
        let s = styles();
        // `*real*` still italicizes and `**bold**` still bolds — the flanking
        // rule only rejects whitespace-adjacent runs.
        let line = style_slice("*real*", &s);
        assert_eq!(rendered(&line), "real");
        let italic: String = line
            .spans
            .iter()
            .filter(|sp| sp.style == s.italic)
            .map(|sp| sp.content.as_ref())
            .collect();
        assert_eq!(italic, "real");

        let line = style_slice("**bold**", &s);
        assert_eq!(rendered(&line), "bold");
        let bold: String = line
            .spans
            .iter()
            .filter(|sp| sp.style == s.bold)
            .map(|sp| sp.content.as_ref())
            .collect();
        assert_eq!(bold, "bold");
    }

    #[test]
    fn emphasis_inside_a_code_span_stays_literal() {
        let s = styles();
        // The `*x*` lives inside inline code — its asterisks are verbatim.
        let line = style_slice("call `*x*` now", &s);
        assert_eq!(rendered(&line), "call `*x*` now", "code is verbatim");
        assert!(
            line.spans.iter().all(|sp| sp.style != s.italic),
            "no italic from sigils inside code"
        );
    }

    #[test]
    fn rendered_text_is_raw_minus_exactly_the_hidden_sigil_pairs() {
        let s = styles();
        let raw = "**bold** and *it* and lone * kept `*z*`";
        let line = style_slice(raw, &s);
        // Two balanced pairs (`**`+`**` and `*`+`*`) → 6 sigil bytes removed;
        // the lone `*` and the in-code `*x*` survive.
        let expected = "bold and it and lone * kept `*z*`";
        assert_eq!(rendered(&line), expected);
    }

    #[test]
    fn column_map_skips_hidden_sigils_and_points_at_source_bytes() {
        let s = styles();
        let raw = "**b** [1]";
        let (line, map) = style_line_mapped(raw, &s);
        assert_eq!(rendered(&line), "b [1]");
        // Rendered chars: 'b'(raw 2) ' '(raw 5) '['(raw 6) '1'(raw 7) ']'(raw 8).
        assert_eq!(map, vec![2, 5, 6, 7, 8]);
    }

    #[test]
    fn non_normal_kinds_keep_the_identity_map() {
        let s = styles();
        let (_, map) = style_line_mapped("## Head", &s);
        assert_eq!(map, (0.."## Head".len()).collect::<Vec<_>>());
    }

    #[test]
    fn intraword_underscores_are_left_verbatim() {
        let s = styles();
        // snake_case identifiers must not be mangled: the `_` are intraword, so
        // they neither open nor close emphasis — kept literal, nothing styled.
        for src in ["foo_bar_baz", "some__thing__glued"] {
            let line = style_slice(src, &s);
            assert_eq!(rendered(&line), src, "{src} stays verbatim");
            assert!(
                line.spans
                    .iter()
                    .all(|sp| sp.style != s.italic && sp.style != s.bold),
                "{src} gets no emphasis styling"
            );
        }
    }

    #[test]
    fn word_boundary_underscores_still_emphasize() {
        let s = styles();
        // `_word_` at word boundaries italicizes and hides its sigils.
        let line = style_slice("_word_", &s);
        assert_eq!(rendered(&line), "word");
        let italic: String = line
            .spans
            .iter()
            .filter(|sp| sp.style == s.italic)
            .map(|sp| sp.content.as_ref())
            .collect();
        assert_eq!(italic, "word");

        // `__dunder__` at word boundaries bolds and hides its sigils.
        let line = style_slice("__dunder__", &s);
        assert_eq!(rendered(&line), "dunder");
        let bold: String = line
            .spans
            .iter()
            .filter(|sp| sp.style == s.bold)
            .map(|sp| sp.content.as_ref())
            .collect();
        assert_eq!(bold, "dunder");
    }

    #[test]
    fn mixed_line_keeps_snake_case_and_styles_real_emphasis_with_correct_map() {
        let s = styles();
        let raw = "snake_case and _real_ emphasis";
        let (line, map) = style_line_mapped(raw, &s);
        // The intraword `_` in snake_case stay; only `_real_`'s sigils are hidden.
        assert_eq!(rendered(&line), "snake_case and real emphasis");
        let italic: String = line
            .spans
            .iter()
            .filter(|sp| sp.style == s.italic)
            .map(|sp| sp.content.as_ref())
            .collect();
        assert_eq!(italic, "real", "only the boundary emphasis is styled");

        // The column map still points every rendered char at its source byte:
        // reconstructing the rendered text via the map reproduces it exactly.
        let rebuilt: String = map
            .iter()
            .map(|&b| raw[b..].chars().next().unwrap())
            .collect();
        assert_eq!(rebuilt, "snake_case and real emphasis");
        // The rendered `real` maps back to the source `real` (byte 16), not the
        // hidden `_` at byte 15.
        let real_col = rendered(&line).find("real").unwrap();
        assert_eq!(&raw[map[real_col]..map[real_col] + 4], "real");
    }

    #[test]
    fn strikethrough_hides_its_sigils_and_crosses_the_run_out() {
        let s = styles();
        let line = style_slice("~~gone~~ [1]", &s);
        assert_eq!(rendered(&line), "gone [1]");
        assert_eq!(line.spans[0].content, "gone");
        assert_eq!(
            line.spans[0].style,
            s.base.add_modifier(Modifier::CROSSED_OUT)
        );
    }

    #[test]
    fn emphasis_pairs_across_a_wrapped_slice_boundary() {
        // The rules see the whole line, so `**a b**` wrapped after `a` is bold
        // on both rows, each slice dropping only its own sigils.
        let s = styles();
        let raw = "**a b**";
        let lines = parse_answer(raw);
        let (first, first_map) = style_slice_mapped(raw, &lines[0], 0..3, &s);
        let (second, second_map) = style_slice_mapped(raw, &lines[0], 4..7, &s);
        assert_eq!(rendered(&first), "a");
        assert_eq!(first.spans[0].style, s.bold);
        assert_eq!(first_map, vec![2]);
        assert_eq!(rendered(&second), "b");
        assert_eq!(second.spans[0].style, s.bold);
        assert_eq!(second_map, vec![0]);
    }
}
