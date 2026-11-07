//! The **Workspace**: the window's root view, composing the editor screen's
//! Panels left to right — the **Activity rail**, the left **Drawer**, the
//! editor Panel (the [`EditorView`], unchanged inside) and the right
//! **Drawer**. ADR-0050.
//!
//! **State.** Which panels are shown, the views in each drawer's **drawer
//! slots** (up to two, stacked), the drawers' widths and split ratios, and
//! which region has the keyboard live in the logic [`PanelLayout`]; this
//! view only renders it and keeps GPUI focus in step with it. A view
//! without a GUI yet (CFG) shows a placeholder in its slot.
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
//! the main thread. The vault can change at runtime (M8b, [`reopen`]):
//! each vault entry gets a channel of its own.
//!
//! **The open note (M5b-2 G2).** Opening, saving and the flush before the
//! window closes are the note session's, hosted in [`note`]; the Footer
//! shows the note's path and whether it is saved.
//!
//! **FILES (M5b-2 G3).** With a vault, the left drawer's FILES view is the
//! [`files::FilesPane`]; the workspace keeps it in step with the app channel
//! and does what its rows ask (open a note, show an attachment).
//!
//! **Links (M5b-2 G4).** The `FollowLink` chord and the editor's
//! double-click follow the link under the caret ([`follow`]).
//!
//! **File operations (M6 G3).** Rename, move and delete, from the note menu
//! (F2, the editor's right-click without a selection), a FILES row's context
//! menu, or the leader and the palette ([`file_ops`], dialogs in
//! [`file_dialogs`]). An operation on the open note (or a folder holding it)
//! flushes it first and is refused when that fails; afterwards the note is
//! followed to its new place or, deleted or moved away, discarded unsaved.
//!
//! **LINKS and OUTLINE (M7a G1).** Two note-scoped views over one list
//! component ([`drawer_list`]): the open note's backlinks, outgoing links
//! and unlinked mentions ([`links`]), and its headings ([`outline`]). They
//! work only while shown: a reveal or another open note refreshes them
//! ([`Workspace::refresh_views`]), and OUTLINE follows each save.
//!
//! **Properties (M7a G2).** The open note's frontmatter properties in one
//! dialog ([`properties`]): listed, added, edited, renamed and deleted
//! through core. The note is flushed before the dialog opens (refused when
//! that fails), a write holds the note like a file operation does, and the
//! reload after a write is the window's (`NoteReloadFromDisk`), so it
//! happens with the dialog gone too.
//!
//! **Attachments (M7a G3).** An attachment opened from FILES or a followed
//! link shows in the editor Panel in place of the editor, read-only
//! ([`attachment`]): the open note is flushed and let go first (refused when
//! the flush fails), so while it shows there is no open note to save.
//!
//! **Finding and making notes (M6 G4).** The note finder (`OpenNote`, leader
//! `f f`) and the link chooser are one picker ([`picker`]); the create dialog
//! (a followed link or an open that names no note), the quick note and
//! today's journal are in [`create`].
//!
//! **Searching (M7b G4).** The search browser (`SearchNotes`, leader `f g` /
//! `f r`, a followed `#tag`) is that picker over the vault's query search,
//! with a preview of the selected note ([`search`]); a note opened from it
//! shows the query's matches until the first edit.
//!
//! **FIND and TAGS (M7b G5).** Two more drawer views over the same list
//! component: FIND runs a query over the vault with a preview of the
//! selected result ([`find`]); TAGS lists the workspace's labels and runs
//! one in FIND ([`tags`]). Like LINKS they work only while shown.
//!
//! **Saved searches (M7b G6).** The Saved Searches (`OpenSavedSearches`,
//! leader `f s`) list the workspace's saved searches in the picker and run
//! one in FIND, named after it; the save-search dialog (`SaveCurrentQuery`
//! in FIND or the search browser, FIND's `Save…`) names FIND's or the
//! browser's query ([`saved_searches`]). Core keeps them, off the UI
//! thread.
//!
//! **Sorting (M7b G7).** The sort dialog (`OpenSortDialog` from the rail or
//! a drawer, inside the search browser, the palette, each list's sort chip)
//! re-sorts FILES in place, or rewrites FIND's or the browser's query order
//! ([`sort`]). Changes apply live; nothing is saved.
//!
//! **The kimün server and SEM (M7c G9).** With a server configured, the
//! window runs the TUI's background sync and follows the status it reports
//! ([`server`]): the rail offers SEM while the server can search, and the
//! Footer says how the server is. SEM is the semantic search view
//! ([`semantic`]).
//!
//! **Ask (M7c G10a).** The Thread, the Ask workspace's conversation, takes
//! the editor Panel in place of the editor (leader `a a` / F6, `+ask`),
//! leaving the open note as the attachment view does; a note opening or an
//! attachment shown takes it down, its conversation kept ([`ask`]). It
//! shows with the drawer view ASK (M7c G10b), the selected turn's Sources
//! and the Source reader ([`ask_sources`]); the rail offers ASK while the
//! server can answer, and ASK is never persisted.
//!
//! **The setup assistant (M8c).** With no workspace configured, and from
//! the leader / palette (`guided setup`), the window shows the setup
//! assistant in place of its Panels ([`onboarding`]): the rail, the drawers
//! and the editor are not drawn and the workspace's chords do nothing until
//! it is finished or left. Its theme choice restyles the window live.
//!
//! **Preferences (M9b).** `OpenPreferences` (F4, Ctrl+,), leader `v p` and
//! the palette show Preferences in place of the Panels ([`preferences`]),
//! the same takeover as the setup assistant. A saved draft applies at once;
//! only a new server address or editor engine enters the vault again.
//! Workspaces (M9b-2) are managed there, each change written as it is made;
//! a vault that opens with names differing only by case (`VaultConflict`)
//! stays read-only and brings Preferences up on Workspaces with the vault's
//! message, so another workspace or folder can be picked.
//!
//! **CFG and the theme picker (M9c).** The CFG drawer view summarises the
//! settings that shape the window (theme, leader and Preferences chords,
//! which-key delay, config file) and launches the theme picker and
//! Preferences ([`config`]); it needs no vault. The theme picker (leader
//! `v t`, the palette, CFG's `Theme…`) restyles the window live as its
//! selection moves, keeps a pick by writing only the theme, and puts the
//! opening theme back on every other way out ([`theme_picker`]).
//!
//! **Pinned notes (M7b G8).** The pinned-notes dialog (leader `f p`, the
//! palette) lists the vault's pins by number: a digit opens one, and pins
//! are raised, lowered and unpinned in place, each change written through
//! core as it is made ([`pinned`]).
//!
//! **Workspace switcher (M8d).** F5 (`SwitchWorkspace`), leader `v s` and
//! the palette list the configured workspaces, the current one marked; a
//! pick moves the window onto another through the vault change
//! ([`switcher`], [`reopen`]). It opens in every window state but under the
//! setup assistant; with no workspace it offers the assistant. While the
//! editor Panel shows an empty state, that state holds the editor's focus
//! handle, so the window's chords still reach it.
//!
//! **The title bar (M10a).** On Linux and Windows the window draws its own
//! title bar in the theme's colours (`[gui] window_decorations = "system"`
//! gives the system's back, read at startup); on macOS a themed bar sits
//! under the native traffic lights. It tops every view, takeovers included,
//! names the workspace, drags, maximizes and closes the window, closing
//! through the close guard; F11 toggles fullscreen ([`titlebar`]).

use std::collections::{HashMap, VecDeque};
#[cfg(test)]
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::ResizeHandleRenderer;
use gpui_kit::base::{HandleEdge, ResizeHandleContext, TestSupportExt, resize_handle};
use gpui_kit::component::button::Button;
use gpui_kit::component::empty::{
    Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::{Icon, WindowExt as _, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, AnyWindowHandle, App, AppContext, Axis, ClickEvent, ClipboardItem, Context,
    DragMoveEvent, Entity, FocusHandle, Focusable, InputPreference, InteractiveElement,
    IntoElement, KeyDownEvent, KeystrokeEvent, MouseButton, MouseDownEvent, ParentElement, Render,
    Rgba, Role, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity,
    Window, div, px, relative, rems,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::ask_flow::AskFlow;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout::{
    FocusMove, LayoutError, Panel, PanelLayout, Region, Side, WidthLimits,
};
use kimun_notes::logic::leader_route::AskStep;
use kimun_notes::logic::list::ListRow as _;
use kimun_notes::logic::message::{AppMessage, AppTx, FileOp};
use kimun_notes::logic::nav::{DrawerView, LinksTab};
use kimun_notes::logic::rag_caps::Caps;
use kimun_notes::logic::session::{NoteSession, SaveCompleted};
use kimun_notes::rag::RagStatus;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, SharedSettings};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::editor_view::{EditorEvent, EditorView};
use crate::input;
use crate::keymap::{self, KeyCtx};
use crate::shell_layout::{self, RAIL_REMS, RAIL_SIDE};
use crate::startup::{self, EmptyState, FirstNote, Start, VaultStart};
use crate::status_row::{self, DocFacts, Status};
use crate::theme;
use kimun_notes::logic::task_slot::TaskSlot;

use files::FilesPane;
#[cfg(test)]
pub(crate) use files::ICONS as FILES_ICONS;
use leader::LeaderState;
use modal::ModalKind;
pub(crate) use note::Closing;
use note::NoteHost;
pub(crate) use palette::Palette;
#[cfg(test)]
pub(crate) use titlebar::WINDOW_CONTROL_ICONS;

/// The key context of the workspace root.
const KEY_CONTEXT: &str = "KimunWorkspace";

/// Where the layout, and the settings the window changes, are persisted.
#[derive(Clone)]
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

    /// Write `settings` to the store as the TUI saves its preferences, the
    /// `[gui]` table on disk kept (`AppSettings::save_to_disk`); a config
    /// that exists but does not parse is left alone and reported. Blocking:
    /// run it off the UI thread ([`Workspace::persist_settings`]).
    fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        let result = match self {
            #[cfg(test)]
            LayoutStore::Off => return Ok(()),
            LayoutStore::UserConfig => settings.save_to_disk_if_parsable(),
            #[cfg(test)]
            LayoutStore::File(path) => {
                let mut settings = settings.clone();
                settings.config_file = Some(path.clone());
                settings.save_to_disk_if_parsable()
            }
        };
        result.map_err(|e| e.to_string())
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

/// The element id of `side`'s placeholder: its empty slot, or the whole
/// drawer while it shows no view (`drawer-left-empty`).
pub(crate) fn empty_slot_id(side: Side) -> &'static str {
    match side {
        Side::Left => "drawer-left-empty",
        Side::Right => "drawer-right-empty",
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
    rail_focus: FocusHandle,
    /// Each drawer's own scope (left, right): never focused itself, it holds
    /// its slots' focus.
    drawer_focus: [FocusHandle; 2],
    /// Each drawer view's slot handle, the keyboard target of a slot whose
    /// view has no focusable content of its own (all but FILES).
    view_focus: Vec<(DrawerView, FocusHandle)>,
    /// The rail cell the keyboard is on (an index into
    /// [`Self::rail_views`]).
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
    /// The take-down timer of each toast with an action, by its key: a toast
    /// shown again replaces its timer (the old one is dropped, so it cannot
    /// take the new toast down early).
    action_timers: HashMap<SharedString, Task<()>>,
    /// Every flash shown, in order, with whether it was an error: what the
    /// tests read (the kit's toasts keep their message and kind private).
    #[cfg(test)]
    pub(crate) flashes: Vec<(SharedString, bool)>,
    /// The keys (texts) of the flash toasts pushed, oldest first, at most
    /// [`MAX_FLASH_TOASTS`]: the ones that may still be up. A toast the kit
    /// hid on its own stays listed until it ages out of here; removing it
    /// again is a no-op.
    toasts: VecDeque<SharedString>,
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
    /// The LINKS and OUTLINE views, when the window has a vault, and the
    /// subscriptions to their lists.
    pub(crate) links: Option<Entity<links::LinksView>>,
    pub(crate) outline: Option<Entity<outline::OutlineView>>,
    /// The FIND and TAGS views, when the window has a vault.
    pub(crate) find: Option<Entity<find::FindView>>,
    pub(crate) tags: Option<Entity<tags::TagsView>>,
    /// The SEM view, when the window has a vault ([`semantic`]).
    pub(crate) semantic: Option<Entity<semantic::SemanticView>>,
    _views_events: Vec<Subscription>,
    /// What the kimün server last reported (`Disabled`: none configured,
    /// or no report yet), and what it can do then ([`server`]).
    rag_status: RagStatus,
    rag_caps: Caps,
    /// The server sync, while one runs: at most one.
    rag_sync: TaskSlot,
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
    /// The shared settings (the leader tree, the vault to open, what
    /// [`reopen`] persists): `None` before [`Self::start`] and in the tests
    /// that never call it.
    settings: Option<SharedSettings>,
    /// The editor's requests that need the window (vim `:`).
    _editor_events: Subscription,
    /// The pending leader sequence and its which-key popup ([`leader`]).
    pub(crate) leader: LeaderState,
    /// The active keymap's chords as the window shows them, rebuilt when
    /// the keymap is replaced, never per frame.
    chord_labels: ChordLabels,
    _keymap_watch: Subscription,
    /// The file dialog in the modal slot, while one is open: tests read
    /// it (like `SaveSearchOpen.dialog`), nothing else does.
    #[cfg(test)]
    pub(crate) file_dialog: Option<file_ops::FileDialogView>,
    /// The progress of the file dialog's operation ([`file_ops`]).
    file_status: Option<Entity<file_dialogs::OpStatus>>,
    /// The file operation running (its flush, the core call, the reload of
    /// a followed note). One at a time.
    file_task: Option<Task<()>>,
    /// Why the editor Panel shows no note: the open one was deleted or
    /// moved away. Cleared when a note opens.
    pub(crate) note_gone: Option<EmptyState>,
    /// The note picker in the modal slot, while one is open ([`picker`]).
    pub(crate) picker: Option<WeakEntity<picker::NotePicker>>,
    /// The quick note in the modal slot, while it is open ([`create`]).
    pub(crate) quick_note: Option<WeakEntity<create::QuickNoteDialog>>,
    /// The progress of the create dialog's or the quick note's note.
    note_status: Option<Entity<file_dialogs::OpStatus>>,
    /// A note being made from a modal (`Create:`, the create dialog, the
    /// quick note). One at a time; the modal waits for it.
    note_task: Option<Task<()>>,
    /// The properties dialog in the modal slot, while one is open
    /// ([`properties`]).
    pub(crate) properties: Option<WeakEntity<properties::PropertiesDialog>>,
    /// The search browser's query syntax is shown over it ([`search`]).
    pub(crate) query_help: bool,
    /// The Saved Searches in the modal slot, while open
    /// ([`saved_searches`]).
    pub(crate) saved_searches:
        Option<WeakEntity<picker::NotePicker<kimun_notes::logic::saved_search::SearchItem>>>,
    /// The save-search dialog in the modal slot, while open.
    pub(crate) save_search: Option<saved_searches::SaveSearchOpen>,
    /// The save running: the dialog waits for it.
    save_task: Option<Task<()>>,
    /// The sort dialog, while open ([`sort`]).
    pub(crate) sort: Option<sort::SortOpen>,
    /// The pinned-notes dialog, while open ([`pinned`]).
    pub(crate) pinned: Option<pinned::PinnedOpen>,
    /// The workspace switcher, while open ([`switcher`]).
    pub(crate) switcher: Option<WeakEntity<switcher::WorkspaceSwitcher>>,
    /// The write of the open note before a search result opens: the
    /// browser waits for it, and it holds the note like a file operation.
    search_flush: Option<Task<()>>,
    /// The flush before the properties dialog opens on the open note.
    properties_opening: Option<Task<()>>,
    /// The property write running, for its note: it holds the note until
    /// its result comes in on the app channel.
    property_write: Option<(VaultPath, Task<()>)>,
    /// The open note being reloaded from disk after a property write.
    note_reload: Option<Task<()>>,
    /// The attachment the editor Panel shows instead of the editor
    /// ([`attachment`]), and its events.
    pub(crate) attachment: Option<Entity<attachment::AttachmentView>>,
    _attachment_events: Option<Subscription>,
    /// The read of an attachment asked for (a newer request replaces it).
    attachment_loading: Option<Task<()>>,
    /// The flush of the open note before an attachment shows: it holds the
    /// note like a file operation does.
    attachment_flushing: Option<Task<()>>,
    /// An attachment asked for while that flush ran: it opens next.
    attachment_wanted: Option<VaultPath>,
    /// The Thread (Ask), when the window has a vault ([`ask`]), and its
    /// events. It lives as long as the window, shown or not.
    pub(crate) ask: Option<Entity<ask::AskView>>,
    _ask_events: Option<Subscription>,
    /// The Sources view (the drawer view ASK), with the Thread.
    pub(crate) ask_sources: Option<Entity<ask_sources::SourcesView>>,
    /// Whether the Thread holds the editor Panel: the one record of it.
    ask_flow: AskFlow,
    /// The flush of the open note before the Thread shows: it holds the
    /// note like a file operation does.
    ask_flushing: Option<Task<()>>,
    /// The Thread's client being built (a newer status replaces it).
    ask_client_task: Option<Task<()>>,
    /// The free path for a saved answer being found.
    ask_saving: Option<Task<()>>,
    /// The client the Thread gets whenever the status says the server can
    /// answer (tests: a stub transport), instead of the configured one.
    #[cfg(test)]
    pub(crate) ask_test_client: Option<Arc<kimun_notes::server_client::RagClient>>,
    /// The vault change under way ([`reopen`]): the open note's flush, then
    /// the new vault being opened. It holds the note like a file operation.
    reopening: Option<Task<()>>,
    /// Another vault change was asked for while one ran: the vault is
    /// opened again from the settings as they are then.
    reopen_again: bool,
    /// The workspace a switch asked for, made current (and persisted) only
    /// once the open note is flushed: a switch the flush refuses leaves the
    /// settings naming the workspace the window stays on.
    switch_to: Option<String>,
    /// The setup assistant, while it has the window ([`onboarding`]).
    pub(crate) onboarding: Option<Entity<onboarding::OnboardingView>>,
    _onboarding_events: Option<Subscription>,
    /// Preferences, while they have the window ([`preferences`]).
    pub(crate) preferences: Option<Entity<preferences::PreferencesView>>,
    _preferences_events: Option<Subscription>,
    /// The CFG drawer view ([`config`]): built with the window, it needs no
    /// vault.
    pub(crate) config: Entity<config::ConfigView>,
    _config_events: Subscription,
    /// The theme picker, while open ([`theme_picker`]).
    pub(crate) theme_picker: Option<theme_picker::ThemePickerOpen>,
    /// The window's title bar and its title ([`titlebar`]).
    pub(crate) titlebar: titlebar::TitleBar,
    /// The update flow: its checks, its dialog, an install ([`update`]).
    pub(crate) updates: update::UpdateHost,
}

/// The labels the window derives from the active keymap's chords.
struct ChordLabels {
    /// What an empty drawer and an empty slot say
    /// ([`shell_layout::empty_slot_hint`]).
    empty_slot_hint: SharedString,
    /// The `Leader` chord (`Ctrl+Shift+B`), or a stand-in when unbound.
    leader: SharedString,
    /// What the editor Panel says with no note open: how to find one.
    no_note_hint: String,
    /// The `FollowLink` chord: the attachment view's `Open externally`.
    follow_link: Option<KeyInput>,
}

impl ChordLabels {
    fn new(cx: &App) -> Self {
        let chord = |action: ActionShortcuts| keymap::chord_label(cx, &action);
        Self {
            empty_slot_hint: shell_layout::empty_slot_hint(
                chord(ActionShortcuts::MoveViewToRightDrawer).as_deref(),
                chord(ActionShortcuts::MoveViewToLeftDrawer).as_deref(),
            )
            .into(),
            leader: leader::gateway_label(chord(ActionShortcuts::Leader)),
            no_note_hint: match chord(ActionShortcuts::OpenNote) {
                Some(finder) => format!("Press {finder} to find or create one."),
                None => "Open one from FILES.".to_string(),
            },
            follow_link: keymap::active(cx).chord_for(&ActionShortcuts::FollowLink),
        }
    }
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
        // The one theme every view of the window draws in.
        theme::set_current(cx, theme);
        let rail_cursor = layout
            .slot_views(Side::Left)
            .first()
            .and_then(|v| {
                shell_layout::rail_views(Caps::default())
                    .iter()
                    .position(|r| r == v)
            })
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
        let keymap_watch = keymap::observe(cx, |this: &mut Self, cx| {
            this.chord_labels = ChordLabels::new(cx);
            this.config.update(cx, |_, cx| cx.notify());
            let chord = this.chord_labels.follow_link;
            if let Some(view) = this.attachment.as_ref() {
                view.update(cx, |view, cx| view.set_open_chord(chord, cx));
            }
            cx.notify();
        });
        let config = cx.new(config::ConfigView::new);
        let config_events = cx.subscribe_in(&config, window, Self::on_config_event);
        Self {
            layout,
            editor,
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
            action_timers: HashMap::new(),
            #[cfg(test)]
            flashes: Vec::new(),
            toasts: VecDeque::new(),
            closing: Closing::Open,
            // Captured now: an app-channel message (no `Window` of its own)
            // may need it to open or close a modal ([`Self::with_window`]).
            window: Some(window.window_handle()),
            restore_pending: false,
            first_open_pending: false,
            files: None,
            _files_events: None,
            links: None,
            outline: None,
            find: None,
            tags: None,
            semantic: None,
            _views_events: Vec::new(),
            rag_status: RagStatus::Disabled,
            rag_caps: Caps::default(),
            rag_sync: TaskSlot::default(),
            following: None,
            opener: follow::system_opener(),
            _editor_watch: editor_watch,
            _on_quit: None,
            _shell_keys: shell_keys,
            modal: None,
            palette: None,
            settings: None,
            _editor_events: editor_events,
            leader: LeaderState::default(),
            chord_labels: ChordLabels::new(cx),
            _keymap_watch: keymap_watch,
            #[cfg(test)]
            file_dialog: None,
            file_status: None,
            file_task: None,
            note_gone: None,
            picker: None,
            quick_note: None,
            note_status: None,
            note_task: None,
            properties: None,
            query_help: false,
            saved_searches: None,
            sort: None,
            pinned: None,
            switcher: None,
            save_search: None,
            save_task: None,
            search_flush: None,
            properties_opening: None,
            property_write: None,
            note_reload: None,
            attachment: None,
            _attachment_events: None,
            attachment_loading: None,
            attachment_flushing: None,
            attachment_wanted: None,
            ask: None,
            _ask_events: None,
            ask_sources: None,
            ask_flow: AskFlow::new(),
            ask_flushing: None,
            ask_client_task: None,
            ask_saving: None,
            #[cfg(test)]
            ask_test_client: None,
            reopening: None,
            reopen_again: false,
            switch_to: None,
            onboarding: None,
            _onboarding_events: None,
            preferences: None,
            _preferences_events: None,
            config,
            _config_events: config_events,
            theme_picker: None,
            titlebar: titlebar::TitleBar::new(),
            updates: update::UpdateHost::new(update::default_backend()),
        }
    }

    /// A CFG launcher: its leader leaf, run (or refused, with the reason)
    /// as the leader runs it.
    fn on_config_event(
        &mut self,
        _: &Entity<config::ConfigView>,
        event: &config::ConfigEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config::ConfigEvent::Launch(action) = *event;
        self.fire_leader(action, window, cx);
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
            // Right-click without a selection: the note menu, for the note
            // the editor shows, with its properties first.
            EditorEvent::ContextMenu => {
                if let Some(note) = self.notes.as_ref().and_then(|h| h.note.clone()) {
                    self.open_file_ops_menu(note, true, window, cx);
                }
            }
        }
    }

    /// Put the window on `start` (see [`startup`]): drain the window's app
    /// channel (`rx`; `tx` is its sender, the editor's too), then enter the
    /// vault ([`Self::enter`]). Must run with a tokio runtime entered.
    pub(crate) fn start(
        &mut self,
        start: Start,
        settings: SharedSettings,
        tx: AppTx,
        rx: UnboundedReceiver<AppMessage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.config
            .update(cx, |view, cx| view.set_settings(settings.clone(), cx));
        self.settings = Some(settings);
        self.set_channel(tx, rx, cx);
        self.enter(start, window, cx);
    }

    /// Make `tx` / `rx` the window's app channel: the workspace drains `rx`
    /// (the listener of any channel before goes, and with it every message
    /// still queued on it) and the editor reports on `tx`.
    fn set_channel(
        &mut self,
        tx: AppTx,
        rx: UnboundedReceiver<AppMessage>,
        cx: &mut Context<Self>,
    ) {
        self.listen(rx, cx);
        self.editor
            .update(cx, |view, _| view.set_channel(tx.clone()));
        self.tx = Some(tx);
    }

    /// Enter what `start` says, on the settings and channel [`Self::start`]
    /// set up: show the empty state, or index the vault in the background
    /// and land on the first note (FILES lists its folder). The window title
    /// names the workspace. Safe to call again once the vault before has
    /// been left ([`reopen`]). Must run with a tokio runtime entered.
    fn enter(&mut self, start: Start, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(settings), Some(tx)) = (self.settings.clone(), self.tx.clone()) else {
            return;
        };
        // The configured leader tree: the engine keeps it for every sequence
        // and the palette lists it. Built again on every entry, so a reopen
        // after the preferences changed picks up their bindings.
        let tree = settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .leader_tree();
        self.leader.set_tree(tree);
        self.set_title(startup::window_title(&start), window);
        self.empty = startup::empty_state(&start);
        // No workspace: the setup assistant makes one. A vault entered
        // leaves nothing for one still up to set up.
        match start {
            Start::NoWorkspace => self.open_onboarding(window, cx),
            Start::Vault(_) => self.drop_onboarding(),
            Start::OpenFailed(_) => {}
        }
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
            self.start_links(vault.clone(), tx.clone(), window, cx);
            self.start_outline(tx.clone(), window, cx);
            self.start_find(vault.clone(), tx.clone(), window, cx);
            self.start_tags(vault.clone(), tx.clone(), window, cx);
            self.start_semantic(vault.clone(), settings.clone(), tx.clone(), window, cx);
            self.start_ask(tx.clone(), window, cx);
            self.start_ask_sources(vault.clone(), tx.clone(), window, cx);
            self.start_rag_sync(vault.clone(), &settings, tx.clone());
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
        // Once per launch, never under the setup assistant (it runs on the
        // entry that follows the assistant's finish).
        self.check_updates_on_start(cx);
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
                self.poll_picker(cx);
                self.poll_views(cx);
            }
            // Clipboard notices, the restore's "is gone", ... There is no
            // screen here that drops a flash, so a parked one shows now.
            AppMessage::FlashMessage(text) | AppMessage::ParkFlash(text) => self.flash(text, cx),
            AppMessage::IndexingDone(result) => {
                self.status.indexing = false;
                match result {
                    // A note opened while the index caught up counted its
                    // backlinks against the old one; so did LINKS.
                    Ok(_) => {
                        self.refresh_note_counts();
                        self.invalidate_views(cx);
                    }
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
                // Preferences > Workspaces, with the vault's own words, so
                // another workspace or folder can be picked from there.
                self.with_window(cx, move |workspace, window, cx| {
                    workspace.show_vault_conflict(details, window, cx)
                });
            }
            AppMessage::OpenPath { path, emphasis } => {
                // The restore's landing (or anything after it): no longer
                // "opening your last note".
                self.restore_pending = false;
                self.first_open_pending = false;
                if path.is_note() {
                    // A search result arrives with its query's needles.
                    self.open_note_with(path, emphasis, cx);
                } else {
                    // A folder (the root, for an empty history): FILES
                    // shows it, as the TUI's sidebar does.
                    self.files_show_dir(path, cx);
                }
            }
            AppMessage::OpenAttachment(path) => self.open_attachment(&path, cx),
            AppMessage::OpenAttachmentExternally => self.open_attachment_externally(cx),
            // Save as note's request (the TUI's Thread sends it): the create
            // dialog on a free path, holding the answer.
            AppMessage::FileOp(FileOp::ShowCreateWithContent { path, content }) => {
                self.offer_saved_answer(path, content, cx)
            }
            // A request to show the properties dialog, not a result.
            AppMessage::FileOp(FileOp::ShowProperties(path)) => {
                self.with_window(cx, move |workspace, window, cx| {
                    workspace.open_properties(path, window, cx)
                });
            }
            AppMessage::FileOp(op) => {
                self.file_op_closes_modals(&op, cx);
                self.files_file_op(&op, cx);
                self.attachment_file_op(&op, cx);
                self.views_file_op(&op, cx);
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
            AppMessage::FollowLink { target, kind } => self.follow_link(target, kind, cx),
            AppMessage::FollowLabel(name) => self.follow_label(&name, cx),
            // A tag's query, in FIND (TAGS runs it directly; this is the
            // channel's door, never under a modal).
            AppMessage::RunTagQuery(label) => {
                self.with_window(cx, move |workspace, window, cx| {
                    if !modal::dialog_open(window, cx) {
                        workspace.run_tag_query(&label, window, cx);
                    }
                });
            }
            // The sort dialog asked for on the channel: the same door as the
            // chord (refused under another modal; the browser's stacks).
            AppMessage::OpenSortDialog(target) => {
                self.with_window(cx, move |workspace, window, cx| {
                    workspace.open_sort(target, window, cx);
                });
            }
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
                    if workspace.onboarding_open() {
                        tracing::warn!("ExecuteLeaderAction({action:?}) dropped: the setup runs");
                        return;
                    }
                    if workspace.preferences_open() {
                        tracing::warn!(
                            "ExecuteLeaderAction({action:?}) dropped: the preferences show"
                        );
                        return;
                    }
                    let route = workspace.route_of(action);
                    workspace.perform_route(route, window, cx);
                });
            }
            // A property write changed a note on disk (sent by the write
            // itself, so it arrives with the dialog closed too).
            AppMessage::NoteReloadFromDisk(path) => self.reload_from_disk(path, cx),
            // Results addressed to an open modal: the properties dialog's
            // (path-guarded); every other one is stale.
            AppMessage::OverlayData(data) => self.properties_data(data, cx),
            // The server sync's report ([`server`]).
            AppMessage::RagStatus(status) => self.on_rag_status(status, cx),
            // An answer for the Thread ([`ask`]).
            AppMessage::Ask(data) => self.ask_data(data, cx),
            // The server's update notice: M10's update surfaces.
            AppMessage::ServerUpdate(_) => {}
            // The configured workspace may have changed (the TUI rebuilds
            // the vault on these): enter it again ([`reopen`]).
            AppMessage::OnboardingFinished | AppMessage::PreferencesSaved => self.reopen(cx),
            AppMessage::WorkspaceSwitched(name) => self.switch_workspace(name, cx),
            // Everything the GUI has no surface for yet.
            _ => {}
        }
        self.ensure_files_listing(cx);
        self.refresh_views(cx);
        cx.notify();
    }

    /// The open workspace's vault; `None` before one opens.
    pub(crate) fn vault(&self) -> Option<Arc<NoteVault>> {
        self.notes.as_ref().map(|h| h.session.vault().clone())
    }

    /// [`Self::vault`], or, without a workspace, the flash that says so
    /// ([`route::NO_WORKSPACE`]).
    pub(crate) fn vault_or_flash(&mut self, cx: &mut Context<Self>) -> Option<Arc<NoteVault>> {
        let vault = self.vault();
        if vault.is_none() {
            self.flash(route::NO_WORKSPACE, cx);
        }
        vault
    }

    /// Show the notice `text` as an info toast in the window's bottom-right
    /// corner ([`flash_toast`]); it hides itself after the kit's autohide
    /// (five seconds, paused while the pointer is over the stack).
    pub(crate) fn flash(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_flash(text.into(), false, None, cx);
    }

    /// Show the notice `text` as an info toast with `action`'s button (the
    /// update flow's `View`). Clicking it closes the toast and runs the
    /// action on the window. The kit never hides a toast with an action on
    /// its own, so the window takes it down after
    /// [`update::ACTION_TOAST_LIFETIME`] unless it was dismissed before.
    pub(crate) fn flash_with_action(
        &mut self,
        text: impl Into<SharedString>,
        action: ToastAction,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        self.show_flash(text.clone(), false, Some(action), cx);
        let key = text.clone();
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(update::ACTION_TOAST_LIFETIME)
                .await;
            this.update(cx, |workspace, cx| {
                workspace.action_timers.remove(&text);
                workspace.remove_flash(text, cx)
            })
            .ok();
        });
        // A toast shown again drops (cancels) the timer it had.
        self.action_timers.insert(key, timer);
    }

    /// Take the flash toast keyed `text` down (a no-op when it is gone).
    fn remove_flash(&mut self, text: SharedString, cx: &mut Context<Self>) {
        self.toasts.retain(|key| key != &text);
        self.with_window(cx, move |_, window, cx| {
            if matches!(window.root::<gpui_kit::base::Root>(), Some(Some(_))) {
                window.remove_notification1::<FlashId>(text, cx);
            }
        });
    }

    /// Show the problem `text` as an error toast, like [`Self::flash`]. A
    /// later notice no longer displaces it: toasts stack.
    pub(crate) fn flash_error(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_flash(text.into(), true, None, cx);
    }

    /// Push the toast once the current update is done: callers hold only
    /// the workspace's `Context` (app-channel messages, tasks), and the
    /// kit's notification layer is the window's (its `Root`). A flash with
    /// no window or no `Root` to show it in is logged and dropped. At most
    /// [`MAX_FLASH_TOASTS`] are up: a new one past that takes the oldest's
    /// place.
    fn show_flash(
        &mut self,
        text: SharedString,
        error: bool,
        action: Option<ToastAction>,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        self.flashes.push((text.clone(), error));
        let Some(handle) = self.window else {
            tracing::warn!("flash dropped, no window: {text}");
            return;
        };
        let evicted = track_toast(&mut self.toasts, &text);
        let workspace = cx.entity().downgrade();
        cx.defer(move |cx| {
            let shown = handle.update(cx, |_, window, cx| {
                if matches!(window.root::<gpui_kit::base::Root>(), Some(Some(_))) {
                    if let Some(oldest) = evicted {
                        window.remove_notification1::<FlashId>(oldest, cx);
                    }
                    let toast = flash_toast(text.clone(), error);
                    let toast = match action {
                        Some(action) => action.attach(toast, workspace),
                        None => toast,
                    };
                    window.push_notification(toast, cx);
                    true
                } else {
                    false
                }
            });
            if !matches!(shown, Ok(true)) {
                tracing::warn!("flash dropped, the window has no toast layer: {text}");
            }
        });
    }

    /// The newest flash and whether it was an error (tests).
    #[cfg(test)]
    pub(crate) fn last_flash(&self) -> Option<(&str, bool)> {
        self.flashes
            .last()
            .map(|(text, error)| (text.as_ref(), *error))
    }

    /// Persist the layout, and bring the views it shows (or stopped
    /// showing) up to date: a reveal refreshes LINKS and OUTLINE.
    /// A change that took ASK out of every slot takes the Thread down.
    fn save_layout(&mut self, cx: &mut Context<Self>) {
        self.leave_ask_if_drawer_moved_on(cx);
        self.store.save(&self.layout);
        self.refresh_views(cx);
    }

    /// Keep the drawer views in step with the layout and the open note:
    /// each reloads (LINKS, FIND, TAGS) or re-reads its headings (OUTLINE)
    /// only while shown, when revealed or when what it lists for changed.
    /// Cheap otherwise.
    pub(crate) fn refresh_views(&mut self, cx: &mut Context<Self>) {
        if let Some(links) = self.links.clone() {
            let shown = self.is_shown(DrawerView::Links);
            let note = self
                .notes
                .as_ref()
                .and_then(|h| h.note.clone())
                .unwrap_or_else(VaultPath::empty);
            links.update(cx, |view, cx| view.refresh(shown, &note, cx));
        }
        self.refresh_outline(cx);
        self.refresh_find(cx);
        self.refresh_tags(cx);
        self.refresh_semantic(cx);
    }

    /// Apply what the views' loads delivered.
    fn poll_views(&mut self, cx: &mut Context<Self>) {
        if let Some(links) = self.links.as_ref() {
            let list = links.read(cx).list.clone();
            list.update(cx, |list, cx| list.poll(cx));
        }
        if let Some(find) = self.find.as_ref() {
            let list = find.read(cx).list.clone();
            list.update(cx, |list, cx| list.poll(cx));
        }
        if let Some(tags) = self.tags.as_ref() {
            let list = tags.read(cx).list.clone();
            list.update(cx, |list, cx| list.poll(cx));
        }
        if let Some(semantic) = self.semantic.as_ref() {
            let list = semantic.read(cx).list.clone();
            list.update(cx, |list, cx| list.poll(cx));
        }
    }

    /// The vault changed under the views' rows (a file operation landed,
    /// the index caught up): they reload, now if shown, else when revealed.
    fn invalidate_views(&mut self, cx: &mut Context<Self>) {
        if let Some(links) = self.links.as_ref() {
            links.update(cx, |view, cx| view.invalidate(cx));
        }
        if let Some(find) = self.find.as_ref() {
            find.update(cx, |view, cx| view.invalidate(cx));
        }
        if let Some(tags) = self.tags.as_ref() {
            tags.update(cx, |view, cx| view.invalidate(cx));
        }
    }

    /// A file operation landed: the views' rows may name paths that moved.
    fn views_file_op(&mut self, op: &FileOp, cx: &mut Context<Self>) {
        let landed = matches!(
            op,
            FileOp::Created(_) | FileOp::Deleted(_) | FileOp::Renamed { .. } | FileOp::Moved { .. }
        );
        if landed {
            self.invalidate_views(cx);
        }
    }

    /// The view in the focused drawer slot, if a slot has the keyboard.
    fn focused_view(&self) -> Option<DrawerView> {
        match self.layout.focused_region() {
            Region::Slot { side, index } => self.slot_view(side, index),
            _ => None,
        }
    }

    /// Whether `opener` (the panel that had the keyboard) is a drawer whose
    /// focused view lists rows a row can be copied from: LINKS, OUTLINE,
    /// FIND, TAGS or the Sources view.
    pub(crate) fn lists_have_keyboard(&self, opener: Panel) -> bool {
        matches!(opener, Panel::Drawer(_))
            && matches!(
                self.focused_view(),
                Some(
                    DrawerView::Links
                        | DrawerView::Outline
                        | DrawerView::Find
                        | DrawerView::Tags
                        | DrawerView::Ask
                )
            )
    }

    /// `YankRow` in a drawer list: copy what the selected row stands for (a
    /// note's path, a heading's text, a tag). A text field with the
    /// keyboard keeps the chord as its own copy (the FILES and TAGS
    /// filters, FIND's query) unless the palette ran it (`chord` false).
    fn yank_row(&mut self, chord: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        let target = match self.focused_view() {
            Some(DrawerView::Links) => self.links.as_ref().map(|links| {
                let list = links.read(cx).list.clone();
                list.read(cx).selected_row().and_then(|r| r.yank_target())
            }),
            Some(DrawerView::Outline) => self.outline.as_ref().map(|outline| {
                let list = outline.read(cx).list.clone();
                list.read(cx).selected_row().and_then(|r| r.yank_target())
            }),
            Some(DrawerView::Find) if !(chord && self.find_field_focused(window, cx)) => {
                self.find.as_ref().map(|find| {
                    let list = find.read(cx).list.clone();
                    list.read(cx).selected_row().and_then(|r| r.yank_target())
                })
            }
            Some(DrawerView::Tags) if !chord || self.tags_list_focused(window, cx) => {
                self.tags.as_ref().map(|tags| {
                    let list = tags.read(cx).list.clone();
                    list.read(cx).selected_row().and_then(|r| r.yank_target())
                })
            }
            // The Sources view: the source's path (the TUI's yank).
            Some(DrawerView::Ask) if !(chord && self.sources_filter_focused(window, cx)) => {
                self.ask_sources.as_ref().map(|sources| {
                    let list = sources.read(cx).list.clone();
                    list.read(cx).selected_row().and_then(|r| r.yank_target())
                })
            }
            _ => return false,
        };
        match target.flatten() {
            Some(target) => {
                cx.write_to_clipboard(ClipboardItem::new_string(target.text));
                self.flash(format!("{} copied", target.noun), cx);
            }
            None => self.flash("Nothing to copy", cx),
        }
        true
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

    /// `view`'s slot handle.
    fn slot_focus(&self, view: DrawerView) -> &FocusHandle {
        self.view_focus
            .iter()
            .find(|(v, _)| *v == view)
            .map(|(_, handle)| handle)
            .expect("every drawer view has a slot handle")
    }

    /// Where the keyboard goes in `view`'s slot: FILES' and TAGS' filters,
    /// so typing filters; FIND's query field; LINKS' and OUTLINE's lists;
    /// the CFG view itself; the slot itself for a view not built yet (no
    /// vault).
    fn view_handle(&self, view: DrawerView, cx: &App) -> FocusHandle {
        let handle = match view {
            DrawerView::Files => self.files.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Links => self.links.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Outline => self.outline.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Find => self.find.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Tags => self.tags.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Semantic => self.semantic.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Ask => self
                .ask_sources
                .as_ref()
                .map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Config => Some(self.config.read(cx).focus_handle(cx)),
        };
        handle.unwrap_or_else(|| self.slot_focus(view).clone())
    }

    /// The handle of a view that has the whole window (the setup assistant
    /// first, then Preferences), if one has.
    fn takeover_focus(&self, cx: &App) -> Option<FocusHandle> {
        if let Some(view) = self.onboarding.as_ref() {
            return Some(view.read(cx).focus_handle(cx));
        }
        self.preferences
            .as_ref()
            .map(|view| view.read(cx).focus_handle(cx))
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
            // The Thread or the attachment view while one shows, else the
            // editor.
            Panel::Editor => match (self.ask.as_ref(), self.attachment.as_ref()) {
                (Some(ask), _) if self.ask_shown() => ask.read(cx).focus_handle(cx),
                (_, Some(view)) => view.read(cx).focus_handle(cx),
                _ => self.editor.read(cx).focus_handle(cx),
            },
        }
    }

    /// Give the keyboard to the region the layout says is focused. Not
    /// while a modal is open: it has the keyboard, and gives it back to its
    /// opener when it closes ([`modal`]).
    pub(crate) fn apply_focus(&self, window: &mut Window, cx: &mut App) {
        if modal::dialog_open(window, cx) {
            return;
        }
        // The setup assistant or Preferences have the window: the keyboard
        // is theirs.
        if let Some(handle) = self.takeover_focus(cx) {
            window.focus(&handle, cx);
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
    /// showed; one showing nothing comes back with its top hidden view, and
    /// one with no placed view at all opens **empty** (a hint in place of a
    /// view; the keyboard stays where it was, there is nothing to focus).
    /// Persisted, visible like any drawer.
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
            if self.layout.open_drawer(side) {
                self.layout.focus(drawer);
            }
            self.ensure_files_listing(cx);
        }
        self.apply_focus(window, cx);
        self.save_layout(cx);
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
        // ASK shows with the Thread: entering it is the Thread's door.
        if view == DrawerView::Ask && !self.is_shown(view) {
            self.open_ask_drawer(window, cx);
            return;
        }
        self.place_rail_cursor(Some(view));
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
        self.save_layout(cx);
        cx.notify();
    }

    /// The view a move acts on: the focused slot's, or the one under the
    /// rail's cursor (placed or not). `None` from the editor.
    fn arranged_view(&self) -> Option<DrawerView> {
        match self.layout.focused_region() {
            Region::Slot { side, index } => self.slot_view(side, index),
            Region::Rail => self.rail_cursor_view(),
            Region::Group(_) => None,
        }
    }

    /// `MoveViewTo…Drawer`: move the focused view to the drawer on `to` and
    /// show it; the keyboard follows it. It fits the target: one with an
    /// empty slot takes it there, a split one keeps the view's slot
    /// position, an unsplit or empty one takes it into its single slot,
    /// replacing the view shown there — never a new split. The model moves,
    /// it never duplicates. Persisted.
    ///
    /// From the rail the move is spatial: the rail sits at the window's
    /// edge ([`RAIL_SIDE`]), so the chord away from that edge moves and
    /// shows the cursor's view in the drawer beside the rail, and the chord
    /// toward the edge is not handled ([`shell_layout::rail_move_target`]).
    /// The rail keeps the keyboard.
    fn move_view(&mut self, to: Side, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.arranged_view() else {
            return false;
        };
        let to = match self.layout.focused_region() {
            Region::Rail => match shell_layout::rail_move_target(to) {
                Some(side) => side,
                None => return false,
            },
            _ => to,
        };
        self.layout.move_view(view, to);
        self.ensure_files_listing(cx);
        self.apply_focus(window, cx);
        self.save_layout(cx);
        true
    }

    /// `MoveViewUp` / `MoveViewDown`: in a split drawer, put the focused
    /// view (or the rail cursor's) at the top (`up`) or bottom slot
    /// position, trading places with the view shown there, or moving into
    /// the drawer's empty slot (which takes the slot it left); a focused
    /// view keeps the keyboard. Already there: handled, nothing moves. With
    /// the rail cursor on a view no visible drawer shows, it goes into an
    /// empty slot (the drawer beside the rail first), the rail keeping the
    /// keyboard. In an unsplit drawer, and for a view placed nowhere with
    /// no empty slot to go to, nothing changes and the key is not handled,
    /// so it passes on. Persisted.
    fn move_view_vertically(
        &mut self,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.layout.focused_region() == Region::Rail
            && let Some(view) = self.arranged_view()
            && !self.is_shown(view)
            && let Some(side) = self.empty_slot_side()
        {
            self.layout.fill_empty_slot(side, view);
            self.ensure_files_listing(cx);
            self.apply_focus(window, cx);
            self.save_layout(cx);
            return true;
        }
        let Some((view, (side, _))) = self
            .arranged_view()
            .and_then(|view| self.layout.placement(&view).map(|p| (view, p)))
        else {
            return false;
        };
        if self.layout.set_view_slot(&view, if up { 0 } else { 1 }) {
            self.apply_focus(window, cx);
            self.save_layout(cx);
            return true;
        }
        self.layout.slots(side).len() > 1
    }

    /// The visible drawer with an empty slot a rail move fills: the one
    /// beside the rail first.
    fn empty_slot_side(&self) -> Option<Side> {
        [RAIL_SIDE, RAIL_SIDE.opposite()].into_iter().find(|side| {
            self.layout.is_visible(Panel::Drawer(*side)) && self.layout.empty_slot(*side).is_some()
        })
    }

    /// `SplitDrawerSlot`: open an **empty slot** at the focused drawer's
    /// free slot position, at the drawer's ratio; the keyboard stays on the
    /// view. A view gets there only by a move (Move Up/Down, a move from
    /// the other drawer, or the rail cursor's Move Up/Down). Session only:
    /// nothing is saved. A full drawer says so.
    fn split_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Panel::Drawer(side) = self.layout.focused() else {
            return false;
        };
        match self.layout.split_empty(side) {
            Ok(_) => self.apply_focus(window, cx),
            Err(LayoutError::SlotCap) => self.flash("This drawer already holds two views", cx),
            Err(_) => {}
        }
        true
    }

    /// `UnsplitDrawerSlot`: close the focused slot, merging every view
    /// placed there into the other slot (the closed view hidden), so the
    /// drawer stays unsplit with the other view focused, and showing any of
    /// its views later replaces the shown one. A drawer with an empty slot
    /// just closes it. A drawer's only slot stays (the toggle hides a
    /// drawer). Persisted.
    fn unsplit_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Region::Slot { side, index } = self.layout.focused_region() else {
            return false;
        };
        if self.layout.close_empty_slot(side) {
            self.apply_focus(window, cx);
            return true;
        }
        if self.layout.slot_views(side).len() < 2 {
            self.flash("This drawer holds one view", cx);
            return true;
        }
        if self.layout.close_slot(side, index).is_ok() {
            self.apply_focus(window, cx);
            self.save_layout(cx);
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
            self.save_layout(cx);
            cx.notify();
        }
    }

    /// The context `panel` resolves chords in; the editor's own (vim, an
    /// open find bar) when it is the editor.
    fn key_ctx(&self, cx: &App) -> KeyCtx {
        match self.layout.focused() {
            // The Thread and the attachment view read no vim keys.
            Panel::Editor if self.attachment.is_some() || self.ask_shown() => KeyCtx {
                panel: Panel::Editor,
                vim: false,
            },
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
        // F11 is the window's (fullscreen), over every view, unless a
        // pending leader sequence owns the key.
        if !self.leader.is_pending() && self.fullscreen_key(event, window, cx) {
            cx.stop_propagation();
            return;
        }
        // The setup assistant and Preferences have every key; no shell chord
        // runs under them.
        if self.onboarding.is_some() || self.preferences.is_some() {
            return;
        }
        // A pending leader sequence owns every key, in every panel, ahead of
        // the editor's text input; a vim Normal Space arms one. Never under
        // a modal: one cannot open while a sequence is pending, and nothing
        // arms a sequence over one. Any other key skips this (and the
        // modal check) cheaply.
        if self.leader_wants(event) {
            if modal::dialog_open(window, cx) {
                self.cancel_leader(cx);
            } else if self.intercept_leader_key(event, window, cx) {
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
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
            ActionShortcuts::OpenCommandPalette => {
                self.toggle_modal(ModalKind::Palette, Self::open_palette, window, cx)
            }
            // The leader gateway. Never seen here while a sequence is
            // pending: the interceptor takes every key then, and the gateway
            // again cancels there ([`Self::intercept_leader_key`]).
            ActionShortcuts::Leader => {
                let gateway = self.leader_chord_label();
                self.start_leader(gateway, window, cx);
            }
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
            ActionShortcuts::FileOperations => self.file_ops_chord(window, cx),
            ActionShortcuts::OpenNote => {
                self.toggle_modal(ModalKind::Finder, Self::open_finder, window, cx)
            }
            ActionShortcuts::SearchNotes => self.toggle_modal(
                ModalKind::Search,
                |workspace, window, cx| workspace.open_search(None, window, cx),
                window,
                cx,
            ),
            ActionShortcuts::OpenSavedSearches => self.toggle_modal(
                ModalKind::SavedSearches,
                Self::open_saved_searches,
                window,
                cx,
            ),
            // FIND's query (field or results); elsewhere the chord goes on.
            ActionShortcuts::SaveCurrentQuery => match self.focused_view() {
                Some(DrawerView::Find) => self.save_find_query(window, cx),
                _ => return false,
            },
            // The focused list (FILES, FIND); the palette's row sorts FILES
            // when no view is focused. Elsewhere the chord goes on.
            ActionShortcuts::OpenSortDialog => return self.sort_command(key.is_some(), window, cx),
            ActionShortcuts::NewJournal => self.open_journal(window, cx),
            ActionShortcuts::QuickNote => {
                self.toggle_modal(ModalKind::QuickNote, Self::open_quick_note, window, cx)
            }
            ActionShortcuts::OpenFileBrowser => self.open_file_browser(window, cx),
            ActionShortcuts::YankRow => return self.yank_row(key.is_some(), window, cx),
            // The Thread, the composer focused (leader `a a`).
            ActionShortcuts::OpenAsk => self.ask_step(AskStep::Focus, window, cx),
            // The workspace switcher, in every window state (M8d).
            // Preferences, in every window state (M9b).
            ActionShortcuts::OpenPreferences => self.open_preferences(window, cx),
            ActionShortcuts::SwitchWorkspace => self.toggle_modal(
                ModalKind::WorkspaceSwitcher,
                Self::open_workspace_switcher,
                window,
                cx,
            ),
            // Nothing followable under the caret: the chord does nothing.
            // On an attachment it opens it externally.
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

    /// The views the rail offers now: SEM only while the server can
    /// search ([`shell_layout::rail_views`]).
    pub(crate) fn rail_views(&self) -> &'static [DrawerView] {
        shell_layout::rail_views(self.rag_caps)
    }

    /// The view under the rail's keyboard cursor.
    pub(crate) fn rail_cursor_view(&self) -> Option<DrawerView> {
        self.rail_views().get(self.rail_cursor).copied()
    }

    /// Put the rail's cursor on `view` when the rail offers it; otherwise
    /// keep its position, clamped to the rail (a cell that went away).
    pub(crate) fn place_rail_cursor(&mut self, view: Option<DrawerView>) {
        let views = self.rail_views();
        match view.and_then(|view| views.iter().position(|v| *v == view)) {
            Some(i) => self.rail_cursor = i,
            None => self.rail_cursor = self.rail_cursor.min(views.len() - 1),
        }
    }

    /// The rail's own keys: Up/Down move between cells, Enter or Space
    /// activates one (as a click does).
    fn rail_key(&mut self, key: KeyInput, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if key.mods != Mods::NONE {
            return false;
        }
        match key.key {
            Key::Up => self.rail_cursor = self.rail_cursor.saturating_sub(1),
            Key::Down => self.rail_cursor = (self.rail_cursor + 1).min(self.rail_views().len() - 1),
            Key::Enter | Key::Char(' ') => {
                if let Some(view) = self.rail_cursor_view() {
                    self.activate_rail_view(view, window, cx);
                }
            }
            _ => return false,
        }
        true
    }

    /// The focused region's marker: a thin bar in the focus colour across
    /// its top edge, laid over it so focusing never shifts content. Subtle,
    /// but always there (design guides: state must be visible).
    fn focus_bar(&self, region: Region, cx: &gpui_kit::App) -> Option<impl IntoElement> {
        (self.layout.focused_region() == region).then(|| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h_0p5()
                .bg(theme::focus_border(&theme::current(cx)))
        })
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_bg = theme::panel_background(&theme::current(cx));
        let focused = self.layout.focused() == Panel::Rail;
        let views = self.rail_views();
        let (top, pinned) = views.split_at(views.len() - 1);
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
        let pinned = cell(views.len() - 1, pinned[0], cx);
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
            .border_color(theme::divider(&theme::current(cx)))
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus_panel(Panel::Rail, window, cx)
            }))
            .children(top)
            .child(div().flex_1())
            .child(pinned)
            // Last, so it paints over a highlighted cell.
            .children(self.focus_bar(Region::Rail, cx))
    }

    fn render_rail_cell(
        &self,
        view: DrawerView,
        active: bool,
        cursor: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let accent = theme::focus_border(&theme::current(cx));
        let glyph = if active {
            accent
        } else if cursor {
            theme::bright(&theme::current(cx))
        } else {
            theme::muted(&theme::current(cx))
        };
        let hover = theme::hover_background(&theme::current(cx));
        let selected = theme::selected_background(&theme::current(cx));
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
        let shown = self.layout.slots(side);
        let split = (shown.len() > 1)
            .then(|| self.layout.split_ratio(side))
            .flatten();
        let label = shown
            .iter()
            .flatten()
            .map(|v| shell_layout::view_name(*v))
            .collect::<Vec<_>>()
            .join(", ");
        let mut index = 0;
        let mut slots: Vec<AnyElement> = Vec::new();
        for (row, view) in shown.iter().enumerate() {
            let share = if row == 0 { split } else { None };
            slots.push(match view {
                Some(view) => {
                    index += 1;
                    self.render_slot(side, index - 1, *view, share, cx)
                }
                None => self.render_empty_slot(side, share, cx),
            });
        }
        if shown.is_empty() {
            slots.push(self.render_empty_slot(side, None, cx));
        }
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
            .bg(theme::panel_background(&theme::current(cx)))
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
                        theme::divider(&theme::current(cx)),
                        theme::focus_border(&theme::current(cx)),
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
                    .text_color(theme::foreground(&theme::current(cx)))
                    .child(shell_layout::slot_title(view)),
            )
            .map(|slot| match self.view_content(view) {
                Some(content) => slot.child(content),
                None => slot.child(
                    div()
                        .px_3()
                        .text_sm()
                        .text_color(theme::muted(&theme::current(cx)))
                        .child("Coming soon"),
                ),
            })
            .children(self.focus_bar(region, cx))
            .when(share.is_some(), |slot| {
                slot.child(self.render_split_divider(side, cx))
            })
            .into_any_element()
    }

    /// What `view`'s slot shows, for the views built so far.
    fn view_content(&self, view: DrawerView) -> Option<AnyElement> {
        match view {
            DrawerView::Files => self.files.clone().map(|v| v.into_any_element()),
            DrawerView::Links => self.links.clone().map(|v| v.into_any_element()),
            DrawerView::Outline => self.outline.clone().map(|v| v.into_any_element()),
            DrawerView::Find => self.find.clone().map(|v| v.into_any_element()),
            DrawerView::Tags => self.tags.clone().map(|v| v.into_any_element()),
            DrawerView::Semantic => self.semantic.clone().map(|v| v.into_any_element()),
            DrawerView::Ask => self.ask_sources.clone().map(|v| v.into_any_element()),
            DrawerView::Config => Some(self.config.clone().into_any_element()),
        }
    }

    /// The divider under the first of `side`'s two stacked slots: dragging
    /// it moves the split.
    fn render_split_divider(&self, side: Side, cx: &mut Context<Self>) -> impl IntoElement {
        resize_handle::<SlotResize, DragGhost>(split_divider_id(side), Axis::Vertical)
            .inside(HandleEdge::Trailing)
            .with_appearance(hairline(
                Axis::Vertical,
                theme::divider(&theme::current(cx)),
                theme::focus_border(&theme::current(cx)),
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
            )
    }

    /// `side`'s placeholder: its empty slot (`share` as for a slot), or the
    /// whole drawer while it shows no view. A quiet hint, not a target: a
    /// view gets here only by a move; the hint names the move chords the
    /// keymap binds ([`ChordLabels`]).
    fn render_empty_slot(
        &self,
        side: Side,
        share: Option<f64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hint = self.chord_labels.empty_slot_hint.clone();
        div()
            .id(empty_slot_id(side))
            .test_support()
            .role(Role::Group)
            .aria_label(hint.clone())
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .min_h_0()
            .px_4()
            .map(|slot| match share {
                Some(share) => slot.flex_none().h(relative(share as f32)),
                None => slot.flex_1(),
            })
            .child(
                div()
                    .text_sm()
                    .text_center()
                    .text_color(theme::muted(&theme::current(cx)))
                    .child(hint),
            )
            .when(share.is_some(), |slot| {
                slot.child(self.render_split_divider(side, cx))
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
        if let Some(gone) = self.note_gone.clone() {
            return Some(gone);
        }
        let (title, detail) = if self.restore_pending {
            (
                "Opening your last note…",
                "It opens once the workspace is indexed.",
            )
        } else if host.is_switching() {
            ("Opening the note…", "")
        } else {
            ("No note open", self.chord_labels.no_note_hint.as_str())
        };
        Some(EmptyState {
            title: title.into(),
            detail: detail.into(),
            failed: false,
        })
    }

    /// The editor Panel: the editor (or the attachment view, or the empty
    /// state) over the Footer.
    fn render_editor(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ask = self
            .ask
            .clone()
            .filter(|_| self.ask_shown() && self.empty.is_none());
        let attachment = self
            .attachment
            .clone()
            .filter(|_| self.empty.is_none() && ask.is_none());
        let empty = match (&ask, &attachment) {
            (None, None) => self.empty.clone().or_else(|| self.note_empty_state()),
            _ => None,
        };
        let editor_shown = empty.is_none() && attachment.is_none() && ask.is_none();
        let document = match (&empty, ask, attachment) {
            // The editor is not drawn: its focus handle sits on the empty
            // state, so the keyboard still reaches the window's chords (F5
            // away from a workspace that would not open, the leader, the
            // palette).
            (Some(empty), _, _) => self
                .render_empty(empty, &self.editor.read(cx).focus_handle(cx), cx)
                .into_any_element(),
            (None, Some(view), _) => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(view)
                .into_any_element(),
            (None, None, Some(view)) => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(view)
                .into_any_element(),
            (None, None, None) => div()
                .flex_1()
                .min_h_0()
                .child(self.editor.clone())
                .into_any_element(),
        };
        // The which-key docks over the bottom of the document, right above
        // the Footer, so showing it never reflows the note.
        let document = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(document)
            .children(self.render_which_key(cx));
        div()
            .id("editor-panel")
            .test_support()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(theme::background(&theme::current(cx)))
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus_panel(Panel::Editor, window, cx)
            }))
            .child(document)
            .child(status_row::render(
                &self.status,
                &self.doc_facts(editor_shown, cx),
                &theme::current(cx),
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.show_links_tab(LinksTab::Backlinks, window, cx)
                }),
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_properties_for_open_note(window, cx)
                }),
            ))
            .children(self.focus_bar(Region::Group(self.layout.active_group()), cx))
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
    /// `focus` is the editor's handle: the editor is not drawn, so the
    /// empty state holds the keyboard for it.
    fn render_empty(
        &self,
        empty: &EmptyState,
        focus: &FocusHandle,
        cx: &gpui_kit::App,
    ) -> impl IntoElement {
        let (icon, tint) = if empty.failed {
            (IconName::TriangleAlert, theme::error(&theme::current(cx)))
        } else {
            (IconName::FolderOpen, theme::muted(&theme::current(cx)))
        };
        div()
            .id(EMPTY_STATE_ID)
            .track_focus(focus)
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
                    .text_color(theme::foreground(&theme::current(cx)))
                    .header(
                        EmptyHeader::new()
                            .media(
                                EmptyMedia::new()
                                    .with_variant(EmptyMediaVariant::Icon)
                                    .bg(theme::panel_background(&theme::current(cx)))
                                    .child(Icon::new(icon).text_color(tint)),
                            )
                            .title(EmptyTitle::new().child(empty.title.clone()))
                            .description(
                                EmptyDescription::new()
                                    .text_color(theme::muted(&theme::current(cx)))
                                    .child(empty.detail.clone()),
                            ),
                    ),
            )
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.takeover_focus(cx)
            .unwrap_or_else(|| self.handle_for(self.layout.focused(), cx))
    }
}

impl Render for Workspace {
    /// The title bar, when the window has one ([`titlebar`]), over the body.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = self.render_title_bar(window, cx);
        let body = self.render_body(window, cx);
        div()
            .id("window-frame")
            .flex()
            .flex_col()
            .size_full()
            .children(bar)
            .child(body)
    }
}

impl Workspace {
    /// Everything under the title bar. It takes the height the bar leaves.
    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // The setup assistant takes the whole body: no Panel is drawn, and
        // the workspace's key handler is not attached, so its chords are
        // inert.
        if let Some(view) = self.onboarding.clone() {
            return div()
                .id("workspace")
                .flex()
                .w_full()
                .flex_1()
                .min_h_0()
                .bg(theme::background(&theme::current(cx)))
                .child(view)
                .into_any_element();
        }
        // Preferences likewise.
        if let Some(view) = self.preferences.clone() {
            return div()
                .id("workspace")
                .flex()
                .w_full()
                .flex_1()
                .min_h_0()
                .bg(theme::background(&theme::current(cx)))
                .child(view)
                .into_any_element();
        }
        let limits = Self::limits(window);
        let drawer = |side: Side| {
            self.layout
                .is_visible(Panel::Drawer(side))
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
            .w_full()
            .flex_1()
            .min_h_0()
            .bg(theme::background(&theme::current(cx)))
            .child(self.render_rail(cx))
            .when_some(left, |row, width| {
                row.child(self.render_drawer(Side::Left, width, cx))
            })
            .child(self.render_editor(cx))
            .when_some(right, |row, width| {
                row.child(self.render_drawer(Side::Right, width, cx))
            })
            .into_any_element()
    }
}

/// How many flash toasts may be up at once. Different texts stack (a
/// repeat replaces its own), so a burst of distinct messages — an autosave
/// failing on every tick with a changing reason, under a hovered stack that
/// pauses every countdown — would otherwise pile up without end.
pub(crate) const MAX_FLASH_TOASTS: usize = 4;

/// The element id of a flash toast's message: its accessible label.
pub(crate) const FLASH_MESSAGE_ID: &str = "flash-message";

/// Record the flash toast keyed `text` as the newest in `toasts` (a repeat
/// moves to the back: the kit replaces its toast). Returns the oldest key
/// once there are more than [`MAX_FLASH_TOASTS`]: that toast must go.
fn track_toast(toasts: &mut VecDeque<SharedString>, text: &SharedString) -> Option<SharedString> {
    toasts.retain(|key| key != text);
    toasts.push_back(text.clone());
    if toasts.len() > MAX_FLASH_TOASTS {
        toasts.pop_front()
    } else {
        None
    }
}

/// A button on a flash toast ([`Workspace::flash_with_action`]): its label
/// and what it runs on the window.
#[derive(Clone)]
pub(crate) struct ToastAction {
    label: SharedString,
    run: update::ToastRun,
}

impl ToastAction {
    pub(crate) fn new(
        label: impl Into<SharedString>,
        run: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            run: Rc::new(run),
        }
    }

    /// `toast` with this action's button: a click closes the toast, then
    /// runs the action on `workspace` (gone: nothing).
    fn attach(self, toast: Notification, workspace: WeakEntity<Workspace>) -> Notification {
        toast.action(move |_, _, cx| {
            let note = cx.entity().downgrade();
            let run = self.run.clone();
            let workspace = workspace.clone();
            Button::new(TOAST_ACTION_ID)
                .label(self.label.clone())
                .on_click(move |_, window, cx| {
                    note.update(cx, |note, cx| note.dismiss(window, cx)).ok();
                    workspace
                        .update(cx, |workspace, cx| (run)(workspace, window, cx))
                        .ok();
                })
        })
    }
}

/// The element id of a flash toast's action button.
pub(crate) const TOAST_ACTION_ID: &str = "toast-action";

/// The kit notification id type of the flashes: keyed by the message, so a
/// repeated flash replaces its toast instead of stacking a second one.
struct FlashId;

/// The kit's toast kind for a flash: an error, or else plain information.
fn toast_kind(error: bool) -> NotificationType {
    if error {
        NotificationType::Error
    } else {
        NotificationType::Info
    }
}

/// The toast for the flash `text`: its kind's icon, the message, in the
/// bottom-right corner (away from the editor's caret and the dialogs,
/// which sit high and centred), hiding itself after the kit's autohide.
///
/// The kit's toast is an accessibility `Alert`, but its `message` is a bare
/// text child with no node of its own, so the alert would be announced
/// without its words. The message is drawn here instead, as the kit draws
/// it (`text_sm` in the toast's column), named by its text.
fn flash_toast(text: SharedString, error: bool) -> Notification {
    let message = text.clone();
    Notification::new()
        .content(move |_, _, _| {
            div()
                .id(FLASH_MESSAGE_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(message.clone())
                .text_sm()
                .child(message.clone())
                .into_any_element()
        })
        .with_type(toast_kind(error))
        .id1::<FlashId>(text)
        .placement(Anchor::BottomRight)
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

mod ask;
mod ask_sources;
mod attachment;
mod config;
mod create;
mod drawer_list;
mod file_dialogs;
mod file_ops;
mod files;
mod find;
mod folder_chooser;
mod follow;
mod key_suggestions;
mod leader;
mod links;
mod modal;
mod note;
mod note_flush;
mod onboarding;
mod outline;
mod palette;
mod picker;
mod pinned;
mod preferences;
mod properties;
mod query_field;
mod reopen;
mod route;
mod saved_searches;
mod search;
mod semantic;
mod server;
mod sort;
mod switcher;
mod tags;
mod theme_picker;
pub(crate) mod titlebar;
mod update;
#[cfg(feature = "fake-update")]
pub(crate) use update::fake_backend_from_env as fake_update_backend_from_env;

#[cfg(test)]
mod ask_sources_tests;
#[cfg(test)]
mod ask_tests;
#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod cfg_tests;
#[cfg(test)]
mod drawer_tests;
#[cfg(test)]
mod file_ops_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod find_tests;
#[cfg(test)]
mod follow_tests;
#[cfg(test)]
mod footer_tests;
#[cfg(test)]
mod leader_tests;
#[cfg(test)]
mod note_tests;
#[cfg(test)]
mod onboarding_tests;
#[cfg(test)]
mod palette_tests;
#[cfg(test)]
mod picker_tests;
#[cfg(test)]
mod pinned_tests;
#[cfg(test)]
mod preferences_tests;
#[cfg(test)]
mod preferences_workspaces_tests;
#[cfg(test)]
mod properties_tests;
#[cfg(test)]
mod reopen_tests;
#[cfg(test)]
mod saved_searches_tests;
#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod semantic_tests;
#[cfg(test)]
mod sort_tests;
#[cfg(test)]
mod start_tests;
#[cfg(test)]
mod switcher_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod theme_picker_tests;
#[cfg(test)]
mod titlebar_tests;
#[cfg(test)]
mod update_tests;
#[cfg(test)]
mod views_tests;
