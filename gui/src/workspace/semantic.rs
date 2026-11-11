//! The **SEM** drawer view (M7c G9), "Semantic search": the notes the kimün
//! server ranks most similar to a query, one row per note, in the server's
//! order, in the shared [`DrawerList`].
//!
//! **Rows** are the logic's (`SemanticSource`: the server's chunk results,
//! one row per note, drawn as FILES draws a note, with the FILES entry
//! menu). The server already ranks by meaning, so nothing re-filters them
//! here by the query's words (the TUI's `LOCAL_FILTER = false`). The source
//! waits out typing itself (250 ms) and the list drops a superseded search,
//! so only the latest query's rows land.
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
//! Escape goes back to the editor. A click opens; a right-click shows the
//! FILES entry menu.
//!
//! **When it loads.** The source is installed on the view's first reveal
//! (the TUI's `ensure_source`); after that each edit of the query searches.
//! A view placed while the server is off stays (its rail cell goes); it
//! says why it cannot search.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Enter, Escape, InputEvent, InputState, MoveDown};
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    Stateful, Styled as _, Subscription, Window, div,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::list::{ListSetup, RowSource};
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::views::SemanticSource;
#[cfg(test)]
use kimun_notes::server_client::RagClient;
use kimun_notes::settings::SharedSettings;
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::drawer_list::{DrawerList, DrawerListEvent, DrawerRow};
use super::files::entry_row;
use super::query_field::{self, query_field};
use crate::theme;

/// Element ids, for tests.
pub(crate) const SEMANTIC_ID: &str = "semantic";
pub(crate) const SEMANTIC_INPUT_ID: &str = "semantic-input";
pub(crate) const SEMANTIC_LIST_ID: &str = "semantic-list";

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

impl DrawerRow for FileListEntry {
    fn element_id(&self) -> SharedString {
        semantic_row_id(self.path())
    }

    /// A note as FILES draws it: its title (the best-matching section's),
    /// its filename and journal date under it.
    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        entry_row(self.element_id(), self, selected, true, None, t)
    }

    fn menu_target(&self) -> Option<VaultPath> {
        match self {
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
    pub(crate) list: Entity<DrawerList<FileListEntry>>,
    input: Entity<InputState>,
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    /// Searches with this client instead of the configured server (tests:
    /// a stub transport).
    #[cfg(test)]
    client: Option<Arc<RagClient>>,
    /// The source is in the list (the first reveal installed it).
    installed: bool,
    /// Why the server cannot search now (`None`: it can).
    unavailable: Option<&'static str>,
    _input_events: Subscription,
}

impl EventEmitter<SemanticEvent> for SemanticView {}

impl SemanticView {}

impl SemanticView {
    fn new(
        vault: Arc<NoteVault>,
        settings: SharedSettings,
        list: Entity<DrawerList<FileListEntry>>,
        unavailable: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let input_events = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.typed(cx);
            }
        });
        let view = Self {
            list,
            input,
            vault,
            settings,
            #[cfg(test)]
            client: None,
            installed: false,
            unavailable,
            _input_events: input_events,
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

    /// What the list says with no row for `query`: why the server cannot
    /// search, else the hint for a blank query, else that nothing matched.
    fn empty_text(&self, query: &str) -> &'static str {
        match self.unavailable {
            Some(why) => why,
            None if query.trim().is_empty() => BLANK_TEXT,
            None => NO_RESULTS_TEXT,
        }
    }

    /// The source a search runs through: the configured server, read per
    /// search (production), or the test's client.
    fn source(&self) -> Arc<dyn RowSource<FileListEntry>> {
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

    /// Up on the results' first row (or with none selected) returns to the
    /// field; every other key is the list's. True when the key was taken.
    fn capture_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        query_field::up_returns_to_field(&self.list, &self.input, event, window, cx)
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
            .child(self.list.clone())
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
        _: &Entity<DrawerList<FileListEntry>>,
        event: &DrawerListEvent<FileListEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Saved first, as every open is (the note session's switch).
            DrawerListEvent::Activate(row) => {
                if let FileListEntry::Note { path, .. } = row {
                    self.focus_panel(Panel::Editor, window, cx);
                    self.open_note(path.clone(), cx);
                }
            }
            DrawerListEvent::FileOp(kind, path) => {
                self.open_file_dialog(*kind, path.clone(), window, cx)
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
