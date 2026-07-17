//! A lean, line-level markdown styler shared by surfaces that want to *style*
//! markdown without editing it (currently the Ask workspace's answer body).
//!
//! Design constraints that shape it:
//!
//! - **Markers stay visible.** Nothing is inserted or deleted — every input
//!   byte appears once, in order, in the styled output. Callers that wrap the
//!   source into byte-range slices (the thread's `wrap_text`) can therefore
//!   keep hit-testing those exact ranges: the displayed columns still map 1:1
//!   to the source bytes. (This is why we don't reuse the editor's
//!   `ParsedBuffer`, which collapses sigils and rewrites the visual line.)
//! - **Citations are the citations module's job.** `[n]` markers are found
//!   only through [`crate::ask::citations::scan`]; we merely *style* the ranges
//!   it reports. Code (fenced blocks and inline spans) is never citation-styled.
//!
//! The unit of work is one *logical* source line: [`classify`] labels it (while
//! threading fenced-code-block state), and [`style_slice`] styles one wrapped
//! visual slice of it.

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
/// `kind`. `slice` must be the exact source text shown on the row (markers
/// included), so the result stays byte-for-byte aligned with it.
pub fn style_slice(slice: &str, kind: LineKind, styles: &MdStyles) -> Line<'static> {
    match kind {
        LineKind::Code => Line::from(Span::styled(slice.to_string(), styles.code)),
        LineKind::Heading => Line::from(Span::styled(slice.to_string(), styles.heading)),
        LineKind::Quote => Line::from(Span::styled(slice.to_string(), styles.quote)),
        LineKind::Normal => Line::from(inline_spans(slice, styles)),
    }
}

/// Split a `Normal` slice into styled spans: inline code (`` `…` ``), bold
/// (`**…**`), italic (`*…*` / `_…_`), and citation `[n]` markers, with all
/// marker characters kept visible. Citation ranges (from
/// [`citations::scan`]) win over emphasis; inline code wins over everything and
/// is never citation-styled. The concatenation of the returned spans always
/// equals `slice`.
fn inline_spans(slice: &str, styles: &MdStyles) -> Vec<Span<'static>> {
    let cites = citations::scan(slice);
    let is_cited = |i: usize| cites.iter().any(|c| c.range.contains(&i));

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut buf_style = styles.base;
    let mut in_code = false;
    let mut in_bold = false;
    let mut in_italic = false;

    let mut push = |buf: &mut String, buf_style: &mut Style, text: &str, style: Style| {
        if style != *buf_style && !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), *buf_style));
        }
        *buf_style = style;
        buf.push_str(text);
    };

    let chars: Vec<(usize, char)> = slice.char_indices().collect();
    let mut k = 0;
    while k < chars.len() {
        let (i, ch) = chars[k];
        if in_code {
            let style = styles.code;
            push(&mut buf, &mut buf_style, &ch.to_string(), style);
            if ch == '`' {
                in_code = false;
            }
            k += 1;
            continue;
        }
        // Inline-code open.
        if ch == '`' {
            in_code = true;
            push(&mut buf, &mut buf_style, "`", styles.code);
            k += 1;
            continue;
        }
        // Citations win over emphasis; keep the whole marker in one style.
        if is_cited(i) {
            let style = styles.citation;
            push(&mut buf, &mut buf_style, &ch.to_string(), style);
            k += 1;
            continue;
        }
        // Bold `**` (consume both markers into the bold style).
        if ch == '*' && k + 1 < chars.len() && chars[k + 1].1 == '*' {
            in_bold = !in_bold;
            push(&mut buf, &mut buf_style, "**", styles.bold);
            k += 2;
            continue;
        }
        // Italic `*` / `_`.
        if ch == '*' || ch == '_' {
            in_italic = !in_italic;
            let style = emphasis_style(in_bold, true, styles);
            push(&mut buf, &mut buf_style, &ch.to_string(), style);
            k += 1;
            continue;
        }
        let style = emphasis_style(in_bold, in_italic, styles);
        push(&mut buf, &mut buf_style, &ch.to_string(), style);
        k += 1;
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, buf_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), styles.base));
    }
    spans
}

/// The style for a plain character given the open emphasis state: bold takes
/// precedence over italic, and neither falls back to `base`.
fn emphasis_style(bold: bool, italic: bool, styles: &MdStyles) -> Style {
    if bold {
        styles.bold
    } else if italic {
        styles.italic
    } else {
        styles.base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles() -> MdStyles {
        MdStyles::from_theme(&Theme::default())
    }

    /// The styled spans, concatenated, must reproduce the input exactly — the
    /// 1:1 invariant that keeps citation hit-testing aligned with the source.
    fn rendered(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
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
    fn inline_markdown_styles_bold_and_code_without_dropping_bytes() {
        let s = styles();
        let line = style_slice("a **b** `c` d", LineKind::Normal, &s);
        assert_eq!(rendered(&line), "a **b** `c` d");
        assert!(
            line.spans.iter().any(|sp| sp.style == s.bold),
            "bold run is styled"
        );
        assert!(
            line.spans.iter().any(|sp| sp.style == s.code),
            "inline code run is styled"
        );
    }
}
