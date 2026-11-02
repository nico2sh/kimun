use crate::logic::editor::markdown::{ElementKind, MOD_BOLD, MOD_ITALIC, MOD_STRIKE};
use crate::settings::themes::Theme;
use ratatui::style::{Modifier, Style};

/// Translates a `MOD_*` mask into ratatui [`Modifier`] flags.
pub(super) fn mask_to_modifier(mask: u8) -> Modifier {
    let mut m = Modifier::empty();
    if mask & MOD_BOLD != 0 {
        m |= Modifier::BOLD;
    }
    if mask & MOD_ITALIC != 0 {
        m |= Modifier::ITALIC;
    }
    if mask & MOD_STRIKE != 0 {
        m |= Modifier::CROSSED_OUT;
    }
    m
}

pub(super) fn span_style(kind: Option<ElementKind>, is_sigil_region: bool, theme: &Theme) -> Style {
    match kind {
        None => {
            if is_sigil_region {
                Style::default().fg(theme.gray.to_ratatui())
            } else {
                Style::default().fg(theme.fg.to_ratatui())
            }
        }
        Some(ElementKind::Bold) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::BOLD),
        Some(ElementKind::Italic) => Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(Modifier::ITALIC),
        Some(ElementKind::Strikethrough) => Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(Modifier::CROSSED_OUT),
        Some(ElementKind::InlineCode) => Style::default()
            .fg(theme.aqua.to_ratatui())
            .bg(theme.bg_soft.to_ratatui()),
        Some(ElementKind::Link) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::UNDERLINED),
        Some(ElementKind::Image) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::ITALIC),
        // H1/H2 bright + bold, H3 yellow + bold.
        Some(ElementKind::HeadingH1) | Some(ElementKind::HeadingH2) => {
            if is_sigil_region {
                Style::default().fg(theme.gray.to_ratatui())
            } else {
                Style::default()
                    .fg(theme.fg_bright.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            }
        }
        Some(ElementKind::HeadingH3) => {
            if is_sigil_region {
                Style::default().fg(theme.gray.to_ratatui())
            } else {
                Style::default()
                    .fg(theme.yellow.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            }
        }
        Some(ElementKind::Blockquote) => Style::default().fg(theme.fg_secondary.to_ratatui()),
        // Wikilink targets are blue + underlined.
        Some(ElementKind::WikiLink) => Style::default()
            .fg(theme.blue.to_ratatui())
            .add_modifier(Modifier::UNDERLINED),
        Some(ElementKind::Label) => Style::default()
            .fg(theme.color_tag.to_ratatui())
            .add_modifier(Modifier::BOLD),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::text_editor::markdown::MarkdownSpanner;
    use crate::logic::editor::markdown::*;
    use ratatui::text::Span;
    fn t() -> Theme {
        Theme::default()
    }
    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }
    #[test]
    fn parse_bold_range() {
        let e = MarkdownSpanner::parse_elements("**bold**");
        let b = e.iter().find(|x| x.kind == ElementKind::Bold).unwrap();
        assert_eq!((b.start_char, b.end_char), (0, 8));
    }
    #[test]
    fn parse_italic() {
        assert!(
            MarkdownSpanner::parse_elements("*hi*")
                .iter()
                .any(|e| e.kind == ElementKind::Italic)
        );
    }
    #[test]
    fn parse_strikethrough() {
        let e = MarkdownSpanner::parse_elements("~~gone~~");
        let s = e
            .iter()
            .find(|x| x.kind == ElementKind::Strikethrough)
            .unwrap();
        assert_eq!((s.start_char, s.end_char), (0, 8));
    }
    #[test]
    fn strikethrough_renders_with_crossed_out_modifier() {
        let s = MarkdownSpanner::render("~~gone~~", "~~gone~~", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "gone");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::CROSSED_OUT))
        );
    }
    #[test]
    fn parse_inline_code() {
        assert!(
            MarkdownSpanner::parse_elements("`x`")
                .iter()
                .any(|e| e.kind == ElementKind::InlineCode)
        );
    }
    #[test]
    fn parse_link() {
        assert!(
            MarkdownSpanner::parse_elements("[t](u)")
                .iter()
                .any(|e| e.kind == ElementKind::Link)
        );
    }

    #[test]
    fn render_image_substitutes_placeholder_text() {
        let line = "before ![alt](pic.gif) after";
        let parsed = ParsedLine::parse(line);
        let spans =
            MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 80, &t());
        let rendered: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("[pic.gif]"),
            "rendered text {rendered:?} should include placeholder"
        );
        assert!(
            !rendered.contains("![alt]"),
            "raw image syntax should not appear in rendered output: {rendered:?}"
        );
    }

    #[test]
    fn render_image_with_empty_alt_uses_filename() {
        let line = "![](image.png)";
        let parsed = ParsedLine::parse(line);
        let spans =
            MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 40, &t());
        let rendered: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(rendered, "[image.png]");
    }

    #[test]
    fn rendered_cursor_col_accounts_for_placeholder_width() {
        // "![](x.png)" → placeholder "[x.png]" (7 chars) replaces 10 source chars.
        let line = "a ![](x.png) b";
        let parsed = ParsedLine::parse(line);
        let after_placeholder = MarkdownSpanner::rendered_cursor_col_with(
            line,
            &parsed,
            0,
            "a ![](x.png) b".chars().count(), // cursor at end
            true,
            false,
        );
        // "a " (2) + "[x.png]" (7) + " b" (2) = 11.
        assert_eq!(after_placeholder, 11);
    }
    #[test]
    fn parse_h1() {
        assert!(
            MarkdownSpanner::parse_elements("# T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH1)
        );
    }
    #[test]
    fn parse_h2() {
        assert!(
            MarkdownSpanner::parse_elements("## T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH2)
        );
    }
    #[test]
    fn parse_h3() {
        assert!(
            MarkdownSpanner::parse_elements("### T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH3)
        );
    }
    #[test]
    fn force_raw_no_styling() {
        let s = MarkdownSpanner::render("**x**", "**x**", 0, None, true, true, 40, &t());
        assert_eq!(text(&s), "**x**");
        assert!(
            !s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD))
        );
    }
    #[test]
    fn plain_text_passthrough() {
        let s = MarkdownSpanner::render("hi", "hi", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "hi");
    }
    #[test]
    fn bold_without_cursor_hides_markers() {
        let s = MarkdownSpanner::render("**bold**", "**bold**", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "bold");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD))
        );
    }
    #[test]
    fn bold_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("**bold**", "**bold**", 0, Some(3), true, false, 40, &t());
        assert_eq!(text(&s), "**bold**");
    }
    #[test]
    fn bold_cursor_outside_stays_rendered() {
        let line = "hello **bold** world";
        let s = MarkdownSpanner::render(line, line, 0, Some(1), true, false, 40, &t());
        assert!(!text(&s).contains("**"));
    }
    #[test]
    fn italic_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("*hi*", "*hi*", 0, Some(1), true, false, 40, &t());
        assert_eq!(text(&s), "*hi*");
    }
    #[test]
    fn inline_code_hides_backticks() {
        let s = MarkdownSpanner::render("`x`", "`x`", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "x");
    }
    #[test]
    fn h1_first_line_contains_hash() {
        let s = MarkdownSpanner::render("# T", "# T", 0, None, true, false, 40, &t());
        assert!(text(&s).contains('#'));
        assert!(text(&s).contains('T'));
    }
    #[test]
    fn continuation_line_no_hash() {
        let s = MarkdownSpanner::render("cont", "# T cont", 2, None, false, false, 40, &t());
        assert!(!text(&s).contains('#'));
    }
    #[test]
    fn unordered_list_shows_marker() {
        let s = MarkdownSpanner::render("- item", "- item", 0, None, true, false, 40, &t());
        assert!(
            text(&s).starts_with("- "),
            "expected '- item', got '{}'",
            text(&s)
        );
        assert!(text(&s).contains("item"));
    }
    #[test]
    fn ordered_list_shows_marker() {
        let s = MarkdownSpanner::render("1. item", "1. item", 0, None, true, false, 40, &t());
        assert!(
            text(&s).starts_with("1. "),
            "expected '1. item', got '{}'",
            text(&s)
        );
    }
    #[test]
    fn nested_list_4space_link_rendered() {
        // 4-space indent + list marker + markdown link.
        let line = "    - [my link](url)";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        // Link styling must appear (UNDERLINED modifier) and the raw "](url)" sigils
        // must be hidden.
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED)),
            "link text should be underlined on a 4-space-indented nested list item"
        );
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            rendered.contains("my link"),
            "link display text should be visible; got {:?}",
            rendered
        );
        assert!(
            !rendered.contains("](url)"),
            "link URL sigil should be hidden; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_tab_bold_rendered() {
        let line = "\t- **bold nested**";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD)),
            "bold text should be styled on a tab-indented nested list item"
        );
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            !rendered.contains("**"),
            "bold markers should be hidden; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_4space_wikilink_rendered() {
        let line = "    - [[Target Note]]";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            !rendered.contains("[["),
            "wikilink brackets should be hidden; got {:?}",
            rendered
        );
        assert!(
            rendered.contains("Target Note"),
            "wikilink target text should render; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_2space_still_renders_link() {
        // Existing 2-space case — must not regress.
        let line = "  - [link](url)";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn empty_heading_shows_hash_sigil() {
        let line = "# ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert!(
            text(&s).contains('#'),
            "hash sigil should render in empty heading"
        );
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 1, true, false);
        assert_eq!(col, 1, "cursor after '#' should be at rendered col 1");
    }
    #[test]
    fn empty_heading_hash_only_shows() {
        let line = "#";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert!(text(&s).contains('#'));
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 1, true, false);
        assert_eq!(col, 1);
    }
    #[test]
    fn heading_trailing_spaces_are_rendered() {
        let line = "# Hello   ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(
            text(&s),
            "# Hello   ",
            "trailing spaces in heading should render"
        );
    }
    #[test]
    fn heading_trailing_spaces_cursor_col_correct() {
        let line = "# Hello   ";
        // cursor at logical pos 9 (last trailing space): positions 0..9 all emit → rendered col 9
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 9, true, false);
        assert_eq!(
            col, 9,
            "cursor in trailing space of heading should map to rendered col 9"
        );
    }
    #[test]
    fn trailing_spaces_are_rendered() {
        let line = "hello   ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "hello   ");
    }
    #[test]
    fn trailing_spaces_cursor_col_correct() {
        let line = "hello   ";
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 7, true, false);
        assert_eq!(col, 7);
    }
    #[test]
    fn list_marker_on_continuation_line_hidden() {
        let s = MarkdownSpanner::render("cont", "- cont", 2, None, false, false, 40, &t());
        assert!(!text(&s).starts_with("- "));
    }
    #[test]
    fn parsed_line_reuse_matches_individual() {
        let line = "**hello** world";
        let parsed = ParsedLine::parse(line);
        let s1 = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        let s2 = MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 40, &t());
        assert_eq!(
            s1.iter().map(|s| s.content.as_ref()).collect::<String>(),
            s2.iter().map(|s| s.content.as_ref()).collect::<String>(),
        );
    }

    // ── WikiLink tests ────────────────────────────────────────────────────────

    #[test]
    fn parse_wikilink() {
        let e = MarkdownSpanner::parse_elements("[[My Note]]");
        let wl = e.iter().find(|x| x.kind == ElementKind::WikiLink).unwrap();
        assert_eq!((wl.start_char, wl.end_char), (0, 11));
    }

    #[test]
    fn wikilink_without_cursor_hides_brackets() {
        let line = "[[My Note]]";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "My Note");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn wikilink_cursor_inside_shows_brackets() {
        let line = "[[My Note]]";
        // cursor at pos 4 (inside "My Note")
        let s = MarkdownSpanner::render(line, line, 0, Some(4), true, false, 40, &t());
        assert_eq!(text(&s), "[[My Note]]");
    }

    #[test]
    fn wikilink_cursor_outside_hides_brackets() {
        let line = "hello [[My Note]] world";
        let s = MarkdownSpanner::render(line, line, 0, Some(1), true, false, 40, &t());
        assert!(!text(&s).contains("[["));
        assert!(!text(&s).contains("]]"));
    }

    #[test]
    fn wikilink_in_heading_rendered() {
        let line = "# See [[Topic]]";
        let e = MarkdownSpanner::parse_elements(line);
        assert!(
            e.iter().any(|x| x.kind == ElementKind::WikiLink),
            "wikilink inside heading should produce a WikiLink element"
        );
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "# See Topic", "wikilink brackets hidden, # kept");
    }

    #[test]
    fn heading_with_link_does_not_leak_bracket() {
        let line = "# [text](http://x)";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        // `# ` sigil stays visible (heading marker); link sigils incl. the
        // leading `[` are hidden, leaving just the display text.
        assert_eq!(text(&s), "# text");
    }

    #[test]
    fn heading_with_bold_does_not_leak_asterisk() {
        let line = "# **bold**";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(
            text(&s),
            "# bold",
            "leading * must not leak into heading sigil"
        );
    }

    #[test]
    fn bold_wikilink_is_bold() {
        let line = "**[[Topic]]**";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "Topic");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD)
                    && sp.content.contains("Topic")),
            "wikilink wrapped in ** must render bold"
        );
    }

    #[test]
    fn italic_link_is_italic() {
        let line = "*[text](http://x)*";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "text");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::ITALIC)
                    && sp.content.contains("text")),
            "link wrapped in * must render italic"
        );
    }

    #[test]
    fn bold_italic_wikilink_is_bold_and_italic() {
        let line = "***[[Topic]]***";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "Topic");
        assert!(
            s.iter().any(|sp| {
                sp.content.contains("Topic")
                    && sp.style.add_modifier.contains(Modifier::BOLD)
                    && sp.style.add_modifier.contains(Modifier::ITALIC)
            }),
            "wikilink in *** *** must render both bold and italic"
        );
    }

    #[test]
    fn bold_italic_plain_text() {
        let line = "***text***";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "text");
        assert!(
            s.iter().any(|sp| {
                sp.content.contains("text")
                    && sp.style.add_modifier.contains(Modifier::BOLD)
                    && sp.style.add_modifier.contains(Modifier::ITALIC)
            }),
            "*** *** must render both bold and italic"
        );
    }

    #[test]
    fn wikilink_mid_sentence() {
        let line = "See [[Topic]] for details";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "See Topic for details");
    }

    #[test]
    fn wikilink_cursor_col_accounts_for_brackets() {
        // "[[Hi]]" — cursor at pos 2 ('H') is inside the element, so it expands.
        // Rendered col counts pos 0 ('['), pos 1 ('[') as visible (expanded sigils) → col = 2.
        let col = MarkdownSpanner::rendered_cursor_col("[[Hi]]", 0, 2, true, false);
        assert_eq!(col, 2);

        // Cursor outside the wikilink (pos 0 on a plain-text line before it):
        // "See [[Hi]] x" with cursor at pos 0 — wikilink not expanded, brackets hidden.
        // pos 0 ('S') is plain text, rendered col = 0.
        let col2 = MarkdownSpanner::rendered_cursor_col("See [[Hi]] x", 0, 0, true, false);
        assert_eq!(col2, 0);
    }
}
