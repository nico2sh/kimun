//! The row FIND and SEM draw a search result with: one shape, so the two
//! lists read alike and have one height ([`ROW_REMS`]).
//!
//! Line 1 is the note's title, with the similarity right-aligned when the
//! result has one (SEM); line 2 the filename; line 3 one line of context
//! (FIND's paragraph, SEM's matched chunk), its whitespace run together and
//! cut to [`CONTEXT_CHARS`].

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_notes::settings::themes::Theme;

use super::files::{ellipsis, lane_row};
use super::find::CONTEXT_CHARS;
use crate::theme;

/// A result row's height, in rems: title, filename and one line of context.
pub(crate) const ROW_REMS: f32 = 3.75;

/// The element id of the score on the row `row_id` (tests).
pub(crate) fn score_id(row_id: &SharedString) -> SharedString {
    format!("{row_id}:score").into()
}

/// The element id of the context line on the row `row_id` (tests).
pub(crate) fn context_id(row_id: &SharedString) -> SharedString {
    format!("{row_id}:context").into()
}

/// `context` as one line: its first [`CONTEXT_CHARS`] characters, with
/// every run of whitespace (line breaks too) one space.
fn one_line(context: &str) -> String {
    let cut = context
        .char_indices()
        .nth(CONTEXT_CHARS)
        .map_or(context, |(at, _)| &context[..at]);
    cut.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A search result's row (see the module docs): `title` (already the
/// filename when the note has none), `filename`, `context` and, when
/// present, `score` (`87%`). `marks` are the words to highlight in the
/// context; none are drawn yet.
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
    let _ = marks;
    let title = SharedString::from(title.to_string());
    let context = SharedString::from(one_line(context));
    let icon = Icon::new(IconName::FileText)
        .size_4()
        .text_color(theme::foreground(t));
    let first = div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .child(
            ellipsis(title.clone())
                .flex_1()
                .text_color(theme::foreground(t)),
        )
        .when_some(score, |line, score| {
            let score = SharedString::from(score);
            line.child(
                div()
                    .id(score_id(&id))
                    .test_support()
                    .aria_label(score.clone())
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme::muted(t))
                    .child(score),
            )
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
            ellipsis(context.clone())
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
