//! Query **syntax highlighting** (spec §9): maps core's query token spans
//! onto theme roles. Reused by every query input — the FIND drawer now, the
//! telescope modal in phase 08 — so the coloring rules live exactly once.
//!
//! Core's lexer ([`kimun_core::query_token_spans`]) owns tokenization; this
//! module only assigns styles, plus two presentation-layer overlays the core
//! grammar doesn't know: `{variable}` placeholders and the leading `?`
//! saved-search sigil.

use kimun_core::QueryTokenClass;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::settings::themes::Theme;

/// Style for a token class, per the spec §9 role table mapped onto the real
/// grammar: field keys yellow, tag values aqua, note targets blue, quoted
/// green, date/number purple, negation red, plain terms fg.
fn class_style(class: QueryTokenClass, theme: &Theme) -> Style {
    match class {
        QueryTokenClass::Negation => Style::default().fg(theme.red.to_ratatui()),
        QueryTokenClass::FieldKey => Style::default().fg(theme.yellow.to_ratatui()),
        QueryTokenClass::LinkValue => Style::default().fg(theme.blue.to_ratatui()),
        QueryTokenClass::TagValue => Style::default().fg(theme.aqua.to_ratatui()),
        QueryTokenClass::Quoted => Style::default().fg(theme.green.to_ratatui()),
        QueryTokenClass::Date | QueryTokenClass::Number => {
            Style::default().fg(theme.purple.to_ratatui())
        }
        QueryTokenClass::Term => Style::default().fg(theme.fg.to_ratatui()),
        QueryTokenClass::Unterminated => Style::default()
            .fg(theme.red.to_ratatui())
            .add_modifier(Modifier::UNDERLINED),
    }
}

/// Build the styled line for a query string. `base` carries the background
/// (and the style for any uncovered whitespace).
pub fn highlight_line(query: &str, theme: &Theme, base: Style) -> Line<'static> {
    // Presentation-layer overlay: a leading `?` expands a saved search —
    // style the sigil and hand the rest to the lexer-driven path with offset.
    if let Some(rest) = query.strip_prefix('?') {
        let mut spans = vec![Span::styled(
            "?".to_string(),
            base.patch(Style::default().fg(theme.gray.to_ratatui())),
        )];
        spans.push(Span::styled(
            rest.to_string(),
            base.patch(Style::default().fg(theme.aqua.to_ratatui())),
        ));
        return Line::from(spans);
    }

    let token_spans = kimun_core::query_token_spans(query);
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(token_spans.len() * 2 + 1);
    let mut pos = 0usize;
    for ts in token_spans {
        if ts.range.start > pos {
            spans.push(Span::styled(query[pos..ts.range.start].to_string(), base));
        }
        let text = &query[ts.range.clone()];
        let style = base.patch(class_style(ts.class, theme));
        // `{variable}` placeholders (e.g. `{note}`) are presentation-layer:
        // restyle them inside whatever value span they sit in.
        push_with_variables(&mut spans, text, style, base, theme);
        pos = ts.range.end;
    }
    if pos < query.len() {
        spans.push(Span::styled(query[pos..].to_string(), base));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    Line::from(spans)
}

/// Split `{var}` placeholders out of `text`, styling them as variables and
/// the rest with `style`.
fn push_with_variables(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    style: Style,
    base: Style,
    theme: &Theme,
) {
    let var_style = base.patch(
        Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::ITALIC),
    );
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        if let Some(close_rel) = rest[open..].find('}') {
            let close = open + close_rel + 1;
            if open > 0 {
                spans.push(Span::styled(rest[..open].to_string(), style));
            }
            spans.push(Span::styled(rest[open..close].to_string(), var_style));
            rest = &rest[close..];
        } else {
            break;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), style));
    }
}

/// The lowercase emphasis needles a query implies: its plain text terms,
/// labels in their in-body `#tag` form, and link targets — what a result's
/// content actually matched on. Shared by the telescope preview and the
/// editor's arrive-from-query emphasis.
pub fn emphasis_needles(query: &str) -> Vec<String> {
    let terms = kimun_core::SearchTerms::from_query_string(query);
    let mut needles: Vec<String> = terms
        .terms
        .iter()
        .map(|t| t.to_lowercase())
        .chain(
            terms
                .labels
                .iter()
                .map(|l| format!("#{}", l.to_lowercase())),
        )
        .chain(terms.links.iter().map(|l| l.to_lowercase()))
        .chain(terms.forward_links.iter().map(|l| l.to_lowercase()))
        .filter(|t| !t.is_empty() && !t.contains('{'))
        .collect();
    needles.dedup();
    needles
}

/// The one-line reason for the query's parse problem, if any — surfaced in
/// the FIND header. The lenient grammar's only real error is an unterminated
/// quote.
pub fn error_reason(query: &str) -> Option<&'static str> {
    kimun_core::query_has_unterminated_quote(query).then_some("unterminated quote")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(line: &Line) -> Vec<String> {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn highlights_round_trip_the_text() {
        let theme = Theme::default();
        let q = r#"-#wip <{note} "two words" 2026-04-01 plain"#;
        let line = highlight_line(q, &theme, Style::default());
        assert_eq!(texts(&line).concat(), q);
    }

    #[test]
    fn variable_is_styled_inside_value() {
        let theme = Theme::default();
        let line = highlight_line("<{note}", &theme, Style::default());
        let texts = texts(&line);
        assert!(texts.contains(&"{note}".to_string()), "got {texts:?}");
    }

    #[test]
    fn saved_search_sigil_styles_whole_input() {
        let theme = Theme::default();
        let line = highlight_line("?todo", &theme, Style::default());
        assert_eq!(texts(&line), vec!["?".to_string(), "todo".to_string()]);
    }

    #[test]
    fn error_reason_only_for_unterminated() {
        assert_eq!(error_reason(r#"a "open"#), Some("unterminated quote"));
        assert_eq!(error_reason(r#"a "closed""#), None);
    }
}
