//! The **Workspace**: the window's root view, composing the editor screen's
//! Panels left to right — the **Activity rail**, the left **Drawer**, and
//! the editor Panel (the [`EditorView`], unchanged inside). ADR-0050.
//!
//! **State.** Which panels are shown, the drawer's view and width, and which
//! panel has the keyboard live in the logic [`PanelLayout`]; this view only
//! renders it and keeps GPUI focus in step with it. The drawer shows a
//! placeholder per view until the views themselves arrive (M5b-2).
//!
//! **Keys.** Keys go to the focused Panel only. A chord the GUI keymap binds
//! to a workspace command (toggle the drawer, move or cycle focus) bubbles
//! here from whichever panel has focus — the editor lets those pass by —
//! and is resolved in that panel's context, as the editor resolves its own.
//! Alt+Up/Down with nothing stacked to move to is handed back to the editor
//! (ADR-0050 §4: not swallowed).
//!
//! **Persistence.** The drawer's width, visibility and view are written to
//! the `[gui]` table when a divider drag ends, the drawer is toggled, or the
//! rail switches its view, through the TUI's own settings save path.
//!
//! **The vault and the app channel (M5b-2).** [`Workspace::start`] puts the
//! window on what `main` decided ([`Start`]): an empty state in the editor
//! Panel when there is no vault, or the vault, indexed in the background
//! while the Footer says so. The workspace owns the window's one app
//! channel: the editor reports on it, background jobs (indexing, the
//! restore, the note session's saves and autosave ticks) send their
//! completions on it, and [`Workspace::on_app_message`] dispatches them on
//! the main thread.
//!
//! **The open note (M5b-2 G2).** Opening, saving and the flush before the
//! window closes are the note session's, hosted in [`note`]; the Footer
//! shows the note's path and whether it is saved.
//!
//! **FILES (M5b-2 G3).** With a vault, the left drawer's FILES view is the
//! [`files::FilesPane`]; the workspace keeps it in step with the app channel
//! and does what its rows ask (open a note, hand over an attachment).
//!
//! **Links (M5b-2 G4).** The `FollowLink` chord and the editor's
//! double-click follow the link under the caret ([`follow`]).

#[cfg(test)]
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{HandleEdge, ResizeHandleContext, TestSupportExt, resize_handle};
use gpui_kit::component::empty::{
    Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::{Icon, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Axis, ClickEvent, Context, DragMoveEvent, Entity,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div, px, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout::{FocusMove, Panel, PanelLayout, Side, WidthLimits};
use kimun_notes::logic::message::{AppMessage, AppTx};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::session::{NoteSession, SaveCompleted};
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, SharedSettings};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::editor_view::EditorView;
use crate::input;
use crate::keymap::{self, KeyCtx};
use crate::shell_layout::{self, RAIL_REMS, RAIL_VIEWS};
use crate::startup::{self, EmptyState, FirstNote, Start, VaultStart};
use crate::status_row::{self, DocFacts, Status};
use crate::theme;

use files::FilesPane;
#[cfg(test)]
pub(crate) use files::ICONS as FILES_ICONS;
pub(crate) use note::Closing;
use note::NoteHost;

/// The key context of the workspace root.
const KEY_CONTEXT: &str = "KimunWorkspace";

/// Where the layout is persisted.
pub(crate) enum LayoutStore {
    /// Nowhere (tests that are not about persistence).
    #[cfg(test)]
    Off,
    /// The user's `config.toml`, the TUI's.
    UserConfig,
    /// A config file at this path (tests).
    #[cfg(test)]
    File(PathBuf),
}

impl LayoutStore {
    /// Write `layout` into the store's `[gui]` table. A failure is logged and
    /// otherwise ignored: the layout in the window is still right.
    fn save(&self, layout: &PanelLayout<DrawerView>) {
        let result = match self {
            #[cfg(test)]
            LayoutStore::Off => return,
            LayoutStore::UserConfig => {
                AppSettings::update_gui(|gui| shell_layout::record(layout, gui))
            }
            #[cfg(test)]
            LayoutStore::File(path) => {
                AppSettings::update_gui_at(path.clone(), |gui| shell_layout::record(layout, gui))
            }
        };
        if let Err(e) = result {
            tracing::warn!("could not save the [gui] layout: {e}");
            eprintln!("kimun-gui: could not save the [gui] layout: {e}");
        }
    }
}

/// The payload of a drawer-divider drag: which drawer it resizes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrawerResize(Side);

/// What a divider drag shows under the pointer: nothing, the drawer itself
/// follows.
struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The Lucide icon of a drawer view's rail cell.
pub(crate) fn rail_icon(view: DrawerView) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName;
    match view {
        DrawerView::Files => IconName::Folder,
        DrawerView::Find => IconName::Search,
        DrawerView::Semantic => IconName::Sparkles,
        DrawerView::Ask => IconName::MessageCircleQuestionMark,
        DrawerView::Tags => IconName::Tag,
        DrawerView::Links => IconName::Link,
        DrawerView::Outline => IconName::ListTree,
        DrawerView::Config => IconName::Settings,
    }
}

/// The element id of `view`'s rail cell (`rail-files`, ...).
pub(crate) fn rail_cell_id(view: DrawerView) -> SharedString {
    format!("rail-{}", view.id()).into()
}

/// The element id of `side`'s drawer (`drawer-left`).
pub(crate) fn drawer_id(side: Side) -> &'static str {
    match side {
        Side::Left => "drawer-left",
        Side::Right => "drawer-right",
    }
}

/// The element id of `side`'s drawer divider.
pub(crate) fn divider_id(side: Side) -> &'static str {
    match side {
        Side::Left => "drawer-left-divider",
        Side::Right => "drawer-right-divider",
    }
}

/// The element id of the editor Panel's empty state.
pub(crate) const EMPTY_STATE_ID: &str = "empty-state";

/// The window's root view. See the module docs.
pub(crate) struct Workspace {
    pub(crate) layout: PanelLayout<DrawerView>,
    pub(crate) editor: Entity<EditorView>,
    theme: Theme,
    rail_focus: FocusHandle,
    drawer_focus: FocusHandle,
    /// The rail cell the keyboard is on (an index into [`RAIL_VIEWS`]).
    rail_cursor: usize,
    /// A divider drag is in progress (persisted when it ends).
    resizing: bool,
    /// Where on the divider the press landed, from the drawer's drawn edge
    /// (px), so the edge follows the pointer without jumping to it.
    grab: f64,
    store: LayoutStore,
    /// Why the editor Panel shows no editor (no workspace, or it would not
    /// open). `None`: the editor.
    empty: Option<EmptyState>,
    /// What the Footer shows.
    pub(crate) status: Status,
    /// The note session over the vault, when the window has one (not in the
    /// empty states, nor in the tests that feed the editor text directly).
    pub(crate) notes: Option<NoteHost>,
    /// The window's app channel, once [`Self::start`] set it up: the sender
    /// background jobs report on, and the task draining its receiver.
    tx: Option<AppTx>,
    _messages: Option<Task<()>>,
    /// Takes the flash down when it expires; replaced by a newer flash.
    flash_expiry: Option<Task<()>>,
    /// Where closing the window stands.
    pub(crate) closing: Closing,
    /// The window, for closing it once the close flush has written.
    window: Option<AnyWindowHandle>,
    /// The restore will land on the history's newest note: the editor Panel
    /// says so until its `OpenPath` arrives.
    restore_pending: bool,
    /// The note named on the command line, or the restore's, has not been
    /// asked for yet: FILES waits for it to pick the folder to list.
    first_open_pending: bool,
    /// The FILES view, when the window has a vault.
    pub(crate) files: Option<Entity<FilesPane>>,
    _files_events: Option<Subscription>,
    /// The link lookup under way (a newer follow replaces it).
    following: Option<Task<()>>,
    /// Hands URLs, images and attachments to the system's default program.
    pub(crate) opener: follow::Opener,
    /// Keeps the Footer's dirty state in step with the editor.
    _editor_watch: Subscription,
    _on_quit: Option<Subscription>,
}

impl Workspace {
    pub(crate) fn new(
        editor: Entity<EditorView>,
        theme: Theme,
        layout: PanelLayout<DrawerView>,
        store: LayoutStore,
        cx: &mut Context<Self>,
    ) -> Self {
        let rail_cursor = layout
            .slot_views(Side::Left)
            .first()
            .and_then(|v| RAIL_VIEWS.iter().position(|r| r == v))
            .unwrap_or(0);
        // Every edit notifies the editor; the Footer only redraws when the
        // dirty state flips.
        let editor_watch = cx.observe(&editor, |this, editor, cx| {
            let dirty = editor.read(cx).editor.is_dirty();
            if this.status.dirty != dirty {
                this.status.dirty = dirty;
                cx.notify();
            }
        });
        Self {
            layout,
            editor,
            theme,
            rail_focus: cx.focus_handle(),
            drawer_focus: cx.focus_handle(),
            rail_cursor,
            resizing: false,
            grab: 0.0,
            store,
            empty: None,
            status: Status::default(),
            notes: None,
            tx: None,
            _messages: None,
            flash_expiry: None,
            closing: Closing::Open,
            window: None,
            restore_pending: false,
            first_open_pending: false,
            files: None,
            _files_events: None,
            following: None,
            opener: follow::system_opener(),
            _editor_watch: editor_watch,
            _on_quit: None,
        }
    }

    /// Put the window on `start` (see [`startup`]): drain the window's app
    /// channel (`rx`; `tx` is its sender, the editor's too), then show the
    /// empty state, or index the vault in the background and land on the
    /// first note (FILES lists its folder). Must run with a tokio runtime
    /// entered.
    pub(crate) fn start(
        &mut self,
        start: Start,
        settings: SharedSettings,
        tx: AppTx,
        rx: UnboundedReceiver<AppMessage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.listen(rx, cx);
        self.tx = Some(tx.clone());
        self.empty = startup::empty_state(&start);
        if let Start::Vault(VaultStart { vault, name, first }) = start {
            self.notes = Some(NoteHost::new(NoteSession::new(
                vault.clone(),
                settings.clone(),
                VaultPath::root(),
            )));
            {
                let settings = settings.read().unwrap_or_else(|e| e.into_inner());
                self.start_files(vault.clone(), name, &settings, tx.clone(), window, cx);
            }
            self.status.indexing = true;
            let restore = first == FirstNote::Restore;
            self.restore_pending = restore;
            self.first_open_pending = matches!(first, FirstNote::Restore | FirstNote::Note(_));
            tokio::spawn(startup::index_and_restore(
                vault,
                settings,
                restore,
                tx.clone(),
            ));
            match first {
                FirstNote::Restore => {}
                FirstNote::Note(path) => {
                    tx.send(AppMessage::open(path)).ok();
                }
                FirstNote::Rejected(reason) => self.flash_error(reason, cx),
            }
            self.ensure_files_listing(cx);
        }
        cx.notify();
    }

    /// Dispatch the window's app channel on the main thread. tokio's
    /// channels need no tokio executor to be awaited, so GPUI's foreground
    /// executor drains it.
    fn listen(&mut self, mut rx: UnboundedReceiver<AppMessage>, cx: &mut Context<Self>) {
        self._messages = Some(cx.spawn(async move |this, cx| {
            while let Some(message) = rx.recv().await {
                if this
                    .update(cx, |workspace, cx| workspace.on_app_message(message, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// Act on one message from the app channel.
    pub(crate) fn on_app_message(&mut self, message: AppMessage, cx: &mut Context<Self>) {
        match message {
            // A background parse/layout job of the editor finished, or
            // FILES' listing has rows to show.
            AppMessage::Redraw => {
                self.editor.update(cx, |_, cx| cx.notify());
                self.poll_files(cx);
            }
            // Clipboard notices, the restore's "is gone", ... There is no
            // screen here that drops a flash, so a parked one shows now.
            AppMessage::FlashMessage(text) | AppMessage::ParkFlash(text) => self.flash(text, cx),
            AppMessage::IndexingDone(result) => {
                self.status.indexing = false;
                match result {
                    // A note opened while the index caught up counted its
                    // backlinks against the old one.
                    Ok(_) => self.refresh_note_counts(),
                    Err(e) => self.flash_error(format!("Indexing failed: {e}"), cx),
                }
            }
            AppMessage::VaultConflict(details) => {
                self.status.indexing = false;
                self.status.read_only = true;
                // Nothing is written from here on: no tick to run.
                if let Some(host) = self.notes.as_mut() {
                    host.session.stop_autosave();
                }
                self.flash_error(conflict_flash(&details), cx);
            }
            AppMessage::OpenPath { path, .. } => {
                // The restore's landing (or anything after it): no longer
                // "opening your last note".
                self.restore_pending = false;
                self.first_open_pending = false;
                if path.is_note() {
                    self.open_note(path, cx);
                } else {
                    // A folder (the root, for an empty history): FILES
                    // shows it, as the TUI's sidebar does.
                    self.files_show_dir(path, cx);
                }
            }
            AppMessage::OpenAttachment(path) => self.open_attachment(&path, cx),
            AppMessage::FileOp(op) => {
                self.files_file_op(&op, cx);
                self.refresh_note_counts();
            }
            // The Footer's counts for the open note.
            message @ (AppMessage::BacklinkCountLoaded { .. }
            | AppMessage::PropertyCountLoaded { .. }) => self.note_count_loaded(message),
            AppMessage::Autosave => self.autosave(cx),
            AppMessage::AutosaveCompleted {
                path,
                saved_revision,
                title,
                error,
            } => self.autosave_completed(
                SaveCompleted {
                    path,
                    saved_revision,
                    title,
                    error,
                },
                cx,
            ),
            // The editor's double-click on a link or a label.
            AppMessage::FollowLink(target) => self.follow_link(target, cx),
            AppMessage::FollowLabel(name) => self.follow_label(&name, cx),
            // Everything the GUI has no surface for yet.
            _ => {}
        }
        self.ensure_files_listing(cx);
        cx.notify();
    }

    /// Show the notice `text` in the Footer for
    /// [`status_row::FLASH_DURATION`], replacing a notice on show. An error
    /// on show keeps its time: the notice is dropped.
    pub(crate) fn flash(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        if self.status.flash.is_some() && self.status.flash_error {
            return;
        }
        self.show_flash(text.into(), false, cx);
    }

    /// Show the problem `text` in the Footer, in the error colour, for
    /// [`status_row::FLASH_DURATION`], replacing any flash on show.
    pub(crate) fn flash_error(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_flash(text.into(), true, cx);
    }

    fn show_flash(&mut self, text: SharedString, error: bool, cx: &mut Context<Self>) {
        self.status.flash = Some(text);
        self.status.flash_error = error;
        self.flash_expiry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(status_row::FLASH_DURATION)
                .await;
            this.update(cx, |workspace, cx| {
                workspace.status.flash = None;
                workspace.status.flash_error = false;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The left drawer's view, when it is shown.
    fn shown_view(&self) -> Option<DrawerView> {
        self.layout
            .is_visible(Panel::Drawer(Side::Left))
            .then(|| self.layout.slot_views(Side::Left).first().copied())
            .flatten()
    }

    fn handle_for(&self, panel: Panel, cx: &App) -> FocusHandle {
        match panel {
            Panel::Rail => self.rail_focus.clone(),
            // FILES: its filter, so typing filters.
            Panel::Drawer(_) => match (self.shown_view(), self.files.as_ref()) {
                (Some(DrawerView::Files), Some(files)) => files.read(cx).focus_handle(cx),
                _ => self.drawer_focus.clone(),
            },
            Panel::Editor => self.editor.read(cx).focus_handle(cx),
        }
    }

    /// Give the keyboard to the panel the layout says is focused.
    pub(crate) fn apply_focus(&self, window: &mut Window, cx: &mut App) {
        let handle = self.handle_for(self.layout.focused(), cx);
        window.focus(&handle, cx);
    }

    /// Focus `panel` (a press anywhere in it).
    fn focus_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.layout.focus(panel);
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Show the left drawer (and focus it, as the TUI does), or hide it and
    /// give its width to the editor. Persisted.
    ///
    /// Hiding the focused drawer gives the keyboard back to the document,
    /// not to the rail the model falls back to (the TUI keeps that).
    fn toggle_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let drawer = Panel::Drawer(Side::Left);
        if self.layout.is_visible(drawer) {
            let had_focus = self.layout.focused() == drawer;
            self.layout.hide(drawer);
            if had_focus {
                self.layout.focus(Panel::Editor);
            }
        } else {
            self.layout.show(drawer);
            self.layout.focus(drawer);
            self.ensure_files_listing(cx);
        }
        self.apply_focus(window, cx);
        self.store.save(&self.layout);
        cx.notify();
    }

    /// A rail cell was activated: the shown view closes the drawer (the
    /// TUI's rail), any other opens in it and takes the keyboard. Persisted.
    fn activate_rail_view(
        &mut self,
        view: DrawerView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(i) = RAIL_VIEWS.iter().position(|v| *v == view) {
            self.rail_cursor = i;
        }
        let drawer = Panel::Drawer(Side::Left);
        if self.shown_view() == Some(view) {
            self.layout.hide(drawer);
        } else {
            let (side, _) = self.layout.open_view(view);
            self.layout.focus(Panel::Drawer(side));
            self.ensure_files_listing(cx);
        }
        self.apply_focus(window, cx);
        self.store.save(&self.layout);
        cx.notify();
    }

    /// The model's sizing floors for this window.
    fn limits(window: &Window) -> WidthLimits {
        shell_layout::limits(
            f64::from(window.viewport_size().width),
            f64::from(window.rem_size()),
        )
    }

    /// Where the left drawer starts: the rail's end, in px.
    fn drawer_left_edge(window: &Window) -> f64 {
        f64::from(RAIL_REMS) * f64::from(window.rem_size())
    }

    /// A press on `side`'s divider: remember where on it, relative to the
    /// drawer's drawn edge.
    fn grab_divider(&mut self, side: Side, x: f64, window: &Window) {
        let edge = Self::drawer_left_edge(window)
            + shell_layout::shown_width(&self.layout, side, Self::limits(window));
        self.grab = x - edge;
    }

    fn on_drag_move(
        &mut self,
        event: &DragMoveEvent<DrawerResize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DrawerResize(side) = *event.drag(cx);
        let width = f64::from(event.event.position.x) - self.grab - Self::drawer_left_edge(window);
        self.layout
            .set_drawer_width(side, width, Self::limits(window));
        self.resizing = true;
        cx.notify();
    }

    /// The pointer came up (in the window or outside it): a divider drag in
    /// progress ends and is persisted. Idempotent.
    fn end_resize(&mut self, cx: &mut Context<Self>) {
        self.grab = 0.0;
        if std::mem::take(&mut self.resizing) {
            self.store.save(&self.layout);
            cx.notify();
        }
    }

    /// The context `panel` resolves chords in; the editor's own (vim, an
    /// open find bar) when it is the editor.
    fn key_ctx(&self, cx: &App) -> KeyCtx {
        match self.layout.focused() {
            Panel::Editor => self.editor.read(cx).key_ctx(),
            panel => KeyCtx { panel, vim: false },
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = input::key_down_input(event) else {
            return;
        };
        let action = keymap::active(cx).resolve(key, self.key_ctx(cx)).cloned();
        let handled = match action {
            Some(action) => self.workspace_command(&action, key, window, cx),
            None => self.layout.focused() == Panel::Rail && self.rail_key(key, window, cx),
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// Act on a keymap command that is the workspace's. False for one that
    /// is not (find, replace: the editor already had its chance).
    fn workspace_command(
        &mut self,
        action: &ActionShortcuts,
        key: KeyInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match action {
            ActionShortcuts::ToggleSidebar => self.toggle_drawer(window, cx),
            ActionShortcuts::SaveNote => self.save_note(cx),
            ActionShortcuts::OpenFileBrowser => self.open_file_browser(window, cx),
            // Nothing followable under the caret: the chord does nothing.
            ActionShortcuts::FollowLink => {
                self.follow_at_cursor(cx);
            }
            ActionShortcuts::FocusRegion(dir) => match self.layout.focus_dir(*dir) {
                FocusMove::Moved(_) => self.apply_focus(window, cx),
                FocusMove::AtEdge => {}
                // Nothing stacked above or below: the chord is the focused
                // panel's own key, not a focus move (ADR-0050 §4).
                FocusMove::NotHandled => {
                    if self.layout.focused() == Panel::Editor {
                        self.editor
                            .update(cx, |editor, cx| editor.editor_key(key, window, cx));
                    }
                }
            },
            ActionShortcuts::FocusEditor | ActionShortcuts::FocusSidebar => {
                let next = if *action == ActionShortcuts::FocusEditor {
                    self.layout.next_panel()
                } else {
                    self.layout.prev_panel()
                };
                if let Some(panel) = next {
                    self.layout.focus(panel);
                    self.apply_focus(window, cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// The rail's own keys: Up/Down move between cells, Enter or Space
    /// activates one (as a click does).
    fn rail_key(&mut self, key: KeyInput, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if key.mods != Mods::NONE {
            return false;
        }
        match key.key {
            Key::Up => self.rail_cursor = self.rail_cursor.saturating_sub(1),
            Key::Down => self.rail_cursor = (self.rail_cursor + 1).min(RAIL_VIEWS.len() - 1),
            Key::Enter | Key::Char(' ') => {
                self.activate_rail_view(RAIL_VIEWS[self.rail_cursor], window, cx);
            }
            _ => return false,
        }
        true
    }

    /// The focused panel's marker: a thin bar in the focus colour across
    /// its top edge, laid over the panel so focusing never shifts content.
    /// Subtle, but always there (design guides: state must be visible).
    fn focus_bar(&self, panel: Panel) -> Option<impl IntoElement> {
        (self.layout.focused() == panel).then(|| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h_0p5()
                .bg(theme::focus_border(&self.theme))
        })
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_bg = theme::panel_background(&self.theme);
        let focused = self.layout.focused() == Panel::Rail;
        let shown = self.shown_view();
        let (top, pinned) = RAIL_VIEWS.split_at(RAIL_VIEWS.len() - 1);
        let cell = |i: usize, view: DrawerView, cx: &mut Context<Self>| {
            self.render_rail_cell(
                view,
                shown == Some(view),
                focused && i == self.rail_cursor,
                cx,
            )
        };
        let top: Vec<AnyElement> = top
            .iter()
            .enumerate()
            .map(|(i, v)| cell(i, *v, cx))
            .collect();
        let pinned = cell(RAIL_VIEWS.len() - 1, pinned[0], cx);
        div()
            .id("rail")
            .test_support()
            .track_focus(&self.rail_focus)
            .role(Role::TabList)
            .aria_label("Activity rail")
            .flex()
            .flex_col()
            .flex_none()
            .h_full()
            .w(rems(RAIL_REMS))
            .gap_1()
            .pb_1()
            .relative()
            .bg(panel_bg)
            .border_r_1()
            .border_color(theme::divider(&self.theme))
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus_panel(Panel::Rail, window, cx)
            }))
            .children(top)
            .child(div().flex_1())
            .child(pinned)
            // Last, so it paints over a highlighted cell.
            .children(self.focus_bar(Panel::Rail))
    }

    fn render_rail_cell(
        &self,
        view: DrawerView,
        active: bool,
        cursor: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = theme::focus_border(&self.theme);
        let glyph = if active {
            accent
        } else if cursor {
            theme::bright(&self.theme)
        } else {
            theme::muted(&self.theme)
        };
        let hover = theme::hover_background(&self.theme);
        let selected = theme::selected_background(&self.theme);
        let name = shell_layout::view_name(view);
        div()
            .id(rail_cell_id(view))
            .test_support()
            .role(Role::Tab)
            .aria_label(name)
            .aria_selected(active)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .mx_1()
            .h_10()
            .rounded_md()
            // Selection is the cell's own surface (design guides): a
            // persistent fill plus the accent glyph, never an edge bar.
            .when(active, |cell| cell.bg(selected))
            .when(!active, |cell| cell.hover(move |style| style.bg(hover)))
            .child(Icon::new(rail_icon(view)).size_5().text_color(glyph))
            // The keyboard cursor: a ring in the focus colour, distinct
            // from the pointer's hover fill.
            .when(cursor, |cell| {
                cell.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded_md()
                        .border_1()
                        .border_color(accent),
                )
            })
            .tooltip(move |window, cx| Tooltip::new(name).build(window, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.activate_rail_view(view, window, cx)
            }))
            .into_any_element()
    }

    fn render_drawer(
        &self,
        view: DrawerView,
        width: f64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let panel_bg = theme::panel_background(&self.theme);
        let line = theme::divider(&self.theme);
        let active = theme::focus_border(&self.theme);
        div()
            .id(drawer_id(Side::Left))
            .test_support()
            .track_focus(&self.drawer_focus)
            .role(Role::Group)
            .aria_label(shell_layout::view_name(view))
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .h_full()
            // Measured runtime geometry: the user's dragged width, in pixels.
            .w(px(width as f32))
            .bg(panel_bg)
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus_panel(Panel::Drawer(Side::Left), window, cx)
            }))
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(theme::foreground(&self.theme))
                    .child(shell_layout::view_name(view)),
            )
            .map(|drawer| match (view, self.files.as_ref()) {
                (DrawerView::Files, Some(files)) => drawer.child(files.clone()),
                _ => drawer.child(
                    div()
                        .px_3()
                        .text_sm()
                        .text_color(theme::muted(&self.theme))
                        .child("Coming soon"),
                ),
            })
            .children(self.focus_bar(Panel::Drawer(Side::Left)))
            .child(
                resize_handle::<DrawerResize, DragGhost>(divider_id(Side::Left), Axis::Horizontal)
                    .inside(HandleEdge::Trailing)
                    .with_appearance(Rc::new(move |handle: &ResizeHandleContext, _, _| {
                        let color = if handle.state() == gpui_kit::base::ResizeHandleState::Idle {
                            line
                        } else {
                            active
                        };
                        // The hairline fills the band's one-pixel content box.
                        Some(
                            div()
                                .flex_none()
                                .h_full()
                                .w(px(1.))
                                .bg(color)
                                .into_any_element(),
                        )
                    }))
                    .on_drag(DrawerResize(Side::Left), |_, _, _, cx| {
                        cx.new(|_| DragGhost)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, _| {
                            this.grab_divider(Side::Left, f64::from(event.position.x), window)
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.end_resize(cx)),
                    ),
            )
    }

    /// What the editor Panel shows in place of the editor while a vault is
    /// open but no note is (yet): typing there could never be saved. `None`
    /// once a note is open, and without a vault (the fixture tests' editor).
    pub(crate) fn note_empty_state(&self) -> Option<EmptyState> {
        let host = self.notes.as_ref()?;
        if host.note.is_some() {
            return None;
        }
        let (title, detail) = if self.restore_pending {
            (
                "Opening your last note…",
                "It opens once the workspace is indexed.",
            )
        } else if host.is_switching() {
            ("Opening the note…", "")
        } else {
            (
                "No note open",
                "Start kimun-gui with a note's path to open it.",
            )
        };
        Some(EmptyState {
            title: title.into(),
            detail: detail.into(),
            failed: false,
        })
    }

    /// The editor Panel: the editor (or the empty state) over the Footer.
    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.empty.clone().or_else(|| self.note_empty_state());
        let document = match &empty {
            None => div()
                .flex_1()
                .min_h_0()
                .child(self.editor.clone())
                .into_any_element(),
            Some(empty) => self.render_empty(empty).into_any_element(),
        };
        div()
            .id("editor-panel")
            .test_support()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(theme::background(&self.theme))
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus_panel(Panel::Editor, window, cx)
            }))
            .child(document)
            .child(status_row::render(
                &self.status,
                &self.doc_facts(empty.is_none(), cx),
                &self.theme,
            ))
            .children(self.focus_bar(Panel::Editor))
    }

    /// What the Footer reads off the editor and the open note this frame.
    /// The caret only while the editor is shown and has the keyboard with
    /// its find bar closed; the find bar's matches while it is open; the
    /// note's counts once loaded, while a note is open.
    pub(crate) fn doc_facts(&self, editor_shown: bool, cx: &App) -> DocFacts {
        let editor = &self.editor.read(cx).editor;
        let focused = editor_shown && self.layout.focused() == Panel::Editor;
        let find_open = editor.find_open();
        let meta = self
            .notes
            .as_ref()
            .filter(|h| h.note.is_some())
            .map(|h| &h.meta);
        DocFacts {
            caret: (focused && !find_open).then(|| {
                let (row, col) = editor.cursor();
                (row + 1, col + 1)
            }),
            find: if editor_shown && find_open {
                editor.find_match_position()
            } else {
                None
            },
            backlinks: meta.and_then(|m| m.backlinks()),
            properties: meta.and_then(|m| m.properties()),
        }
    }

    /// Why there is no note to edit, centred in the editor Panel.
    fn render_empty(&self, empty: &EmptyState) -> impl IntoElement {
        let (icon, tint) = if empty.failed {
            (IconName::TriangleAlert, theme::error(&self.theme))
        } else {
            (IconName::FolderOpen, theme::muted(&self.theme))
        };
        div()
            .id(EMPTY_STATE_ID)
            .test_support()
            .role(Role::Group)
            .aria_label(SharedString::from(empty.title.clone()))
            .aria_description(SharedString::from(empty.detail.clone()))
            .flex()
            .flex_1()
            .min_h_0()
            .child(
                Empty::new()
                    .border_0()
                    .text_color(theme::foreground(&self.theme))
                    .header(
                        EmptyHeader::new()
                            .media(
                                EmptyMedia::new()
                                    .with_variant(EmptyMediaVariant::Icon)
                                    .bg(theme::panel_background(&self.theme))
                                    .child(Icon::new(icon).text_color(tint)),
                            )
                            .title(EmptyTitle::new().child(empty.title.clone()))
                            .description(
                                EmptyDescription::new()
                                    .text_color(theme::muted(&self.theme))
                                    .child(empty.detail.clone()),
                            ),
                    ),
            )
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.handle_for(self.layout.focused(), cx)
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let limits = Self::limits(window);
        let drawer = self.shown_view().map(|view| {
            (
                view,
                shell_layout::shown_width(&self.layout, Side::Left, limits),
            )
        });
        div()
            .id("workspace")
            .key_context(KEY_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_drag_move(cx.listener(Self::on_drag_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_resize(cx)),
            )
            // A drag released outside the window ends there too.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_resize(cx)),
            )
            .flex()
            .flex_row()
            .items_stretch()
            .size_full()
            .bg(theme::background(&self.theme))
            .child(self.render_rail(cx))
            .when_some(drawer, |row, (view, width)| {
                row.child(self.render_drawer(view, width, cx))
            })
            .child(self.render_editor(cx))
    }
}

/// The flash for a case conflict: one line, the count; the full list went to
/// the log (`index_and_restore` logs it).
fn conflict_flash(details: &str) -> String {
    let count = details
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .count();
    let (names, differ) = if count == 1 {
        ("name", "differs")
    } else {
        ("names", "differ")
    };
    format!(
        "Read-only: {count} {names} in this vault {differ} only by case (see the log). \
         Rename them, then restart."
    )
}

mod files;
mod follow;
mod note;

#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod follow_tests;
#[cfg(test)]
mod footer_tests;
#[cfg(test)]
mod note_tests;
#[cfg(test)]
mod start_tests;
#[cfg(test)]
mod tests;
