use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use tokio::sync::mpsc::UnboundedSender;

use kimun_core::{NoteVault, nfs::VaultPath};

use crate::components::file_list::{SortField, SortOrder};

/// Which panel a sort selection applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortTarget {
    Sidebar,
    Query,
}

/// The surface a save-current-query action sourced its query from. Carried
/// through the save-search dialog so the editor knows whether the Query
/// panel's breadcrumb should re-pin after the save — by identity, not by
/// comparing query text (equal text from different surfaces must not collide).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveSource {
    QueryPanel,
    NoteBrowser,
}

/// All events that flow through the system — both input events (from crossterm)
/// and app-level messages sent by components / screens to the main loop.
#[derive(Debug)]
pub enum AppEvent {
    Input(InputEvent),
    OpenScreen(ScreenEvent),

    // ── App-level messages ───────────────────────────────────────────────────
    Quit,
    Redraw,
    /// Background RAG sync task reporting its connection/sync status. Rendered
    /// in the editor footer.
    RagStatus(crate::rag::RagStatus),
    Autosave,
    /// Background autosave task finished. `saved_revision` carries the
    /// editor's `content_revision` at the moment the save was *issued*
    /// on success, `None` if the write failed. The editor screen uses
    /// `path` to ignore stale completions for notes the user has
    /// already navigated away from, and `saved_revision` to clear the
    /// dirty flag iff the buffer is still at that revision (i.e. no
    /// edits during the save). `NonZeroU64` because the editor's
    /// `content_revision` is never zero.
    AutosaveCompleted {
        path: VaultPath,
        saved_revision: Option<NonZeroU64>,
        /// The note's recomputed title (first body line) from the save, so the
        /// sidebar row can be retitled. `None` when the save failed.
        title: Option<String>,
    },
    /// Open a note (or directory) — `emphasis` carries the originating
    /// query's needles when the open comes from a query result, so the
    /// editor lights up the matched spans (spec §5.1). Use
    /// [`AppEvent::open`] for the plain case.
    OpenPath {
        path: VaultPath,
        emphasis: Option<Vec<String>>,
    },
    /// Open an attachment (a non-note file) in the editor area's read-only
    /// attachment view (see ADR-0017). Sent by the file browser when an
    /// attachment row is activated.
    OpenAttachment(VaultPath),
    FocusSidebar,
    /// Switch the drawer to the given view and reveal it (sent by the
    /// activity rail and, later, by leader paths / mouse clicks).
    OpenDrawerView(crate::components::drawer::DrawerView),
    /// Run the query `#<label>` in the FIND drawer (sent by the TAGS drawer).
    RunTagQuery(String),
    /// Jump the editor cursor to the first heading with this text (sent by
    /// the OUTLINE drawer).
    JumpToHeading(String),
    /// Run a leader-tree action (sent by the command palette after it has
    /// closed itself, so the action sees no open overlay).
    ExecuteLeaderAction(crate::keys::leader::LeaderAction),
    /// Show a transient footer flash — async tasks report results with it.
    FlashMessage(String),
    /// The self-update lifecycle (one owner in main.rs for the app-global
    /// bookkeeping, one in the editor screen for display).
    Update(UpdateFlow),
    /// Apply (and optionally persist) a resolved theme — sent by the theme
    /// picker: previews on selection move, persists on Enter. Carries the
    /// full `Theme` so applying never re-reads the themes directory.
    ApplyTheme {
        theme: Box<crate::settings::themes::Theme>,
        persist: bool,
    },
    /// Async-loaded backlink count for the link target under the editor
    /// cursor (status line 2's `→ target · N backlinks` affordance).
    LinkTargetMeta {
        target: String,
        count: usize,
    },
    /// Async-loaded backlink count for the note at `path` (status line 2).
    BacklinkCountLoaded {
        path: VaultPath,
        count: usize,
    },
    /// Async-loaded workspace git summary for the status bar, `None` when
    /// the workspace is not a git repository.
    GitStatusLoaded(Option<String>),
    /// Sent by PreferencesScreen when user confirms Save. The shared settings
    /// reference already contains the updated values.
    PreferencesSaved,
    /// Sent by OnboardingScreen when the user confirms Finish on the summary
    /// step. The shared settings already contain the committed draft; main.rs
    /// rebuilds the vault and navigates to Start (same as PreferencesSaved).
    OnboardingFinished,
    /// Sent by PreferencesScreen when user discards or closes unchanged.
    ClosePreferences,
    /// Sent by VaultSection; PreferencesScreen::handle_app_message intercepts.
    OpenFileBrowser,
    /// Sent by IndexingSection; PreferencesScreen intercepts.
    TriggerFastReindex,
    TriggerFullReindex,
    /// Sent by indexing tokio task on completion.
    IndexingDone(Result<Duration, String>),
    /// Open (or create) today's journal entry and switch to it in the editor.
    OpenJournal,
    /// Dismiss the active editor overlay (note browser, Saved Searches modal,
    /// or dialog). The single close path for everything owned by `OverlayHost`.
    CloseOverlay,
    /// Follow the link under the editor cursor: note name/path or external URL.
    FollowLink(String),
    /// Open the search modal pre-filled with `#<name>` to browse notes by label.
    FollowLabel(String),
    /// Insert raw text at the editor's cursor (replacing any active selection).
    /// Used by the screen layer to deliver async results back to the editor —
    /// e.g. the markdown link generated after a clipboard image is saved as an attachment.
    InsertAtCursor(String),

    /// File-operation requests and confirmations — owned by the editor
    /// screen's `handle_file_op`.
    FileOp(FileOp),
    /// An async result addressed to the open overlay (see **Overlay data** in
    /// CONTEXT.md). Routed only to the `OverlayHost`; with no (or the wrong)
    /// overlay open it is stale by definition and dropped.
    OverlayData(OverlayData),
    /// An async result addressed to the Ask workspace (see CONTEXT.md: Ask
    /// workspace). Its own family — Ask is a panel, not an overlay, so it is
    /// never routed through `OverlayData` (adr/0030).
    Ask(AskData),

    /// A vault was found to be structurally unusable (conflicts, invalid layout, etc.).
    /// Carries a formatted, human-readable error message.
    ///
    /// Handled by `handle_app_message` in `main.rs`, which clears the workspace,
    /// saves settings, and opens the settings screen with an error overlay.
    /// To add a new conflict source: emit this event from the detection site; no
    /// other files need to change.
    VaultConflict(String),

    // ── Workspace messages ──────────────────────────────────────────────
    /// User switched to a different workspace. Carries the workspace name.
    /// Handled by main.rs to rebuild the vault and navigate to StartScreen.
    WorkspaceSwitched(String),

    /// The saved-search save/select flow — owned by the editor screen's
    /// `handle_saved_search`.
    SavedSearch(SavedSearchFlow),

    /// Sort selection changed in the sort dialog — apply live to `target`.
    /// When `persist` is set (sidebar's "save as default"), also write the
    /// choice to settings. `group_directories` is sidebar-only (the query panel
    /// ignores it).
    SortChanged {
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        group_directories: bool,
        persist: bool,
    },
}

/// Async data addressed to the Ask workspace. Its own family — Ask is a
/// panel, and `OverlayData` is routed only to the OverlayHost (adr/0030).
#[derive(Debug)]
pub enum AskData {
    /// A completed (or failed) answer for the turn with this id. Stale ids
    /// (cleared thread, superseded regenerate) are dropped by `Thread`.
    AnswerReady {
        turn_id: u64,
        result: Result<(String, Vec<crate::ask::AskSource>), String>,
    },
    /// The note text the source reader asked for. `None` = load failed.
    ReaderNote {
        path: VaultPath,
        text: Option<String>,
    },
}

/// The self-update lifecycle. Two owners by design: `main.rs` keeps the
/// app-global copy (seeding later-opened screens, persisting dismissals) and
/// forwards; the editor screen owns display (footer indicator, dialog).
#[derive(Debug, Clone)]
pub enum UpdateFlow {
    /// A newer release was found by the background update check.
    Available(crate::update::UpdateStatus),
    /// User chose "Update now" in the update dialog → run the self-update.
    Apply,
    /// User skipped a version in the update dialog → persist the dismissal and
    /// clear the indicator. Carries the version being skipped.
    Dismiss(String),
    /// Open the update dialog for the currently-known update (manual check).
    ShowDialog,
    /// Self-update finished installing → clear the pending notice (restart
    /// still required to run the new binary).
    Applied,
}

/// File-operation requests (open a dialog) and confirmations (an operation
/// succeeded). One owner: the editor screen's `handle_file_op`.
#[derive(Debug, Clone)]
pub enum FileOp {
    /// Request to show the file-operations menu (delete / rename / move).
    ShowMenu(VaultPath),
    /// Request to show the delete confirmation dialog for the given entry.
    ShowDelete(VaultPath),
    /// Request to show the rename dialog for the given entry.
    ShowRename(VaultPath),
    /// Request to show the move dialog for the given entry.
    ShowMove(VaultPath),
    /// Request to show the create-note dialog pre-filled with body content —
    /// the Ask "save as note" action (`e` in `ThreadPanel`, adr/0030). Plain
    /// creates (follow-link, missing-note open) go straight through
    /// `ActiveDialog::create_note` inside the editor screen instead, since
    /// they already hold `vault` and don't need to cross a component
    /// boundary.
    ShowCreateWithContent { path: VaultPath, content: String },
    /// Notification that a note was just created at this path. The current
    /// screen refreshes its sidebar if it is browsing the note's directory.
    /// Opening the note is a separate concern (the creator emits `OpenPath`).
    Created(VaultPath),
    /// Confirmation that the given entry was successfully deleted.
    Deleted(VaultPath),
    /// Confirmation that an entry was successfully renamed.
    Renamed { from: VaultPath, to: VaultPath },
    /// Confirmation that an entry was successfully moved.
    Moved { from: VaultPath, to: VaultPath },
}

/// An async result addressed to the open overlay — **Overlay data** in
/// CONTEXT.md. The `OverlayHost` is the only consumer; arriving with no (or
/// the wrong) overlay open means the overlay was closed or replaced while
/// the task ran, so the result is stale and dropped.
#[derive(Debug, Clone)]
pub enum OverlayData {
    /// Rename dialog: name availability check result.
    RenameValidation { available: bool },
    /// Move dialog: directory list has loaded.
    MoveDirectoriesLoaded(Vec<VaultPath>),
    /// Move dialog: fuzzy filter results are ready.
    MoveFilterResults(Vec<VaultPath>),
    /// Move dialog: destination existence check result.
    MoveDestValidation { available: bool },
    /// Save-search dialog: existing saved-search names have loaded (drives
    /// the update/overwrite/save-new hint).
    SavedSearchNamesLoaded(Vec<String>),
    /// An overlay-initiated operation failed; carries a human-readable
    /// error message.
    Error(String),
    /// A RAG answer job finished (or failed) — delivered to the answer
    /// overlay. `request_id` correlates the result to the ask that produced
    /// it, so a late answer from a closed/superseded ask can't clobber the
    /// current overlay.
    RagAnswerReady {
        request_id: u64,
        result: std::result::Result<crate::rag::RagAnswer, String>,
    },
}

/// The saved-search save/select flow. One owner: the editor screen's
/// `handle_saved_search`.
#[derive(Debug, Clone)]
pub enum SavedSearchFlow {
    /// Persist a saved search (emitted by the save-search dialog on submit).
    /// `source` is the surface the query was sourced from, decided when the
    /// dialog opened — it drives whether the panel breadcrumb re-pins.
    Confirmed {
        name: String,
        query: String,
        source: SaveSource,
    },
    /// A saved search was written to disk (success path of `Confirmed`).
    /// The editor re-pins the panel breadcrumb here — only once the write
    /// actually succeeded.
    Persisted {
        name: String,
        query: String,
        source: SaveSource,
    },
    /// The background saved-search write failed; surface it to the user.
    SaveFailed { name: String },
    /// A saved search was chosen in the Saved Searches modal.
    Selected { query: String, name: String },
}

impl AppEvent {
    pub fn send_input(event: InputEvent) -> Self {
        AppEvent::Input(event)
    }

    /// `OpenPath` without query emphasis — the common case.
    pub fn open(path: kimun_core::nfs::VaultPath) -> Self {
        AppEvent::OpenPath {
            path,
            emphasis: None,
        }
    }
}

// ── Input events ────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// Bracketed-paste payload from the terminal. On macOS this is what
    /// Cmd+V delivers, since the terminal intercepts Cmd combos before they
    /// reach the TUI. The string may be empty when the clipboard holds only
    /// non-text content (e.g. an image).
    Paste(String),
}

// ── Screen events ────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum ScreenEvent {
    Start,
    OpenPreferences,
    /// Open the guided-setup (onboarding) screen.
    OpenOnboarding,
    /// Open the settings screen with an error overlay already shown.
    OpenPreferencesWithError(String),
    /// Navigate to the editor for the given vault root path.
    OpenEditor(Arc<NoteVault>, VaultPath),
    /// Navigate to the browse screen for the given vault root and directory path.
    OpenBrowse(Arc<NoteVault>, VaultPath),
}

/// Convenience alias used throughout the codebase.
pub type AppTx = UnboundedSender<AppEvent>;

/// Sender helpers for the create-then-open sequence shared by every
/// note-creation site (create dialog, quick note, note browser, sidebar,
/// journal).
pub trait AppTxExt {
    /// Announce a freshly created note so sidebars browsing its directory
    /// refresh, then open it. The notification is gated on `created` (an
    /// already-existing note needs no refresh); the note is opened regardless.
    fn announce_and_open(&self, path: VaultPath, created: bool);
}

impl AppTxExt for AppTx {
    fn announce_and_open(&self, path: VaultPath, created: bool) {
        if created {
            self.send(AppEvent::FileOp(FileOp::Created(path.clone())))
                .ok();
        }
        self.send(AppEvent::open(path)).ok();
    }
}

/// Build a `Send + Sync` callback that fires `AppEvent::Redraw` on the
/// app event bus. Used by long-lived components (autocomplete query
/// task, etc.) that need to wake the render loop from a background
/// thread but should not be aware of `AppEvent` themselves.
pub fn redraw_callback(tx: AppTx) -> Arc<dyn Fn() + Send + Sync + 'static> {
    Arc::new(move || {
        let _ = tx.send(AppEvent::Redraw);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn _assert_new_variants_exist(e: AppEvent) {
        match e {
            AppEvent::FileOp(FileOp::ShowDelete(_)) => {}
            AppEvent::FileOp(FileOp::ShowRename(_)) => {}
            AppEvent::FileOp(FileOp::ShowMove(_)) => {}
            AppEvent::FileOp(FileOp::ShowCreateWithContent {
                path: _,
                content: _,
            }) => {}
            AppEvent::FileOp(FileOp::Deleted(_)) => {}
            AppEvent::FileOp(FileOp::Renamed { from: _, to: _ }) => {}
            AppEvent::FileOp(FileOp::Moved { from: _, to: _ }) => {}
            AppEvent::OverlayData(OverlayData::Error(_)) => {}
            AppEvent::Ask(AskData::AnswerReady {
                turn_id: _,
                result: _,
            }) => {}
            AppEvent::Ask(AskData::ReaderNote { path: _, text: _ }) => {}
            _ => {}
        }
    }

    #[test]
    fn ask_data_variants_construct() {
        let _ = AppEvent::Ask(AskData::AnswerReady {
            turn_id: 1,
            result: Ok(("answer".to_string(), vec![])),
        });
        let _ = AppEvent::Ask(AskData::AnswerReady {
            turn_id: 2,
            result: Err("failed".to_string()),
        });
        let _ = AppEvent::Ask(AskData::ReaderNote {
            path: VaultPath::new("note.md"),
            text: Some("body".to_string()),
        });
        let _ = AppEvent::Ask(AskData::ReaderNote {
            path: VaultPath::new("missing.md"),
            text: None,
        });
    }

    #[test]
    fn sort_events_construct() {
        use crate::components::file_list::{SortField, SortOrder};
        let _ = AppEvent::SortChanged {
            target: SortTarget::Sidebar,
            field: SortField::Name,
            order: SortOrder::Ascending,
            group_directories: true,
            persist: false,
        };
        let _ = AppEvent::SortChanged {
            target: SortTarget::Query,
            field: SortField::Title,
            order: SortOrder::Descending,
            group_directories: false,
            persist: true,
        };
    }
}
