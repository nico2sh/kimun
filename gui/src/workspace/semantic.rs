//! The **SEM** drawer view (M7c G9), "Semantic search": the notes the kimün
//! server ranks most similar to a query, one row per note, in the server's
//! order, in the shared [`DrawerList`], and a preview of the selected note
//! that grows Collapsed → Context → Full.
//!
//! **Rows** are the logic's (`SemanticSource`: the server's chunk results,
//! one [`SemanticHit`] per note, keeping its best chunk). A row is drawn as
//! FIND draws one ([`result_row`]): the title (the matched section's), the
//! similarity, the filename and the chunk as one line of context; a
//! right-click shows the FILES entry menu. The server already ranks by
//! meaning, so nothing re-filters them here by the query's words (the
//! TUI's `LOCAL_FILTER = false`). The source waits out typing itself
//! (250 ms) and the list drops a superseded search, so only the latest
//! query's rows land.
//!
//! **Preview** ([`SearchPreview`], the Source reader's way): only while
//! expanded, the selected note is read after a short wait, scrolled to the
//! matched section and that section marked. Context shows it under the
//! list; Full gives it the whole view (the list keeps the keyboard).
//!
//! **Where it runs.** Every search is the source's, on tokio: it reads the
//! vault id and the server settings, then makes one HTTP request. The UI
//! thread only takes the finished rows. A blank query asks nothing.
//!
//! **States.** A blank query says how to start; a search under way says
//! "Searching…"; no rows says "No results", or, while the server cannot
//! search, why (the TUI's own wording, `semantic_unavailable_message`).
//!
//! **Keys.** The query field has the keyboard when the view opens: Down or
//! Enter give the results the keyboard, Escape gives it back to the
//! editor. In the results: Up/Down/Home/End move, Up on the first row
//! returns to the field, Enter opens the note (saved first, as every open),
//! Right grows the preview, Left shrinks it, Space cycles it; in Full,
//! Up/Down scroll the preview. Escape goes back to the editor.
//!
//! **Pointer.** A click opens the note; the chevron on the selected or
//! hovered row, and the one on the preview's header, step the preview.
//!
//! **When it loads.** The source is installed on the view's first reveal
//! (the TUI's `ensure_source`); after that each edit of the query searches.
//! A view placed while the server is off stays (its rail cell goes); it
//! says why it cannot search.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Enter, Escape, InputEvent, InputState, MoveDown};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, StyleRefinement, Styled as _, Subscription, Window,
    div,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::list::{ListSetup, RowSource};
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::preview::PreviewModel;
use kimun_notes::logic::views::{SemanticHit, SemanticSource};
#[cfg(test)]
use kimun_notes::server_client::RagClient;
use kimun_notes::settings::SharedSettings;
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::ask_sources::score_label;
use super::chevron::{Reach, preview_header};
use super::drawer_list::{DrawerList, DrawerListEvent, DrawerRow, EXPAND};
use super::files::{display_title, note_count_text};
use super::query_field::{self, query_field};
use super::result_row::result_row;
use super::search::{Mark, SearchPreview, SectionMark};
use crate::input;
use crate::theme;

/// Element ids, for tests.
pub(crate) const SEMANTIC_ID: &str = "semantic";
pub(crate) const SEMANTIC_INPUT_ID: &str = "semantic-input";
pub(crate) const SEMANTIC_LIST_ID: &str = "semantic-list";
pub(crate) const SEMANTIC_BAR_ID: &str = "semantic-bar";
pub(crate) const SEMANTIC_PREVIEW_ID: &str = "semantic-preview";
pub(crate) const SEMANTIC_PREVIEW_HEADER_ID: &str = "semantic-preview-header";
pub(crate) const SEMANTIC_PREVIEW_ROW_ID: &str = "semantic-preview-row";

/// What the view says before a query is typed.
pub(crate) const BLANK_TEXT: &str = "Type to search by meaning";
/// What it says while a search is under way with nothing listed yet (the
/// TUI's words).
pub(crate) const SEARCHING_TEXT: &str = "Searching…";
/// What a search that found nothing says (the TUI's words).
pub(crate) const NO_RESULTS_TEXT: &str = "No results";

/// The element id of the row for `path`.
pub(crate) fn semantic_row_id(path: &VaultPath) -> SharedString {
    format!("semantic-row:{path}").into()
}

/// A hit's title as its row shows it: the matched section's, else the
/// filename.
fn hit_title(hit: &SemanticHit) -> &str {
    match &hit.entry {
        FileListEntry::Note {
            title, filename, ..
        } => display_title(title, filename),
        _ => "",
    }
}

fn hit_filename(hit: &SemanticHit) -> &str {
    match &hit.entry {
        FileListEntry::Note { filename, .. } => filename,
        _ => "",
    }
}

impl DrawerRow for SemanticHit {
    fn element_id(&self) -> SharedString {
        semantic_row_id(self.entry.path())
    }

    /// A result as FIND draws one, with the similarity and the matched
    /// chunk as its context.
    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        result_row(
            self.element_id(),
            selected,
            hit_title(self),
            hit_filename(self),
            &self.chunk,
            Some(score_label(self.score)),
            &[],
            t,
        )
    }

    fn menu_target(&self) -> Option<VaultPath> {
        match &self.entry {
            FileListEntry::Note { path, .. } => Some(path.clone()),
            _ => None,
        }
    }
}

/// What SEM asks of the workspace (the list's own events go there
/// directly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SemanticEvent {
    /// Escape in the query field: back to the editor.
    Dismiss,
}

/// The SEM view. See the module docs.
pub(crate) struct SemanticView {
    pub(crate) list: Entity<DrawerList<SemanticHit>>,
    pub(crate) preview: Entity<SearchPreview>,
    input: Entity<InputState>,
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    /// How much of the selected note shows, and for which row.
    model: PreviewModel,
    /// Searches with this client instead of the configured server (tests:
    /// a stub transport).
    #[cfg(test)]
    client: Option<Arc<RagClient>>,
    /// The source is in the list (the first reveal installed it).
    installed: bool,
    /// Why the server cannot search now (`None`: it can).
    unavailable: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SemanticEvent> for SemanticView {}

impl SemanticView {}

impl SemanticView {
    fn new(
        vault: Arc<NoteVault>,
        settings: SharedSettings,
        list: Entity<DrawerList<SemanticHit>>,
        unavailable: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let preview_vault = vault.clone();
        let preview = cx.new(|cx| {
            SearchPreview::new(preview_vault, cx)
                .with_ids(SEMANTIC_PREVIEW_ID, SEMANTIC_PREVIEW_ROW_ID)
        });
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.typed(cx);
                }
            }),
            // A selection move, or rows landing: the preview follows.
            cx.observe(&list, |this, _, cx| this.sync_preview(cx)),
        ];
        list.update(cx, |list, cx| list.set_chevron(Some(Reach::Grow), cx));
        let view = Self {
            list,
            preview,
            input,
            vault,
            settings,
            model: PreviewModel::new(),
            #[cfg(test)]
            client: None,
            installed: false,
            unavailable,
            _subscriptions: subscriptions,
        };
        let empty = view.empty_text("");
        view.list.update(cx, |list, cx| list.set_empty(empty, cx));
        view
    }

    /// Search with `client` from now on, instead of the configured server
    /// (tests: a stub transport). Before the first reveal.
    #[cfg(test)]
    pub(crate) fn use_client(&mut self, client: Arc<RagClient>) {
        debug_assert!(!self.installed, "the source is already in the list");
        self.client = Some(client);
    }

    /// The query in the field.
    pub(crate) fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Whether the source is in the list (tests).
    #[cfg(test)]
    pub(crate) fn is_installed(&self) -> bool {
        self.installed
    }

    /// The preview's state (tests).
    #[cfg(test)]
    pub(crate) fn preview_model(&self) -> &PreviewModel {
        &self.model
    }

    /// What the list says with no row for `query`: why the server cannot
    /// search, else the hint for a blank query, else that nothing matched.
    fn empty_text(&self, query: &str) -> &'static str {
        match self.unavailable {
            Some(why) => why,
            None if query.trim().is_empty() => BLANK_TEXT,
            None => NO_RESULTS_TEXT,
        }
    }

    /// The result count under the field: only once a search has landed
    /// with rows (while loading, for a blank query, with the server
    /// unavailable, or with no rows - the list says "No results" itself -
    /// there is nothing to count).
    fn count_bar(&self, t: &Theme, cx: &App) -> Option<impl IntoElement + use<>> {
        let list = self.list.read(cx);
        let counted = self.installed
            && self.unavailable.is_none()
            && !self.query(cx).trim().is_empty()
            && !list.is_loading()
            && list.len() > 0;
        counted.then(|| {
            let text = note_count_text(list.len());
            div()
                .id(SEMANTIC_BAR_ID)
                .test_support()
                .aria_label(text.clone())
                .flex_none()
                .px_3()
                .pb_1()
                .text_xs()
                .text_color(theme::muted(t))
                .child(text)
        })
    }

    /// The source a search runs through: the configured server, read per
    /// search (production), or the test's client.
    fn source(&self) -> Arc<dyn RowSource<SemanticHit>> {
        #[cfg(test)]
        if let Some(client) = self.client.clone() {
            return Arc::new(SemanticSource::with_client(self.vault.clone(), client));
        }
        Arc::new(SemanticSource::new(
            self.vault.clone(),
            self.settings.clone(),
        ))
    }

    /// The view was shown or hidden: the first reveal installs the source
    /// (and searches the field's query, if any). Later reveals keep the
    /// query and its rows.
    pub(crate) fn refresh(&mut self, shown: bool, cx: &mut Context<Self>) {
        if !shown || self.installed {
            return;
        }
        self.installed = true;
        let query = self.query(cx);
        let setup = ListSetup {
            query: query.clone(),
            ..ListSetup::default()
        };
        let source = self.source();
        let empty = self.empty_text(&query);
        self.list
            .update(cx, |list, cx| list.load_with(source, setup, empty, cx));
    }

    /// The kimün server's capability changed: `unavailable` is why it
    /// cannot search now (`None`: it can). Only what the empty list says
    /// changes; rows already listed stay.
    pub(crate) fn set_unavailable(
        &mut self,
        unavailable: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        self.unavailable = unavailable;
        let empty = self.empty_text(&self.query(cx));
        self.list.update(cx, |list, cx| list.set_empty(empty, cx));
        cx.notify();
    }

    /// The field changed: search it (the source waits for typing to
    /// pause; a blank query asks the server nothing).
    fn typed(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let empty = self.empty_text(&query);
        self.list.update(cx, |list, cx| {
            list.set_empty(empty, cx);
            list.set_query(query, cx);
        });
    }

    fn focus_list(&self, window: &mut Window, cx: &mut Context<Self>) {
        query_field::focus_list(&self.list, window, cx);
    }

    fn list_focused(&self, window: &Window, cx: &App) -> bool {
        query_field::list_focused(&self.list, window, cx)
    }

    // ---- The preview -----------------------------------------------------------

    fn selected_path(&self, cx: &App) -> Option<VaultPath> {
        self.list
            .read(cx)
            .selected_row()
            .map(|row| row.entry.path().clone())
    }

    /// Keep the preview on the selected row's matched section: another row
    /// collapses Full (Context stays open, TUI parity); collapsed, nothing
    /// is read. The same note and section again leave the preview as it is.
    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let changed = self.model.sync(self.selected_path(cx));
        let wanted = if self.model.is_collapsed() {
            None
        } else {
            self.list.read(cx).selected_row().map(|hit| {
                let mark = Mark::Section(SectionMark {
                    ordinal: hit.ordinal,
                    heading: hit.heading.as_str().into(),
                    chunk: hit.chunk.as_str().into(),
                });
                (hit.entry.path().clone(), mark)
            })
        };
        let (path, mark) = match wanted {
            Some((path, mark)) => (Some(path), mark),
            None => (None, Mark::Needles(Arc::from(Vec::new()))),
        };
        self.preview
            .update(cx, |preview, cx| preview.show_marked(path, mark, cx));
        self.sync_chevron(cx);
        if changed {
            cx.notify();
        }
    }

    /// The rows' chevron follows the preview: `›` until Full, `‹` there.
    fn sync_chevron(&mut self, cx: &mut Context<Self>) {
        let reach = Reach::of(&self.model);
        self.list
            .update(cx, |list, cx| list.set_chevron(Some(reach), cx));
    }

    /// A click on a chevron (a row's or the header's): Full shrinks as Left
    /// does, else the preview grows as Right does; the list has the
    /// keyboard after.
    pub(crate) fn chevron_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.is_full() {
            self.shrink(cx);
        } else {
            self.grow(cx);
        }
        self.focus_list(window, cx);
    }

    /// The preview's one-line header: the selected note's title and the
    /// chevron. `None` with no row selected.
    fn preview_header(&self, t: &Theme, cx: &Context<Self>) -> Option<impl IntoElement + use<>> {
        let list = self.list.read(cx);
        let title = SharedString::from(hit_title(list.selected_row()?).to_string());
        Some(
            preview_header(
                SEMANTIC_PREVIEW_HEADER_ID.into(),
                title,
                Reach::of(&self.model),
                t,
            )
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if event.standard_click() {
                    this.chevron_step(window, cx);
                }
            }))
            .test_support(),
        )
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

    /// Space: Collapsed → Context → Full → Collapsed.
    fn cycle(&mut self, cx: &mut Context<Self>) {
        self.model.toggle(self.selected_path(cx));
        self.sync_preview(cx);
        cx.notify();
    }

    // ---- Keys ------------------------------------------------------------------

    /// Keys the results must see before the list does: in Full, Up/Down
    /// scroll the preview and Page Up/Page Down do nothing (the list is
    /// hidden); Up on the first row (or with none selected)
    /// returns to the field. True when the key was taken.
    fn capture_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.model.is_full()
            && self.list_focused(window, cx)
            && let Some(key) = input::key_down_input(event)
            && key.mods == Mods::NONE
        {
            match key.key {
                Key::Up | Key::Down => {
                    let rows = if key.key == Key::Up { -1 } else { 1 };
                    self.preview
                        .update(cx, |preview, cx| preview.scroll_rows(rows, cx));
                    return true;
                }
                // The list is hidden: the page keys must not move it.
                Key::PageUp | Key::PageDown => return true,
                _ => {}
            }
        }
        query_field::up_returns_to_field(&self.list, &self.input, event, window, cx)
    }

    /// Keys the list left alone: the preview's, while the results have the
    /// keyboard.
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(key) = input::key_down_input(event) else {
            return false;
        };
        if key.mods != Mods::NONE || !self.list_focused(window, cx) {
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
}

impl Focusable for SemanticView {
    /// The query field: typing in SEM searches.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Render for SemanticView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
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
        let header = self.preview_header(t, cx);
        let bar = self.count_bar(t, cx);
        let pane = (!self.model.is_collapsed()).then(|| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .when(!full, |pane| {
                    pane.border_t_1().border_color(theme::divider(t))
                })
                .children(header)
                .child(
                    self.preview
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                )
        });
        div()
            .id(SEMANTIC_ID)
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .text_color(theme::foreground(t))
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
                this.focus_list(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                cx.emit(SemanticEvent::Dismiss);
                cx.stop_propagation();
            }))
            .child(query_field(&self.input, SEMANTIC_INPUT_ID, "Query", t))
            .children(bar)
            .child(list)
            .children(pane)
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Make the SEM view over the vault the window opened (nothing is
    /// searched until it is shown and a query typed).
    pub(super) fn start_semantic(
        &mut self,
        vault: Arc<NoteVault>,
        settings: SharedSettings,
        tx: AppTx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let list = DrawerList::spawn_with(SEMANTIC_LIST_ID, "Results", tx, cx, |list| {
            list.with_loading_text(SEARCHING_TEXT)
        });
        let unavailable = self.semantic_block();
        let view =
            cx.new(|cx| SemanticView::new(vault, settings, list.clone(), unavailable, window, cx));
        self._views_events
            .push(cx.subscribe_in(&list, window, Self::on_semantic_list_event));
        self._views_events
            .push(cx.subscribe_in(&view, window, Self::on_semantic_event));
        self.semantic = Some(view);
    }

    fn on_semantic_list_event(
        &mut self,
        _: &Entity<DrawerList<SemanticHit>>,
        event: &DrawerListEvent<SemanticHit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Saved first, as every open is (the note session's switch).
            DrawerListEvent::Activate(row) => {
                if let FileListEntry::Note { path, .. } = &row.entry {
                    self.focus_panel(Panel::Editor, window, cx);
                    self.open_note(path.clone(), cx);
                }
            }
            DrawerListEvent::FileOp(kind, path) => {
                self.open_file_dialog(*kind, path.clone(), window, cx)
            }
            DrawerListEvent::Verb(_, EXPAND) => {
                if let Some(view) = self.semantic.clone() {
                    view.update(cx, |view, cx| view.chevron_step(window, cx));
                }
            }
            DrawerListEvent::Verb(..) => {}
            DrawerListEvent::Dismiss => self.dismiss_to_editor(window, cx),
        }
    }

    fn on_semantic_event(
        &mut self,
        _: &Entity<SemanticView>,
        event: &SemanticEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SemanticEvent::Dismiss => self.dismiss_to_editor(window, cx),
        }
    }

    /// Bring SEM up to the layout (hidden: nothing).
    pub(super) fn refresh_semantic(&mut self, cx: &mut Context<Self>) {
        if let Some(semantic) = self.semantic.clone() {
            let shown = self.is_shown(DrawerView::Semantic);
            semantic.update(cx, |view, cx| view.refresh(shown, cx));
        }
    }

    /// Leader `drawer.semantic` (the id is the TUI's, shared by both
    /// frontends' leader trees), the palette's row: SEM shown and given the
    /// keyboard, or, while the server cannot search, the flash that says
    /// why (the TUI's words) and nothing opens.
    pub(crate) fn open_semantic(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = self.semantic_block() {
            self.flash(why, cx);
            return;
        }
        self.show_view(DrawerView::Semantic, window, cx);
    }
}
