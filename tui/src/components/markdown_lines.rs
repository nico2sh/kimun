//! A lean, line-level markdown styler shared by surfaces that want to *style*
//! markdown without editing it (currently the Ask workspace's answer body).
//!
//! Design constraints that shape it:
//!
//! - **Emphasis sigils are hidden; structural markers stay.** Balanced
//!   `**`/`__` (bold) and `*`/`_` (italic) delimiters are dropped from the
//!   rendered text (the run between them is styled instead), matching what a
//!   reader expects. Everything else stays visible: `#` headings, `>` quotes,
//!   fences, list markers, and citation `[n]` markers. Because hiding breaks
//!   the old 1:1 byte↔column identity, [`style_slice_mapped`] emits, alongside
//!   the styled line, a **column map** (`rendered char index → source byte
//!   offset`) so callers can still hit-test a click back to the right source
//!   byte. (This is why we don't reuse the editor's `ParsedBuffer`, which fully
//!   re-lays-out the visual line.)
//! - **Only same-line, balanced pairs are hidden.** A sigil is hidden only when
//!   an opener and a closer of the same kind appear in the *same wrapped slice*
//!   (the per-slice approximation — we never look across the wrap boundary). A
//!   lone `*` (an unmatched sigil, a bullet, arithmetic) stays visible and does
//!   not emphasize anything. Sigils inside inline code are literal.
//! - **Intraword `_` is not emphasis.** Following CommonMark, a `_`/`__` run may
//!   open only when the char before it is absent/non-alphanumeric and close only
//!   when the char after it is — so `snake_case` and `foo_bar_baz` identifiers
//!   render verbatim. `*`/`**` keep the laxer rule (intraword `*` is legal).
//! - **Citations are the citations module's job.** `[n]` markers are found
//!   only through [`crate::ask::citations::scan`]; we merely *style* the ranges
//!   it reports. Code (fenced blocks and inline spans) is never citation-styled.
//!
//! The unit of work is one *logical* source line: [`classify`] labels it (while
//! threading fenced-code-block state), and [`style_slice_mapped`] styles one
//! wrapped visual slice of it and returns its column map.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ask::citations;
use crate::settings::themes::Theme;

/// The block role of one logical source line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    /// A fenced-code delimiter (```` ``` ````/`~~~`) or a line inside a fence:
    /// styled as code verbatim, with no inline markdown or citation restyling.
    Code,
    /// An ATX heading (`#`..`######`).
    Heading,
    /// A blockquote line (`>`).
    Quote,
    /// Paragraph text or a list item — inline styling (bold/italic/inline code)
    /// and citations apply.
    Normal,
}

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
    /// (`text_editor::markdown::span_style`): headings bright+bold, inline/code
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
}

/// Classify one logical (newline-free) source `line`, threading fenced-code
/// state through `in_fence`: a fence delimiter flips it, and every line while
/// it is set is [`LineKind::Code`]. Call in source order so the state stays
/// coherent across lines.
pub fn classify(line: &str, in_fence: &mut bool) -> LineKind {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        // The fence delimiter line itself renders as code; the state flips for
        // the lines that follow.
        *in_fence = !*in_fence;
        return LineKind::Code;
    }
    if *in_fence {
        return LineKind::Code;
    }
    if is_atx_heading(trimmed) {
        return LineKind::Heading;
    }
    if trimmed.starts_with('>') {
        return LineKind::Quote;
    }
    LineKind::Normal
}

/// An ATX heading is 1–6 `#` followed by a space or end-of-line.
fn is_atx_heading(trimmed: &str) -> bool {
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes)
        && trimmed[hashes..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ' || c == '\t')
}

/// Style one wrapped visual `slice` of a logical line whose block role is
/// `kind`, returning the styled [`Line`] and its **column map**: `map[k]` is
/// the source byte offset (into `slice`) of the `k`-th *rendered* character.
///
/// For every kind except `Normal` nothing is hidden, so the map is the identity
/// over the slice's chars. For `Normal`, balanced emphasis sigils are dropped
/// (see the module doc), so `map` skips their bytes — a caller resolving a
/// rendered column back to a source byte walks `map`.
///
/// `slice` must be the exact source text shown on the row (structural markers
/// included).
pub fn style_slice_mapped(slice: &str, kind: LineKind, styles: &MdStyles) -> (Line<'static>, Vec<usize>) {
    match kind {
        LineKind::Code => whole_slice(slice, styles.code),
        LineKind::Heading => whole_slice(slice, styles.heading),
        LineKind::Quote => whole_slice(slice, styles.quote),
        LineKind::Normal => inline_spans(slice, styles),
    }
}

/// Style `slice` as a single verbatim span (no hiding) with the identity map.
fn whole_slice(slice: &str, style: Style) -> (Line<'static>, Vec<usize>) {
    let map: Vec<usize> = slice.char_indices().map(|(i, _)| i).collect();
    (Line::from(Span::styled(slice.to_string(), style)), map)
}

/// Split a `Normal` slice into styled spans — dropping balanced emphasis
/// sigils and returning the column map alongside. Inline code (`` `…` ``) is
/// verbatim; citation `[n]` ranges (from [`citations::scan`]) win over
/// emphasis; inline code wins over everything and is never citation-styled.
/// The concatenation of the returned spans equals `slice` with exactly the
/// hidden sigil pairs removed.
fn inline_spans(slice: &str, styles: &MdStyles) -> (Line<'static>, Vec<usize>) {
    let chars: Vec<(usize, char)> = slice.char_indices().collect();
    let code_mask = code_mask(&chars);
    let (hidden, bold, italic) = analyze_emphasis(&chars, &code_mask);

    let cites = citations::scan(slice);
    let is_cited = |i: usize| cites.iter().any(|c| c.range.contains(&i));

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut buf_style = styles.base;
    let mut map: Vec<usize> = Vec::new();

    for (k, &(i, ch)) in chars.iter().enumerate() {
        if hidden[k] {
            continue; // a balanced sigil — dropped from the rendered text.
        }
        let style = if code_mask[k] {
            styles.code
        } else if is_cited(i) {
            styles.citation
        } else if bold[k] {
            styles.bold
        } else if italic[k] {
            styles.italic
        } else {
            styles.base
        };
        if style != buf_style && !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut buf), buf_style));
        }
        buf_style = style;
        buf.push(ch);
        map.push(i);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, buf_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), styles.base));
    }
    (Line::from(spans), map)
}

/// Per-char mask marking inline-code spans (backticks included). A backtick
/// opens a span; every char up to and including the next backtick is code. An
/// unclosed span runs to the slice end (matching how a terminal would show it).
fn code_mask(chars: &[(usize, char)]) -> Vec<bool> {
    let mut mask = vec![false; chars.len()];
    let mut in_code = false;
    for (k, &(_, ch)) in chars.iter().enumerate() {
        if in_code {
            mask[k] = true;
            if ch == '`' {
                in_code = false;
            }
        } else if ch == '`' {
            in_code = true;
            mask[k] = true;
        }
    }
    mask
}

/// The four emphasis delimiter kinds, each paired independently.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Emph {
    Star,        // `*…*`  → italic
    Under,       // `_…_`  → italic
    DoubleStar,  // `**…**` → bold
    DoubleUnder, // `__…__` → bold
}

struct Delim {
    /// First char index of the delimiter.
    k: usize,
    /// Number of chars (1 or 2).
    len: usize,
    kind: Emph,
    /// Whether this run may *open* emphasis — always true for `*`/`**`; for
    /// `_`/`__` only when the preceding char is absent or non-alphanumeric
    /// (CommonMark's intraword-underscore rule, simplified left-flanking).
    can_open: bool,
    /// Whether this run may *close* emphasis — always true for `*`/`**`; for
    /// `_`/`__` only when the following char is absent or non-alphanumeric.
    can_close: bool,
}

/// Decide, per char, which emphasis sigils to *hide* and which chars fall under
/// bold / italic styling. Delimiters are found outside inline code and paired
/// within each kind with a stack (nearest matching opener). `*`/`**` open and
/// close freely (intraword `*` is legal CommonMark); `_`/`__` obey the
/// intraword rule — a `_` run may only open when the char before it is
/// absent/non-alphanumeric and only close when the char after it is, so
/// `snake_case` identifiers are never mangled. An unmatched delimiter stays
/// visible and styles nothing. This is the per-slice approximation — we never
/// pair across the wrap boundary.
fn analyze_emphasis(chars: &[(usize, char)], code_mask: &[bool]) -> (Vec<bool>, Vec<bool>, Vec<bool>) {
    let n = chars.len();
    let mut hidden = vec![false; n];
    let mut bold = vec![false; n];
    let mut italic = vec![false; n];

    // Collect delimiter tokens (greedy: `**`/`__` before `*`/`_`), recording
    // each run's open/close capability from its flanking chars.
    let mut delims: Vec<Delim> = Vec::new();
    let mut k = 0;
    while k < n {
        if code_mask[k] {
            k += 1;
            continue;
        }
        let ch = chars[k].1;
        let next_same = k + 1 < n && !code_mask[k + 1] && chars[k + 1].1 == ch;
        let (len, kind) = match ch {
            '*' if next_same => (2, Emph::DoubleStar),
            '_' if next_same => (2, Emph::DoubleUnder),
            '*' => (1, Emph::Star),
            '_' => (1, Emph::Under),
            _ => {
                k += 1;
                continue;
            }
        };
        let is_under = matches!(kind, Emph::Under | Emph::DoubleUnder);
        let before = (k > 0).then(|| chars[k - 1].1);
        let after = chars.get(k + len).map(|&(_, c)| c);
        let free = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        let can_open = !is_under || free(before);
        let can_close = !is_under || free(after);
        delims.push(Delim { k, len, kind, can_open, can_close });
        k += len;
    }

    // Pair each kind with a stack: a closer binds to the nearest open delimiter
    // of the same kind. Matched pairs hide their sigils and style the run.
    for kind in [Emph::Star, Emph::Under, Emph::DoubleStar, Emph::DoubleUnder] {
        let is_bold = matches!(kind, Emph::DoubleStar | Emph::DoubleUnder);
        let mut open_stack: Vec<usize> = Vec::new();
        for (di, d) in delims.iter().enumerate() {
            if d.kind != kind {
                continue;
            }
            if d.can_close && !open_stack.is_empty() {
                let oi = open_stack.pop().unwrap();
                let (open_k, open_len) = (delims[oi].k, delims[oi].len);
                let close_k = d.k;
                for slot in &mut hidden[open_k..open_k + open_len] {
                    *slot = true;
                }
                for slot in &mut hidden[close_k..close_k + d.len] {
                    *slot = true;
                }
                let run = &mut (if is_bold { &mut bold } else { &mut italic })[open_k + open_len..close_k];
                for slot in run {
                    *slot = true;
                }
            } else if d.can_open {
                open_stack.push(di);
            }
        }
    }
    (hidden, bold, italic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles() -> MdStyles {
        MdStyles::from_theme(&Theme::default())
    }

    /// The styled spans, concatenated — the rendered text of a line.
    fn rendered(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Just the styled line (the common case; the map is asserted separately).
    fn style_slice(slice: &str, kind: LineKind, styles: &MdStyles) -> Line<'static> {
        style_slice_mapped(slice, kind, styles).0
    }

    #[test]
    fn classify_toggles_fenced_code_blocks() {
        let mut fence = false;
        assert_eq!(classify("```rust", &mut fence), LineKind::Code); // opener
        assert_eq!(classify("let x = 1;", &mut fence), LineKind::Code); // body
        assert_eq!(classify("```", &mut fence), LineKind::Code); // closer
        assert_eq!(classify("after", &mut fence), LineKind::Normal); // out again
    }

    #[test]
    fn classify_labels_headings_and_quotes() {
        let mut fence = false;
        assert_eq!(classify("# Title", &mut fence), LineKind::Heading);
        assert_eq!(classify("###### h6", &mut fence), LineKind::Heading);
        assert_eq!(classify("####### too many", &mut fence), LineKind::Normal);
        assert_eq!(classify("#nospace", &mut fence), LineKind::Normal);
        assert_eq!(classify("> quoted", &mut fence), LineKind::Quote);
        assert_eq!(classify("plain text", &mut fence), LineKind::Normal);
    }

    #[test]
    fn code_slice_is_never_citation_styled() {
        let s = styles();
        // A `[1]` sitting inside a code line keeps the code style — no accent.
        let line = style_slice("let n = arr[1];", LineKind::Code, &s);
        assert_eq!(line.spans.len(), 1, "code renders as one verbatim span");
        assert_eq!(line.spans[0].style, s.code);
        assert!(line.spans[0].style != s.citation);
        assert_eq!(rendered(&line), "let n = arr[1];");
    }

    #[test]
    fn heading_slice_gets_heading_styling() {
        let s = styles();
        let line = style_slice("## Overview", LineKind::Heading, &s);
        assert_eq!(line.spans[0].style, s.heading);
        assert_eq!(rendered(&line), "## Overview");
    }

    #[test]
    fn prose_citation_gets_citation_style_and_preserves_bytes() {
        let s = styles();
        let line = style_slice("See [1] and [2].", LineKind::Normal, &s);
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
        let line = style_slice("a **b** `c` d", LineKind::Normal, &s);
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
        for (src, want) in [("an *em* word", "an em word"), ("an _em_ word", "an em word")] {
            let line = style_slice(src, LineKind::Normal, &s);
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
        let line = style_slice("2 * 3 = 6 and rest", LineKind::Normal, &s);
        assert_eq!(rendered(&line), "2 * 3 = 6 and rest", "lone sigil kept");
        assert!(
            line.spans.iter().all(|sp| sp.style != s.italic),
            "no run is italicized by an unmatched sigil"
        );
    }

    #[test]
    fn emphasis_inside_a_code_span_stays_literal() {
        let s = styles();
        // The `*x*` lives inside inline code — its asterisks are verbatim.
        let line = style_slice("call `*x*` now", LineKind::Normal, &s);
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
        let line = style_slice(raw, LineKind::Normal, &s);
        // Two balanced pairs (`**`+`**` and `*`+`*`) → 6 sigil bytes removed;
        // the lone `*` and the in-code `*x*` survive.
        let expected = "bold and it and lone * kept `*z*`";
        assert_eq!(rendered(&line), expected);
    }

    #[test]
    fn column_map_skips_hidden_sigils_and_points_at_source_bytes() {
        let s = styles();
        let raw = "**b** [1]";
        let (line, map) = style_slice_mapped(raw, LineKind::Normal, &s);
        assert_eq!(rendered(&line), "b [1]");
        // Rendered chars: 'b'(raw 2) ' '(raw 5) '['(raw 6) '1'(raw 7) ']'(raw 8).
        assert_eq!(map, vec![2, 5, 6, 7, 8]);
    }

    #[test]
    fn non_normal_kinds_keep_the_identity_map() {
        let s = styles();
        let (_, map) = style_slice_mapped("## Head", LineKind::Heading, &s);
        assert_eq!(map, (0.."## Head".len()).collect::<Vec<_>>());
    }

    #[test]
    fn intraword_underscores_are_left_verbatim() {
        let s = styles();
        // snake_case identifiers must not be mangled: the `_` are intraword, so
        // they neither open nor close emphasis — kept literal, nothing styled.
        for src in ["foo_bar_baz", "some__thing__glued"] {
            let line = style_slice(src, LineKind::Normal, &s);
            assert_eq!(rendered(&line), src, "{src} stays verbatim");
            assert!(
                line.spans.iter().all(|sp| sp.style != s.italic && sp.style != s.bold),
                "{src} gets no emphasis styling"
            );
        }
    }

    #[test]
    fn word_boundary_underscores_still_emphasize() {
        let s = styles();
        // `_word_` at word boundaries italicizes and hides its sigils.
        let line = style_slice("_word_", LineKind::Normal, &s);
        assert_eq!(rendered(&line), "word");
        let italic: String = line
            .spans
            .iter()
            .filter(|sp| sp.style == s.italic)
            .map(|sp| sp.content.as_ref())
            .collect();
        assert_eq!(italic, "word");

        // `__dunder__` at word boundaries bolds and hides its sigils.
        let line = style_slice("__dunder__", LineKind::Normal, &s);
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
        let (line, map) = style_slice_mapped(raw, LineKind::Normal, &s);
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
        let rebuilt: String = map.iter().map(|&b| raw[b..].chars().next().unwrap()).collect();
        assert_eq!(rebuilt, "snake_case and real emphasis");
        // The rendered `real` maps back to the source `real` (byte 16), not the
        // hidden `_` at byte 15.
        let real_col = rendered(&line).find("real").unwrap();
        assert_eq!(&raw[map[real_col]..map[real_col] + 4], "real");
    }
}
