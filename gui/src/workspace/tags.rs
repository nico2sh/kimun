//! The **TAGS** drawer view (M7b G5): every label in the workspace with
//! how many notes carry it, most-used first, in the shared [`DrawerList`],
//! under a filter.
//!
//! **Rows** are the logic's (`TagSource`: core's label counts, loaded off
//! the UI thread). The filter narrows them here (the list's fuzzy filter
//! over the label); nothing is loaded again per keystroke.
//!
//! **Keys.** The filter has the keyboard: Up/Down move the selection, Enter
//! runs the selected tag's query in FIND (quoted, as the TUI's
//! `RunTagQuery`), Escape clears the filter, then gives the keyboard back to
//! the editor. A click on a row runs it too.
//!
//! **When it loads.** Only while shown: on each reveal (labels change as
//! notes are written), and after a file operation landed or the index
//! caught up. A newer load replaces an older one.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Enter, Escape, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    Styled as _, Subscription, Window, div,
};
use kimun_core::NoteVault;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::list::{Filter, ListSetup};
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::views::{TagEntry, TagSource};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::drawer_list::{DrawerList, DrawerListEvent, DrawerRow};
use super::files::{ellipsis, lane_row};
use super::query_field::query_field;
use crate::theme;

/// Element ids, for tests.
pub(crate) const TAGS_ID: &str = "tags";
pub(crate) const TAGS_FILTER_ID: &str = "tags-filter";
pub(crate) const TAGS_LIST_ID: &str = "tags-list";

/// A row's height, in rems: one line of small text.
const ROW_REMS: f32 = 1.75;

/// What TAGS says with no label in the workspace, or none left by the
/// filter.
pub(crate) const NO_TAGS_TEXT: &str = "No tags in this workspace";

/// The element id of the row for `label`.
pub(crate) fn tag_row_id(label: &str) -> SharedString {
    format!("tags-row:{label}").into()
}

/// How many notes, as a row says it.
fn notes(count: usize) -> String {
    match count {
        1 => "1 note".to_string(),
        n => format!("{n} notes"),
    }
}

impl DrawerRow for TagEntry {
    fn element_id(&self) -> SharedString {
        tag_row_id(&self.label)
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        let label = SharedString::from(self.label.clone());
        let name = SharedString::from(format!("{}, {}", self.label, notes(self.count)));
        let icon = Icon::new(IconName::Tag)
            .size_3p5()
            .text_color(theme::tag(t));
        lane_row(
            self.element_id(),
            name,
            selected,
            ROW_REMS,
            Some(icon.into_any_element()),
            [ellipsis(label)
                .text_color(theme::foreground(t))
                .into_any_element()],
            t,
        )
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_xs()
                .text_color(theme::muted(t))
                .child(self.count.to_string()),
        )
    }
}

/// What TAGS asks of the workspace (the list's own events go there
/// directly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TagsEvent {
    /// Enter in the filter: run this label's query in FIND.
    Run(String),
    /// Escape with an empty filter: back to the editor.
    Dismiss,
}

/// The TAGS view. See the module docs.
pub(crate) struct TagsView {
    pub(crate) list: Entity<DrawerList<TagEntry>>,
    filter: Entity<InputState>,
    vault: Arc<NoteVault>,
    /// Whether the view was shown at the last refresh: a reveal reloads.
    shown: bool,
    /// The vault changed since the labels were read.
    stale: bool,
    theme: Theme,
    _filter_events: Subscription,
}

impl EventEmitter<TagsEvent> for TagsView {}

impl TagsView {
    /// Draw in `t` from now on (the window's theme changed).
    pub(crate) fn set_theme(&mut self, t: &Theme, cx: &mut Context<Self>) {
        self.theme = t.clone();
        self.list.update(cx, |list, cx| list.set_theme(t, cx));
        cx.notify();
    }
}

impl TagsView {
    fn new(
        vault: Arc<NoteVault>,
        list: Entity<DrawerList<TagEntry>>,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter tags"));
        let filter_events = cx.subscribe(&filter, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.list.update(cx, |list, cx| list.set_query(query, cx));
            }
        });
        Self {
            list,
            filter,
            vault,
            shown: false,
            stale: false,
            theme,
            _filter_events: filter_events,
        }
    }

    fn query(&self, cx: &App) -> String {
        self.filter.read(cx).value().to_string()
    }

    /// Read the labels again if the view is `shown` and was just revealed
    /// (or the vault changed meanwhile). Hidden: nothing.
    pub(crate) fn refresh(&mut self, shown: bool, cx: &mut Context<Self>) {
        let revealed = shown && !self.shown;
        self.shown = shown;
        if shown && (revealed || self.stale) {
            self.load(cx);
        }
    }

    /// The vault changed under the labels: read again now if shown, else
    /// when next revealed.
    pub(crate) fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.stale = true;
        if self.shown {
            self.load(cx);
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.stale = false;
        let setup = ListSetup {
            filter: Filter::Fuzzy,
            query: self.query(cx),
            ..ListSetup::default()
        };
        let source = Arc::new(TagSource::new(self.vault.clone()));
        self.list.update(cx, |list, cx| {
            list.load_with(source, setup, NO_TAGS_TEXT, cx)
        });
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // `set_value` reports no change: the list is told too.
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.list.update(cx, |list, cx| list.set_query("", cx));
    }
}

impl Focusable for TagsView {
    /// The filter: typing in TAGS narrows it.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter.read(cx).focus_handle(cx)
    }
}

impl Render for TagsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(TAGS_ID)
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .text_color(theme::foreground(t))
            // The filter's keys that mean something else here.
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.list.update(cx, |list, cx| list.step(false, cx));
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.list.update(cx, |list, cx| list.step(true, cx));
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, _, cx| {
                if let Some(row) = this.list.read(cx).selected_row() {
                    cx.emit(TagsEvent::Run(row.label.clone()));
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.query(cx).is_empty() {
                    cx.emit(TagsEvent::Dismiss);
                } else {
                    this.clear_filter(window, cx);
                }
                cx.stop_propagation();
            }))
            .child(query_field(&self.filter, TAGS_FILTER_ID, "Filter tags", t))
            .child(self.list.clone())
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Make the TAGS view over the vault the window opened (nothing is
    /// read until it is shown).
    pub(super) fn start_tags(
        &mut self,
        vault: Arc<NoteVault>,
        tx: AppTx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = self.theme.clone();
        let list = DrawerList::spawn(TAGS_LIST_ID, "Tags", tx, &theme, cx);
        let view = cx.new(|cx| TagsView::new(vault, list.clone(), theme, window, cx));
        self._views_events
            .push(cx.subscribe_in(&list, window, Self::on_tags_list_event));
        self._views_events
            .push(cx.subscribe_in(&view, window, Self::on_tags_event));
        self.tags = Some(view);
    }

    fn on_tags_list_event(
        &mut self,
        _: &Entity<DrawerList<TagEntry>>,
        event: &DrawerListEvent<TagEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DrawerListEvent::Activate(row) => self.run_tag_query(&row.label, window, cx),
            DrawerListEvent::Verb(..) => {}
            DrawerListEvent::Dismiss => self.focus_panel(Panel::Editor, window, cx),
            // Tags are not vault entries: no menu.
            DrawerListEvent::FileOp(..) => {}
        }
    }

    fn on_tags_event(
        &mut self,
        _: &Entity<TagsView>,
        event: &TagsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TagsEvent::Run(label) => self.run_tag_query(label, window, cx),
            TagsEvent::Dismiss => self.focus_panel(Panel::Editor, window, cx),
        }
    }

    /// Whether TAGS' list (not its filter) has the keyboard.
    pub(super) fn tags_list_focused(&self, window: &Window, cx: &App) -> bool {
        self.tags.as_ref().is_some_and(|tags| {
            let list = tags.read(cx).list.clone();
            list.read(cx).focus_handle(cx).is_focused(window)
        })
    }

    /// Bring TAGS up to the layout (hidden: nothing).
    pub(super) fn refresh_tags(&mut self, cx: &mut Context<Self>) {
        if let Some(tags) = self.tags.clone() {
            let shown = self.is_shown(DrawerView::Tags);
            tags.update(cx, |view, cx| view.refresh(shown, cx));
        }
    }
}

crate::theme::impl_themed!(TagsView);
