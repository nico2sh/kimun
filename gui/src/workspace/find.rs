//! The **FIND** drawer view (M7b G5), the GUI's Query panel: a query over
//! the vault, its results in the shared [`DrawerList`], and a preview of the
//! selected result that grows Collapsed → Context → Full. One instance
//! (ADR-0050).
//!
//! **Query.** A plain field (no autocomplete, no colouring: decision 4). It
//! starts on the open note's backlinks (`<{note}`). Typing runs the query
//! once it has held still for [`FIND_DEBOUNCE`]; Enter runs it at once and
//! gives the results the keyboard. Each run replaces the one under way (its
//! rows never land). `{note}` and the bare `<` / `>` resolve against the
//! open note at load time, and a query that uses them runs again when the
//! open note changes. A query core cannot read says why under the field.
//!
//! **Rows** are the logic's bounded query page (`QueryPageSource`, off the
//! UI thread): the first [`ROW_CAP`] results in the TUI's order, each with
//! a context paragraph cut to [`CONTEXT_CHARS`]; no row keeps its note's
//! text. More results than that say so under the list.
//!
//! **Preview** ([`SearchPreview`], the search browser's): only while
//! expanded, the selected note is read after a short wait, its matches
//! marked and centred, soft-wrapped to the pane. Context shows it under the
//! list; Full gives it the whole view (the list keeps the keyboard).
//!
//! **Keys.** In the field: Down or Enter go to the results, Escape back to
//! the editor, F1 (the `QuerySyntax` chord) shows the query syntax. In the results: Up/Down/Home/End
//! move (Up from the first row returns to the field), Enter opens the note
//! with the query's matches emphasised, Right grows the preview, Left
//! shrinks it, Space cycles it; in Full, Up/Down scroll the preview.
//! Desktop keys only, as LINKS and OUTLINE (decision 13).
//!
//! **Pointer.** A click opens a row, as Enter does (the pointing hand over
//! it; [`ClickPolicy::Open`](super::files::ClickPolicy)); the preview button
//! cycles the preview; `Save…` saves the query (as the `SaveCurrentQuery`
//! chord does here); the `Syntax` button shows the query syntax; the sort
//! chip opens the sort dialog, which rewrites the query's order directive
//! (M7b G7, [`super::sort`]).
//!
//! **Saved searches (M7b G6).** A saved search run from the Saved Searches
//! names FIND: its title is the search's name, `name • edited` once the
//! query is edited away from the saved text, back to the name when saved
//! again (the TUI's breadcrumb, `logic::query::SavedSearchBreadcrumb`).
//!
//! **When it works.** Only while shown: hidden, it neither runs queries nor
//! reads notes; revealed, it keeps the query as it was and catches up.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Enter, Escape, InputEvent, InputState, MoveDown};
use gpui_kit::component::{Disableable as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    Role, SharedString, Stateful, StatefulInteractiveElement as _, StyleRefinement, Styled as _,
    Subscription, Task, Window, div,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::list::{ListSetup, ResolvingRowSource, RowSource, Unresolvable};
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::message::SortTarget;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::preview::PreviewModel;
use kimun_notes::logic::query::{
    QueryContext, QueryHit, QueryPageSource, SavedSearchBreadcrumb, emphasis_needles, error_reason,
    is_default_query, query_has_variables, resolve_query,
};
use kimun_notes::logic::sort::{SortField, SortOrder, order_of_query, query_with_sort};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::drawer_list::{DrawerList, DrawerListEvent, DrawerRow};
use super::files::{display_title, ellipsis, lane_row};
use super::picker::query_problem;
use super::query_field::{self, query_field};
use super::search::SearchPreview;
use super::search::label_query;
use super::sort::{chip_label, sort_chip};
use crate::input;
use crate::keymap;
use crate::theme;

/// `Save…`'s tooltip: what it does, and the chord that does it here.
fn save_tip(cx: &App) -> SharedString {
    match keymap::chord_label(cx, &ActionShortcuts::SaveCurrentQuery) {
        Some(chord) => format!("Save the query as a saved search ({chord})").into(),
        None => "Save the query as a saved search".into(),
    }
}

/// Element ids, for tests.
pub(crate) const FIND_ID: &str = "find";
pub(crate) const FIND_INPUT_ID: &str = "find-query";
pub(crate) const FIND_LIST_ID: &str = "find-list";
pub(crate) const FIND_TITLE_ID: &str = "find-title";
pub(crate) const FIND_SORT_ID: &str = "find-sort";
pub(crate) const FIND_EXPAND_ID: &str = "find-expand";
pub(crate) const FIND_HELP_ID: &str = "find-help";
pub(crate) const FIND_SAVE_ID: &str = "find-save";
pub(crate) const FIND_ERROR_ID: &str = "find-query-error";
pub(crate) const FIND_MORE_ID: &str = "find-more";
pub(crate) const FIND_PREVIEW_ID: &str = "find-preview";
pub(crate) const FIND_PREVIEW_ROW_ID: &str = "find-preview-row";

/// How long typing must pause before the query runs.
pub(crate) const FIND_DEBOUNCE: Duration = Duration::from_millis(150);
/// The most results a query lists; the rest are counted, not read.
pub(crate) const ROW_CAP: usize = 500;
/// The longest context paragraph a row keeps, in characters.
pub(crate) const CONTEXT_CHARS: usize = 300;
/// A row's height, in rems: title, filename and one line of context.
const ROW_REMS: f32 = 3.75;

/// What the list says for a blank query, and for a query without results.
pub(crate) const BLANK_TEXT: &str = "Type a query to search the workspace (F1: syntax)";
pub(crate) const NO_RESULTS_TEXT: &str = "No notes match";

/// The element id of the row for `path`.
pub(crate) fn find_row_id(path: &VaultPath) -> SharedString {
    format!("find-row:{path}").into()
}

impl DrawerRow for QueryHit {
    fn element_id(&self) -> SharedString {
        find_row_id(&self.entry.path)
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        let entry = &self.entry;
        let title = SharedString::from(display_title(&entry.title, &entry.filename).to_string());
        // One line of context: the paragraph's lines run together.
        let context: SharedString = entry
            .context
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .into();
        let icon = Icon::new(IconName::FileText)
            .size_4()
            .text_color(theme::foreground(t));
        lane_row(
            self.element_id(),
            title.clone(),
            selected,
            ROW_REMS,
            Some(icon.into_any_element()),
            [
                ellipsis(title)
                    .text_color(theme::foreground(t))
                    .into_any_element(),
                ellipsis(entry.filename.clone())
                    .text_xs()
                    .text_color(theme::secondary(t))
                    .into_any_element(),
                ellipsis(context)
                    .text_xs()
                    .text_color(theme::muted(t))
                    .into_any_element(),
            ],
            t,
        )
    }

    fn menu_target(&self) -> Option<VaultPath> {
        Some(self.entry.path.clone())
    }
}

/// What FIND asks of the workspace (the list's own events go there
/// directly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindEvent {
    /// The `QuerySyntax` chord (F1) or the `Syntax` button: the query syntax.
    Help,
    /// `Save…`: save the query.
    Save,
    /// The sort chip: the sort dialog on FIND.
    Sort,
    /// Escape in the field: back to the editor.
    Dismiss,
}

/// The FIND view. See the module docs.
pub(crate) struct FindView {
    pub(crate) list: Entity<DrawerList<QueryHit>>,
    pub(crate) preview: Entity<SearchPreview>,
    pub(crate) input: Entity<InputState>,
    vault: Arc<NoteVault>,
    /// The note `{note}` resolves to, read by each load.
    note: Arc<Mutex<VaultPath>>,
    /// How much of the selected note shows, and for which row.
    model: PreviewModel,
    breadcrumb: SavedSearchBreadcrumb,
    /// The wait before a typed query runs; replaced by each keystroke.
    debounce: Option<Task<()>>,
    /// The field holds a query the list has not run.
    needs_run: bool,
    /// The list holds the query source (not the blank state).
    started: bool,
    /// The vault changed under the rows: they reload when next shown.
    stale: bool,
    /// Whether the view was shown at the last refresh.
    shown: bool,
    /// The needles of (query, note), memoised.
    needles: (String, VaultPath, Arc<[String]>),
    /// The `SaveCurrentQuery` chord as `Save…`'s tooltip names it, rebuilt
    /// when the keymap is replaced.
    save_tip: SharedString,
    _subscriptions: Vec<Subscription>,
    /// Queries run (tests: the debounce runs one per pause).
    #[cfg(test)]
    pub(crate) runs: usize,
}

impl EventEmitter<FindEvent> for FindView {}

impl FindView {}

impl FindView {
    fn new(
        vault: Arc<NoteVault>,
        list: Entity<DrawerList<QueryHit>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            // Starts empty like the TUI's: backlinks are LINKS' (leader `f b`).
            InputState::new(window, cx).placeholder("Search notes")
        });
        let preview_vault = vault.clone();
        let preview = cx.new(|cx| {
            SearchPreview::new(preview_vault, cx).with_ids(FIND_PREVIEW_ID, FIND_PREVIEW_ROW_ID)
        });
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.typed(cx);
                }
            }),
            // A selection move, or rows landing: the preview follows.
            cx.observe(&list, |this, _, cx| this.sync_preview(cx)),
            keymap::observe(cx, |this: &mut Self, cx| {
                this.save_tip = save_tip(cx);
                cx.notify();
            }),
        ];
        Self {
            list,
            preview,
            input,
            vault,
            note: Arc::new(Mutex::new(VaultPath::empty())),
            model: PreviewModel::new(),
            breadcrumb: SavedSearchBreadcrumb::default(),
            debounce: None,
            needs_run: true,
            started: false,
            stale: false,
            shown: false,
            needles: (String::new(), VaultPath::empty(), Arc::from(Vec::new())),
            save_tip: save_tip(cx),
            _subscriptions: subscriptions,
            #[cfg(test)]
            runs: 0,
        }
    }

    /// The query in the field.
    pub(crate) fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// The preview's state (tests).
    #[cfg(test)]
    pub(crate) fn preview_model(&self) -> &PreviewModel {
        &self.model
    }

    /// Whether a typed query waits for its pause (tests).
    #[cfg(test)]
    pub(crate) fn is_waiting(&self) -> bool {
        self.debounce.is_some()
    }

    fn note(&self) -> VaultPath {
        self.note.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    // ---- Running the query -----------------------------------------------------

    /// The field changed (typing): run it once typing pauses.
    fn typed(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let blank = query.trim().is_empty() || is_default_query(&query);
        self.breadcrumb.on_query_consumed(None, &query, blank);
        // The matches move: the preview anchors on them again.
        self.model.re_anchor();
        self.needs_run = true;
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIND_DEBOUNCE).await;
            this.update(cx, |view, cx| view.run(cx)).ok();
        }));
        cx.notify();
    }

    /// Run the field's query now (hidden: when next shown). The run under
    /// way is replaced; the same query again reloads.
    fn run(&mut self, cx: &mut Context<Self>) {
        self.debounce = None;
        if !self.shown {
            self.needs_run = true;
            return;
        }
        self.needs_run = false;
        self.stale = false;
        let query = self.query(cx);
        #[cfg(test)]
        {
            self.runs += 1;
        }
        if query.trim().is_empty() {
            self.started = false;
            self.list
                .update(cx, |list, cx| list.set_rows(Vec::new(), BLANK_TEXT, cx));
        } else if self.started {
            self.list.update(cx, |list, cx| {
                if list.query() == query {
                    list.reload(cx);
                } else {
                    list.set_query(query, cx);
                }
            });
        } else {
            let note = self.note.clone();
            let source: Arc<dyn RowSource<QueryHit>> = Arc::new(ResolvingRowSource::new(
                Arc::new(QueryPageSource::new(
                    self.vault.clone(),
                    ROW_CAP,
                    CONTEXT_CHARS,
                )),
                move || {
                    QueryContext::with_note(Some(
                        note.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                    ))
                },
                Unresolvable::Empty,
            ));
            let setup = ListSetup {
                query,
                ..ListSetup::default()
            };
            self.list.update(cx, |list, cx| {
                list.load_with(source, setup, NO_RESULTS_TEXT, cx)
            });
            self.started = true;
        }
        self.sync_preview(cx);
        cx.notify();
    }

    /// Put `query` in the field and run it (a tag, a leader route, a saved
    /// search `name`, which then names FIND): the preview starts collapsed.
    /// Hidden, it runs when shown.
    pub(crate) fn apply_query(
        &mut self,
        query: String,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input.update(cx, |input, cx| {
            input.set_value(query.clone(), window, cx);
        });
        self.breadcrumb.set(name, &query);
        self.model.reset();
        self.run(cx);
    }

    /// A sort dialog pick: the query's order directive becomes `field` /
    /// `order` and the query runs again. A sort is an edit: a saved
    /// search's name reads "• edited" after it. A property sort with no key
    /// changes nothing.
    pub(crate) fn apply_sort(
        &mut self,
        field: &SortField,
        order: SortOrder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.query(cx);
        let Some(sorted) = query_with_sort(&query, field, order) else {
            return;
        };
        if sorted == query {
            return;
        }
        // `set_value` reports no change: the edit's rules run here.
        self.input.update(cx, |input, cx| {
            input.set_value(sorted.clone(), window, cx);
        });
        let blank = sorted.trim().is_empty() || is_default_query(&sorted);
        self.breadcrumb.on_query_consumed(None, &sorted, blank);
        self.model.reset();
        self.run(cx);
    }

    /// The saved search the query came from (FIND's name for it), if any:
    /// what the save dialog offers as the name.
    pub(crate) fn saved_name(&self) -> Option<String> {
        self.breadcrumb.name().map(str::to_string)
    }

    /// The query was saved as `name` (a FIND-sourced save whose name is not
    /// the query itself): FIND is named for it, no longer edited.
    pub(crate) fn repin(&mut self, name: String, query: &str, cx: &mut Context<Self>) {
        self.breadcrumb.set(Some(name), query);
        cx.notify();
    }

    /// FIND's title as shown (tests).
    #[cfg(test)]
    pub(crate) fn shown_title(&self, cx: &App) -> String {
        self.title(&self.query(cx)).to_string()
    }

    /// Enter in the field: run a query still waiting, and give the results
    /// the keyboard.
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.needs_run {
            self.run(cx);
        }
        self.focus_list(window, cx);
    }

    fn focus_list(&self, window: &mut Window, cx: &mut Context<Self>) {
        query_field::focus_list(&self.list, window, cx);
    }

    fn list_focused(&self, window: &Window, cx: &App) -> bool {
        query_field::list_focused(&self.list, window, cx)
    }

    /// Bring the rows up to the layout and `note` (the open note, empty for
    /// none): hidden, nothing. Shown: the first reveal runs the query; a
    /// query waiting runs; a query over the open note runs again when that
    /// note changed, or on a reveal; rows the vault changed under reload.
    pub(crate) fn refresh(&mut self, shown: bool, note: &VaultPath, cx: &mut Context<Self>) {
        let revealed = shown && !self.shown;
        self.shown = shown;
        if !shown {
            return;
        }
        let other_note = self.note() != *note;
        if other_note {
            *self.note.lock().unwrap_or_else(|e| e.into_inner()) = note.clone();
        }
        // Cheap unless something changed: the query is read only then.
        if self.needs_run && self.debounce.is_none() {
            self.run(cx);
        } else if self.started
            && (self.stale || (other_note || revealed) && query_has_variables(&self.query(cx)))
        {
            if other_note {
                self.model.reset();
            }
            self.stale = false;
            self.list.update(cx, |list, cx| list.reload(cx));
            self.sync_preview(cx);
        }
    }

    /// The vault changed under the rows (a file operation landed, the
    /// index caught up): reloaded now if shown, else when next revealed.
    pub(crate) fn invalidate(&mut self, cx: &mut Context<Self>) {
        if !self.started {
            return;
        }
        if self.shown {
            self.list.update(cx, |list, cx| list.reload(cx));
        } else {
            self.stale = true;
        }
    }

    // ---- The preview -----------------------------------------------------------

    /// The needles of the listed query resolved against the open note:
    /// what the preview marks and an open emphasises.
    fn needles(&mut self, cx: &App) -> Arc<[String]> {
        let query = self.list.read(cx).query().to_string();
        let note = self.note();
        if self.needles.0 != query || self.needles.1 != note {
            let resolved = resolve_query(&query, &QueryContext::with_note(Some(note.clone())));
            self.needles = (query, note, emphasis_needles(&resolved).into());
        }
        self.needles.2.clone()
    }

    /// What an open from FIND emphasises in the note: the needles, if any.
    pub(crate) fn emphasis(&mut self, cx: &App) -> Option<Vec<String>> {
        let needles = self.needles(cx);
        (!needles.is_empty()).then(|| needles.to_vec())
    }

    fn selected_path(&self, cx: &App) -> Option<VaultPath> {
        self.list
            .read(cx)
            .selected_row()
            .map(|row| row.entry.path.clone())
    }

    /// Keep the preview on the selected row: another row collapses Full
    /// (Context stays open, TUI parity); collapsed, nothing is read.
    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let selected = if self.started {
            self.selected_path(cx)
        } else {
            None
        };
        let changed = self.model.sync(selected.clone());
        let wanted = if self.model.is_collapsed() {
            None
        } else {
            selected
        };
        let needles = self.needles(cx);
        self.preview
            .update(cx, |preview, cx| preview.show(wanted, needles, cx));
        if changed {
            cx.notify();
        }
    }

    /// Right: Collapsed → Context → Full (Full stays).
    fn grow(&mut self, cx: &mut Context<Self>) {
        if !self.model.is_full() {
            self.model.toggle(self.selected_path(cx));
        }
        self.sync_preview(cx);
        cx.notify();
    }

    /// Left: Full → Context → Collapsed.
    fn shrink(&mut self, cx: &mut Context<Self>) {
        self.model.collapse_step(self.selected_path(cx));
        self.sync_preview(cx);
        cx.notify();
    }

    /// Space and the preview button: Collapsed → Context → Full → Collapsed.
    fn cycle(&mut self, cx: &mut Context<Self>) {
        self.model.toggle(self.selected_path(cx));
        self.sync_preview(cx);
        cx.notify();
    }

    // ---- Keys ------------------------------------------------------------------

    /// Keys the results must see before the list does: in Full, Up/Down
    /// scroll the preview; Up from the first row returns to the field.
    fn capture_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.list_focused(window, cx) {
            return false;
        }
        let Some(key) = input::key_down_input(event) else {
            return false;
        };
        if key.mods != Mods::NONE {
            return false;
        }
        match key.key {
            Key::Up | Key::Down if self.model.is_full() => {
                let rows = if key.key == Key::Up { -1 } else { 1 };
                self.preview
                    .update(cx, |preview, cx| preview.scroll_rows(rows, cx));
                true
            }
            Key::Up if query_field::at_top(&self.list, cx) => {
                query_field::focus_input(&self.input, window, cx);
                true
            }
            _ => false,
        }
    }

    /// Keys the list left alone: the preview's, while the results have the
    /// keyboard; the `QuerySyntax` chord anywhere in the view (the workspace
    /// usually runs it first, from the same keymap).
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(key) = input::key_down_input(event) else {
            return false;
        };
        if crate::keymap::active(cx).is_bound(key, &ActionShortcuts::QuerySyntax) {
            cx.emit(FindEvent::Help);
            return true;
        }
        if key.mods != Mods::NONE {
            return false;
        }
        if !self.list_focused(window, cx) {
            return false;
        }
        match key.key {
            Key::Right => self.grow(cx),
            Key::Left => self.shrink(cx),
            Key::Char(' ') => self.cycle(cx),
            _ => return false,
        }
        true
    }

    // ---- Rendering -------------------------------------------------------------

    /// The query's name: "Backlinks" for the default, else the saved
    /// search's breadcrumb, else the query itself.
    fn title(&self, query: &str) -> SharedString {
        if query.trim().is_empty() {
            "No query".into()
        } else if is_default_query(query) {
            "Backlinks".into()
        } else {
            self.breadcrumb
                .label(query)
                .unwrap_or_else(|| query.trim().to_string())
                .into()
        }
    }

    fn render_bar(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = theme::current(cx);
        let t = &*theme;
        let list = self.list.read(cx);
        let count = (self.started && !list.is_loading()).then(|| {
            let total = list.first_row().map_or(0, |r| r.total);
            match total {
                1 => "1 note".to_string(),
                n => format!("{n} notes"),
            }
        });
        let title = self.title(query);
        let querying = !query.trim().is_empty();
        let chip = querying.then(|| {
            let (field, order) = order_of_query(query);
            sort_chip(FIND_SORT_ID, chip_label(&field, order), t)
                .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(FindEvent::Sort)))
                .test_support()
        });
        let (icon, tip) = if self.model.is_full() {
            (IconName::Minimize, "Back to the results (Space)")
        } else if self.model.is_context() {
            (IconName::Maximize, "Fill the view with the preview (Right)")
        } else {
            (
                IconName::PanelBottomOpen,
                "Preview the selected note (Right)",
            )
        };
        let expand = Button::new(FIND_EXPAND_ID)
            .ghost()
            .xsmall()
            .icon(icon)
            .tooltip(tip)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.cycle(cx);
                this.focus_list(window, cx);
            }));
        let save = Button::new(FIND_SAVE_ID)
            .ghost()
            .xsmall()
            .label("Save…")
            .tooltip(self.save_tip.clone())
            .disabled(!querying)
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(FindEvent::Save)));
        let help = Button::new(FIND_HELP_ID)
            .ghost()
            .xsmall()
            .label("Syntax")
            .tooltip("Query syntax (F1)")
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(FindEvent::Help)));
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .px_3()
            .pb_1()
            .text_xs()
            .child(
                ellipsis(title.clone())
                    .id(FIND_TITLE_ID)
                    .test_support()
                    .aria_label(title)
                    .flex_1()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::foreground(t)),
            )
            .when_some(count, |bar, count| {
                bar.child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_color(theme::muted(t))
                        .child(count),
                )
            })
            .children(chip)
            .child(expand)
            .child(save)
            .child(help)
    }
}

impl Focusable for FindView {
    /// The field: typing in FIND edits the query.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Render for FindView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let query = self.query(cx);
        let error = error_reason(&query).map(query_problem);
        let (shown, total) = {
            let list = self.list.read(cx);
            (list.len(), list.first_row().map_or(0, |r| r.total))
        };
        let more = (self.started && total > shown).then(|| {
            SharedString::from(format!("{} more results — refine the query", total - shown))
        });
        let full = self.model.is_full();
        let list = div()
            .flex()
            .flex_col()
            .min_h_0()
            // In Full the list keeps the keyboard out of sight.
            .map(|area| {
                if full {
                    area.h_0().overflow_hidden()
                } else {
                    area.flex_1()
                }
            })
            .child(self.list.clone());
        let pane = (!self.model.is_collapsed()).then(|| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .when(!full, |pane| {
                    pane.border_t_1().border_color(theme::divider(&t))
                })
                .child(
                    self.preview
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                )
        });
        div()
            .id(FIND_ID)
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .text_color(theme::foreground(&t))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.capture_key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            // The field's keys that mean something else here.
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                this.focus_list(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.commit(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                cx.emit(FindEvent::Dismiss);
                cx.stop_propagation();
            }))
            .child(query_field(&self.input, FIND_INPUT_ID, "Query", &t))
            .child(self.render_bar(&query, cx))
            .when_some(error, |view, error| {
                view.child(
                    div()
                        .id(FIND_ERROR_ID)
                        .test_support()
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .flex_none()
                        .px_3()
                        .pb_1()
                        .text_xs()
                        .text_color(theme::error(&t))
                        .child(error),
                )
            })
            .child(list)
            .children(pane)
            .when_some(more.filter(|_| !full), |view, more| {
                view.child(
                    div()
                        .id(FIND_MORE_ID)
                        .test_support()
                        .role(Role::Note)
                        .aria_label(more.clone())
                        .flex_none()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme::muted(&t))
                        .child(more),
                )
            })
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Make the FIND view over the vault the window opened (nothing runs
    /// until it is shown).
    pub(super) fn start_find(
        &mut self,
        vault: Arc<NoteVault>,
        tx: AppTx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let list = DrawerList::spawn(FIND_LIST_ID, "Results", tx, cx);
        let view = cx.new(|cx| FindView::new(vault, list.clone(), window, cx));
        self._views_events
            .push(cx.subscribe_in(&list, window, Self::on_find_list_event));
        self._views_events
            .push(cx.subscribe_in(&view, window, Self::on_find_event));
        self.find = Some(view);
    }

    fn on_find_list_event(
        &mut self,
        _: &Entity<DrawerList<QueryHit>>,
        event: &DrawerListEvent<QueryHit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Saved first, as every open is; the query's matches emphasised.
            DrawerListEvent::Activate(row) => {
                let emphasis = self
                    .find
                    .as_ref()
                    .and_then(|find| find.update(cx, |view, cx| view.emphasis(cx)));
                self.focus_panel(Panel::Editor, window, cx);
                self.open_note_with(row.entry.path.clone(), emphasis, cx);
            }
            DrawerListEvent::FileOp(kind, path) => {
                self.open_file_dialog(*kind, path.clone(), window, cx)
            }
            DrawerListEvent::Verb(..) => {}
            DrawerListEvent::Dismiss => self.dismiss_to_editor(window, cx),
        }
    }

    fn on_find_event(
        &mut self,
        _: &Entity<FindView>,
        event: &FindEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FindEvent::Help => self.open_query_help(window, cx),
            FindEvent::Save => self.save_find_query(window, cx),
            FindEvent::Sort => self.open_sort(SortTarget::Query, window, cx),
            FindEvent::Dismiss => self.dismiss_to_editor(window, cx),
        }
    }

    /// Bring FIND up to the layout and the open note (hidden: nothing).
    pub(super) fn refresh_find(&mut self, cx: &mut Context<Self>) {
        if let Some(find) = self.find.clone() {
            let shown = self.is_shown(DrawerView::Find);
            let note = self
                .notes
                .as_ref()
                .and_then(|h| h.note.clone())
                .unwrap_or_else(VaultPath::empty);
            find.update(cx, |view, cx| view.refresh(shown, &note, cx));
        }
    }

    /// FIND on `query` (leader `f b`, a tag), shown and given the keyboard:
    /// the one way to reveal FIND with a query (the TUI's
    /// `open_find_with_query`).
    pub(crate) fn open_find_with_query(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_find_named(query, None, window, cx);
    }

    /// [`Self::open_find_with_query`] for a saved search: FIND is named
    /// `name` (the TUI's `apply_saved_search`).
    pub(crate) fn open_find_named(
        &mut self,
        query: String,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(find) = self.find.clone() else {
            self.flash(super::route::NO_WORKSPACE, cx);
            return;
        };
        // Put in first: hidden, it waits for the reveal, which runs it once
        // (never the query it replaces), for the open note.
        find.update(cx, |view, cx| view.apply_query(query, name, window, cx));
        self.show_view(DrawerView::Find, window, cx);
    }

    /// `RunTagQuery` (a TAGS row): the search browser on the label, quoted
    /// as one term (a frontmatter label may hold spaces), as a `#tag`
    /// followed in the editor does. It is a modal, so no view is hidden or
    /// moved; [`Self::open_browser_query_in_find`] carries the query to FIND.
    pub(crate) fn run_tag_query(
        &mut self,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_search(Some(label_query(label)), window, cx);
    }

    /// The `OpenQueryInSidebar` chord in the browser: its typed query goes
    /// to [`Self::open_browser_query_in_find`].
    pub(crate) fn promote_browser_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_ref().and_then(|p| p.upgrade()) else {
            return;
        };
        let query = picker.read(cx).input.read(cx).value().to_string();
        self.open_browser_query_in_find(query, window, cx);
    }

    /// The browser's `Open in FIND` button, with the query typed in it:
    /// the browser closes and FIND shows the query
    /// ([`Self::open_find_with_query`]). A blank query says so and keeps the
    /// browser.
    pub(crate) fn open_browser_query_in_find(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if query.trim().is_empty() {
            self.flash("No query to open", cx);
            return;
        }
        if self.find.is_none() {
            self.flash(super::route::NO_WORKSPACE, cx);
            return;
        }
        self.close_modal(window, cx);
        self.open_find_with_query(query, window, cx);
    }

    /// Whether the keyboard is in FIND's query field (its copy chord is the
    /// field's).
    pub(super) fn find_field_focused(&self, window: &Window, cx: &App) -> bool {
        self.find.as_ref().is_some_and(|find| {
            find.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        })
    }
}
