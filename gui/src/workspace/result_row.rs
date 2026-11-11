//! The row FIND and SEM draw a search result with: one shape, so the two
//! lists read alike and have one height ([`ROW_REMS`]).
//!
//! Line 1 is the note's title, the similarity just after it when the
//! result has one (SEM: `Coffee notes · 87%`; the title gives way first,
//! and the line stays clear of the chevron's zone); line 2 the filename;
//! line 3 one line of context (FIND's paragraph, SEM's matched chunk), its
//! whitespace run together and cut to [`CONTEXT_CHARS`].

use std::ops::Range;

use gpui_kit::HighlightStyle;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledText, div,
};
use kimun_notes::logic::text_match::match_ranges;
use kimun_notes::settings::themes::Theme;

use super::chevron::clear_of_zone;
use super::files::{ellipsis, ellipsis_box, lane_row};
use super::find::CONTEXT_CHARS;
use crate::theme;

/// A result row's height, in rems: title, filename and one line of context.
pub(crate) const ROW_REMS: f32 = 3.75;

/// The element id of the score on the row `row_id` (tests).
pub(crate) fn score_id(row_id: &SharedString) -> SharedString {
    format!("{row_id}:score").into()
}

/// The element id of the title on the row `row_id` (tests).
pub(crate) fn title_id(row_id: &SharedString) -> SharedString {
    format!("{row_id}:title").into()
}

/// The element id of the context line on the row `row_id` (tests).
pub(crate) fn context_id(row_id: &SharedString) -> SharedString {
    format!("{row_id}:context").into()
}

/// `context` as one line: every run of whitespace (line breaks too) one
/// space, then its first [`CONTEXT_CHARS`] characters (characters, not
/// bytes, so the cut is on a character boundary).
pub(crate) fn one_line(context: &str) -> String {
    let joined = context.split_whitespace().collect::<Vec<_>>().join(" ");
    match joined.char_indices().nth(CONTEXT_CHARS) {
        Some((at, _)) => joined[..at].to_string(),
        None => joined,
    }
}

/// The byte ranges of `line` (the displayed one-line context, not the
/// original paragraph) the `marks` match; each lies on char boundaries.
pub(crate) fn context_highlights(line: &str, marks: &[String]) -> Vec<Range<usize>> {
    if marks.is_empty() {
        return Vec::new();
    }
    match_ranges(line, marks)
        .into_iter()
        .filter(|&(from, to)| from < to && line.is_char_boundary(from) && line.is_char_boundary(to))
        .map(|(from, to)| from..to)
        .collect()
}

/// The score as it follows a title (SEM, the Sources view): `· 87%`, dim
/// and never cut short (`flex_none`), `id` its element id and the score
/// its accessible name.
pub(crate) fn score_beside_title(
    id: SharedString,
    score: SharedString,
    t: &Theme,
) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(score.clone())
        .flex()
        .flex_none()
        .gap_1()
        .whitespace_nowrap()
        .text_xs()
        .text_color(theme::muted(t))
        .child("·")
        .child(score)
}

/// A search result's row (see the module docs): `title` (already the
/// filename when the note has none), `filename`, `context` and, when
/// present, `score` (`87%`). `marks` are the words to highlight, in the
/// accent colour, in the context line.
#[allow(clippy::too_many_arguments)]
pub(crate) fn result_row(
    id: SharedString,
    selected: bool,
    title: &str,
    filename: &str,
    context: &str,
    score: Option<String>,
    marks: &[String],
    t: &Theme,
) -> Stateful<Div> {
    let title = SharedString::from(title.to_string());
    let context = SharedString::from(one_line(context));
    let highlights = context_highlights(&context, marks);
    let icon = Icon::new(IconName::FileText)
        .size_4()
        .text_color(theme::foreground(t));
    // The title gives way (an ellipsis) before the score does; a line with
    // a score stops short of the chevron's zone, so the score always shows
    // (FIND's title-only line keeps its whole width).
    let first = div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .when(score.is_some(), clear_of_zone)
        .child(
            ellipsis(title.clone())
                .id(title_id(&id))
                .test_support()
                .text_color(theme::foreground(t)),
        )
        .when_some(score, |line, score| {
            line.child(score_beside_title(score_id(&id), score.into(), t))
        });
    lane_row(
        id.clone(),
        title,
        selected,
        ROW_REMS,
        Some(icon.into_any_element()),
        [
            first.into_any_element(),
            ellipsis(filename.to_string())
                .text_xs()
                .text_color(theme::secondary(t))
                .into_any_element(),
            if highlights.is_empty() {
                ellipsis(context.clone())
            } else {
                let mark = HighlightStyle {
                    color: Some(theme::accent(t).into()),
                    ..Default::default()
                };
                ellipsis_box().child(
                    StyledText::new(context.clone())
                        .with_highlights(highlights.into_iter().map(|range| (range, mark))),
                )
            }
            .id(context_id(&id))
            .test_support()
            .aria_label(context)
            .text_xs()
            .text_color(theme::muted(t))
            .into_any_element(),
        ],
        t,
    )
}
