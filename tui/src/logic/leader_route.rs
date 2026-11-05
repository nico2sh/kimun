//! The **Leader route**: what a fired leader leaf means, decided once for
//! every door onto it — the leader sequence, the command palette, vim `:` —
//! and for both frontends. [`route`] is pure over a [`RouteFacts`] snapshot
//! and returns data ([`LeaderRoute`]); each frontend *performs* the route
//! with its own surfaces (the TUI's `EditorScreen::execute_leader_action`,
//! the GUI workspace's palette runner). The asynchronous leaves (pinning,
//! pinned jumps, reindex, the update check) are toolkit-free jobs in [`jobs`]
//! that report on the app channel.

use kimun_core::nfs::VaultPath;

use crate::keys::leader::LeaderAction;
use crate::logic::message::FileOp;
use crate::logic::nav::{DrawerView, LinksTab};
use crate::logic::overlay_open::OverlayOpen;
use crate::logic::text_action::TextAction;

/// The screen facts a route depends on. Keep it minimal — every field is a
/// reason the route can change.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteFacts {
    /// The editor session's path (the open note, an attachment, or the
    /// vault root when nothing is open).
    session_path: VaultPath,
    /// The Ask workspace holds the editor area.
    ask_shown: bool,
}

impl RouteFacts {
    pub fn new(session_path: VaultPath, ask_shown: bool) -> Self {
        Self {
            session_path,
            ask_shown,
        }
    }

    /// The note the editor area is showing, if any — what every "+this
    /// note" leader action (`m r`/`m m`/`m d`/`m c`/`m y`/`m i`) acts on.
    /// `None` on the vault root, in the attachment view (the session path is
    /// then the attachment's path, never a note) and while the Ask
    /// workspace is up (the session path still names the note it replaced, but
    /// the user is not looking at it). The one definition of "this note",
    /// so the m-group cannot disagree about it.
    pub fn open_note(&self) -> Option<&VaultPath> {
        if self.ask_shown || !self.session_path.is_note() {
            None
        } else {
            Some(&self.session_path)
        }
    }
}

/// A dialog about the open note, opened through a [`FileOp`] request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteDialog {
    Rename,
    Move,
    Delete,
    Properties,
}

impl NoteDialog {
    /// The `FileOp` request that opens this dialog on `path`.
    pub fn file_op(self, path: VaultPath) -> FileOp {
        match self {
            NoteDialog::Rename => FileOp::ShowRename(path),
            NoteDialog::Move => FileOp::ShowMove(path),
            NoteDialog::Delete => FileOp::ShowDelete(path),
            NoteDialog::Properties => FileOp::ShowProperties(path),
        }
    }
}

/// A drawer-layout step of the `+window` group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutStep {
    /// Hide the drawer and focus the editor.
    Zen,
    GrowDrawer,
    ShrinkDrawer,
}

/// An Ask workspace conversation step (`+ask`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskStep {
    Focus,
    New,
    Copy,
    Save,
    Regenerate,
    Source,
}

/// An asynchronous leaf, run by [`jobs`].
#[derive(Debug, Clone, PartialEq)]
pub enum LeaderJob {
    /// Pin the note, or unpin it if pinned ([`jobs::toggle_pin`]).
    TogglePin(VaultPath),
    /// Open pinned note `n`, 1-based ([`jobs::jump_to_pinned`]).
    JumpToPinned(u8),
    /// Fast reindex ([`jobs::reindex`]).
    Reindex,
    /// Show the cached update notice, else run a forced check
    /// ([`jobs::check_updates`]).
    CheckUpdates,
}

/// What a fired leader leaf means. Data only, so the performer is a
/// mechanical `match` and the decision is table-tested here.
#[derive(Debug, Clone, PartialEq)]
pub enum LeaderRoute {
    /// Open this overlay (refused by the host when one is already open).
    Overlay(OverlayOpen),
    /// Reveal and focus this drawer view.
    Drawer(DrawerView),
    /// Reveal FIND running this query.
    FindWithQuery(String),
    /// Reveal LINKS on this tab.
    LinksTab(LinksTab),
    /// Reveal FILES, whose filter doubles as the create field, and say so.
    NewNote,
    OpenJournal,
    Quit,
    OpenPreferences,
    OpenOnboarding,
    /// Open a dialog about the open note.
    NoteDialog(NoteDialog, VaultPath),
    /// The leaf acts on the open note and there is none: flash "no note open".
    NoNoteOpen,
    /// Copy `text` to the clipboard and flash `label`.
    Yank {
        text: String,
        label: &'static str,
    },
    /// Apply markdown formatting to the editor's selection (when the editor
    /// accepts a text action — the performer's rule).
    Text(TextAction),
    /// Flush the autosave now.
    Save,
    Layout(LayoutStep),
    Job(LeaderJob),
    /// Refresh and flash the workspace git summary.
    GitStatus,
    Ask(AskStep),
    /// A leaf whose surface has not landed yet: flash this notice.
    Soon(&'static str),
}

/// The leaf `action` resolved against `facts`.
pub fn route(action: LeaderAction, facts: &RouteFacts) -> LeaderRoute {
    use LeaderAction as A;
    use LeaderRoute as R;
    let note_dialog = |dialog: NoteDialog| match facts.open_note() {
        Some(path) => R::NoteDialog(dialog, path.clone()),
        None => R::NoNoteOpen,
    };
    match action {
        A::OpenDrawer(view) => R::Drawer(view),
        A::AppCheckUpdates => R::Job(LeaderJob::CheckUpdates),

        // +find
        A::FindFiles => R::Overlay(OverlayOpen::FileFinder),
        A::FindGrep | A::FindRecent => R::Overlay(OverlayOpen::SearchBrowser),
        A::FindTags => R::Drawer(DrawerView::Tags),
        A::FindBacklinks => R::FindWithQuery("<{note}".to_string()),
        A::FindSaved => R::Overlay(OverlayOpen::SavedSearches),
        A::FindHeadings => R::Drawer(DrawerView::Outline),
        A::FindPinned => R::Overlay(OverlayOpen::PinnedNotes),

        // +note
        A::NoteNew => R::NewNote,
        A::NoteDaily => R::OpenJournal,
        A::NoteFromTemplate => R::Soon("templates — coming soon"),
        A::NoteRename => note_dialog(NoteDialog::Rename),
        A::NoteMove => note_dialog(NoteDialog::Move),
        A::NoteProperties => note_dialog(NoteDialog::Properties),
        A::NoteDelete => note_dialog(NoteDialog::Delete),

        // +links
        A::LinksTab(tab) => R::LinksTab(tab),
        A::LinksGraph => R::Soon("local graph — coming soon"),

        // +git/sync — status is live; the rest are display-only stubs.
        A::GitStatus => R::GitStatus,
        A::GitSync | A::GitLog | A::GitDiff => R::Soon("git is display-only for now"),

        // +vault
        A::VaultSwitch => R::Overlay(OverlayOpen::WorkspaceSwitcher),
        A::VaultReindex => R::Job(LeaderJob::Reindex),
        A::VaultConfig => R::Drawer(DrawerView::Config),
        A::VaultTheme => R::Overlay(OverlayOpen::ThemePicker),
        A::VaultPreferences => R::OpenPreferences,
        A::AppOnboarding => R::OpenOnboarding,

        // +window
        A::WindowZen => R::Layout(LayoutStep::Zen),
        A::WindowSplit => R::Soon("editor splits — coming soon"),
        A::WindowGrowDrawer => R::Layout(LayoutStep::GrowDrawer),
        A::WindowShrinkDrawer => R::Layout(LayoutStep::ShrinkDrawer),

        // +this note
        A::NoteToggleTodo => R::Soon("toggle todo — coming soon"),
        A::NotePreview => R::Soon("preview — lands with phase 09"),
        A::NoteExport => R::Soon("export — coming soon"),
        A::NoteCopyWikilink => match facts.open_note() {
            Some(path) => R::Yank {
                text: format!("[[{}]]", path.get_clean_name()),
                label: "wikilink copied",
            },
            None => R::NoNoteOpen,
        },
        A::NoteYankPath => match facts.open_note() {
            Some(path) => R::Yank {
                text: path.to_string(),
                label: "note path copied",
            },
            None => R::NoNoteOpen,
        },
        A::NoteTogglePin => match facts.open_note() {
            Some(path) => R::Job(LeaderJob::TogglePin(path.clone())),
            None => R::NoNoteOpen,
        },
        A::PinnedJump(n) => R::Job(LeaderJob::JumpToPinned(n)),

        // +text
        A::TextBold => R::Text(TextAction::Bold),
        A::TextItalic => R::Text(TextAction::Italic),
        A::TextStrikethrough => R::Text(TextAction::Strikethrough),

        // +ask
        A::AskFocus => R::Ask(AskStep::Focus),
        A::AskNew => R::Ask(AskStep::New),
        A::AskCopy => R::Ask(AskStep::Copy),
        A::AskSave => R::Ask(AskStep::Save),
        A::AskRegenerate => R::Ask(AskStep::Regenerate),
        A::AskSource => R::Ask(AskStep::Source),

        A::Palette => R::Overlay(OverlayOpen::CommandPalette),
        A::Help => R::Overlay(OverlayOpen::Cheatsheet),
        A::NoteSave => R::Save,
        A::AppQuit => R::Quit,
    }
}

/// The asynchronous leader leaves. Each reports only on the app channel
/// (a flash, an `OpenPath`, an update notice), so either frontend can spawn
/// it on its own runtime and handle the messages it already handles.
pub mod jobs {
    use std::sync::Arc;

    use kimun_core::NoteVault;
    use kimun_core::error::{FSError, VaultError};
    use kimun_core::nfs::VaultPath;

    use crate::logic::message::{AppMessage, AppTx, UpdateFlow};

    fn flash(tx: &AppTx, msg: impl Into<String>) {
        tx.send(AppMessage::FlashMessage(msg.into())).ok();
    }

    /// Leader `m i`: pin `path`, or unpin it if pinned. Refused with a flash
    /// when the note is no longer on disk or the vault holds the cap.
    pub async fn toggle_pin(vault: Arc<NoteVault>, path: VaultPath, tx: AppTx) {
        let msg = match vault.toggle_pinned_note(&path).await {
            Ok(kimun_core::PinToggle::Pinned { position }) => format!(
                "pinned as {} of {}",
                position + 1,
                kimun_core::PINNED_NOTES_CAP
            ),
            Ok(kimun_core::PinToggle::Unpinned) => "unpinned".to_string(),
            Ok(kimun_core::PinToggle::Full) => format!(
                "already {} pinned — unpin one first",
                kimun_core::PINNED_NOTES_CAP
            ),
            Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
                format!("note not found: {path}")
            }
            Err(e) => {
                tracing::warn!("failed to toggle pin on {path}: {e}");
                format!("could not update pinned notes: {e}")
            }
        };
        flash(&tx, msg);
    }

    /// Leader digit `n`: open pinned note `n` (1-based). A slot with no pin,
    /// or a pin whose note is missing on disk, flashes instead.
    /// The rules are the pinned-notes dialog's ([`crate::logic::pinned`]).
    pub async fn jump_to_pinned(vault: Arc<NoteVault>, n: u8, tx: AppTx) {
        let rows = match crate::logic::pinned::load(&vault).await {
            Ok(rows) => rows,
            Err(e) => {
                flash(&tx, e);
                return;
            }
        };
        match crate::logic::pinned::open_target(&rows, usize::from(n).saturating_sub(1)) {
            Ok(path) => {
                tx.send(AppMessage::OpenPath {
                    path,
                    emphasis: None,
                })
                .ok();
            }
            Err(e) => flash(&tx, e),
        }
    }

    /// Leader `v r`: a fast reindex — the same pipeline the Settings screen
    /// runs — reported as a flash. The caller announces the start.
    pub async fn reindex(vault: Arc<NoteVault>, tx: AppTx) {
        let started = std::time::Instant::now();
        let result = vault.index_notes(kimun_core::NotesValidation::Fast).await;
        let msg = match result {
            Ok(_) => format!("reindexed in {:.1?}", started.elapsed()),
            Err(e) => format!("reindex failed: {e}"),
        };
        flash(&tx, msg);
    }

    /// Leader `u` with no cached notice: a forced update check. An update
    /// surfaces the notice *and* opens the dialog the user asked for (shown
    /// even if previously skipped); otherwise a flash says what happened.
    /// The caller announces the start.
    pub async fn check_updates(tx: AppTx) {
        let Ok(config_dir) = crate::settings::config_dir() else {
            return;
        };
        match crate::update::check_now(config_dir, true).await {
            Ok(Some(status)) if status.update_available => {
                tx.send(AppMessage::Update(UpdateFlow::Available(status)))
                    .ok();
                tx.send(AppMessage::Update(UpdateFlow::ShowDialog)).ok();
            }
            Ok(_) => flash(&tx, "kimün is up to date"),
            Err(e) => flash(&tx, format!("Update check failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests;
