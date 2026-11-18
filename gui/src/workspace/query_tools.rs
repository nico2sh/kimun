//! The **query tools**: the buttons beside a query field, the same in
//! every view that has one (FIND, the search browser).
//!
//! Two groups. Beside the field: `Build…` (compose the query), then `Save…`
//! (keep it); `Save…` is the one outlined button, because it is the one that
//! keeps work. On the row under the field, with the result count: the sort
//! chip, the view's extras, then the quiet `?` (the syntax). Only two
//! buttons take room from the field. A view supplies the ids (tests find buttons by them),
//! the click handlers and the chip; the shape lives here, so the views
//! cannot drift apart again.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ParentElement as _, SharedString, Styled as _, Window, div,
};
use kimun_notes::logic::actions::ActionShortcuts;

use super::chrome::tooltip_with_chord;
use super::query_builder::BUILD_TIP;

/// The element ids of one view's tools.
pub(super) struct QueryToolIds {
    pub build: &'static str,
    pub save: &'static str,
    pub help: &'static str,
}

/// A click handler, as the kit's buttons take it (`cx.listener` makes one).
pub(super) type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// What a view hands over to get its tools.
pub(super) struct QueryTools {
    pub ids: QueryToolIds,
    /// The sort chip, already clicked and tagged (absent while the view has
    /// no query to sort).
    pub chip: Option<AnyElement>,
    /// `Save…` is offered only when the query can be saved.
    pub can_save: bool,
    /// The view is working (the browser while a `Create:` runs): `Build…`
    /// and `Save…` wait, as their chords do.
    pub busy: bool,
    pub save_tip: SharedString,
    /// Buttons only this view has, between the sort chip and `?`.
    pub extras: Vec<AnyElement>,
    pub on_build: Click,
    pub on_save: Click,
    pub on_help: Click,
}

/// `Save…`'s tooltip: what it does, and the chord that does it.
pub(super) fn save_tip(cx: &App) -> SharedString {
    tooltip_with_chord(
        "Save the query as a saved search",
        &ActionShortcuts::SaveCurrentQuery,
        cx,
    )
}

impl QueryTools {
    /// The two groups of tools: the **actions**, to sit at the right of the
    /// query field (`Build…`, `Save…`: they work on the query), and the
    /// **meta**, to sit on the row under it with the result count (the sort
    /// chip, the view's extras, `?`: they describe or explain the results).
    /// Two buttons beside the field leave it the room to type in.
    pub(super) fn render(self, cx: &App) -> (Div, Div) {
        let build = Button::new(self.ids.build)
            .ghost()
            .small()
            .label("Build…")
            .tooltip(tooltip_with_chord(
                BUILD_TIP,
                &ActionShortcuts::QueryBuilder,
                cx,
            ))
            .disabled(self.busy)
            .on_click(self.on_build);
        let save = Button::new(self.ids.save)
            .outline()
            .small()
            .label("Save…")
            .tooltip(self.save_tip)
            .disabled(!self.can_save || self.busy)
            .on_click(self.on_save);
        let help = Button::new(self.ids.help)
            .ghost()
            .xsmall()
            .icon(IconName::CircleQuestionMark)
            .accessibility_label("Query syntax")
            .tooltip(tooltip_with_chord(
                "Query syntax",
                &ActionShortcuts::QuerySyntax,
                cx,
            ))
            .on_click(self.on_help);
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(build)
            .child(save);
        let meta = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .children(self.chip)
            .children(self.extras)
            .child(help);
        (actions, meta)
    }
}
