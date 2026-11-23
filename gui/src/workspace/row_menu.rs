//! The **row menu**: the context menu a right-click on a list row that
//! stands for a vault entry opens — a FILES row (a note, a folder, an
//! attachment, a pinned note) and a FIND, LINKS or SEM row (a note).
//!
//! The entries are a pure function of a small [`RowMenuContext`]
//! ([`row_menu_entries`]): the kind of row ([`RowKind`]), whether the row is
//! drawn in FILES (which has no `Reveal in file browser`), and whether the
//! note is pinned. In order: the ways to open the entry (none for a folder,
//! no split for an attachment), a separator, the helper items (`Copy path`,
//! `Pin note` or `Unpin note` for a note, `Reveal in file browser` outside
//! FILES), a separator, then `Rename…`, `Move…` and `Delete…`.
//!
//! The menu is the kit's context menu ([`with_row_menu`]). Whether a note
//! is pinned is read from [`PinnedPaths`], a GPUI global that FILES sets
//! each time it reads the pins; with none set, a note offers `Pin note`.
//! That read can be stale, so the two items are explicit, never a toggle:
//! `Pin note` on a pinned note and `Unpin note` on one that is not pinned
//! change nothing.
//! A pick emits the list owner's event once the menu has given the keyboard
//! back; the owner hands it to [`Workspace::run_row_pick`]. A right-click
//! never activates the row.
//!
//! An **Inline image** in a note's editor has the same menu, for its
//! attachment, as a row drawn outside FILES ([`Workspace::image_entry_menu`]).
//!
//! A pick runs a moment after the menu opened: an open or a reveal of an
//! entry that was moved or deleted since flashes an error and does nothing.

use std::rc::Rc;

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, EventEmitter, Global, InteractiveElement, IntoElement,
    ParentElement, Styled, WeakEntity, Window,
};
use kimun_core::error::{FSError, VaultError};
use kimun_core::nfs::VaultPath;
use kimun_core::nfs::pinned_notes;
use kimun_core::{NoteVault, PINNED_NOTES_CAP, PinOutcome};
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::layout_tree::Axis as TreeAxis;

use super::Workspace;
use super::file_dialogs::FileOpKind;
use super::note::OpenAt;
use crate::editor_view::EntryMenu;

/// The flash after `Copy path`.
pub(crate) const PATH_COPIED: &str = "Path copied";

/// The flash after `Pin note` on a note that is already pinned.
pub(crate) const ALREADY_PINNED: &str = "already pinned";

/// The flash after `Unpin note` on a note that is not pinned.
pub(crate) const NOT_PINNED: &str = "not pinned";

/// What a row stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowKind {
    Note,
    Folder,
    Attachment,
}

impl RowKind {
    /// The kind of a FILES row; `None` for `..` and `Create:`, which are
    /// not entries.
    pub(crate) fn of(entry: &FileListEntry) -> Option<Self> {
        match entry {
            FileListEntry::Note { .. } => Some(Self::Note),
            FileListEntry::Directory { .. } => Some(Self::Folder),
            FileListEntry::Attachment { .. } => Some(Self::Attachment),
            FileListEntry::Up { .. } | FileListEntry::CreateNote { .. } => None,
        }
    }
}

/// A row menu item that is not a file operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowAction {
    Open,
    OpenNewTab,
    OpenSplitRight,
    OpenSplitDown,
    OpenInDefaultApp,
    CopyPath,
    Pin,
    Unpin,
    RevealInFiles,
}

/// What a row menu item does: a file operation's dialog, or an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowPick {
    File(FileOpKind),
    Act(RowAction),
}

impl RowPick {
    /// The item's label, as the menu shows it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            RowPick::File(FileOpKind::Rename) => "Rename…",
            RowPick::File(FileOpKind::Move) => "Move…",
            RowPick::File(FileOpKind::Delete) => "Delete…",
            RowPick::File(FileOpKind::Properties) => "Properties…",
            RowPick::Act(action) => match action {
                RowAction::Open => "Open",
                RowAction::OpenNewTab => "Open in new tab",
                RowAction::OpenSplitRight => "Open in split right",
                RowAction::OpenSplitDown => "Open in split down",
                RowAction::OpenInDefaultApp => "Open in default app",
                RowAction::CopyPath => "Copy path",
                RowAction::Pin => "Pin note",
                RowAction::Unpin => "Unpin note",
                RowAction::RevealInFiles => "Reveal in file browser",
            },
        }
    }
}

/// One line of the row menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowMenuEntry {
    Item(RowPick),
    Separator,
}

/// What decides which entries show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowMenuContext {
    pub kind: RowKind,
    /// The row is drawn in FILES: no `Reveal in file browser`.
    pub in_files: bool,
    /// The note is pinned: `Unpin note` instead of `Pin note`. Ignored for
    /// a folder or an attachment, which do not pin.
    pub pinned: bool,
}

/// The entries to show, in order (see the module docs).
pub(crate) fn row_menu_entries(ctx: &RowMenuContext) -> Vec<RowMenuEntry> {
    use RowAction::*;
    let act = |action| RowMenuEntry::Item(RowPick::Act(action));
    let mut entries = Vec::new();
    match ctx.kind {
        RowKind::Note => entries.extend([
            act(Open),
            act(OpenNewTab),
            act(OpenSplitRight),
            act(OpenSplitDown),
            RowMenuEntry::Separator,
        ]),
        RowKind::Attachment => entries.extend([
            act(Open),
            act(OpenNewTab),
            act(OpenInDefaultApp),
            RowMenuEntry::Separator,
        ]),
        RowKind::Folder => {}
    }
    entries.push(act(CopyPath));
    if ctx.kind == RowKind::Note {
        entries.push(act(if ctx.pinned { Unpin } else { Pin }));
    }
    if ctx.kind != RowKind::Folder && !ctx.in_files {
        entries.push(act(RevealInFiles));
    }
    entries.push(RowMenuEntry::Separator);
    entries.extend(
        [FileOpKind::Rename, FileOpKind::Move, FileOpKind::Delete]
            .map(|kind| RowMenuEntry::Item(RowPick::File(kind))),
    );
    entries
}

/// The vault's pinned notes as FILES last read them (core's absolute
/// paths), for the row menu's `Pin note` / `Unpin note`. Set by
/// `FilesPane::reload_pins` after each read.
#[derive(Debug, Clone, Default)]
pub(crate) struct PinnedPaths(pub Vec<VaultPath>);

impl Global for PinnedPaths {}

/// `row` with the row menu for the entry at `path`, a `kind` row drawn in
/// FILES or not. A pick emits `event(pick, path)` from `owner` once the
/// menu has given the keyboard back, so a dialog it opens returns it there
/// too.
pub(super) fn with_row_menu<R, T, E>(
    row: R,
    path: VaultPath,
    kind: RowKind,
    in_files: bool,
    owner: WeakEntity<T>,
    event: fn(RowPick, VaultPath) -> E,
) -> AnyElement
where
    R: InteractiveElement + ParentElement + Styled + IntoElement + 'static,
    T: EventEmitter<E>,
    E: 'static,
{
    row.context_menu(move |menu, _, cx| {
        let owner = owner.clone();
        fill_row_menu(
            menu,
            &path,
            kind,
            in_files,
            cx,
            move |pick, path, window, cx| {
                let event = event(pick, path);
                let owner = owner.clone();
                window.defer(cx, move |_, cx| {
                    owner.update(cx, |_, cx| cx.emit(event)).ok();
                });
            },
        )
    })
    .into_any_element()
}

/// `menu` with the row menu's entries for the entry at `path`, a `kind`
/// entry drawn in FILES or not. Clicking an item runs `on_pick` with it and
/// the path.
fn fill_row_menu(
    mut menu: PopupMenu,
    path: &VaultPath,
    kind: RowKind,
    in_files: bool,
    cx: &App,
    on_pick: impl Fn(RowPick, VaultPath, &mut Window, &mut App) + 'static,
) -> PopupMenu {
    // Core's own comparison: its pins are absolute, the row's path may
    // not be.
    let pinned = cx
        .try_global::<PinnedPaths>()
        .is_some_and(|pins| pinned_notes::position_of(&pins.0, path).is_some());
    let ctx = RowMenuContext {
        kind,
        in_files,
        pinned,
    };
    let on_pick = Rc::new(on_pick);
    for entry in row_menu_entries(&ctx) {
        menu = match entry {
            RowMenuEntry::Separator => menu.separator(),
            RowMenuEntry::Item(pick) => {
                let (on_pick, path) = (on_pick.clone(), path.clone());
                menu.item(
                    PopupMenuItem::new(pick.label())
                        .on_click(move |_, window, cx| on_pick(pick, path.clone(), window, cx)),
                )
            }
        };
    }
    menu
}

impl Workspace {
    /// The **Entry menu** of an inline image's attachment, for a note's
    /// editor ([`EditorView::set_entry_menu`]): the row menu of an
    /// attachment drawn outside FILES. A pick runs here once the menu has
    /// given the keyboard back, as a row's does.
    ///
    /// [`EditorView::set_entry_menu`]: crate::editor_view::EditorView::set_entry_menu
    pub(super) fn image_entry_menu(cx: &Context<Self>) -> EntryMenu {
        let workspace = cx.weak_entity();
        Rc::new(move |menu, path, _, cx| {
            let workspace = workspace.clone();
            fill_row_menu(
                menu,
                &path,
                RowKind::Attachment,
                false,
                cx,
                move |pick, path, window, cx| {
                    let workspace = workspace.clone();
                    window.defer(cx, move |window, cx| {
                        workspace
                            .update(cx, |w, cx| w.run_row_pick(pick, path, window, cx))
                            .ok();
                    });
                },
            )
        })
    }

    /// Run `pick`, chosen in the row menu of the entry at `path`. A file
    /// operation opens its dialog; `Copy path` puts the vault path, rooted
    /// at the vault, on the clipboard; `Pin note` pins and `Unpin note`
    /// unpins, each a no-op when the note is already so
    /// ([`Self::set_pinned`]; FILES' pinned section follows). An open or a reveal first checks the entry is still
    /// there: one moved or deleted since the menu opened flashes an error.
    pub(crate) fn run_row_pick(
        &mut self,
        pick: RowPick,
        path: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = match pick {
            RowPick::File(kind) => return self.open_file_dialog(kind, path, window, cx),
            RowPick::Act(action) => action,
        };
        match action {
            RowAction::CopyPath => {
                // One text per note whatever list the row is in: rooted at
                // the vault, as the app shows a note's path elsewhere (the
                // file dialogs, the index's paths). A row's path may or may
                // not be rooted.
                let shown = path.flatten().absolute();
                cx.write_to_clipboard(ClipboardItem::new_string(shown.to_string()));
                self.flash(PATH_COPIED, cx);
            }
            // Explicit, never a toggle: the label came from `PinnedPaths`,
            // which can be stale. A pin of a note gone from disk can still
            // be dropped.
            RowAction::Pin => self.set_pinned(path, true, cx),
            RowAction::Unpin => self.set_pinned(path, false, cx),
            RowAction::Open
            | RowAction::OpenNewTab
            | RowAction::OpenSplitRight
            | RowAction::OpenSplitDown
            | RowAction::OpenInDefaultApp
            | RowAction::RevealInFiles => self.run_row_action_if_there(action, path, cx),
        }
    }

    /// Pin the note at `path` (`pin`) or unpin it, as the row menu's
    /// `Pin note` / `Unpin note`: a pin of a note already pinned and an
    /// unpin of a note not pinned change nothing. A flash says what
    /// happened, in the words of the Note menu's toggle, and FILES' pinned
    /// section (with [`PinnedPaths`]) follows.
    fn set_pinned(&mut self, path: VaultPath, pin: bool, cx: &mut Context<Self>) {
        let Some(vault) = self.vault() else {
            return;
        };
        let job = tokio::spawn(async move {
            if pin {
                pin_note(&vault, &path).await
            } else {
                unpin_note(&vault, &path).await
            }
        });
        cx.spawn(async move |this, cx| {
            let msg = job.await.ok();
            this.update(cx, |workspace, cx| {
                if let Some(msg) = msg {
                    workspace.flash(msg, cx);
                }
                workspace.reload_pins(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Run `action` on `path` once core says the entry is still there;
    /// flash an error when it is not.
    fn run_row_action_if_there(
        &mut self,
        action: RowAction,
        path: VaultPath,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault() else {
            return;
        };
        let checking = {
            let path = path.clone();
            tokio::spawn(async move { vault.exists(&path).await })
        };
        cx.spawn(async move |this, cx| {
            let there = checking.await.unwrap_or(false);
            this.update(cx, |workspace, cx| {
                if !there {
                    workspace.flash_error(format!("{path} was moved or deleted"), cx);
                    return;
                }
                workspace.with_window(cx, move |workspace, window, cx| {
                    workspace.run_row_action(action, path, window, cx)
                });
            })
            .ok();
        })
        .detach();
    }

    /// An open goes where `action` says and the editor takes the keyboard
    /// (a handoff to the default program leaves it where it is). It opens
    /// the plain note or attachment: unlike a click on a FIND row, it does
    /// not emphasise the query's words in the note. A reveal shows FILES at
    /// the entry.
    fn run_row_action(
        &mut self,
        action: RowAction,
        path: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = match action {
            RowAction::Open => OpenAt::Here,
            RowAction::OpenNewTab => OpenAt::NewTab,
            RowAction::OpenSplitRight => OpenAt::Split(TreeAxis::Horizontal),
            RowAction::OpenSplitDown => OpenAt::Split(TreeAxis::Vertical),
            RowAction::OpenInDefaultApp => OpenAt::Externally,
            RowAction::RevealInFiles => return self.reveal_in_files(path, window, cx),
            RowAction::CopyPath | RowAction::Pin | RowAction::Unpin => return,
        };
        if at != OpenAt::Externally {
            self.focus_panel(Panel::Editor, window, cx);
        }
        if path.is_note() {
            self.open_note_at(path, at, cx);
        } else {
            self.open_attachment_at(&path, at, cx);
        }
        cx.notify();
    }
}

/// Pin the note at `path` unless it is pinned already; the flash text.
/// Core checks and appends under one lock, so a stale menu never unpins.
async fn pin_note(vault: &NoteVault, path: &VaultPath) -> String {
    match vault.pin_note(path).await {
        Ok(PinOutcome::Pinned { position }) => {
            format!("pinned as {} of {}", position + 1, PINNED_NOTES_CAP)
        }
        Ok(PinOutcome::AlreadyPinned) => ALREADY_PINNED.to_string(),
        Ok(PinOutcome::Full) => {
            format!("already {PINNED_NOTES_CAP} pinned — unpin one first")
        }
        Err(e) => pin_error(path, e),
    }
}

/// Unpin the note at `path`; nothing when it is not pinned. The flash text.
async fn unpin_note(vault: &NoteVault, path: &VaultPath) -> String {
    match vault.unpin_note(path).await {
        Ok(true) => "unpinned".to_string(),
        Ok(false) => NOT_PINNED.to_string(),
        Err(e) => pin_error(path, e),
    }
}

/// The flash for a failed pin or unpin, as the Note menu's toggle words it.
fn pin_error(path: &VaultPath, e: VaultError) -> String {
    match e {
        VaultError::FSError(FSError::VaultPathNotFound { .. }) => {
            format!("note not found: {path}")
        }
        e => {
            tracing::warn!("failed to update the pin on {path}: {e}");
            format!("could not update pinned notes: {e}")
        }
    }
}
