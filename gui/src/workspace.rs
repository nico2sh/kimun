//! The **Workspace**: the window's root view, composing the editor screen
//! left to right — the **Activity rail**, then the **layout tree** of
//! **slots** and **editor groups** (ADR-0052). The editor group is the
//! [`EditorView`], unchanged inside.
//!
//! **State.** The tree, the views placed in each slot (one shown), and
//! which leaf has the keyboard live in the logic [`LayoutTree`]; this view
//! only renders it ([`tree_view`]) and keeps GPUI focus in step with it. A
//! slot with no shown view is not drawn. Which views a side toggle hid is
//! not persisted.
//!
//! **Arranging.** The rail shows a view where it is placed, hiding the
//! view sharing its slot, and a click on a shown view hides it; the move,
//! split and close chords rearrange the views from the rail or a slot (a
//! split asks which view the new slot holds, [`view_chooser`]); the
//! side toggles hide or bring back every view left or right of the active
//! group. A view is shown in at most one slot, so FILES (one entity)
//! renders wherever it is.
//!
//! **Keys.** Keys go to the focused leaf only. A chord the GUI keymap binds
//! to a workspace command (toggle a side, move or cycle focus) bubbles
//! here from whichever leaf has focus — the editor lets those pass by —
//! and is resolved in that leaf's context, as the editor resolves its own.
//! With the rail or a slot focused, the workspace resolves its chords
//! before any key binding, so a field inside a slot (the FILES filter)
//! cannot swallow them.
//! Alt+Up/Down with nothing stacked to move to is handed back to the editor
//! (ADR-0050 §4: not swallowed).
//!
//! **Persistence.** The tree (weights and placements) is written to
//! `[gui.layout]` whenever a view is opened, moved, hidden or its slot
//! closed, and when a side is toggled, through the TUI's own settings save
//! path ([`crate::shell_tree`]).
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
//! **FILES (M5b-2 G3).** With a vault, the left slot's FILES view is the
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
//! a slot, inside the search browser, the palette, each list's sort chip)
//! re-sorts FILES in place, or rewrites FIND's or the browser's query order
//! ([`sort`]). Changes apply live; nothing is saved.
//!
//! **The kimün server and SEM (M7c G9).** With a server configured, the
//! window runs the TUI's background sync and follows the status it reports
//! ([`server`]): the rail offers SEM while the server can search, and the
//! Footer says how the server is. SEM is the semantic search view
//! ([`semantic`]).
//!
//! **Ask (M7c G10a/b, ADR-0053).** The view ASK, in a slot like any
//! other (right by default): the Thread with the selected turn's sources
//! and the Source reader expanding in place ([`ask`], [`ask_sources`]).
//! It never replaces the editor, so the open note stays open beside it;
//! the rail offers ASK while the server can answer, and its placement is
//! persisted like every view's.
//!
//! **The setup assistant (M8c).** With no workspace configured, and from
//! the leader / palette (`guided setup`), the window shows the setup
//! assistant in place of its Panels ([`onboarding`]): the rail, the slots
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
//! through the close guard ([`titlebar`]); fullscreen hides it.
//!
//! **The keyboard reference (M10d).** F1 (`KeyboardHelp`), leader `?`, the
//! palette and the CFG view list every chord of the active keymap under its
//! heading and every leader sequence, filtered as you type; it runs nothing
//! ([`keyboard_help`]). Where a query is typed (FIND with the keyboard, the
//! search browser) F1 is the query syntax instead: the `QuerySyntax` action,
//! whose keymap scope shadows `KeyboardHelp` there, rebindable like any
//! other.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
#[cfg(test)]
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::button::Button;
use gpui_kit::component::empty::{
    Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::{Icon, WindowExt as _, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, AnyWindowHandle, App, AppContext, ClickEvent, ClipboardItem, Context,
    Entity, FocusHandle, Focusable, InputPreference, InteractiveElement, IntoElement, KeyDownEvent,
    KeystrokeEvent, MouseButton, MouseDownEvent, ParentElement, Render, Role, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity, Window, div,
    rems,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::ask_flow::AskFlow;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::layout_tree::spec::ChildSpec;
use kimun_notes::logic::layout_tree::{
    Axis as TreeAxis, Dir, Focus, GroupId, LayoutTree, Leaf, RailPosition, SlotId, TreeFocusMove,
};
use kimun_notes::logic::leader_route::AskStep;
use kimun_notes::logic::list::ListRow as _;
use kimun_notes::logic::message::{AppMessage, AppTx, FileOp};
use kimun_notes::logic::nav::{DrawerView, LinksTab};
use kimun_notes::logic::rag_caps::Caps;
use kimun_notes::logic::session::{NoteSession, SaveCompleted};
use kimun_notes::logic::view_registry;
use kimun_notes::rag::RagStatus;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, SharedSettings};
use tokio::sync::mpsc::UnboundedReceiver;

use self::drop_zones::{DragFrom, Dragged, LiveDrag, ViewDrag};
use self::placement::Placement;
use crate::editor_view::{EditorEvent, EditorView};
use crate::input;
use crate::keymap::{self, KeyCtx};
use crate::shell_layout::{self, RAIL_REMS};
use crate::shell_tree;
use crate::startup::{self, EmptyState, FirstNote, Start, VaultStart};
use crate::status_row::{self, DocFacts, Status};
use crate::theme;
use kimun_notes::logic::task_slot::TaskSlot;

use files::FilesPane;
#[cfg(test)]
pub(crate) use files::ICONS as FILES_ICONS;
#[cfg(test)]
pub(crate) use group_header::ICONS as GROUP_HEADER_ICONS;
use leader::LeaderState;
use modal::ModalKind;
pub(crate) use note::Closing;
use note::NoteHost;
pub(crate) use palette::Palette;
#[cfg(test)]
pub(crate) use tab_strip::ICONS as TAB_STRIP_ICONS;
#[cfg(test)]
pub(crate) use titlebar::WINDOW_CONTROL_ICONS;

/// The key context of the workspace root.
const KEY_CONTEXT: &str = "KimunWorkspace";

/// Why a slot does not close: it is the last on its side of the editor.
pub(crate) const LAST_ON_SIDE: &str = "This is the last slot on this side";
/// Why the split chord does nothing on the editor while zoomed.
pub(crate) const ZOOMED_NO_GROUP_SPLIT: &str = "Leave zoom to split the editor";

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

/// The rail's place and visibility, saved with every layout write
/// (ADR-0055).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RailState {
    pub(crate) position: RailPosition,
    pub(crate) hidden: bool,
}

/// What a layout save does with the recorded `layout_preset`.
#[derive(Clone, Copy)]
enum PresetRecord<'a> {
    /// Leave it as it is (a drag, a toggle, a moved view).
    Keep,
    /// Record this preset (it was just applied).
    Set(&'a str),
    /// Remove it (the default layout was installed).
    Clear,
}

/// Why [`Workspace::place_new_slot`] made no slot; nothing changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoNewSlot {
    /// The leaf to place beside is not in the tree.
    Refused,
    /// The window has no room for the slot; a flash said so.
    NoRoom,
}

/// What [`Workspace::spawn_at`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spawn {
    /// A slot was made and the view put in it.
    Placed,
    /// The window had no room; nothing changed, a flash said so.
    NoRoom,
    /// The leaf to place beside is not in the tree, or the move is
    /// pointless; nothing changed.
    Refused,
}

impl Spawn {
    /// Whether the request was dealt with: placed, or refused for lack of
    /// room with the user told. False only for [`Spawn::Refused`], where a
    /// caller may try something else.
    pub(crate) fn handled(self) -> bool {
        self != Spawn::Refused
    }
}

impl LayoutStore {
    /// Write `layout` into the store's `[gui]` table, and per `preset`
    /// the name of the preset it was applied from (`layout_preset`). A
    /// failure is logged and otherwise ignored: the layout in the window is
    /// still right.
    fn save(&self, layout: &LayoutTree<DrawerView>, preset: PresetRecord<'_>, rail: RailState) {
        let edit = |gui: &mut kimun_notes::settings::gui::GuiSettings| {
            shell_tree::record(layout, gui);
            gui.set_rail(rail.position);
            gui.set_rail_hidden(rail.hidden);
            match preset {
                PresetRecord::Keep => {}
                PresetRecord::Set(name) => gui.set_layout_preset(Some(name)),
                PresetRecord::Clear => gui.set_layout_preset(None),
            }
        };
        let result = match self {
            #[cfg(test)]
            LayoutStore::Off => return,
            LayoutStore::UserConfig => AppSettings::update_gui(edit),
            #[cfg(test)]
            LayoutStore::File(path) => AppSettings::update_gui_at(path.clone(), edit),
        };
        if let Err(e) = result {
            tracing::warn!("could not save the [gui] layout: {e}");
            eprintln!("kimun-gui: could not save the [gui] layout: {e}");
        }
    }

    /// Write `settings` to the store as the TUI saves its preferences, the
    /// `[gui]` table on disk kept (`AppSettings::save_to_disk`) but for the
    /// keys `settings` changed from `gui_before` (a desktop-only preference
    /// such as `reduce_motion`), which land; a config that exists but does
    /// not parse is left alone and reported. Blocking: run it off the UI
    /// thread ([`Workspace::persist_settings`]).
    fn save_settings(
        &self,
        settings: &AppSettings,
        gui_before: &kimun_notes::settings::gui::GuiSettings,
    ) -> Result<(), String> {
        let edit = |gui: &mut kimun_notes::settings::gui::GuiSettings| {
            gui.apply_changes(gui_before, &settings.gui)
        };
        let result = match self {
            #[cfg(test)]
            LayoutStore::Off => return Ok(()),
            LayoutStore::UserConfig => settings.save_to_disk_if_parsable_with_gui(edit),
            #[cfg(test)]
            LayoutStore::File(path) => {
                let mut settings = settings.clone();
                settings.config_file = Some(path.clone());
                settings.save_to_disk_if_parsable_with_gui(edit)
            }
        };
        result.map_err(|e| e.to_string())
    }
}

/// Index of `side` in the per-side arrays.
fn side_ix(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
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

/// The element id of the slot showing `view` (`slot-files`, ...): a view
/// is shown in at most one slot, so its id names the slot.
pub(crate) fn slot_id(view: DrawerView) -> SharedString {
    format!("slot-{}", view.id()).into()
}

/// The element id of the editor Panel's empty state.
pub(crate) const EMPTY_STATE_ID: &str = "empty-state";

/// The window's root view. See the module docs.
pub(crate) struct Workspace {
    pub(crate) layout: LayoutTree<DrawerView>,
    /// The state of each editor group (editor, note session, attachment).
    pub(crate) groups: HashMap<GroupId, group::GroupState>,
    rail_focus: FocusHandle,
    /// Each slot's handle: the slot's scope, and the keyboard target of a
    /// slot whose view has no focusable content of its own (or that shows
    /// no view).
    slot_focus: HashMap<SlotId, FocusHandle>,
    /// The slot that last held the keyboard ([`Self::apply_focus`]),
    /// where the rail places a view (ADR-0054 §3). Session only; cleared
    /// when that slot closes.
    last_slot: Option<SlotId>,
    /// The views a side toggle hid (left, right), shown again when that
    /// side is toggled back on. Session only.
    side_memory: [Vec<DrawerView>; 2],
    /// The leaves the window had no room for at the last frame
    /// ([`Self::fit`]): not drawn, not focusable. Session only, refitted
    /// every frame; the tree is never changed by it.
    collapsed: HashSet<Leaf>,
    /// The title bar's height in px as last drawn (0 without one): what
    /// the tree's height is fitted against.
    title_bar_height: Rc<std::cell::Cell<f64>>,
    /// The drawn extent of the two cells each divider sits between, keyed
    /// by the divider's leaf ([`tree_view`]), measured as the tree is laid
    /// out: what a drag turns the pointer into a share of.
    divider_cells: tree_view::DividerCells,
    /// A divider drag is in progress (saved when it ends).
    resizing: bool,
    /// How far from its boundary the divider press landed (px along the
    /// drag's axis): a drag moves the boundary by the pointer's travel,
    /// without jumping to it.
    press_offset: f64,
    /// The rail cell the keyboard is on (an index into
    /// [`Self::rail_views`]).
    rail_cursor: usize,
    store: LayoutStore,
    /// Background writes of the layout to `store`, the newest only.
    layout_writer: layout_writer::LayoutWriter,
    /// The layout presets as `layouts.toml` last loaded ([`presets`]).
    presets: BTreeMap<String, ChildSpec>,
    /// The preset the live layout was last applied from, as `[gui]`
    /// records it: what "Reset to preset" applies again.
    layout_preset: Option<String>,
    /// The `rail` each loaded preset sets, by preset name (ADR-0055 §2).
    preset_rails: BTreeMap<String, RailPosition>,
    /// The window edge the rail sits on (ADR-0055), as `[gui]` records it.
    rail: RailPosition,
    /// Whether the rail is hidden (ADR-0055 §8), as `[gui]` records it.
    rail_hidden: bool,
    /// Why the editor Panel shows no editor (no workspace, or it would not
    /// open). `None`: the editor.
    empty: Option<EmptyState>,
    /// What the Footer shows.
    pub(crate) status: Status,
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
    /// The close in progress is a quit (Cmd+Q, the palette's quit, the leader's
    /// quit), not the window's close button. On macOS only a quit ends the app:
    /// closing the window leaves it running in the dock, to open again from there.
    /// Everywhere else closing the window ends the app anyway.
    pub(crate) quit_on_close: bool,
    /// This is macOS: the app lives on in the dock when its window closes. A field,
    /// not a `cfg!` at each use, so the tests run both behaviors on any platform.
    pub(crate) stays_in_dock: bool,
    /// How many times the app was asked to end (the test platform's quit is a no-op).
    #[cfg(test)]
    pub(crate) quits: usize,
    /// How many times a window close waited for the layout's last write.
    #[cfg(test)]
    pub(crate) layout_flushes: usize,
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
    /// The newer server release the server last reported, so the same news
    /// is told once.
    server_update: Option<kimun_notes::server_client::sync::ServerUpdate>,
    /// The vault the running server sync belongs to; `server_update` is
    /// forgotten only when this changes.
    rag_sync_root: Option<std::path::PathBuf>,
    rag_caps: Caps,
    /// The server sync, while one runs: at most one.
    rag_sync: TaskSlot,
    /// The link lookup under way (a newer follow replaces it).
    following: Option<Task<()>>,
    /// Hands URLs, images and attachments to the system's default program.
    pub(crate) opener: follow::Opener,
    /// Keeps the Footer's dirty state in step with the first group's
    /// editor (a group made later holds its own, [`group::GroupState`]).
    _editor_watch: Subscription,
    _on_quit: Option<Subscription>,
    /// Resolves the workspace's chords before a slot's own key bindings
    /// can take them ([`Self::intercept_shell_key`]).
    _shell_keys: Subscription,
    /// The view or group being dragged onto a drop zone, while one is
    /// ([`Self::live_drag`], [`Self::dragged_group`]).
    drag: Option<LiveDrag>,
    /// What a drop on each zone would do, worked out while a view or a
    /// group is dragged ([`Self::refresh_drop_outcomes`]), once per drag;
    /// empty otherwise.
    drop_outcomes: HashMap<drop_zones::DropTarget, drop_zones::DropOutcome>,
    /// The drag and window size [`Self::drop_outcomes`] were worked out for.
    drop_outcomes_for: Option<(Dragged, gpui_kit::Size<gpui_kit::Pixels>)>,
    /// The zone the drag is over, for its label ([`Self::zone_tag`]).
    hovered_zone: Option<drop_zones::DropTarget>,
    /// What the window's one modal slot holds, while the kit says a dialog
    /// is open ([`modal`]).
    pub(crate) modal: Option<ModalKind>,
    /// The note menu, while it is open ([`note_menu`]).
    pub(crate) note_menu: Option<note_menu::OpenNoteMenu>,
    /// Where each drawn group was last laid out ([`note_menu::GroupBounds`]).
    group_bounds: note_menu::GroupBounds,
    /// The command palette's view while it is open (tests read its rows).
    pub(crate) palette: Option<WeakEntity<Palette>>,
    /// The view chooser while it is open: split, side or move
    /// ([`view_chooser`]).
    view_chooser: Option<view_chooser::OpenViewChooser>,
    /// The shared settings (the leader tree, the vault to open, what
    /// [`reopen`] persists): `None` before [`Self::start`] and in the tests
    /// that never call it.
    settings: Option<SharedSettings>,
    /// The first group's editor's requests that need the window (vim `:`).
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
    /// The close of a tab or a group under way: the flush of its groups,
    /// then their removal ([`tab_strip`]). One at a time; it holds the
    /// notes as a file operation does.
    groups_closing: Option<(tab_strip::CloseTarget, Task<()>)>,
    /// The tab strip's sideways scroll ([`tab_strip`]).
    tab_scroll: ScrollHandle,
    /// The tab whose pill the pointer is over: it shows its close button
    /// ([`tab_strip`]).
    tab_hover: Option<kimun_notes::logic::editor_region::TabId>,
    /// Why a group shows no note: the one it held was deleted or moved
    /// away. Only that group says so; cleared when a note opens there.
    pub(crate) note_gone: Option<(GroupId, EmptyState)>,
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
    /// The workspace switcher, while open ([`switcher`]). Tests reach the
    /// view through it; the window never reads it back (the modal slot holds
    /// the view).
    #[cfg(test)]
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
    /// The view ASK (the Thread), when the window has a vault ([`ask`]),
    /// and its events. It lives as long as the vault session, shown or not.
    pub(crate) ask: Option<Entity<ask::AskView>>,
    _ask_events: Option<Subscription>,
    /// The sources part folded into ASK (under the selected turn).
    pub(crate) ask_sources: Option<Entity<ask_sources::SourcesView>>,
    /// Which turn the sources part mirrors: the one record of it.
    ask_flow: AskFlow,
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
    /// The workspace's session file: the tabs and groups recorded as they
    /// change, and restored when the workspace opens ([`session`]).
    session: session::SessionHost,
}

/// The labels the window derives from the active keymap's chords.
struct ChordLabels {
    /// The `Leader` chord (`Ctrl+Shift+B`), or a stand-in when unbound.
    leader: SharedString,
    /// What the editor Panel says with no note open: how to find one.
    no_note_hint: String,
    /// The `FollowLink` chord: the attachment view's `Open externally`.
    follow_link: Option<KeyInput>,
    /// What the Footer says while zoomed: `Zoomed · {chord} to restore`.
    zoom_hint: SharedString,
}

impl ChordLabels {
    fn new(cx: &App) -> Self {
        let chord = |action: ActionShortcuts| keymap::chord_label(cx, &action);
        Self {
            leader: leader::gateway_label(chord(ActionShortcuts::Leader)),
            no_note_hint: match chord(ActionShortcuts::OpenNote) {
                Some(finder) => format!("Press {finder} to find or create one."),
                None => "Open one from FILES.".to_string(),
            },
            follow_link: keymap::active(cx).chord_for(&ActionShortcuts::FollowLink),
            zoom_hint: match chord(ActionShortcuts::ToggleZoom) {
                Some(zoom) => format!("Zoomed · {zoom} to restore").into(),
                None => "Zoomed".into(),
            },
        }
    }
}

impl Workspace {
    pub(crate) fn new(
        editor: Entity<EditorView>,
        theme: Theme,
        layout: LayoutTree<DrawerView>,
        store: LayoutStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The one theme every view of the window draws in.
        theme::set_current(cx, theme);
        // The rail's place and visibility, as the config records them: read
        // here so a window opens as it was left, whoever builds it.
        let recorded_gui = store.recorded_gui();
        let rail_cursor = layout
            .slots()
            .into_iter()
            .find_map(|s| layout.shown(s))
            .and_then(|v| {
                shell_layout::rail_views(Caps::default())
                    .iter()
                    .position(|r| *r == v)
            })
            .unwrap_or(0);
        // Every edit notifies the editor; the Footer only redraws when the
        // dirty state flips.
        let editor_watch = cx.observe(&editor, Self::editor_changed);
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
            if let Some(view) = this.active_state().attachment.as_ref() {
                view.update(cx, |view, cx| view.set_open_chord(chord, cx));
            }
            cx.notify();
        });
        let config = cx.new(config::ConfigView::new);
        let config_events = cx.subscribe_in(&config, window, Self::on_config_event);
        let mut this = Self {
            groups: HashMap::from([(layout.active_group(), group::GroupState::new(editor))]),
            layout,
            rail_focus: cx.focus_handle(),
            slot_focus: HashMap::new(),
            last_slot: None,
            side_memory: [Vec::new(), Vec::new()],
            collapsed: HashSet::new(),
            title_bar_height: Default::default(),
            divider_cells: Default::default(),
            resizing: false,
            press_offset: 0.0,
            rail_cursor,
            layout_writer: layout_writer::LayoutWriter::new(store.clone()),
            store,
            presets: BTreeMap::new(),
            layout_preset: None,
            preset_rails: BTreeMap::new(),
            rail: recorded_gui.rail(),
            rail_hidden: recorded_gui.rail_hidden(),
            empty: None,
            status: Status::default(),
            tx: None,
            _messages: None,
            action_timers: HashMap::new(),
            #[cfg(test)]
            flashes: Vec::new(),
            toasts: VecDeque::new(),
            closing: Closing::Open,
            quit_on_close: false,
            // The tests run the macOS behavior (the window goes, the app stays) unless
            // one says otherwise: most of them are about the window closing.
            stays_in_dock: cfg!(any(test, target_os = "macos")),
            #[cfg(test)]
            quits: 0,
            #[cfg(test)]
            layout_flushes: 0,
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
            server_update: None,
            rag_sync_root: None,
            rag_caps: Caps::default(),
            rag_sync: TaskSlot::default(),
            following: None,
            opener: follow::system_opener(),
            _editor_watch: editor_watch,
            _on_quit: None,
            _shell_keys: shell_keys,
            drag: None,
            drop_outcomes: HashMap::new(),
            drop_outcomes_for: None,
            hovered_zone: None,
            modal: None,
            note_menu: None,
            group_bounds: Default::default(),
            palette: None,
            view_chooser: None,
            settings: None,
            _editor_events: editor_events,
            leader: LeaderState::default(),
            chord_labels: ChordLabels::new(cx),
            _keymap_watch: keymap_watch,
            #[cfg(test)]
            file_dialog: None,
            file_status: None,
            file_task: None,
            groups_closing: None,
            tab_scroll: ScrollHandle::new(),
            tab_hover: None,
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
            #[cfg(test)]
            switcher: None,
            save_search: None,
            save_task: None,
            search_flush: None,
            properties_opening: None,
            property_write: None,
            note_reload: None,
            ask: None,
            _ask_events: None,
            ask_sources: None,
            ask_flow: AskFlow::new(),
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
            session: session::SessionHost::default(),
        };
        this.watch_session(cx);
        this.sync_slot_handles(cx);
        this.load_presets(cx);
        this
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
        editor: &Entity<EditorView>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::OpenPalette => self.open_palette(window, cx),
            // Right-click without a selection: the note menu at the
            // pointer, for the group whose editor it is (the press already
            // made it the active group).
            EditorEvent::ContextMenu { at } => {
                let group = self
                    .groups
                    .iter()
                    .find(|(_, state)| &state.editor == editor)
                    .map(|(group, _)| *group);
                if let Some(group) = group {
                    self.open_note_menu(group, *at, window, cx);
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
        // Every group's editor, not only the one shown.
        for state in self.groups.values() {
            state
                .editor
                .update(cx, |view, _| view.set_channel(tx.clone()));
        }
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
        match &start {
            Start::Vault(vault) => self.set_workspace_name(vault.name.as_str(), window),
            Start::OpenFailed(_) => self.set_no_workspace(true, window),
            Start::NoWorkspace => self.set_no_workspace(false, window),
        }
        self.empty = startup::empty_state(&start);
        // No workspace: the setup assistant makes one. A vault entered
        // leaves nothing for one still up to set up.
        match start {
            Start::NoWorkspace => self.open_onboarding(window, cx),
            Start::Vault(_) => self.drop_onboarding(),
            Start::OpenFailed(_) => {}
        }
        if let Start::Vault(VaultStart { vault, name, first }) = start {
            // Every group gets a note session over the vault, on no note
            // yet: the groups a vault change left stay, empty.
            for state in self.groups.values_mut() {
                state.notes = Some(NoteHost::new(NoteSession::new(
                    vault.clone(),
                    settings.clone(),
                    VaultPath::root(),
                )));
                // The editor completes `[[` and `#` from this vault's notes and tags.
                state.editor.update(cx, |view, _| {
                    view.set_suggestions(Some(std::sync::Arc::new(
                        kimun_notes::logic::autocomplete::VaultSuggestions {
                            vault: vault.clone(),
                        },
                    )));
                });
            }
            self.refresh_git();
            let session_file = {
                let settings = settings.read().unwrap_or_else(|e| e.into_inner());
                let session_file = settings.session_for(&name);
                self.start_files(vault.clone(), name, &settings, tx.clone(), window, cx);
                session_file
            };
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
            // The arrangement is recorded from now on ([`session`]). A
            // start with no note (or a refused one) and a stored record
            // lands on that record when it has a note left to open, else
            // (no argument only) on the history; opens wait for it. With no
            // record the history restores as it always did, opens going
            // through at once. A note on the command line opens alone, and
            // the record stays as it is until a structural change.
            // (The record is a small file of the app's own: read here, so
            // opens know at once whether to wait.)
            let record = matches!(first, FirstNote::Restore | FirstNote::Rejected(_))
                .then(|| session_file.load())
                .flatten();
            let restoring = record.is_some();
            let start = match &first {
                FirstNote::Note(_) => session::SessionStart::Launch,
                _ if restoring => session::SessionStart::Restore { history: restore },
                // A file there that did not load (a newer build's, a read
                // that failed) is kept until a real change.
                _ if session_file.exists() => {
                    session::SessionStart::Unreadable { history: restore }
                }
                _ => session::SessionStart::Fresh,
            };
            self.begin_session(session_file, start);
            let job = tokio::spawn(startup::index_and_restore(
                vault,
                settings,
                restore,
                record,
                tx.clone(),
            ));
            if restoring {
                self.await_session_restore(job, cx);
            }
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
                for state in self.groups.values() {
                    state.editor.update(cx, |_, cx| cx.notify());
                }
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
                    // backlinks against the old one; so did LINKS, and
                    // FILES its folders' note counts.
                    Ok(_) => {
                        self.refresh_note_counts();
                        self.invalidate_views(cx);
                        self.files_reload_counts(cx);
                    }
                    Err(e) => self.flash_error(format!("Indexing failed: {e}"), cx),
                }
            }
            AppMessage::VaultConflict(details) => {
                self.status.indexing = false;
                self.status.read_only = true;
                // Nothing is written from here on: no tick to run.
                for state in self.groups.values_mut() {
                    if let Some(host) = state.notes.as_mut() {
                        host.session.stop_autosave();
                    }
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
                // "opening your last note". Unless the session record is
                // still restoring: the open waits for it, and so does the
                // Panel's word that something is on its way.
                if !self.session_restoring() {
                    self.restore_pending = false;
                    self.first_open_pending = false;
                }
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
                // Every result changed the working tree.
                self.refresh_git();
            }
            AppMessage::GitStatusLoaded { root, status } => self.git_loaded(&root, status, cx),
            // The Footer's counts for the open note.
            message @ (AppMessage::BacklinkCountLoaded { .. }
            | AppMessage::PropertyCountLoaded { .. }) => self.note_count_loaded(message),
            AppMessage::Autosave => self.autosave(cx),
            AppMessage::AutosaveOf(tick) => self.autosave_tick(tick, cx),
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
            // The server's own update notice (the TUI's footer hint): no
            // surface here ([`server`]).
            AppMessage::ServerUpdate(update) => self.on_server_update(update, cx),
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
        self.active_state()
            .notes
            .as_ref()
            .map(|h| h.session.vault().clone())
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
        self.flash_actionable(text.into(), false, action, cx);
    }

    /// [`Self::flash_with_action`] for a problem: an error toast with a button
    /// (the update flow's `Retry`).
    pub(crate) fn flash_error_with_action(
        &mut self,
        text: impl Into<SharedString>,
        action: ToastAction,
        cx: &mut Context<Self>,
    ) {
        self.flash_actionable(text.into(), true, action, cx);
    }

    fn flash_actionable(
        &mut self,
        text: SharedString,
        error: bool,
        action: ToastAction,
        cx: &mut Context<Self>,
    ) {
        self.show_flash(text.clone(), error, Some(action), cx);
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

    /// The views the slots of `side` show, in reading order (tests).
    #[cfg(test)]
    pub(crate) fn shown_on(&self, side: Side) -> Vec<DrawerView> {
        self.side_slots(side)
            .into_iter()
            .filter_map(|s| self.layout.shown(s))
            .collect()
    }

    /// Whether a slot of `side` is drawn (tests).
    #[cfg(test)]
    pub(crate) fn side_drawn(&self, side: Side) -> bool {
        self.side_slots(side)
            .into_iter()
            .any(|s| self.slot_drawn(s))
    }

    /// Move `view` into the first slot of `side`, shown there (tests).
    #[cfg(test)]
    pub(crate) fn move_to_side(&mut self, view: DrawerView, side: Side) {
        let slot = self.side_slots(side)[0];
        self.layout.move_view(view, slot);
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
    fn save_layout(&mut self, cx: &mut Context<Self>) {
        self.save_layout_with(PresetRecord::Keep, cx);
    }

    /// [`Self::save_layout`], recording `name` as the preset the layout
    /// was applied from ([`presets`]).
    fn save_layout_as_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        self.save_layout_with(PresetRecord::Set(name), cx);
    }

    /// [`Self::save_layout`], removing the recorded preset: the layout
    /// comes from none ("Reset layout to default").
    fn save_layout_clearing_preset(&mut self, cx: &mut Context<Self>) {
        self.save_layout_with(PresetRecord::Clear, cx);
    }

    /// The rail's place and visibility as they save.
    fn rail_state(&self) -> RailState {
        RailState {
            position: self.rail,
            hidden: self.rail_hidden,
        }
    }

    fn save_layout_with(&mut self, preset: PresetRecord<'_>, cx: &mut Context<Self>) {
        self.tidy_layout(cx);
        self.layout_writer
            .submit(self.layout.clone(), preset, self.rail_state());
        // The shared settings follow what the layout save writes for the
        // rail, so Preferences start from the position on screen and a
        // later save of theirs does not move it back.
        if let Some(settings) = &self.settings {
            let mut settings = settings.write().unwrap_or_else(|e| e.into_inner());
            settings.gui.set_rail(self.rail);
            settings.gui.set_rail_hidden(self.rail_hidden);
        }
        self.refresh_views(cx);
    }

    /// Bring the session state in step with the tree: every slot has its
    /// focus handle (and no closed slot keeps one), and focus never rests
    /// on a slot that is not drawn.
    fn tidy_layout(&mut self, cx: &mut Context<Self>) {
        self.sync_slot_handles(cx);
        let slots = self.layout.slots();
        self.last_slot = self.last_slot.filter(|s| slots.contains(s));
        self.fix_focus();
    }

    /// One focus handle per slot of the tree.
    fn sync_slot_handles(&mut self, cx: &mut Context<Self>) {
        let slots = self.layout.slots();
        self.slot_focus.retain(|s, _| slots.contains(s));
        for slot in slots {
            self.slot_focus
                .entry(slot)
                .or_insert_with(|| cx.focus_handle());
        }
    }

    /// Keep the drawer views in step with the layout and the open note:
    /// each reloads (LINKS, FIND, TAGS) or re-reads its headings (OUTLINE)
    /// only while shown, when revealed or when what it lists for changed.
    /// Cheap otherwise.
    pub(crate) fn refresh_views(&mut self, cx: &mut Context<Self>) {
        if let Some(links) = self.links.clone() {
            let shown = self.is_shown(DrawerView::Links);
            let note = self
                .active_state()
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
            // Suggestions for the field may have landed.
            find.update(cx, |_, cx| cx.notify());
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

    /// The view in the focused slot, if a slot has the keyboard.
    fn focused_view(&self) -> Option<DrawerView> {
        self.focused_slot().and_then(|s| self.layout.shown(s))
    }

    /// The slot that has the keyboard, if one has.
    pub(crate) fn focused_slot(&self) -> Option<SlotId> {
        match self.layout.focused() {
            Focus::Leaf(Leaf::Slot(s)) => Some(s),
            _ => None,
        }
    }

    /// Whether `opener` (the panel that had the keyboard) is a slot whose
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
            // The sources part: the source's path (the TUI's yank). The
            // chord is the composer's or the filter's own copy elsewhere.
            Some(DrawerView::Ask) if !chord || self.sources_list_focused(window, cx) => {
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

    /// Whether `view` is shown in a slot: its rail cell reads as selected.
    pub(crate) fn is_shown(&self, view: DrawerView) -> bool {
        self.layout
            .view_slot(&view)
            .is_some_and(|slot| self.layout.shown(slot) == Some(view))
    }

    /// Whether `slot` is drawn: it shows a view.
    pub(crate) fn slot_drawn(&self, slot: SlotId) -> bool {
        self.layout.shown(slot).is_some()
    }

    /// Whether `leaf` is drawn: the editor, and the drawn slots.
    fn leaf_drawn(&self, leaf: Leaf) -> bool {
        match leaf {
            Leaf::Editor => true,
            Leaf::Slot(slot) => self.slot_drawn(slot),
        }
    }

    /// The slots of `side` in reading order ([`LayoutTree::side_slots`],
    /// Ruling 13).
    pub(crate) fn side_slots(&self, side: Side) -> Vec<SlotId> {
        self.layout.side_slots(side)
    }

    /// The side `slot` reads as for the key contexts and the palette.
    fn slot_side(&self, slot: SlotId) -> Side {
        self.layout.slot_side(slot)
    }

    /// What has the keyboard, as the keymap and the palette read it: the
    /// rail, a slot (on its side of the active group), or the editor.
    pub(crate) fn focused_panel(&self) -> Panel {
        match self.layout.focused() {
            Focus::Rail => Panel::Rail,
            Focus::Leaf(Leaf::Editor) => Panel::Editor,
            Focus::Leaf(Leaf::Slot(slot)) => Panel::Drawer(self.slot_side(slot)),
        }
    }

    /// Focus `panel` in the model: the rail, the active group, or the first
    /// drawn slot of that side (nothing changes when it has none).
    fn set_focus_panel(&mut self, panel: Panel) {
        let focus = match panel {
            // A hidden rail takes no keyboard focus (ADR-0055 §9): a modal
            // that closes gives it back to the rail it was opened from.
            Panel::Rail if !self.rail_hidden => Focus::Rail,
            Panel::Rail | Panel::Editor => Focus::Leaf(Leaf::Editor),
            Panel::Drawer(side) => match self
                .side_slots(side)
                .into_iter()
                .find(|s| self.leaf_visible(Leaf::Slot(*s)))
            {
                Some(slot) => Focus::Leaf(Leaf::Slot(slot)),
                None => return,
            },
        };
        self.layout.focus(focus);
    }

    /// Focus never rests on a slot that is not on screen (not drawn, or
    /// collapsed): it goes to the first visible slot of that side, else to
    /// the editor.
    fn fix_focus(&mut self) {
        let Some(slot) = self.focused_slot() else {
            return;
        };
        if self.leaf_visible(Leaf::Slot(slot)) {
            return;
        }
        let side = self.slot_side(slot);
        match self
            .side_slots(side)
            .into_iter()
            .find(|s| self.leaf_visible(Leaf::Slot(*s)))
        {
            Some(other) => self.layout.focus(Focus::Leaf(Leaf::Slot(other))),
            None => self.layout.focus(Focus::Leaf(Leaf::Editor)),
        };
    }

    /// Show `view` where it is placed (one placed nowhere: in its home
    /// slot, `view_registry`'s `home_right`) and return its slot.
    fn open_view(&mut self, view: DrawerView) -> Option<SlotId> {
        let home_right = view_registry::spec(view).home_right;
        self.layout.show_view(&view, home_right)
    }

    /// [`Self::open_view`], then give its slot the keyboard (in the model;
    /// [`Self::apply_focus`] hands GPUI focus over).
    pub(crate) fn open_view_focused(&mut self, view: DrawerView) -> Option<SlotId> {
        let slot = self.open_view(view)?;
        self.focus_slot_ending_zoom(slot);
        Some(slot)
    }

    /// Give `slot` the keyboard. A zoom on any other leaf ends: the user
    /// asked to see this slot, which the zoom would hide (ADR-0053).
    fn focus_slot_ending_zoom(&mut self, slot: SlotId) {
        let leaf = Leaf::Slot(slot);
        if self.layout.zoomed().is_some_and(|z| z != leaf) {
            self.layout.zoom(None);
        }
        self.layout.focus(Focus::Leaf(leaf));
    }

    /// `slot`'s focus handle (the rail's while it has none yet: a slot is
    /// given one as soon as the tree changes, [`Self::tidy_layout`]).
    fn slot_handle(&self, slot: SlotId) -> FocusHandle {
        self.slot_focus
            .get(&slot)
            .cloned()
            .unwrap_or_else(|| self.rail_focus.clone())
    }

    /// Where the keyboard goes in `view`'s slot: FILES' and TAGS' filters,
    /// so typing filters; FIND's query field; LINKS' and OUTLINE's lists;
    /// the CFG view itself; the slot itself (`slot`) for a view that needs
    /// a workspace while none is open.
    fn view_handle(&self, view: DrawerView, slot: SlotId, cx: &App) -> FocusHandle {
        let handle = match view {
            DrawerView::Files => self.files.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Links => self.links.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Outline => self.outline.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Find => self.find.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Tags => self.tags.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Semantic => self.semantic.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Ask => self.ask.as_ref().map(|v| v.read(cx).focus_handle(cx)),
            DrawerView::Config => Some(self.config.read(cx).focus_handle(cx)),
        };
        handle.unwrap_or_else(|| self.slot_handle(slot))
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

    fn handle_for(&self, focus: Focus, cx: &App) -> FocusHandle {
        match focus {
            Focus::Rail => self.rail_focus.clone(),
            // The slot's shown view, else the slot itself.
            Focus::Leaf(Leaf::Slot(slot)) => match self.layout.shown(slot) {
                Some(view) => self.view_handle(view, slot, cx),
                None => self.slot_handle(slot),
            },
            // The attachment view while one shows, else the editor.
            Focus::Leaf(Leaf::Editor) => match self.active_state().attachment.as_ref() {
                Some(view) => view.read(cx).focus_handle(cx),
                None => self.active_state().editor.read(cx).focus_handle(cx),
            },
        }
    }

    /// Give the keyboard to the leaf the layout says is focused. Not
    /// while a modal is open: it has the keyboard, and gives it back to its
    /// opener when it closes ([`modal`]). A focused slot is remembered
    /// as the last one to hold the keyboard.
    pub(crate) fn apply_focus(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(slot) = self.focused_slot() {
            self.last_slot = Some(slot);
        }
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
        // The open note menu keeps the keyboard; the handle is where it
        // goes back.
        if self.keep_focus_on_note_menu(handle.clone(), window, cx) {
            return;
        }
        window.focus(&handle, cx);
    }

    /// Focus `panel` (a press anywhere in the rail or the editor).
    fn focus_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.set_focus_panel(panel);
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Focus `slot` (a press anywhere in it).
    fn focus_slot(&mut self, slot: SlotId, window: &mut Window, cx: &mut Context<Self>) {
        self.layout.focus(Focus::Leaf(Leaf::Slot(slot)));
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Toggle `side` (Ruling 13): the slots left (right) of the active
    /// group. With any of them drawn, every view shown there is hidden and
    /// remembered for this session; the keyboard, if it was there, goes
    /// back to the document. Otherwise the
    /// remembered views show again (the first of them takes the keyboard,
    /// as the TUI's toggle does); with none remembered, the first slot of
    /// the side holding a placed view shows its first one; with nothing
    /// placed there, a chooser asks which view the side's first slot shows
    /// ([`Self::open_side_chooser`], ADR-0054 §5): nothing opens until one
    /// is picked. Persisted: hidden views keep their placement, no
    /// visibility flag is written.
    fn toggle_side(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let slots = self.side_slots(side);
        if slots.is_empty() {
            self.flash("No slot on that side", cx);
            return;
        }
        if slots.iter().any(|s| self.slot_drawn(*s)) {
            let had_focus = self.focused_slot().is_some_and(|s| slots.contains(&s));
            let mut hidden = Vec::new();
            for slot in &slots {
                if let Some(view) = self.layout.shown(*slot) {
                    self.layout.hide_view(&view);
                    hidden.push(view);
                }
            }
            self.side_memory[side_ix(side)] = hidden;
            if had_focus {
                self.set_focus_panel(Panel::Editor);
            }
        } else {
            let remembered = self.side_memory[side_ix(side)].clone();
            let mut first = None;
            for view in remembered {
                if let Some(slot) = self.layout.view_slot(&view)
                    && slots.contains(&slot)
                    && self.layout.shown(slot).is_none()
                {
                    self.layout.show_view(&view, false);
                    first.get_or_insert(slot);
                }
            }
            if first.is_none()
                && let Some((slot, view)) = slots.iter().find_map(|s| {
                    self.layout
                        .slot_views(*s)
                        .into_iter()
                        .next()
                        .map(|v| (*s, v))
                })
            {
                self.layout.show_view(&view, false);
                first = Some(slot);
            }
            match first {
                Some(slot) => {
                    self.side_memory[side_ix(side)].clear();
                    self.focus_slot_ending_zoom(slot)
                }
                None => {
                    // Nothing to show: the chooser has the keyboard, and
                    // nothing changes until it picks. Under a dialog it
                    // cannot open: say so, keeping the side's memory.
                    if modal::dialog_open(window, cx) {
                        self.flash("Close the dialog first", cx);
                    } else if let Some(slot) = slots.first() {
                        self.side_memory[side_ix(side)].clear();
                        self.open_side_chooser(*slot, window, cx);
                    }
                    return;
                }
            }
            self.ensure_files_listing(cx);
        }
        self.apply_focus(window, cx);
        self.save_layout(cx);
        cx.notify();
    }

    /// A rail cell was activated. A shown view is hidden, keeping its
    /// placement: its slot is no longer drawn, and another drawn slot on
    /// its side takes the keyboard (the rail keeps it otherwise). Any other
    /// view is shown where it is placed (a view placed nowhere: in its home
    /// slot), hiding the view sharing its slot, and takes the keyboard.
    /// Persisted.
    fn activate_rail_view(
        &mut self,
        view: DrawerView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.place_rail_cursor(Some(view));
        // A view shown in a slot the zoom hides: the user asked to see it,
        // so zoom ends and its slot takes the keyboard (it is not hidden).
        if let Some(zoomed) = self.layout.zoomed()
            && let Some(slot) = self.layout.view_slot(&view)
            && self.is_shown(view)
            && zoomed != Leaf::Slot(slot)
        {
            self.layout.zoom(None);
            self.layout.focus(Focus::Leaf(Leaf::Slot(slot)));
            self.apply_focus(window, cx);
            cx.notify();
            return;
        }
        if self.is_shown(view) {
            if let Some(slot) = self.layout.hide_view(&view) {
                let side = self.slot_side(slot);
                if let Some(other) = self
                    .side_slots(side)
                    .into_iter()
                    .find(|s| self.leaf_visible(Leaf::Slot(*s)))
                {
                    self.layout.focus(Focus::Leaf(Leaf::Slot(other)));
                }
            }
        } else {
            self.open_view_focused(view);
            self.ensure_files_listing(cx);
        }
        self.tidy_layout(cx);
        self.apply_focus(window, cx);
        self.save_layout(cx);
        cx.notify();
    }

    /// The view a move acts on: the focused slot's, or the one under the
    /// rail's cursor (placed or not). `None` from the editor.
    fn arranged_view(&self) -> Option<DrawerView> {
        match self.layout.focused() {
            Focus::Leaf(Leaf::Slot(slot)) => self.layout.shown(slot),
            Focus::Rail => self.rail_cursor_view(),
            Focus::Leaf(Leaf::Editor) => None,
        }
    }

    /// The slot that touches the rail's edge, drawn or not
    /// ([`LayoutTree::slot_beside_rail`]): where a move from the rail lands.
    fn slot_beside_rail(&self) -> Option<SlotId> {
        self.layout.slot_beside_rail(self.rail)
    }

    /// `MoveViewToNeighbour(Left / Right)`: move the focused view to the
    /// nearest slot toward `dir` ([`LayoutTree::move_view_dir`]) and show
    /// it there, replacing the view shown there (which stays placed,
    /// hidden); the keyboard follows it. Nothing that way: the view goes
    /// to a new slot on that side when its slot holds other views
    /// ([`Self::spawn_beside`]); alone there, handled, nothing moves. A
    /// slot the move empties closes ([`Self::close_if_vacated`]).
    /// Persisted.
    ///
    /// From the rail the move is spatial: the direction from the rail into
    /// the layout moves and shows the cursor's view in the slot beside the
    /// rail, and any other is not handled (ADR-0055 §6; the directions along
    /// the rail's axis are [`Self::place_from_rail`]'s). The rail keeps the
    /// keyboard.
    fn move_view(&mut self, dir: Dir, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.arranged_view() else {
            return false;
        };
        let from = self.layout.view_slot(&view);
        match self.layout.focused() {
            Focus::Rail => {
                let target = if dir == self.rail.toward() {
                    self.slot_beside_rail()
                } else {
                    None
                };
                let Some(target) = target else {
                    return false;
                };
                self.layout.move_view(view, target);
            }
            _ => {
                if self.layout.move_view_dir(&view, dir) {
                    if self.focused_slot() == from
                        && let Some(to) = self.layout.view_slot(&view)
                    {
                        self.layout.focus(Focus::Leaf(Leaf::Slot(to)));
                    }
                } else if let Some(from) = from
                    && self.place(view, Placement::Beside(from, dir), window, cx)
                {
                    return true;
                }
            }
        }
        if let Some(from) = from {
            self.close_if_vacated(from, view);
        }
        self.ensure_files_listing(cx);
        self.tidy_layout(cx);
        self.apply_focus(window, cx);
        self.save_layout(cx);
        true
    }

    /// A move toward nothing (ADR-0054 §2, §8): put `view` into a new slot
    /// on the `dir` side of `target` ([`LayoutTree::place_beside`]) when
    /// nothing is zoomed. The slot the view leaves, if any, closes when
    /// that emptied it, else the first of its views shows in the view's
    /// place ([`Self::close_if_vacated`]), so it stays drawn. A view alone
    /// in `target` has nothing to leave behind there: [`Spawn::Refused`],
    /// nothing changes. A view no slot holds (a rail entry, dropped) is placed.
    /// The keyboard follows the view. Persisted. With no room for the new
    /// slot nothing changes and a flash says so ([`Self::place_new_slot`]);
    /// that is [`Spawn::NoRoom`].
    fn spawn_beside(
        &mut self,
        view: DrawerView,
        target: SlotId,
        dir: Dir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Spawn {
        let from = self.layout.view_slot(&view);
        if self.layout.zoomed().is_some()
            || (from == Some(target) && self.layout.slot_views(target).len() < 2)
        {
            return Spawn::Refused;
        }
        self.spawn_at(view, Leaf::Slot(target), dir, window, cx)
    }

    /// Put `view` in a new slot on the `dir` side of `beside`
    /// ([`Self::place_new_slot`]) and give that slot the keyboard; settled
    /// and persisted. [`Spawn::NoRoom`] (nothing changed, a flash said so)
    /// when the window has no room for it; [`Spawn::Refused`] (nothing
    /// changed) when `beside` is not in the tree.
    pub(super) fn spawn_at(
        &mut self,
        view: DrawerView,
        beside: Leaf,
        dir: Dir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Spawn {
        match self.place_new_slot(view, beside, dir, window, cx) {
            Ok(new) => {
                self.layout.focus(Focus::Leaf(Leaf::Slot(new)));
                self.settle_placement(window, cx);
                cx.notify();
                Spawn::Placed
            }
            Err(NoNewSlot::NoRoom) => Spawn::NoRoom,
            Err(NoNewSlot::Refused) => Spawn::Refused,
        }
    }

    /// Every placement that makes a slot goes through here (ADR-0054 §8):
    /// put `view` in a new slot on the `dir` side of `beside`
    /// ([`LayoutTree::place_beside`], or
    /// [`LayoutTree::place_beside_editor`] for the editor region); the slot
    /// the view left closes, or shows its next view
    /// ([`Self::close_if_vacated`]); then the window makes room for the
    /// new slot ([`Self::make_room`]). With no room, the layout is put
    /// back as it was and a flash says `No room for <view> here`. Focus,
    /// settling and saving are the caller's.
    fn place_new_slot(
        &mut self,
        view: DrawerView,
        beside: Leaf,
        dir: Dir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<SlotId, NoNewSlot> {
        let placed = self.try_new_slot(view, beside, dir, window);
        if placed == Err(NoNewSlot::NoRoom) {
            let name = view_registry::spec(view).name;
            self.flash(format!("No room for {name} here"), cx);
        }
        placed
    }

    /// The model steps of [`Self::place_new_slot`], without its flash: on
    /// `Err` the layout is as it was.
    fn try_new_slot(
        &mut self,
        view: DrawerView,
        beside: Leaf,
        dir: Dir,
        window: &Window,
    ) -> Result<SlotId, NoNewSlot> {
        let visible = self.fitting_leaves(window);
        let from = self.layout.view_slot(&view);
        self.try_layout(|ws| {
            let new = match beside {
                Leaf::Slot(slot) => ws.layout.place_beside(view, slot, dir),
                Leaf::Editor => Ok(ws.layout.place_beside_editor(view, dir)),
            }
            .map_err(|_| NoNewSlot::Refused)?;
            if let Some(from) = from {
                ws.close_if_vacated(from, view);
            }
            if ws.make_room(new, &visible, window) {
                Ok(new)
            } else {
                Err(NoNewSlot::NoRoom)
            }
        })
    }

    /// Whether the window has room for `view` in a new slot on the `dir`
    /// side of `beside`: the steps of [`Self::place_new_slot`] run and
    /// undone, the layout as it was either way.
    pub(super) fn new_slot_fits(
        &mut self,
        view: DrawerView,
        beside: Leaf,
        dir: Dir,
        window: &Window,
    ) -> bool {
        let undo = self.layout.clone();
        let fits = self.try_new_slot(view, beside, dir, window).is_ok();
        self.layout = undo;
        fits
    }

    /// Run `edit`, which may change the layout and nothing else of the
    /// workspace; on `Err` the layout is put back as it was. The one
    /// rollback for a multi-step layout change, so a step added to `edit`
    /// later cannot leave a half-made layout behind. Anything `edit` would
    /// change outside `self.layout` is not undone: do it after.
    fn try_layout<T, E>(&mut self, edit: impl FnOnce(&mut Self) -> Result<T, E>) -> Result<T, E> {
        let undo = self.layout.clone();
        let result = edit(self);
        if result.is_err() {
            self.replace_layout(undo);
        }
        result
    }

    /// Put `layout` in place of the live one, keeping the live editor
    /// region: its tabs and groups belong to the window, not to an
    /// arrangement of slots, so a preset or an undo never touches them
    /// (ADR-0058). `groups` therefore stays in step with the layout's
    /// groups, the invariant the `group` accessors rely on.
    pub(super) fn replace_layout(&mut self, mut layout: LayoutTree<DrawerView>) {
        *layout.region_mut() = self.layout.region().clone();
        self.layout = layout;
        debug_assert!(
            self.layout
                .region()
                .all_groups()
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                == self.groups.keys().copied().collect(),
            "groups out of step with the layout"
        );
    }

    /// The shared tail of every placement: settle and persist. Focus is
    /// the caller's: a drop and a chooser move it to the new slot, the
    /// rail keeps it.
    fn settle_placement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_files_listing(cx);
        self.tidy_layout(cx);
        self.apply_focus(window, cx);
        self.save_layout(cx);
    }

    /// After a move of `view` out of `slot` (ADR-0054 §1): close `slot`
    /// when it holds no views any more and is not the last slot on its
    /// side of the editor ([`LayoutTree::vacate_close`]). A zoom on it moves
    /// to `view`'s new slot. True when it closed.
    fn close_if_vacated(&mut self, slot: SlotId, view: DrawerView) -> bool {
        let zoomed = self.layout.zoomed() == Some(Leaf::Slot(slot));
        if self.layout.vacate_close(slot).is_none() {
            // Kept: holding other views, the first shows in the place of
            // the one that left, so the slot stays drawn.
            if self.layout.shown(slot).is_none()
                && let Some(next) = self.layout.slot_views(slot).first().copied()
            {
                self.layout.show_view(&next, false);
            }
            return false;
        }
        if zoomed && let Some(to) = self.layout.view_slot(&view) {
            self.layout.zoom(Some(Leaf::Slot(to)));
        }
        true
    }

    /// Whether another drawn slot shares `slot`'s column (stacked above or
    /// below it).
    fn stacked(&self, slot: SlotId) -> bool {
        self.layout.slot_stacked(slot, &|s| self.slot_drawn(s))
    }

    /// `MoveViewToNeighbour(Up / Down)`: move the focused view to the
    /// nearest drawn slot above (`up`) or below it, trading places with the
    /// view shown there; the view keeps the keyboard. Nothing drawn that
    /// way: it goes to a new slot on that side when its slot holds other
    /// views ([`Self::spawn_beside`]); otherwise handled when slots are
    /// stacked there, else not handled, so the key passes on. From the rail
    /// see [`Self::move_view`] and [`Self::place_from_rail`]. A slot the move
    /// empties closes ([`Self::close_if_vacated`]). Persisted.
    fn move_view_vertically(
        &mut self,
        up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let dir = if up { Dir::Up } else { Dir::Down };
        let Some((view, from)) = self
            .arranged_view()
            .and_then(|view| self.layout.view_slot(&view).map(|s| (view, s)))
        else {
            return false;
        };
        let before: Vec<(SlotId, Option<DrawerView>)> = self
            .layout
            .slots()
            .into_iter()
            .map(|s| (s, self.layout.shown(s)))
            .collect();
        let undo = self.layout.clone();
        if self.layout.move_view_dir(&view, dir) {
            let to = self.layout.view_slot(&view);
            // Only onto a slot that is drawn: one taking no space is not
            // stacked there.
            if !to.is_some_and(|to| undo.shown(to).is_some()) {
                self.replace_layout(undo);
                return self.place(view, Placement::Beside(from, dir), window, cx)
                    || self.stacked(from);
            }
            // A swap: the view shown where it landed takes the slot it left.
            if let Some(Some(other)) = before
                .iter()
                .find(|(s, _)| Some(*s) == to)
                .map(|(_, shown)| *shown)
            {
                self.layout.move_view(other, from);
            }
            if self.focused_slot() == Some(from)
                && let Some(to) = self.layout.view_slot(&view)
            {
                self.layout.focus(Focus::Leaf(Leaf::Slot(to)));
            }
            self.close_if_vacated(from, view);
            self.tidy_layout(cx);
            self.apply_focus(window, cx);
            self.save_layout(cx);
            return true;
        }
        self.place(view, Placement::Beside(from, dir), window, cx) || self.stacked(from)
    }

    /// A place direction from the rail (ADR-0054 §3, rotated by ADR-0055 §6:
    /// Up and Down on a column, Left and Right on a row), on the cursor's
    /// view: the view is placed in a new slot on that side of the slot that last
    /// held the keyboard, while it exists and is drawn, else the slot beside
    /// the rail ([`LayoutTree::place_beside`]); it leaves the slot it was
    /// in, which closes when emptied. Handled, with a flash and nothing changed, while
    /// zoomed (the new slot would not be on screen), with no drawn slot to
    /// place beside, when the view is alone in that slot already, or with
    /// no room for the new slot ([`Self::place_new_slot`]). The rail
    /// keeps the keyboard. Persisted.
    fn place_from_rail(&mut self, dir: Dir, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.rail_cursor_view() else {
            return false;
        };
        let from = self.layout.view_slot(&view);
        let Some(target) = self
            .last_slot
            .filter(|s| self.slot_drawn(*s))
            .or_else(|| self.slot_beside_rail().filter(|s| self.slot_drawn(*s)))
        else {
            self.flash("No slot to place this view beside", cx);
            return true;
        };
        if self.layout.zoomed().is_some() {
            self.flash("Leave zoom to place a view", cx);
            return true;
        }
        let alone = from == Some(target) && self.layout.slot_views(target).len() < 2;
        if alone {
            self.flash("Nowhere to place this view", cx);
            return true;
        }
        // Out of the slot it is placed beside: the first view left there
        // shows in its place ([`Self::close_if_vacated`]).
        match self.place_new_slot(view, Leaf::Slot(target), dir, window, cx) {
            Ok(_) => self.settle_placement(window, cx),
            Err(NoNewSlot::Refused) => self.flash("Nowhere to place this view", cx),
            Err(NoNewSlot::NoRoom) => {}
        }
        true
    }

    /// `SplitSlotRight` / `SplitSlotDown` on the focused leaf. On a slot,
    /// the slot splits ([`Self::split_focused_slot`]). On the editor
    /// region, the active group splits (ADR-0058): an empty group comes
    /// after it, right of it (`axis` horizontal) or below it, becomes the
    /// active group and takes the keyboard; it shows the empty-group hint,
    /// which offers the note finder. Not handled on the rail.
    pub(crate) fn split_focused(
        &mut self,
        axis: TreeAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.layout.focused() {
            Focus::Leaf(Leaf::Editor) => {
                self.split_active_group(axis, cx);
                true
            }
            Focus::Leaf(Leaf::Slot(_)) => self.split_focused_slot(axis, window, cx),
            Focus::Rail => false,
        }
    }

    /// Split the active group along `axis` (see [`Self::split_focused`]).
    /// Refused with a flash while zoomed (the new group would replace the
    /// zoomed one on screen) and while the tab's close writes its notes (it
    /// would go with the tab); nothing without a vault or while the window
    /// closes. While a stored record restores, it waits for it
    /// ([`session`]).
    fn split_active_group(&mut self, axis: TreeAxis, cx: &mut Context<Self>) {
        if matches!(self.closing, Closing::Requested | Closing::Flushing) {
            return;
        }
        // The restore would replace the new group: the split waits.
        if self.hold_while_restoring(session::Queued::Split(axis)) {
            return;
        }
        if self.layout.zoomed().is_some() {
            self.flash(ZOOMED_NO_GROUP_SPLIT, cx);
            return;
        }
        let active = self.layout.active_group();
        if self.tab_mid_close(self.layout.region().shown()) {
            self.flash_error(tab_strip::TAB_CLOSING, cx);
            return;
        }
        let Some(state) = self.new_group_state(cx) else {
            return;
        };
        let Some(group) = self.layout.region_mut().split_group(active, axis) else {
            return;
        };
        self.insert_group(group, state, cx);
        self.focus_group(group, cx);
    }

    /// `CloseSlot` on the focused leaf. On a slot, the slot closes
    /// ([`Self::close_focused_slot`]). On the editor region, the active
    /// group closes ([`Self::close_editor_group`]): its note written first,
    /// a neighbour takes its space; the last group of a tab closes the tab,
    /// and the last group of the last tab stays, empty. Not handled on the
    /// rail.
    pub(crate) fn close_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.layout.focused() {
            Focus::Leaf(Leaf::Editor) => {
                let active = self.layout.active_group();
                self.close_editor_group(active, cx);
                true
            }
            Focus::Leaf(Leaf::Slot(_)) => self.close_focused_slot(window, cx),
            Focus::Rail => false,
        }
    }

    /// Split the focused slot (ADR-0054 §1): ask which view the new slot,
    /// right of it (`axis` horizontal) or below it, holds
    /// ([`Self::open_split_chooser`]); the slot arrives filled. Handled
    /// whenever a slot has the keyboard, except under another modal, where
    /// the key passes on; zoomed it flashes and splits nothing (the new
    /// slot would not be drawn).
    fn split_focused_slot(
        &mut self,
        axis: TreeAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(slot) = self.focused_slot() else {
            return false;
        };
        if self.layout.zoomed().is_some() {
            self.flash("Leave zoom to split a slot", cx);
            return true;
        }
        let dir = match axis {
            TreeAxis::Horizontal => Dir::Right,
            TreeAxis::Vertical => Dir::Down,
        };
        self.open_split_chooser(slot, dir, window, cx)
    }

    /// Close the focused slot (Ruling 15). While zoomed only the zoomed
    /// slot is on screen, so that is the one that closes (and zoom ends).
    /// Its views join the neighbouring slot that takes over its area,
    /// hidden, and the keyboard goes there when that slot is drawn (else to
    /// the editor). The only slot stays. Persisted.
    fn close_focused_slot(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(focused) = self.focused_slot() else {
            return false;
        };
        self.close_slot(focused, window, cx);
        true
    }

    /// Close `slot` ([`LayoutTree::close_slot`]): its views join the
    /// neighbouring slot that takes over its area, hidden, and the keyboard
    /// goes there when that slot is drawn (else to the editor). The last
    /// slot on its side of the editor stays ([`LayoutTree::last_on_side`]),
    /// so that side can be shown again, and a flash says so. Persisted.
    /// The `CloseSlot` chord and the slot menu's `Close slot`.
    pub(super) fn close_slot(&mut self, slot: SlotId, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout.last_on_side(slot) {
            self.flash(LAST_ON_SIDE, cx);
            return;
        }
        // An error is only a slot gone meanwhile: the last slot overall is
        // the last on its side, refused above.
        if self.layout.close_slot(slot).is_ok() {
            self.tidy_layout(cx);
            self.apply_focus(window, cx);
            self.save_layout(cx);
        }
        cx.notify();
    }

    /// `ToggleZoom` (ADR-0053): zoom the focused leaf, a slot or the
    /// editor group, over everything right of the rail; while zoomed, leave
    /// zoom. Nothing in the layout changes, nothing is saved. From the rail
    /// (not zoomed) there is nothing to zoom.
    pub(crate) fn toggle_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout.zoomed().is_some() {
            self.leave_zoom(window, cx);
            return;
        }
        if let Focus::Leaf(leaf) = self.layout.focused() {
            self.layout.zoom(Some(leaf));
            self.apply_focus(window, cx);
            cx.notify();
        }
    }

    /// Leave zoom: the arrangement is drawn again as it was. The footer's
    /// restore button.
    pub(crate) fn leave_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.layout.zoom(None);
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Whether a list's or field's Esc, which hands the keyboard back to
    /// the editor, must keep it where it is: a slot is zoomed, so the
    /// editor is not on screen. Esc never leaves zoom (ADR-0053 decision
    /// 3).
    pub(crate) fn zoom_keeps_keyboard(&self) -> bool {
        self.layout
            .zoomed()
            .is_some_and(|leaf| matches!(leaf, Leaf::Slot(_)))
    }

    /// A list or field in a slot was dismissed (Esc): the keyboard goes
    /// back to the editor, unless a slot is zoomed
    /// ([`Self::zoom_keeps_keyboard`]).
    pub(crate) fn dismiss_to_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.zoom_keeps_keyboard() {
            return;
        }
        self.focus_panel(Panel::Editor, window, cx);
    }

    /// `FocusRegion(dir)`: move focus geometrically over the visible
    /// leaves (drawn, not collapsed) and the rail while it is shown
    /// ([`LayoutTree::focus_dir_in`]).
    fn focus_dir(&mut self, dir: Dir) -> TreeFocusMove {
        let drawn: HashSet<Leaf> = self
            .layout
            .regions()
            .into_iter()
            .map(|(leaf, _)| leaf)
            .filter(|leaf| self.leaf_visible(*leaf))
            .collect();
        let rail = (!self.rail_hidden).then_some(self.rail);
        self.layout
            .focus_dir_in(dir, &|leaf| drawn.contains(&leaf), rail)
    }

    /// `FocusEditor` / `FocusSidebar`: the next (previous) of the rail (unless
    /// it is hidden) and the visible leaves in reading order, wrapping.
    fn cycle_focus(&mut self, forward: bool) -> Option<Focus> {
        // A hidden rail takes no keyboard focus (ADR-0055 §9).
        let mut order = if self.rail_hidden {
            Vec::new()
        } else {
            vec![Focus::Rail]
        };
        order.extend(
            self.layout
                .regions()
                .into_iter()
                .map(|(leaf, _)| leaf)
                .filter(|leaf| self.leaf_visible(*leaf))
                .map(Focus::Leaf),
        );
        let at = order.iter().position(|f| *f == self.layout.focused())?;
        let n = order.len();
        let next = if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        };
        Some(order[next])
    }

    /// The context `panel` resolves chords in; the editor's own (vim, an
    /// open find bar) when it is the editor. A query is typed there when
    /// FIND has the keyboard (the search browser, a modal, says so itself:
    /// [`Self::intercept_modal_key`]).
    fn key_ctx(&self, cx: &App) -> KeyCtx {
        match self.focused_panel() {
            // The attachment view reads no vim keys.
            Panel::Editor if self.active_state().attachment.is_some() => KeyCtx {
                panel: Panel::Editor,
                vim: false,
                query: false,
                browser: false,
                vim_pending: false,
                vim_insert: false,
            },
            Panel::Editor => self.active_state().editor.read(cx).key_ctx(),
            panel => KeyCtx {
                panel,
                vim: false,
                query: self.find_has_keyboard(),
                browser: false,
                vim_pending: false,
                vim_insert: false,
            },
        }
    }

    /// Whether FIND (its field or its results) has the keyboard.
    fn find_has_keyboard(&self) -> bool {
        self.focused_view() == Some(DrawerView::Find)
    }

    /// A key while the rail or a slot holds GPUI focus, seen before any
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
        // Esc during a view's drag ends the drag, ahead of any binding.
        if self.escape_ends_drag(event, window, cx) {
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
                .slot_focus
                .values()
                .any(|handle| handle.contains_focused(window, cx));
        if !in_shell || self.focused_panel() == Panel::Editor {
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
            None => self.layout.focused() == Focus::Rail && self.rail_key(key, window, cx),
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
        // The note menu is not modal: a chord that runs while it is open may
        // change what it acts on (another note, tab or screen), so it closes
        // first, the keyboard back where it was. Its own chord toggles it.
        if *action != ActionShortcuts::FileOperations {
            self.close_note_menu(window, cx);
        }
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
            ActionShortcuts::Quit => self.request_quit(window, cx),
            ActionShortcuts::ToggleSidebar => self.toggle_side(Side::Left, window, cx),
            ActionShortcuts::ToggleRightDrawer => self.toggle_side(Side::Right, window, cx),
            // From the rail the chords turn with it (ADR-0055 §6): along its
            // axis they place, toward the layout they show, and facing the
            // rail they are not handled.
            ActionShortcuts::MoveViewToNeighbour(dir) if self.layout.focused() == Focus::Rail => {
                return if self.rail.place_dir(*dir) {
                    self.place_from_rail(*dir, window, cx)
                } else {
                    self.move_view(*dir, window, cx)
                };
            }
            ActionShortcuts::MoveViewToNeighbour(dir @ (Dir::Left | Dir::Right)) => {
                return self.move_view(*dir, window, cx);
            }
            ActionShortcuts::MoveViewToNeighbour(Dir::Up) => {
                return self.move_view_vertically(true, window, cx);
            }
            ActionShortcuts::MoveViewToNeighbour(Dir::Down) => {
                return self.move_view_vertically(false, window, cx);
            }
            ActionShortcuts::SplitSlotRight => {
                return self.split_focused(TreeAxis::Horizontal, window, cx);
            }
            ActionShortcuts::SplitSlotDown => {
                return self.split_focused(TreeAxis::Vertical, window, cx);
            }
            ActionShortcuts::CloseSlot => return self.close_focused(window, cx),
            ActionShortcuts::ToggleZoom => self.toggle_zoom(window, cx),
            ActionShortcuts::ToggleRail => self.toggle_rail(window, cx),
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
            // The keyboard reference (M10d), in every window state. Where a
            // query is typed its F1 is the query syntax's (the keymap's
            // `Query` scope), so it never arrives here from FIND.
            ActionShortcuts::KeyboardHelp => self.toggle_modal(
                ModalKind::KeyboardHelp,
                Self::open_keyboard_help,
                window,
                cx,
            ),
            // FIND's query syntax (the search browser's is the modal's own
            // key); elsewhere the chord goes on.
            ActionShortcuts::QuerySyntax => {
                if !self.find_has_keyboard() {
                    return false;
                }
                self.open_query_help(window, cx);
            }
            // The search browser's own chord (handled by the modal); with
            // no browser open there is no query to carry, so it goes on.
            ActionShortcuts::OpenQueryInSidebar => return false,
            // Nothing followable under the caret: the chord does nothing.
            // On an attachment it opens it externally.
            ActionShortcuts::FollowLink => {
                self.follow_at_cursor(cx);
            }
            ActionShortcuts::FocusRegion(dir) => {
                match (self.layout.active_group(), self.focus_dir(*dir)) {
                    (active_before, TreeFocusMove::Moved(_)) => {
                        // A move between groups changed the active group: the
                        // window follows it as on every other route
                        // ([`Self::focus_group`]).
                        if self.layout.active_group() != active_before {
                            self.follow_active_group(cx);
                        }
                        self.apply_focus(window, cx)
                    }
                    (_, TreeFocusMove::AtEdge) => {}
                    // Nothing stacked above or below: the chord is the focused
                    // panel's own key, not a focus move (ADR-0050 §4). The
                    // editor already let it pass, so it is handed back; anywhere
                    // else it goes on unswallowed.
                    (_, TreeFocusMove::NotHandled) => {
                        let Some(key) = key else {
                            return true;
                        };
                        if self.focused_panel() != Panel::Editor {
                            return false;
                        }
                        self.active_state()
                            .editor
                            .update(cx, |editor, cx| editor.editor_key(key, window, cx));
                    }
                }
            }
            // The tabs (ADR-0058).
            ActionShortcuts::NewTab => self.new_tab(cx),
            ActionShortcuts::CloseTab => self.close_shown_tab(cx),
            ActionShortcuts::NextTab => self.step_tab(true, cx),
            ActionShortcuts::PrevTab => self.step_tab(false, cx),
            ActionShortcuts::FocusEditor | ActionShortcuts::FocusSidebar => {
                if let Some(next) = self.cycle_focus(*action == ActionShortcuts::FocusEditor) {
                    self.layout.focus(next);
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

    /// The rail's own keys: the two arrows along its axis move between
    /// cells, Enter or Space activates one (as a click does).
    fn rail_key(&mut self, key: KeyInput, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if key.mods != Mods::NONE {
            return false;
        }
        // The cursor moves along the rail's axis: Up and Down on a column,
        // Left and Right on a row (ADR-0055 §6).
        let (prev, next) = if self.rail.horizontal() {
            (Key::Left, Key::Right)
        } else {
            (Key::Up, Key::Down)
        };
        let last = self.rail_views().len() - 1;
        match key.key {
            k if k == prev => self.rail_cursor = self.rail_cursor.saturating_sub(1),
            k if k == next => self.rail_cursor = (self.rail_cursor + 1).min(last),
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
    fn focus_bar(&self, focus: Focus, cx: &gpui_kit::App) -> Option<impl IntoElement> {
        (self.layout.focused() == focus).then(|| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h_0p5()
                .bg(theme::focus_border(&theme::current(cx)))
        })
    }

    /// The rail's button: hides or shows the rail, tooltip naming the chord.
    /// In the title bar when it shows, else the footer's first segment.
    pub(super) fn rail_segment(
        &self,
        cx: &mut Context<Self>,
    ) -> status_row::RailSegment<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static> {
        let verb = if self.rail_hidden { "Show" } else { "Hide" };
        let label =
            chrome::tooltip_with_chord(&format!("{verb} rail"), &ActionShortcuts::ToggleRail, cx);
        status_row::RailSegment {
            position: self.rail,
            hidden: self.rail_hidden,
            label,
            on_toggle: cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_rail(window, cx)),
        }
    }

    /// `ToggleRail` (ADR-0055 §8, §9): hide the rail, or show it again.
    /// Saved. Hiding while the rail has the keyboard moves it to the last
    /// focused slot when that slot is drawn, else to the active editor
    /// group; showing never moves it. The tree is refitted to the room the
    /// rail leaves or takes.
    fn toggle_rail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rail_hidden = !self.rail_hidden;
        if self.rail_hidden && self.layout.focused() == Focus::Rail {
            let leaf = self
                .last_slot
                .filter(|slot| self.slot_drawn(*slot))
                .map_or(Leaf::Editor, Leaf::Slot);
            self.layout.focus(Focus::Leaf(leaf));
        }
        self.apply_focus(window, cx);
        self.save_layout(cx);
        cx.notify();
    }

    /// Put the rail at `rail` (ADR-0055 §3): from the palette, and from
    /// Preferences once saved. Saved with the layout, which it does not
    /// change; the keyboard and the hidden state stay as they are.
    pub(super) fn set_rail(&mut self, rail: RailPosition, cx: &mut Context<Self>) {
        if self.rail == rail {
            return;
        }
        self.rail = rail;
        self.save_layout(cx);
        cx.notify();
    }

    /// Whether the rail comes before the tree in the body: at the left or
    /// the top.
    fn rail_leads(&self) -> bool {
        matches!(self.rail, RailPosition::Left | RailPosition::Top)
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel_bg = theme::panel_background(&theme::current(cx));
        let focused = self.layout.focused() == Focus::Rail;
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
        let horizontal = self.rail.horizontal();
        div()
            .id("rail")
            .test_support()
            .track_focus(&self.rail_focus)
            .role(Role::TabList)
            .aria_label("Activity rail")
            .flex()
            .flex_none()
            .gap_1()
            .relative()
            .bg(panel_bg)
            // A column for a left or right rail, a row for a top or bottom
            // one; the thickness is the same (ADR-0055 §4, §5).
            .when(horizontal, |rail| {
                rail.flex_row()
                    .items_center()
                    .w_full()
                    .h(rems(RAIL_REMS))
                    .px_1()
            })
            .when(!horizontal, |rail| {
                rail.flex_col().h_full().w(rems(RAIL_REMS)).py_1()
            })
            // The border is on the edge that faces the tree.
            .map(|rail| match self.rail {
                RailPosition::Left => rail.border_r_1(),
                RailPosition::Right => rail.border_l_1(),
                RailPosition::Top => rail.border_b_1(),
                RailPosition::Bottom => rail.border_t_1(),
            })
            .border_color(theme::divider(&theme::current(cx)))
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus_panel(Panel::Rail, window, cx)
            }))
            .children(top)
            .child(div().flex_1())
            .child(self.render_side_toggle(Side::Left, cx))
            .child(self.render_side_toggle(Side::Right, cx))
            .child(pinned)
            // Last, so it paints over a highlighted cell.
            .children(self.focus_bar(Focus::Rail, cx))
    }

    /// The rail's button for `side` (the pointer's way to the toggle chords,
    /// ADR-0052 Ruling 13): lit while a slot on that side is drawn, it
    /// runs [`Self::toggle_side`]. Not a rail cell: it takes no cursor.
    fn render_side_toggle(&self, side: Side, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::assets::IconName;
        let theme = theme::current(cx);
        let open = self.side_slots(side).iter().any(|s| self.slot_drawn(*s));
        let (id, name, icon, action) = match side {
            Side::Left => (
                "rail-toggle-left",
                "Toggle left side",
                if open {
                    IconName::PanelLeftOpen
                } else {
                    IconName::PanelLeft
                },
                ActionShortcuts::ToggleSidebar,
            ),
            Side::Right => (
                "rail-toggle-right",
                "Toggle right side",
                if open {
                    IconName::PanelRightOpen
                } else {
                    IconName::PanelRight
                },
                ActionShortcuts::ToggleRightDrawer,
            ),
        };
        let tip: SharedString = chrome::tooltip_with_chord(name, &action, cx);
        let glyph = if open {
            theme::focus_border(&theme)
        } else {
            theme::muted(&theme)
        };
        let hover = theme::hover_background(&theme);
        div()
            .id(id)
            .test_support()
            .role(Role::Button)
            .aria_label(name)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h_8()
            .when(self.rail.horizontal(), |button| button.w_8().mx_0p5())
            .when(!self.rail.horizontal(), |button| button.mx_1())
            .rounded_md()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(Icon::new(icon).size_4().text_color(glyph))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.toggle_side(side, window, cx)
            }))
            .into_any_element()
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
        let name = view_registry::spec(view).name;
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
            .h_10()
            .when(self.rail.horizontal(), |cell| cell.w_10().mx_0p5())
            .when(!self.rail.horizontal(), |cell| cell.mx_1())
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
            // A drag source only: the rail is not a drop target (ADR-0054 §8).
            .cursor_grab()
            .on_drag(
                ViewDrag {
                    view,
                    from: DragFrom::Rail,
                },
                self.view_drag_ghost(cx),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.activate_rail_view(view, window, cx)
            }))
            .into_any_element()
    }

    /// What the editor Panel shows in place of the editor while a vault is
    /// open but no note is (yet): typing there could never be saved. `None`
    /// once a note is open, and without a vault (the fixture tests' editor).
    /// The active group's ([`Self::note_empty_state_in`]).
    pub(crate) fn note_empty_state(&self) -> Option<EmptyState> {
        self.note_empty_state_in(self.layout.active_group())
    }

    /// [`Self::note_empty_state`] for `group`: only the group whose note
    /// went away says so, and only the active group waits for the restore.
    pub(crate) fn note_empty_state_in(&self, group: GroupId) -> Option<EmptyState> {
        let host = self.state(group)?.notes.as_ref()?;
        if host.note.is_some() {
            return None;
        }
        if let Some((_, gone)) = self.note_gone.as_ref().filter(|(g, _)| *g == group) {
            return Some(gone.clone());
        }
        let (title, detail) = if self.restore_pending && group == self.layout.active_group() {
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

    /// Whether `group` shows its editor ([`Self::render_group_document`]).
    fn group_editor_shown(&self, group: GroupId) -> bool {
        self.empty.is_none()
            && self.state(group).is_some_and(|s| s.attachment.is_none())
            && self.note_empty_state_in(group).is_none()
    }

    /// What `group` shows: its attachment view, else why there is no note
    /// (the window's own empty state first), else its editor. Whether the
    /// editor is what shows.
    pub(super) fn render_group_document(
        &self,
        group: GroupId,
        cx: &mut Context<Self>,
    ) -> (AnyElement, bool) {
        let Some(state) = self.state(group) else {
            return (div().flex_1().into_any_element(), false);
        };
        let attachment = state.attachment.clone().filter(|_| self.empty.is_none());
        let empty = match &attachment {
            None => self
                .empty
                .clone()
                .or_else(|| self.note_empty_state_in(group)),
            Some(_) => None,
        };
        let editor_shown = empty.is_none() && attachment.is_none();
        let document = match (&empty, attachment) {
            // The editor is not drawn: its focus handle sits on the empty
            // state, so the keyboard still reaches the window's chords (F5
            // away from a workspace that would not open, the leader, the
            // palette).
            (Some(empty), _) => self
                .render_empty(empty, &state.editor.read(cx).focus_handle(cx), cx)
                .into_any_element(),
            (None, Some(view)) => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(view)
                .into_any_element(),
            (None, None) => div()
                .flex_1()
                .min_h_0()
                .child(state.editor.clone())
                .into_any_element(),
        };
        (document, editor_shown)
    }

    /// The editor Panel, `size` px (its layout cell): the tab strip, the
    /// shown tab's groups and the Footer. One group draws its document
    /// alone, as the Panel always did; two or more are laid out side by
    /// side, each under its header ([`Self::render_groups`]). Zoomed, only
    /// the active group shows, with no strip and no header (ADR-0058).
    pub(super) fn render_editor(
        &self,
        size: (f64, f64),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let zoomed = self.layout.zoomed() == Some(Leaf::Editor);
        let region = self.layout.region();
        let several = !zoomed && region.groups(region.shown()).len() > 1;
        let active = self.layout.active_group();
        let (document, editor_shown) = if several {
            (
                self.render_groups(size, f64::from(window.rem_size()), cx),
                self.group_editor_shown(active),
            )
        } else {
            self.render_group_document(active, cx)
        };
        // The which-key docks over the bottom of the document, right above
        // the Footer, so showing it never reflows the note.
        let document = div()
            .id("editor-document")
            .group(group_header::LONE_GROUP_HOVER)
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(document)
            // A lone group (no header) has its note-menu button floating
            // at the top-right corner of the document.
            .children((!several).then(|| self.render_corner_menu_button(active, cx)))
            // A lone group's bounds are the document's; several groups
            // record their own.
            .children((!several).then(|| note_menu::group_bounds_probe(&self.group_bounds, active)))
            // A lone group (a tab just opened mid-drag) takes a group's
            // drop too; several groups carry their own layers.
            .children(
                (!several)
                    .then(|| self.group_zone_layer(0, active, cx))
                    .flatten(),
            )
            .children(self.render_which_key(cx));
        div()
            .id("editor-panel")
            .test_support()
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .p(tree_view::LEAF_MARGIN)
            .child(
                Self::leaf_surface(
                    theme::background(&theme::current(cx)),
                    self.leaf_border(Focus::Leaf(Leaf::Editor), cx),
                )
                .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.focus_panel(Panel::Editor, window, cx)
                }))
                // The tab strip over the shown tab, with two tabs or more;
                // a zoom shows the group alone.
                .children(if zoomed {
                    None
                } else {
                    self.render_tab_strip(cx)
                })
                .child(document)
                .child(self.render_footer(editor_shown, window, cx)),
            )
            // While a view is dragged: where it fills or makes a side.
            .children(self.window_strips(cx))
    }

    /// The Footer: under the editor Panel, or under the zoomed slot while
    /// one is zoomed (with the zoom hint and restore button then).
    pub(super) fn render_footer(
        &self,
        editor_shown: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let zoom = self.layout.zoomed().map(|_| status_row::ZoomSegment {
            hint: self.chord_labels.zoom_hint.clone(),
            on_restore: cx.listener(|this, _: &ClickEvent, window, cx| this.leave_zoom(window, cx)),
        });
        // No title bar (system decorations, fullscreen): the footer carries
        // the workspace chip, so the name is always on screen.
        let t = theme::current(cx);
        let chip = if self.bar_visible(window) {
            None
        } else {
            self.workspace_chip(theme::muted(&t), window, cx)
        };
        // With no title bar the footer carries the rail's button too.
        let rail = (!self.bar_visible(window)).then(|| self.rail_segment(cx));
        status_row::render(
            &self.status,
            &self.doc_facts(editor_shown, cx),
            &t,
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.show_links_tab(LinksTab::Backlinks, window, cx)
            }),
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.open_properties_for_open_note(window, cx)
            }),
            status_row::Leading {
                rail,
                workspace: chip,
                zoom,
            },
        )
        .into_any_element()
    }

    /// What the Footer reads off the editor and the open note this frame.
    /// The caret only while the editor is shown and has the keyboard with
    /// its find bar closed; the find bar's matches while it is open; the
    /// note's counts once loaded, while a note is open.
    pub(crate) fn doc_facts(&self, editor_shown: bool, cx: &App) -> DocFacts {
        let editor = &self.active_state().editor.read(cx).editor;
        let focused = editor_shown && self.focused_panel() == Panel::Editor;
        let find_open = editor.find_open();
        let meta = self
            .active_state()
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
        self.prune_drag(cx);
        self.refresh_drop_outcomes(window, cx);
        let bar = self.render_title_bar(window, cx);
        let body = self.render_body(window, cx);
        // The tree is fitted to the height the bar leaves; a change in the
        // bar's height (fullscreen, first frame) draws again.
        let has_bar = bar.is_some();
        let bar_height = self.title_bar_height.clone();
        div()
            .on_children_prepainted(move |bounds, window, _| {
                let height = match bounds.first() {
                    Some(bar) if has_bar => f64::from(bar.size.height),
                    _ => 0.0,
                };
                if (bar_height.get() - height).abs() > 0.5 {
                    bar_height.set(height);
                    window.refresh();
                }
            })
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
        div()
            .id("workspace")
            .key_context(KEY_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_drag_move(cx.listener(Self::on_divider_move))
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
            // The rail's edge sets the direction (ADR-0055 §4): beside the
            // tree for left and right, above or under it for top and bottom.
            .when(self.rail.horizontal(), |body| body.flex_col())
            .when(!self.rail.horizontal(), |body| body.flex_row())
            .items_stretch()
            .w_full()
            .flex_1()
            .min_h_0()
            .bg(theme::background(&theme::current(cx)))
            .when(!self.rail_hidden && self.rail_leads(), |body| {
                body.child(self.render_rail(cx))
            })
            .child({
                self.refit(window, cx);
                self.render_tree(window, cx)
            })
            .when(!self.rail_hidden && !self.rail_leads(), |body| {
                body.child(self.render_rail(cx))
            })
            // Inside the workspace's key context: a key the menu does not
            // use reaches the workspace's chords.
            .children(self.render_note_menu(window, cx))
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
mod chevron;
mod chrome;
mod config;
mod create;
mod drawer_list;
mod drop_zones;
mod file_dialogs;
mod file_ops;
mod files;
mod find;
mod folder_chooser;
mod follow;
mod group;
mod group_drop;
mod group_header;
mod input_autocomplete;
mod key_suggestions;
mod keyboard_help;
mod layout_writer;
mod leader;
mod links;
mod modal;
mod note;
mod note_flush;
mod note_menu;
mod onboarding;
mod outline;
mod palette;
mod picker;
mod pinned;
mod placement;
mod preferences;
mod presets;
mod properties;
mod query_field;
mod reopen;
mod result_row;
mod route;
mod saved_searches;
mod search;
mod semantic;
mod server;
mod session;
mod sort;
mod switcher;
mod tab_strip;
mod tags;
mod theme_picker;
pub(crate) mod titlebar;
mod tree_view;
mod update;
mod view_chooser;
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
mod chevron_tests;
#[cfg(test)]
mod drop_tests;
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
mod keyboard_help_tests;
#[cfg(test)]
mod leader_tests;
#[cfg(test)]
mod note_menu_tests;
#[cfg(test)]
mod note_tests;
#[cfg(test)]
mod onboarding_tests;
#[cfg(test)]
mod paging_tests;
#[cfg(test)]
mod palette_tests;
#[cfg(test)]
mod picker_tests;
#[cfg(test)]
mod pinned_tests;
#[cfg(test)]
mod placement_tests;
#[cfg(test)]
mod preferences_tests;
#[cfg(test)]
mod preferences_workspaces_tests;
#[cfg(test)]
mod preset_tests;
#[cfg(test)]
mod properties_tests;
#[cfg(test)]
mod rail_tests;
#[cfg(test)]
mod reopen_tests;
#[cfg(test)]
mod saved_searches_tests;
#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod semantic_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod slot_tests;
#[cfg(test)]
mod sort_tests;
#[cfg(test)]
mod split_tests;
#[cfg(test)]
mod start_tests;
#[cfg(test)]
mod switcher_tests;
#[cfg(test)]
mod tab_strip_tests;
#[cfg(test)]
mod tabs_tests;
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
#[cfg(test)]
mod zoom_tests;
