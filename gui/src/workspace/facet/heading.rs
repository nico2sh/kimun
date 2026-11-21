//! The view title row and the collection line of the **Facet view**.
//!
//! Tabs do not always show, so the view itself says what it shows. The
//! title row sits at the top of the view, outside the scrolling column, so
//! the height table does not count it. The collection line sits under the
//! bar and holds a collection's note numbers.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Div, FontWeight, InteractiveElement as _, ParentElement as _, Pixels, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_notes::settings::themes::Theme;

use super::collection::dateline_title_size;
use super::rows;
use super::{COLLECTION_LINE_ID, FacetView, VIEW_TITLE_ID, VIEW_TITLE_KIND_ID, VIEW_TITLE_TEXT_ID};
use crate::theme;

/// How much bigger the view title is than a dateline title: one type step.
const TITLE_STEP: f32 = 1.25;

/// The heading level of the view title: the first level of the outline.
pub(crate) const VIEW_TITLE_LEVEL: usize = 1;

/// The heading level of a dateline: one level under the view title.
pub(crate) const DATELINE_LEVEL: usize = 2;

/// The word before the query in a collection's title row.
const COLLECTION_WORD: &str = "Collection";

/// The size of the view title: one type step above a dateline title, so it
/// stays above the note titles at any editor size.
pub(crate) fn view_title_size(cx: &App) -> Pixels {
    dateline_title_size(cx) * TITLE_STEP
}

impl FacetView {
    /// The view title row: what the view shows, one line at `size`,
    /// semibold, cut with an ellipsis when too long. A collection shows a
    /// small muted `Collection`, then its query, with the full query in a
    /// tooltip. A single-note view shows its note's title, which follows new
    /// text and a rename ([`super::members`]). A screen reader hears it as
    /// the first heading level, named with the full text.
    pub(super) fn render_view_title(&self, size: Pixels, t: &Theme) -> Option<Div> {
        let query = self.query().map(SharedString::from);
        let text = match &query {
            Some(query) => query.clone(),
            None => SharedString::from(self.single()?.title.clone()),
        };
        let name = match &query {
            Some(query) => SharedString::from(format!("{COLLECTION_WORD}: {query}")),
            None => text.clone(),
        };
        let kind = query.is_some().then(|| {
            div()
                .id(VIEW_TITLE_KIND_ID)
                .flex_none()
                .text_size(rows::small_text())
                .text_color(theme::muted(t))
                .child(COLLECTION_WORD)
                .test_support()
        });
        let shown = div()
            .id(VIEW_TITLE_TEXT_ID)
            .min_w_0()
            .truncate()
            .text_size(size)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::foreground(t))
            .when_some(query, |el, query| {
                el.tooltip(move |window, cx| Tooltip::new(query.clone()).build(window, cx))
            })
            .child(text)
            .test_support();
        Some(
            div().flex_none().child(
                div()
                    .id(VIEW_TITLE_ID)
                    .role(Role::Heading)
                    .aria_level(VIEW_TITLE_LEVEL)
                    .aria_label(name)
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .px_4()
                    .when_some(self.corner_room, |row, room| row.pr(room))
                    .pt_3()
                    .children(kind)
                    .child(shown)
                    .test_support(),
            ),
        )
    }

    /// The collection line, muted, under the bar: always in a collection,
    /// never in a single-note view. At rest it is a label. While it reports
    /// a load or a refresh it is a status, so a screen reader says it.
    pub(super) fn render_collection_line(&self, t: &Theme) -> Option<Div> {
        let text = SharedString::from(self.collection_line()?);
        let role = match self.collection_line_reports() {
            true => Role::Status,
            false => Role::Label,
        };
        Some(
            div().flex_none().px_4().pt_2().child(
                div()
                    .id(COLLECTION_LINE_ID)
                    .role(role)
                    .aria_label(text.clone())
                    .text_size(rows::small_text())
                    .text_color(theme::muted(t))
                    .child(text)
                    .test_support(),
            ),
        )
    }
}
