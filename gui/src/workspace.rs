//! The **Workspace**: the window's root view, composing the editor screen's
//! Panels left to right — the **Activity rail**, the left **Drawer**, the
//! editor Panel (the [`EditorView`], unchanged inside) and the right
//! **Drawer**. ADR-0050.
//!
//! **State.** Which panels are shown, the views in each drawer's **drawer
//! slots** (up to two, stacked), the drawers' widths and split ratios, and
//! which region has the keyboard live in the logic [`PanelLayout`]; this
//! view only renders it and keeps GPUI focus in step with it. A view
//! without a GUI yet shows a placeholder in its slot.
//!
//! **Arranging (M5b-3).** Each view keeps a placement (side and drawer
//! slot) in the layout model, shown or hidden. The rail shows a view where
//! it is placed, hiding the view sharing its slot, and a click on a shown
//! view hides it (its slot closes while the drawer shows another view, else
//! the drawer hides); the move, swap, split and unsplit chords rearrange
//! the views from the rail or a drawer; each drawer has its own toggle. A
//! view is shown in at most one slot, so FILES (one entity) renders
//! wherever it is.
//!
//! **Keys.** Keys go to the focused Panel only. A chord the GUI keymap binds
//! to a workspace command (toggle the drawer, move or cycle focus) bubbles
//! here from whichever panel has focus — the editor lets those pass by —
//! and is resolved in that panel's context, as the editor resolves its own.
//! With the rail or a drawer focused, the workspace resolves its chords
//! before any key binding, so a field inside a drawer (the FILES filter)
//! cannot swallow them.
//! Alt+Up/Down with nothing stacked to move to is handed back to the editor
//! (ADR-0050 §4: not swallowed).
//!
//! **Persistence.** Both drawers' widths, visibility, views and split
//! ratios are written to the `[gui]` table when a divider drag ends, a
//! drawer is toggled, or a view is opened, moved, split off or folded away,
//! through the TUI's own settings save path.
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
use gpui_kit::base::ResizeHandleRenderer;
use gpui_kit::base::{HandleEdge, ResizeHandleContext, TestSupportExt, resize_handle};
use gpui_kit::component::empty::{
    Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::{Icon, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Axis, ClickEvent, Context, DragMoveEvent, Entity,
    FocusHandle, Focusable, InputPreference, InteractiveElement, IntoElement, KeyDownEvent,
    KeystrokeEvent, MouseButton, MouseDownEvent, ParentElement, Render, Rgba, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity, Window, div, px, relative,
    rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::{self, LeaderNode};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout::{
    FocusMove, LayoutError, MAX_DRAWER_SLOTS, Panel, PanelLayout, Region, Side, WidthLimits,
};
use kimun_notes::logic::message::{AppMessage, AppTx};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::session::{NoteSession, SaveCompleted};
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, SharedSettings};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::editor_view::{EditorEvent, EditorView};
use crate::input;
use crate::keymap::{self, KeyCtx};
use crate::shell_layout::{self, RAIL_REMS, RAIL_VIEWS};
use crate::startup::{self, EmptyState, FirstNote, Start, VaultStart};
use crate::status_row::{self, DocFacts, Status};
use crate::theme;

use files::FilesPane;
#[cfg(test)]
pub(crate) use files::ICONS as FILES_ICONS;
use modal::ModalKind;
pub(crate) use note::Closing;
use note::NoteHost;
pub(crate) use palette::Palette;

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

/// The payload of a slot-divider drag: whose drawer's split it moves.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SlotResize(Side);

/// Index of `side` in the per-side arrays.
fn side_ix(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
    }
}

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

/// The element id of the divider between `side`'s two stacked slots.
pub(crate) fn split_divider_id(side: Side) -> &'static str {
    match side {
        Side::Left => "drawer-left-split",
        Side::Right => "drawer-right-split",
    }
}

/// The element id of the slot showing `view` (`slot-files`, ...): a view
/// is shown in at most one slot, so its id names the slot.
pub(crate) fn slot_id(view: DrawerView) -> SharedString {
    format!("slot-{}", view.id()).into()
}

/// A divider's look: a one-pixel hairline (a physical boundary, not
/// zoomed) along the band, in the divider colour at rest and the focus
/// colour while hovered, pressed or dragged.
fn hairline(axis: Axis, line: Rgba, active: Rgba) -> ResizeHandleRenderer {
    Rc::new(move |handle: &ResizeHandleContext, _, _| {
        let color = if handle.state() == gpui_kit::base::ResizeHandleState::Idle {
            line
        } else {
            active
        };
        // The hairline fills the band's one-pixel content box.
        let bar = div().flex_none().bg(color);
        Some(
            match axis {
                Axis::Horizontal => bar.h_full().w(px(1.)),
                Axis::Vertical => bar.w_full().h(px(1.)),
            }
            .into_any_element(),
        )
    })
}

/// The element id of the editor Panel's empty state.
pub(crate) const EMPTY_STATE_ID: &str = "empty-state";

/// The window's root view. See the module docs.
pub(crate) struct Workspace {
    pub(crate) layout: PanelLayout<DrawerView>,
    pub(crate) editor: Entity<EditorView>,
    theme: Theme,
    rail_focus: FocusHandle,
    /// Each drawer's own scope (left, right): never focused itself, it holds
    /// its slots' focus.
    drawer_focus: [FocusHandle; 2],
    /// Each drawer view's slot handle, the keyboard target of a slot whose
    /// view has no focusable content of its own (all but FILES).
    view_focus: Vec<(DrawerView, FocusHandle)>,
    /// The rail cell the keyboard is on (an index into [`RAIL_VIEWS`]).
    rail_cursor: usize,
    /// A divider drag is in progress (persisted when it ends).
    resizing: bool,
    /// Where the divider press landed (px along the drag's axis) and the
    /// size it was resizing then (the drawn width, or the split ratio): a
    /// drag moves the edge by the pointer's travel, without jumping to it.
    press: (f64, f64),
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
    /// Resolves the workspace's chords before a drawer's own key bindings
    /// can take them ([`Self::intercept_shell_key`]).
    _shell_keys: Subscription,
    /// What the window's one modal slot holds, while the kit says a dialog
    /// is open ([`modal`]).
    pub(crate) modal: Option<ModalKind>,
    /// The command palette's view while it is open (tests read its rows).
    pub(crate) palette: Option<WeakEntity<Palette>>,
    /// The settings the window started on (the leader tree): `None` before
    /// [`Self::start`] and in the tests that never call it.
    settings: Option<SharedSettings>,
    /// The editor's requests that need the window (vim `:`).
    _editor_events: Subscription,
}

impl Workspace {
    pub(crate) fn new(
        editor: Entity<EditorView>,
        theme: Theme,
        layout: PanelLayout<DrawerView>,
        store: LayoutStore,
        window: &mut Window,
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
        let this = cx.entity().downgrade();
        let shell_keys = cx.intercept_keystrokes(move |event, window, cx| {
            this.update(cx, |workspace, cx| {
                workspace.intercept_shell_key(event, window, cx)
            })
            .ok();
        });
        let editor_events = cx.subscribe_in(&editor, window, Self::on_editor_event);
        Self {
            layout,
            editor,
            theme,
            rail_focus: cx.focus_handle(),
            drawer_focus: [cx.focus_handle(), cx.focus_handle()],
            view_focus: DrawerView::ALL
                .into_iter()
                .map(|view| (view, cx.focus_handle()))
                .collect(),
            rail_cursor,
            resizing: false,
            press: (0.0, 0.0),
            store,
            empty: None,
            status: Status::default(),
            notes: None,
            tx: None,
            _messages: None,
            flash_expiry: None,
            closing: Closing::Open,
            // Captured now: an app-channel message (no `Window` of its own)
            // may need it to open or close a modal ([`Self::with_window`]).
            window: Some(window.window_handle()),
            restore_pending: false,
            first_open_pending: false,
            files: None,
            _files_events: None,
            following: None,
            opener: follow::system_opener(),
            _editor_watch: editor_watch,
            _on_quit: None,
            _shell_keys: shell_keys,
            modal: None,
            palette: None,
            settings: None,
            _editor_events: editor_events,
        }
    }

    /// The editor asked for something only the window has.
    fn on_editor_event(
        &mut self,
        _: &Entity<EditorView>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::OpenPalette => self.open_palette(window, cx),
        }
    }

    /// The configured leader tree (`[leader]` overrides applied), the one
    /// the palette lists; the default tree before [`Self::start`].
    pub(crate) fn leader_tree(&self) -> LeaderNode {
        match self.settings.as_ref() {
            Some(settings) => settings
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .leader_tree(),
            None => leader::leader_tree(),
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
        self.settings = Some(settings.clone());
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
                self.file_op_closes_modals(&op, cx);
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
            // A leader leaf asked for from outside a key handler (the TUI's
            // palette and vim `:` send these): its route runs with the
            // window. Never under a modal — a modal closes before it runs
            // what it chose — but never dropped silently either.
            AppMessage::ExecuteLeaderAction(action) => {
                self.with_window(cx, move |workspace, window, cx| {
                    if modal::dialog_open(window, cx) {
                        tracing::warn!("ExecuteLeaderAction({action:?}) dropped: a modal is open");
                        return;
                    }
                    let route = workspace.route_of(action);
                    workspace.perform_route(route, window, cx);
                });
            }
            // Results addressed to an open modal: none of the GUI's modals
            // takes data yet, so every one is stale by definition.
            AppMessage::OverlayData(_) => {}
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

    /// Whether `view` is in a slot of a drawer that is shown: its rail cell
    /// reads as selected.
    fn is_shown(&self, view: DrawerView) -> bool {
        self.layout
            .view_location(&view)
            .is_some_and(|(side, _)| self.layout.is_visible(Panel::Drawer(side)))
    }

    /// The view in `side`'s slot `index`.
    fn slot_view(&self, side: Side, index: usize) -> Option<DrawerView> {
        self.layout.slot_views(side).get(index).copied()
    }

    /// The first view the rail offers that is shown nowhere (placed
    /// nowhere, or hidden in a drawer): what fills a drawer's free slot when
    /// the drawer has no hidden view of its own.
    fn first_unshown(&self) -> Option<DrawerView> {
        RAIL_VIEWS
            .into_iter()
            .find(|v| self.layout.view_location(v).is_none())
    }

    /// What a split or an empty drawer being shown fills its slot with: the
    /// view the drawer's last unsplit merged away (this session only), else
    /// the drawer's next hidden view, else the first view shown nowhere.
    fn next_for(&self, side: Side) -> Option<DrawerView> {
        self.layout
            .next_hidden(side)
            .or_else(|| self.first_unshown())
    }

    /// `view`'s slot handle.
    fn slot_focus(&self, view: DrawerView) -> &FocusHandle {
        self.view_focus
            .iter()
            .find(|(v, _)| *v == view)
            .map(|(_, handle)| handle)
            .expect("every drawer view has a slot handle")
    }

    /// Where the keyboard goes in `view`'s slot: FILES' filter, so typing
    /// filters; the slot itself for the views without content yet.
    fn view_handle(&self, view: DrawerView, cx: &App) -> FocusHandle {
        match (view, self.files.as_ref()) {
            (DrawerView::Files, Some(files)) => files.read(cx).focus_handle(cx),
            _ => self.slot_focus(view).clone(),
        }
    }

    fn handle_for(&self, panel: Panel, cx: &App) -> FocusHandle {
        match panel {
            Panel::Rail => self.rail_focus.clone(),
            // The drawer's focused slot.
            Panel::Drawer(side) => match self
                .layout
                .focused_slot(side)
                .and_then(|index| self.slot_view(side, index))
            {
                Some(view) => self.view_handle(view, cx),
                None => self.drawer_focus[side_ix(side)].clone(),
            },
            Panel::Editor => self.editor.read(cx).focus_handle(cx),
        }
    }

    /// Give the keyboard to the region the layout says is focused. Not
    /// while a modal is open: it has the keyboard, and gives it back to its
    /// opener when it closes ([`modal`]).
    pub(crate) fn apply_focus(&self, window: &mut Window, cx: &mut App) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let handle = self.handle_for(self.layout.focused(), cx);
        window.focus(&handle, cx);
    }

    /// Focus `panel` (a press anywhere in it).
    fn focus_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.layout.focus(panel);
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Focus `region` (a press anywhere in a drawer slot).
    fn focus_region(&mut self, region: Region, window: &mut Window, cx: &mut Context<Self>) {
        self.layout.focus_region(region);
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Show `side`'s drawer (and focus it, as the TUI does), or hide it and
    /// give its width to the editor. The drawer comes back with the views it
    /// showed; one showing nothing (its views moved away or hidden) comes
    /// back with its next hidden view, else the first view shown nowhere.
    /// Persisted.
    ///
    /// Hiding the focused drawer gives the keyboard back to the document,
    /// not to the rail the model falls back to (the TUI keeps that).
    fn toggle_drawer(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let drawer = Panel::Drawer(side);
        if self.layout.is_visible(drawer) {
            let had_focus = self.layout.focused() == drawer;
            self.layout.hide(drawer);
            if had_focus {
                self.layout.focus(Panel::Editor);
            }
        } else {
            if self.layout.slot_views(side).is_empty()
                && let Some(view) = self.next_for(side)
            {
                self.layout.move_view(view, side);
            }
            self.layout.show(drawer);
            self.layout.focus(drawer);
            self.ensure_files_listing(cx);
        }
        self.apply_focus(window, cx);
        self.store.save(&self.layout);
        cx.notify();
    }

    /// A rail cell was activated. A shown view is hidden: in a split drawer
    /// its slot closes and the other view fills the drawer and takes the
    /// keyboard; as a drawer's only view, the drawer hides (the TUI's rail).
    /// Either way the view stays placed (side and slot position). Any other
    /// view is shown where it is placed (a view placed nowhere: in the left
    /// drawer's focused slot), hiding the view sharing its slot position,
    /// and takes the keyboard. Persisted.
    fn activate_rail_view(
        &mut self,
        view: DrawerView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(i) = RAIL_VIEWS.iter().position(|v| *v == view) {
            self.rail_cursor = i;
        }
        match self.layout.view_location(&view) {
            Some(_) if self.is_shown(view) => {
                if let Some(side) = self.layout.hide_view(&view)
                    && self.layout.is_visible(Panel::Drawer(side))
                    && let Some(index) = self.layout.focused_slot(side)
                {
                    self.layout.focus_region(Region::Slot { side, index });
                }
            }
            _ => {
                let (side, index) = self.layout.open_view(view);
                self.layout.focus_region(Region::Slot { side, index });
                self.ensure_files_listing(cx);
            }
        }
        self.apply_focus(window, cx);
        self.store.save(&self.layout);
        cx.notify();
    }

    /// The view a move acts on: the focused slot's, or the one under the
    /// rail's cursor (placed or not). `None` from the editor.
    fn arranged_view(&self) -> Option<DrawerView> {
        match self.layout.focused_region() {
            Region::Slot { side, index } => self.slot_view(side, index),
            Region::Rail => RAIL_VIEWS.get(self.rail_cursor).copied(),
            Region::Group(_) => None,
        }
    }

    /// `MoveViewTo…Drawer`: move the focused view (or the rail cursor's) to
    /// the drawer on `to` and show it; the keyboard follows a focused view.
    /// It fits the target: a split one keeps the view's slot position, an
    /// unsplit or empty one takes it into its single slot, replacing the
    /// view shown there — never a new split. The model moves, it never
    /// duplicates. Persisted.
    fn move_view(&mut self, to: Side, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.arranged_view() else {
            return false;
        };
        self.layout.move_view(view, to);
        self.ensure_files_listing(cx);
        self.apply_focus(window, cx);
        self.store.save(&self.layout);
        true
    }

    /// `MoveViewUp` / `MoveViewDown`: in a split drawer, put the focused
    /// view (or the rail cursor's) at the top (`up`) or bottom slot
    /// position, trading places with the view shown there; a focused view
    /// keeps the keyboard. Already there: handled, nothing moves. In an
    /// unsplit drawer, and for a view placed nowhere, nothing changes and
    /// the key is not handled, so it passes on. Persisted.
    fn move_view_vertically(
        &mut self,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((view, (side, _))) = self
            .arranged_view()
            .and_then(|view| self.layout.placement(&view).map(|p| (view, p)))
        else {
            return false;
        };
        if self.layout.set_view_slot(&view, if up { 0 } else { 1 }) {
            self.apply_focus(window, cx);
            self.store.save(&self.layout);
            return true;
        }
        self.layout.slot_views(side).len() > 1
    }

    /// `SplitDrawerSlot`: show a view in the focused drawer's free slot
    /// position — the view its last unsplit merged away, else its next
    /// hidden view, else the first view shown nowhere ([`Self::next_for`])
    /// — and focus it (a rail click then puts another view there). Unsplit
    /// then split returns to the same split, at the drawer's ratio. A full
    /// drawer says so. Persisted.
    fn split_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Panel::Drawer(side) = self.layout.focused() else {
            return false;
        };
        if self.layout.slot_views(side).len() >= MAX_DRAWER_SLOTS {
            self.flash("This drawer already holds two views", cx);
            return true;
        }
        let Some(view) = self.next_for(side) else {
            return true;
        };
        match self.layout.split_slot(side, view) {
            Ok(index) => {
                self.layout.focus_region(Region::Slot { side, index });
                self.ensure_files_listing(cx);
                self.apply_focus(window, cx);
                self.store.save(&self.layout);
            }
            Err(LayoutError::SlotCap) => self.flash("This drawer already holds two views", cx),
            Err(_) => {}
        }
        true
    }

    /// `UnsplitDrawerSlot`: close the focused slot, merging every view
    /// placed there into the other slot (the closed view hidden), so the
    /// drawer stays unsplit with the other view focused, and showing any of
    /// its views later replaces the shown one. A drawer's only slot stays
    /// (the toggle hides a drawer). Persisted.
    fn unsplit_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Region::Slot { side, index } = self.layout.focused_region() else {
            return false;
        };
        if self.layout.slot_views(side).len() < 2 {
            self.flash("This drawer holds one view", cx);
            return true;
        }
        if self.layout.close_slot(side, index).is_ok() {
            self.apply_focus(window, cx);
            self.store.save(&self.layout);
        }
        true
    }

    /// The model's sizing floors for this window.
    fn limits(window: &Window) -> WidthLimits {
        shell_layout::limits(
            f64::from(window.viewport_size().width),
            f64::from(window.rem_size()),
        )
    }

    /// A press on `side`'s width divider at `x`: remember it with the width
    /// drawn then.
    fn grab_divider(&mut self, side: Side, x: f64, window: &Window) {
        self.press = (
            x,
            shell_layout::shown_width(&self.layout, side, Self::limits(window)),
        );
    }

    fn on_drag_move(
        &mut self,
        event: &DragMoveEvent<DrawerResize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DrawerResize(side) = *event.drag(cx);
        let (from, width) = self.press;
        let travel = f64::from(event.event.position.x) - from;
        // The left drawer's edge is its trailing one, the right's its
        // leading one: dragging toward the editor widens either.
        let width = match side {
            Side::Left => width + travel,
            Side::Right => width - travel,
        };
        self.layout
            .set_drawer_width(side, width, Self::limits(window));
        self.resizing = true;
        cx.notify();
    }

    /// A press on `side`'s split divider at `y`: remember it with the ratio
    /// then.
    fn grab_split(&mut self, side: Side, y: f64) {
        self.press = (y, self.layout.split_ratio(side).unwrap_or(0.5));
    }

    /// A split-divider drag over `side`'s drawer (`event.bounds` is the
    /// drawer): the first slot's share follows the pointer, clamped by the
    /// model so both slots keep a usable share.
    fn on_split_drag(
        &mut self,
        side: Side,
        event: &DragMoveEvent<SlotResize>,
        cx: &mut Context<Self>,
    ) {
        let SlotResize(dragged) = *event.drag(cx);
        let height = f64::from(event.bounds.size.height);
        if dragged != side || height <= 0.0 {
            return;
        }
        let (from, ratio) = self.press;
        let travel = f64::from(event.event.position.y) - from;
        self.layout.set_split_ratio(side, ratio + travel / height);
        self.resizing = true;
        cx.notify();
    }

    /// The pointer came up (in the window or outside it): a divider drag in
    /// progress ends and is persisted. Idempotent.
    fn end_resize(&mut self, cx: &mut Context<Self>) {
        self.press = (0.0, 0.0);
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

    /// A key while the rail or a drawer holds GPUI focus, seen before any
    /// key binding: a chord the keymap gives the workspace there runs now.
    /// GPUI dispatches bindings before key-down listeners, so without this
    /// a focused element's own binding (the FILES filter's Alt+Shift+Left is
    /// the kit input's select-word-left) would swallow the chord before
    /// [`Self::on_key_down`] sees it. With any other region focused app
    /// chords win (ADR-0050 §6). A key the workspace does not handle goes on
    /// as usual; text the platform composed (AltGr) is left alone.
    fn intercept_shell_key(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.input_preference == InputPreference::CharacterInput {
            return;
        }
        // A modal owns every key but its own chord (the palette's toggle).
        if modal::dialog_open(window, cx) {
            if self.intercept_modal_key(event, window, cx) {
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        let in_shell = self.rail_focus.contains_focused(window, cx)
            || self
                .drawer_focus
                .iter()
                .any(|handle| handle.contains_focused(window, cx));
        if !in_shell || self.layout.focused() == Panel::Editor {
            return;
        }
        let Some(key) = input::key_input(&event.keystroke) else {
            return;
        };
        let Some(action) = keymap::active(cx).resolve(key, self.key_ctx(cx)).cloned() else {
            return;
        };
        if self.workspace_command(&action, Some(key), window, cx) {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = input::key_down_input(event) else {
            return;
        };
        let action = keymap::active(cx).resolve(key, self.key_ctx(cx)).cloned();
        let handled = match action {
            Some(action) => self.workspace_command(&action, Some(key), window, cx),
            None => self.layout.focused() == Panel::Rail && self.rail_key(key, window, cx),
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// Act on a keymap command that is the workspace's. False for one that
    /// is not (find, replace: the editor already had its chance), and for a
    /// focus move with nothing to move to, which is the focused panel's key.
    /// `key` is the chord pressed; `None` when the command palette runs the
    /// command (nothing to hand back to the editor then).
    fn workspace_command(
        &mut self,
        action: &ActionShortcuts,
        key: Option<KeyInput>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match action {
            ActionShortcuts::OpenCommandPalette => self.toggle_palette(window, cx),
            ActionShortcuts::Quit => self.request_close(window, cx),
            ActionShortcuts::ToggleSidebar => self.toggle_drawer(Side::Left, window, cx),
            ActionShortcuts::ToggleRightDrawer => self.toggle_drawer(Side::Right, window, cx),
            ActionShortcuts::MoveViewToLeftDrawer => return self.move_view(Side::Left, window, cx),
            ActionShortcuts::MoveViewToRightDrawer => {
                return self.move_view(Side::Right, window, cx);
            }
            ActionShortcuts::MoveViewUp => return self.move_view_vertically(true, window, cx),
            ActionShortcuts::MoveViewDown => {
                return self.move_view_vertically(false, window, cx);
            }
            ActionShortcuts::SplitDrawerSlot => return self.split_drawer(window, cx),
            ActionShortcuts::UnsplitDrawerSlot => return self.unsplit_drawer(window, cx),
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
                // panel's own key, not a focus move (ADR-0050 §4). The
                // editor already let it pass, so it is handed back; anywhere
                // else it goes on unswallowed.
                FocusMove::NotHandled => {
                    let Some(key) = key else {
                        return true;
                    };
                    if self.layout.focused() != Panel::Editor {
                        return false;
                    }
                    self.editor
                        .update(cx, |editor, cx| editor.editor_key(key, window, cx));
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

    /// The focused region's marker: a thin bar in the focus colour across
    /// its top edge, laid over it so focusing never shifts content. Subtle,
    /// but always there (design guides: state must be visible).
    fn focus_bar(&self, region: Region) -> Option<impl IntoElement> {
        (self.layout.focused_region() == region).then(|| {
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
        let (top, pinned) = RAIL_VIEWS.split_at(RAIL_VIEWS.len() - 1);
        let cell = |i: usize, view: DrawerView, cx: &mut Context<Self>| {
            self.render_rail_cell(
                view,
                self.is_shown(view),
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
            .children(self.focus_bar(Region::Rail))
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

    /// `side`'s drawer, `width` px wide: its slots stacked top to bottom
    /// (the first at the persisted share when there are two), and the width
    /// divider on the edge it shares with the editor.
    fn render_drawer(&self, side: Side, width: f64, cx: &mut Context<Self>) -> impl IntoElement {
        let views = self.layout.slot_views(side);
        let split = (views.len() > 1)
            .then(|| self.layout.split_ratio(side))
            .flatten();
        let label = views
            .iter()
            .map(|v| shell_layout::view_name(*v))
            .collect::<Vec<_>>()
            .join(", ");
        let slots: Vec<AnyElement> = views
            .iter()
            .enumerate()
            .map(|(index, view)| {
                let share = if index == 0 { split } else { None };
                self.render_slot(side, index, *view, share, cx)
            })
            .collect();
        let edge = match side {
            Side::Left => HandleEdge::Trailing,
            Side::Right => HandleEdge::Leading,
        };
        div()
            .id(drawer_id(side))
            .test_support()
            .track_focus(&self.drawer_focus[side_ix(side)])
            .role(Role::Group)
            .aria_label(SharedString::from(label))
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .h_full()
            // Measured runtime geometry: the user's dragged width, in pixels.
            .w(px(width as f32))
            .bg(theme::panel_background(&self.theme))
            // The split divider's drag, measured against this drawer.
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<SlotResize>, _, cx| {
                    this.on_split_drag(side, event, cx)
                }),
            )
            .children(slots)
            .child(
                resize_handle::<DrawerResize, DragGhost>(divider_id(side), Axis::Horizontal)
                    .inside(edge)
                    .with_appearance(hairline(
                        Axis::Horizontal,
                        theme::divider(&self.theme),
                        theme::focus_border(&self.theme),
                    ))
                    .on_drag(DrawerResize(side), |_, _, _, cx| cx.new(|_| DragGhost))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, _| {
                            this.grab_divider(side, f64::from(event.position.x), window)
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.end_resize(cx)),
                    ),
            )
    }

    /// Slot `index` of `side`'s drawer, showing `view`: its title over the
    /// view (FILES, or the placeholder of a view not built yet). `share` is
    /// the first slot's part of a split drawer's height; that slot also
    /// carries the divider between the two.
    fn render_slot(
        &self,
        side: Side,
        index: usize,
        view: DrawerView,
        share: Option<f64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let region = Region::Slot { side, index };
        div()
            .id(slot_id(view))
            .test_support()
            .track_focus(self.slot_focus(view))
            .role(Role::Group)
            .aria_label(shell_layout::view_name(view))
            .relative()
            .flex()
            .flex_col()
            .min_h_0()
            .map(|slot| match share {
                Some(share) => slot.flex_none().h(relative(share as f32)),
                None => slot.flex_1(),
            })
            .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                this.focus_region(region, window, cx)
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
            .map(|slot| match (view, self.files.as_ref()) {
                (DrawerView::Files, Some(files)) => slot.child(files.clone()),
                _ => slot.child(
                    div()
                        .px_3()
                        .text_sm()
                        .text_color(theme::muted(&self.theme))
                        .child("Coming soon"),
                ),
            })
            .children(self.focus_bar(region))
            .when(share.is_some(), |slot| {
                slot.child(
                    resize_handle::<SlotResize, DragGhost>(split_divider_id(side), Axis::Vertical)
                        .inside(HandleEdge::Trailing)
                        .with_appearance(hairline(
                            Axis::Vertical,
                            theme::divider(&self.theme),
                            theme::focus_border(&self.theme),
                        ))
                        .on_drag(SlotResize(side), |_, _, _, cx| cx.new(|_| DragGhost))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, _| {
                                this.grab_split(side, f64::from(event.position.y))
                            }),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.end_resize(cx)),
                        ),
                )
            })
            .into_any_element()
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
            .children(self.focus_bar(Region::Group(self.layout.active_group())))
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
        let drawer = |side: Side| {
            (self.layout.is_visible(Panel::Drawer(side))
                && self.layout.focused_slot(side).is_some())
            .then(|| shell_layout::shown_width(&self.layout, side, limits))
        };
        let (left, right) = (drawer(Side::Left), drawer(Side::Right));
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
            .when_some(left, |row, width| {
                row.child(self.render_drawer(Side::Left, width, cx))
            })
            .child(self.render_editor(cx))
            .when_some(right, |row, width| {
                row.child(self.render_drawer(Side::Right, width, cx))
            })
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
mod modal;
mod note;
mod palette;
mod route;

#[cfg(test)]
mod drawer_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod follow_tests;
#[cfg(test)]
mod footer_tests;
#[cfg(test)]
mod note_tests;
#[cfg(test)]
mod palette_tests;
#[cfg(test)]
mod start_tests;
#[cfg(test)]
mod tests;
