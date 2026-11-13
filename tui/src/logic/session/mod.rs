//! **NoteSession** — the open note's lifecycle, shared by every frontend:
//! which note is open, saving it (explicit, periodic, before a switch, on
//! exit), opening another, and following a rename, a delete or a property
//! write that changed it on disk.
//!
//! The session asks for side effects instead of performing them: every entry
//! point returns an outcome (`SaveOutcome`, `OpenOutcome`, …) and the host
//! does the UI part — dialogs, the FILES marker and retitle, footer flashes,
//! drawers. The buffer is reached through [`NoteBuffer`], so one session
//! drives the TUI's textarea and nvim engines and the GUI's `Editor`.
//!
//! # Prepare / run / apply
//!
//! Every step that touches the vault comes in three pieces so a host that
//! cannot hold `&mut` borrows across an await (a GPUI `cx.spawn`) can drive
//! it: a **sync prepare** that borrows the session and the buffer and returns
//! an owned request, an **async run** that is `'static + Send` (it owns an
//! `Arc<NoteVault>` and the path), and a **sync apply** that takes the result
//! back. The `async fn` forms (`save_now`, `open`, `on_renamed`, …) compose
//! the three and are what the TUI awaits.
//!
//! | step | prepare | run | apply |
//! |---|---|---|---|
//! | background save | [`NoteSession::prepare_save`] | [`SaveRequest::run`] | [`NoteSession::on_autosave_completed`] |
//! | flush (save now) | [`NoteSession::prepare_flush`] | [`FlushRequest::run`] | [`NoteSession::apply_flush`] |
//! | open | [`NoteSession::begin_open`] | [`ReadRequest::run`] | [`NoteSession::finish_open`] |
//! | rename | [`NoteSession::begin_rename`] | [`ReadRequest::run`] | [`NoteSession::finish_rename`] |
//! | reload | [`NoteSession::begin_reload`] | [`ReadRequest::run`] | [`NoteSession::finish_reload`] |
//! | removed | [`NoteSession::begin_remove`] | [`FlushRequest::run`] | [`NoteSession::apply_flush`] |
//!
//! # Never two writers
//!
//! A background save lives in a single slot; a flush waits it out (at most
//! [`SAVE_TIMEOUT`]) before writing, and on a timeout aborts it and writes
//! nothing (the buffer stays dirty). While a flush is prepared and not yet
//! applied, [`NoteSession::spawn_autosave`] declines. A rename or a reload
//! aborts the background save before retargeting, so a stale write cannot
//! recreate a renamed-away path or revert a disk change.
//!
//! Only one flush may be outstanding: a second [`NoteSession::prepare_flush`]
//! before the first is applied (or dropped) is refused with
//! [`FlushPending`] and the host retries after applying the first.
//!
//! While an open is pending (between [`NoteSession::begin_open`] and
//! [`NoteSession::finish_open`]) [`NoteSession::path`] still names the old
//! note, and every save declines ([`NoteSession::prepare_save`] returns
//! `None`), so a tick, a Save chord or a second switch in that window can
//! never write the old buffer into the new path.
//!
//! There is no disk-conflict detection: `NoteVault::save_note` writes
//! unconditionally (last writer wins).
//!
//! # Driving contract (async)
//!
//! - Every `run` future and [`NoteSession::restart_autosave`] need a **tokio
//!   context**: they use `tokio::time`, tokio `JoinHandle`s, `tokio::spawn`
//!   and `NoteVault`'s tokio-backed I/O. Drive them where the runtime is
//!   entered (the GUI's main thread, or `runtime.spawn(..)` and await the
//!   handle); on an executor with no tokio reactor they panic.
//! - The `run` futures are `Send + 'static` (pinned by a compile-time test).
//! - **Dropping a [`FlushRequest`] (or its `run` future) before it finishes
//!   aborts the background save it took over** (`SingleSlotTask` aborts on
//!   drop): a cancelled host task kills that save mid-flight. Dropping a
//!   [`SaveRequest`] or [`ReadRequest`] future only cancels that write/read.
//!
//! # Hazards a frontend must avoid (kept as-is in the TUI)
//!
//! - **Flush after delete/move.** [`NoteSession::on_removed`] flushes a dirty
//!   buffer to the removed path, which recreates a deleted note (or leaves a
//!   copy at a moved note's old path). A deleted/moved *directory* holding the
//!   open note is not detected at all (`is_like` is slice equality). GUI
//!   rule: flush *before* issuing a delete or move; on removal stop the tick
//!   and discard (never flush); treat "`from` is an ancestor of the open
//!   note" as removed.
//! - **Rename drops edits.** [`NoteSession::on_renamed`] reloads the buffer
//!   from the new path. GUI rule: flush before running the rename.
//! - **Old text under a new path after a failed open.** On `NotFound` /
//!   `ReadFailed` the session names the new path but the buffer still holds
//!   the old note and the old tick keeps running, so typing autosaves the old
//!   text into the missing path. GUI rule: on `NotFound`/`ReadFailed` call
//!   [`NoteSession::stop_autosave`] and do not save until a note is opened or
//!   created (or clear the buffer).
//! - **Per-open config save.** The TUI host rewrites the whole `config.toml`
//!   on every open (vestigial; history lives in `HistoryFile`). GUI rule:
//!   never call `save_to_disk`; write only `[gui]` via `update_gui`.

pub mod autosave_timer;
mod buffer;

use std::future::Future;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_core::error::{FSError, VaultError};
use kimun_core::nfs::VaultPath;

use crate::logic::message::{AppMessage, AppTx};
use crate::settings::SharedSettings;
use crate::util::single_slot_task::SingleSlotTask;

pub use autosave_timer::AutosaveTimer;
pub use buffer::NoteBuffer;

/// Hard cap on every blocking save path so a stuck disk (NFS hang,
/// fsync stall) cannot freeze quit, navigation, or the next autosave
/// tick. The same value is used both to wait for an in-flight
/// background autosave and to bound our own synchronous save call —
/// the worst-case quit time is therefore one cap plus the
/// observability of `abort()` on a syscall in progress.
pub const SAVE_TIMEOUT: Duration = Duration::from_secs(5);

/// A save that wrote: the note and the raw title core recomputed from it
/// (the host retitles the note's FILES row from it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: VaultPath,
    pub title: String,
}

/// What a flush ([`NoteSession::save_now`], [`NoteSession::close`], the
/// save before a switch) did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The buffer was written and marked saved.
    Saved(Saved),
    /// Nothing to write: no buffer mounted, or it was clean.
    Clean,
    /// A background save did not finish within [`SAVE_TIMEOUT`]; it was
    /// aborted and nothing was written. The buffer stays dirty.
    InFlightTimedOut,
    /// The write failed or timed out. The buffer stays dirty.
    Failed(String),
    /// Another flush is outstanding ([`FlushPending`]); nothing was written.
    /// Never happens in the TUI (each flush is awaited in place).
    FlushPending,
}

/// A background save's result, the data of `AppMessage::AutosaveCompleted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveCompleted {
    pub path: VaultPath,
    /// The revision written, `None` when the save failed.
    pub saved_revision: Option<NonZeroU64>,
    /// The recomputed raw title, `None` when the save failed.
    pub title: Option<String>,
    /// Why the save failed, `None` on success.
    pub error: Option<String>,
}

impl From<SaveCompleted> for AppMessage {
    fn from(done: SaveCompleted) -> Self {
        AppMessage::AutosaveCompleted {
            path: done.path,
            saved_revision: done.saved_revision,
            title: done.title,
            error: done.error,
        }
    }
}

/// [`NoteSession::prepare_flush`] refused: another flush is still
/// outstanding (prepared and not yet applied or dropped). Nothing was taken
/// or captured; retry after applying the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushPending;

/// What [`NoteSession::on_autosave_completed`] found, for the host's
/// follow-ups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Retitle this note's row (any note, not only the open one).
    pub saved: Option<Saved>,
    /// The completion is for the open note (compared with `==`, as the
    /// completion carries the path the session itself baked in).
    pub is_open_note: bool,
    /// The save wrote (`saved_revision` was `Some`).
    pub wrote: bool,
    /// Why the save failed (for a flash), `None` on success.
    pub error: Option<String>,
}

/// What kind of open a path asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenKind {
    /// A note: save the current one, then read this one into the buffer.
    Note,
    /// An attachment: the host shows it in place of the note.
    Attachment,
    /// Anything else (a directory, a missing non-note): the host browses it.
    Directory,
}

/// What [`NoteSession::finish_open`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// The note is in the buffer, pushed to the history, autosave restarted.
    Opened(VaultPath),
    /// No note at the path. The session now names it (so a create lands
    /// there); the host offers to create it. History untouched.
    NotFound(VaultPath),
    /// The read failed for another reason; the host browses `parent`.
    ReadFailed { path: VaultPath, parent: VaultPath },
    /// A later `begin_open` replaced this one before it finished; ignore.
    Superseded,
}

/// The composed [`NoteSession::open`]'s result: the save before the switch,
/// and the open itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    /// `None` when no switch happened (an attachment or directory).
    pub saved: Option<SaveOutcome>,
    pub outcome: OpenResult,
}

/// Where [`NoteSession::open`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenResult {
    /// The path is an attachment; nothing was saved or changed.
    Attachment(VaultPath),
    /// The path is a directory (or a missing non-note); nothing changed.
    Directory(VaultPath),
    /// A note open ran.
    Note(OpenOutcome),
}

/// What [`NoteSession::on_renamed`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameOutcome {
    /// The renamed note is not the open one; only its row changes.
    NotOpen,
    /// The open note now lives at this path; the buffer was reloaded from
    /// it (clean) and autosave restarted.
    Retargeted(VaultPath),
    /// The renamed note could not be read; autosave is stopped and the host
    /// browses `parent` (a dirty buffer must not clobber the rewrite).
    ReloadFailed { parent: VaultPath },
}

/// What [`NoteSession::on_removed`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovedOutcome {
    /// Another entry went away; the host refreshes `parent` if it shows it.
    NotOpen { parent: VaultPath },
    /// The open note went away: autosave stopped, buffer flushed; the host
    /// browses `parent`.
    OpenNoteRemoved {
        saved: SaveOutcome,
        parent: VaultPath,
    },
}

/// The first, sync half of a reload ([`NoteSession::begin_reload`]).
#[derive(Debug)]
pub enum ReloadStep {
    /// Not the open note: nothing to do.
    NotOpen,
    /// The open note has unsaved edits: they win, and the autosave carrying
    /// them is kept. The host says the file changed underneath.
    KeptDirty,
    /// Read the note and hand the result to [`NoteSession::finish_reload`].
    Read(ReadRequest),
}

/// What [`NoteSession::on_reload_from_disk`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    NotOpen,
    KeptDirty,
    /// The buffer now holds the disk text, clean.
    Reloaded,
    /// The read failed; the buffer still holds the old text (the host warns
    /// an edit would revert the disk change).
    Failed,
}

/// An owned, `'static` background save: the text and revision captured from
/// the buffer at prepare time.
pub struct SaveRequest {
    vault: Arc<NoteVault>,
    path: VaultPath,
    text: String,
    revision: NonZeroU64,
}

impl SaveRequest {
    /// The note this request writes.
    pub fn path(&self) -> &VaultPath {
        &self.path
    }

    /// Write through core. Never fails: a failed write reports `None`s and
    /// the error text. Needs a tokio context (see the module docs).
    pub async fn run(self) -> SaveCompleted {
        let (saved_revision, title, error) =
            match self.vault.save_note(&self.path, &self.text).await {
                Ok((_, content)) => (Some(self.revision), Some(content.title), None),
                Err(e) => (None, None, Some(e.to_string())),
            };
        SaveCompleted {
            path: self.path,
            saved_revision,
            title,
            error,
        }
    }
}

/// An owned, `'static` flush: the background save it must wait out (moved
/// out of the session's slot) and the save it then makes. While it (or its
/// [`FlushResult`]) is alive, [`NoteSession::spawn_autosave`] declines and a
/// second [`NoteSession::prepare_flush`] is refused.
///
/// **Dropping it (or its `run` future) before it finishes aborts the
/// background save it took over.** Needs a tokio context.
pub struct FlushRequest {
    in_flight: SingleSlotTask<()>,
    save: Option<SaveRequest>,
    timeout: Duration,
    guard: Arc<()>,
}

impl FlushRequest {
    /// Wait out the background save (≤ [`SAVE_TIMEOUT`]; on a timeout abort
    /// it and write nothing), then write (≤ [`SAVE_TIMEOUT`]). Needs a tokio
    /// context; dropping the future mid-way aborts the background save.
    pub async fn run(mut self) -> FlushResult {
        // Dropping a JoinHandle detaches the task rather than cancelling it,
        // so on a timeout the prior save keeps running: writing on top of it
        // is the two-writer race the slot exists to prevent. abort() is
        // best-effort (it will not unwind a syscall in progress) but stops
        // the task at its next await point. The buffer stays dirty so the
        // next save retries.
        if self.in_flight.is_in_flight()
            && self
                .in_flight
                .await_with_timeout(self.timeout)
                .await
                .is_none()
        {
            self.in_flight.abort();
            return FlushResult {
                outcome: SaveOutcome::InFlightTimedOut,
                text: None,
                _guard: self.guard,
            };
        }
        let Some(req) = self.save else {
            return FlushResult {
                outcome: SaveOutcome::Clean,
                text: None,
                _guard: self.guard,
            };
        };
        // Same cap on our own save so quit cannot hang on a stuck disk.
        let save = req.vault.save_note(&req.path, &req.text);
        let outcome = match tokio::time::timeout(self.timeout, save).await {
            Ok(Ok((_, content))) => {
                return FlushResult {
                    outcome: SaveOutcome::Saved(Saved {
                        path: req.path,
                        title: content.title,
                    }),
                    text: Some(req.text),
                    _guard: self.guard,
                };
            }
            Ok(Err(e)) => SaveOutcome::Failed(e.to_string()),
            Err(_) => SaveOutcome::Failed("save timed out".to_string()),
        };
        FlushResult {
            outcome,
            text: None,
            _guard: self.guard,
        }
    }
}

/// A finished flush, applied with [`NoteSession::apply_flush`].
pub struct FlushResult {
    outcome: SaveOutcome,
    /// The text written, for the sync mark-saved.
    text: Option<String>,
    _guard: Arc<()>,
}

/// An owned, `'static` note read.
pub struct ReadRequest {
    vault: Arc<NoteVault>,
    path: VaultPath,
}

impl std::fmt::Debug for ReadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadRequest")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ReadRequest {
    pub fn path(&self) -> &VaultPath {
        &self.path
    }

    /// Read through core. Needs a tokio context; dropping the future only
    /// cancels the read.
    pub async fn run(self) -> NoteRead {
        let result = self.vault.get_note_text(&self.path).await;
        NoteRead {
            path: self.path,
            result,
        }
    }
}

/// A finished read, only made by [`ReadRequest::run`] and only consumed by
/// the session's `finish_*` (a host cannot forge one).
pub struct NoteRead {
    path: VaultPath,
    result: Result<String, VaultError>,
}

impl NoteRead {
    /// The note that was read.
    pub fn path(&self) -> &VaultPath {
        &self.path
    }
}

/// What opening `path` means, owned so a host can await it with no borrow
/// of the session. Only non-notes touch the vault (`entry_kind`). Needs a
/// tokio context.
pub async fn classify_open(vault: Arc<NoteVault>, path: VaultPath) -> OpenKind {
    if path.is_note() {
        return OpenKind::Note;
    }
    // Classify so a stray attachment open still lands in the attachment
    // view rather than the directory browser.
    match vault.entry_kind(&path).await {
        Ok(kimun_core::EntryKind::Attachment) => OpenKind::Attachment,
        _ => OpenKind::Directory,
    }
}

/// The open note's lifecycle. See the module docs.
pub struct NoteSession {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    /// What the editor area shows: the open note, the note a create is being
    /// offered for, or an attachment's path. The vault root before the
    /// first open.
    path: VaultPath,
    autosave: AutosaveTimer,
    /// The most recent background save. `is_in_flight()` is the source of
    /// truth for "is a save still in flight"; both completion and panic flip
    /// it, so one panicked task cannot disable autosave. Drop aborts it.
    autosave_task: SingleSlotTask<()>,
    /// One token shared by every outstanding [`FlushRequest`] /
    /// [`FlushResult`] (each holds a clone): `strong_count > 1` means a flush
    /// is outstanding.
    flush_token: Arc<()>,
    /// The note a [`Self::begin_open`] is reading, until its
    /// [`Self::finish_open`].
    pending_open: Option<VaultPath>,
    /// [`SAVE_TIMEOUT`]; shorter in tests.
    save_timeout: Duration,
}

impl NoteSession {
    pub fn new(vault: Arc<NoteVault>, settings: SharedSettings, path: VaultPath) -> Self {
        Self {
            vault,
            settings,
            path,
            autosave: AutosaveTimer::new(),
            autosave_task: SingleSlotTask::empty(),
            flush_token: Arc::new(()),
            pending_open: None,
            save_timeout: SAVE_TIMEOUT,
        }
    }

    /// What the editor area shows (see the field).
    pub fn path(&self) -> &VaultPath {
        &self.path
    }

    /// Point the session at `path` without reading it (an attachment shown
    /// in place of the note).
    pub fn set_path(&mut self, path: VaultPath) {
        self.path = path;
    }

    pub fn vault(&self) -> &Arc<NoteVault> {
        &self.vault
    }

    /// Whether a background save is running.
    pub fn is_autosave_in_flight(&self) -> bool {
        self.autosave_task.is_in_flight()
    }

    /// Abort the background save, if any (best-effort).
    pub fn abort_autosave(&mut self) {
        self.autosave_task.abort();
    }

    /// Stop the periodic autosave tick.
    pub fn stop_autosave(&mut self) {
        self.autosave.stop();
    }

    /// (Re)start the periodic tick (`AppMessage::Autosave` every
    /// `autosave_interval_secs`). Calls `tokio::spawn`: needs a tokio context.
    pub fn restart_autosave(&mut self, tx: &AppTx) {
        let interval = self.settings.read().unwrap().autosave_interval_secs;
        self.autosave.restart(interval, tx.clone());
    }

    /// The running tick's period in seconds, `None` while no tick runs.
    pub fn autosave_interval(&self) -> Option<u64> {
        self.autosave.interval_secs()
    }

    /// The settings' `autosave_interval_secs` may have changed (the
    /// preferences were saved): a running tick is restarted on the new
    /// period; a stopped one stays stopped (whatever stopped it starts it
    /// again, on the settings as they are then). True when it was restarted.
    /// Calls `tokio::spawn`: needs a tokio context.
    pub fn retime_autosave(&mut self, tx: &AppTx) -> bool {
        let wanted = self.settings.read().unwrap().autosave_interval_secs;
        match self.autosave.interval_secs() {
            Some(running) if running != wanted => {
                self.autosave.restart(wanted, tx.clone());
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_save_timeout(&mut self, timeout: Duration) {
        self.save_timeout = timeout;
    }

    #[cfg(test)]
    pub(crate) fn is_ticking(&self) -> bool {
        self.autosave.is_running()
    }

    /// Whether an open is between [`Self::begin_open`] and
    /// [`Self::finish_open`].
    pub fn is_open_pending(&self) -> bool {
        self.pending_open.is_some()
    }

    /// The note being opened, between [`Self::begin_open`] and
    /// [`Self::finish_open`].
    pub fn pending_open(&self) -> Option<&VaultPath> {
        self.pending_open.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn autosave_task_mut(&mut self) -> &mut SingleSlotTask<()> {
        &mut self.autosave_task
    }

    fn flushing(&self) -> bool {
        Arc::strong_count(&self.flush_token) > 1
    }

    fn read(&self, path: VaultPath) -> ReadRequest {
        ReadRequest {
            vault: self.vault.clone(),
            path,
        }
    }

    // ---- Saving ------------------------------------------------------------

    /// Capture a save of the open note, or `None` when there is no buffer,
    /// it is clean, or an open is pending (the buffer may not match
    /// [`Self::path`] then).
    pub fn prepare_save<B: NoteBuffer>(&self, buf: Option<&B>) -> Option<SaveRequest> {
        if self.pending_open.is_some() {
            return None;
        }
        let buf = buf.filter(|b| b.is_dirty())?;
        Some(SaveRequest {
            vault: self.vault.clone(),
            path: self.path.clone(),
            text: buf.get_text(),
            revision: buf.content_revision(),
        })
    }

    /// Fire-and-forget save (the periodic tick and the explicit Save): runs
    /// in the slot and reports `AppMessage::AutosaveCompleted` on `tx`, to be
    /// applied with [`Self::on_autosave_completed`]. Declines (returns
    /// `false`) while a save is in flight, a flush or an open is pending, or
    /// when there is nothing to save. The completion carries the error text
    /// of a failed write.
    pub fn spawn_autosave<B: NoteBuffer>(&mut self, buf: Option<&B>, tx: &AppTx) -> bool {
        // A previous task that hasn't reported completion yet still holds the
        // lock on the file system + SQLite path; let it finish first.
        if self.autosave_task.is_in_flight() || self.flushing() {
            return false;
        }
        let Some(req) = self.prepare_save(buf) else {
            return false;
        };
        let tx = tx.clone();
        self.autosave_task.spawn(async move {
            let _ = tx.send(req.run().await.into());
        });
        true
    }

    /// Apply a background save's result: the buffer is marked clean iff the
    /// completion is for the open note and the buffer is still at the
    /// revision written. Stale completions (another path, an older revision)
    /// mark nothing.
    pub fn on_autosave_completed<B: NoteBuffer>(
        &self,
        done: SaveCompleted,
        buf: Option<&mut B>,
    ) -> Completion {
        let is_open_note = done.path == self.path;
        if is_open_note
            && let Some(rev) = done.saved_revision
            && let Some(buf) = buf
        {
            buf.mark_saved_at_revision(rev);
        }
        // The slot needs no clearing: `is_in_flight()` flipped when the task
        // returned, and clearing here could wipe a fresher handle a flush
        // spawned meanwhile.
        Completion {
            error: done.error,
            wrote: done.saved_revision.is_some(),
            saved: done.title.map(|title| Saved {
                path: done.path,
                title,
            }),
            is_open_note,
        }
    }

    /// Prepare a flush: take the background save out of the slot (the flush
    /// waits it out) and capture the buffer if dirty (not while an open is
    /// pending). Background saves decline, and a second flush is refused with
    /// [`FlushPending`], until the result is applied or dropped.
    pub fn prepare_flush<B: NoteBuffer>(
        &mut self,
        buf: Option<&B>,
    ) -> Result<FlushRequest, FlushPending> {
        if self.flushing() {
            return Err(FlushPending);
        }
        Ok(FlushRequest {
            in_flight: std::mem::replace(&mut self.autosave_task, SingleSlotTask::empty()),
            save: self.prepare_save(buf),
            timeout: self.save_timeout,
            guard: self.flush_token.clone(),
        })
    }

    /// Let go of a flush that will not run: the background save it took
    /// over goes back to the slot and finishes as it would have (dropping
    /// the request would abort it). Background saves and flushes are
    /// allowed again once it is given back.
    pub fn give_back_flush(&mut self, flush: FlushRequest) {
        let FlushRequest { in_flight, .. } = flush;
        // Nothing else could start a save while the flush was out.
        if !self.autosave_task.is_in_flight() {
            self.autosave_task = in_flight;
        }
    }

    /// Apply a flush: a write of the open note marks the buffer saved
    /// (synchronously: clean iff the text written is still its text).
    pub fn apply_flush<B: NoteBuffer>(
        &mut self,
        result: FlushResult,
        buf: Option<&mut B>,
    ) -> SaveOutcome {
        if let (SaveOutcome::Saved(saved), Some(text), Some(buf)) =
            (&result.outcome, result.text, buf)
            && saved.path == self.path
        {
            buf.mark_saved(text);
        }
        result.outcome
    }

    /// Save now and wait (≤ 2 × [`SAVE_TIMEOUT`]): the save before a switch,
    /// before the properties dialog, on exit. Composes
    /// [`Self::prepare_flush`], [`FlushRequest::run`], [`Self::apply_flush`].
    pub async fn save_now<B: NoteBuffer>(&mut self, buf: Option<&mut B>) -> SaveOutcome {
        let Ok(req) = self.prepare_flush(buf.as_deref()) else {
            return SaveOutcome::FlushPending;
        };
        let result = req.run().await;
        self.apply_flush(result, buf)
    }

    /// The session is ending (quit, screen switch): stop the tick, flush.
    pub async fn close<B: NoteBuffer>(&mut self, buf: Option<&mut B>) -> SaveOutcome {
        self.autosave.stop();
        self.save_now(buf).await
    }

    // ---- Opening -----------------------------------------------------------

    /// What opening `path` means ([`classify_open`]). The future owns its
    /// inputs (no borrow of the session), so a host can await it anywhere.
    pub fn open_kind(&self, path: &VaultPath) -> impl Future<Output = OpenKind> + Send + 'static {
        classify_open(self.vault.clone(), path.clone())
    }

    /// Start opening the note `path` and return its read. Call after the
    /// save before the switch. [`Self::path`] keeps naming the old note
    /// until [`Self::finish_open`], and saves decline meanwhile. A later
    /// `begin_open` supersedes this one.
    pub fn begin_open(&mut self, path: VaultPath) -> ReadRequest {
        self.pending_open = Some(path.clone());
        self.read(path)
    }

    /// Give up the pending open (its read never came back: the task running
    /// it panicked or was cancelled). Nothing changes but the pending mark:
    /// [`Self::path`] still names the note the buffer holds, so saves resume.
    /// A read for the cancelled open arriving later is `Superseded`.
    pub fn cancel_open(&mut self) {
        self.pending_open = None;
    }

    /// Apply an open's read: load the buffer, record the history (only once
    /// the read succeeded: a note that isn't there was never opened), and
    /// restart the autosave tick. The session names the new path in every
    /// arm (`NotFound` and `ReadFailed` included, as the TUI always did); a
    /// failed read leaves the buffer and the tick alone (see the module docs'
    /// hazards). A read for anything but the latest `begin_open` is
    /// `Superseded` and changes nothing.
    pub fn finish_open<B: NoteBuffer>(
        &mut self,
        read: NoteRead,
        buf: Option<&mut B>,
        tx: &AppTx,
    ) -> OpenOutcome {
        if self.pending_open.as_ref() != Some(&read.path) {
            return OpenOutcome::Superseded;
        }
        self.pending_open = None;
        self.path = read.path.clone();
        match read.result {
            Ok(text) => {
                self.settings.read().unwrap().add_path_history(&read.path);
                if let Some(buf) = buf {
                    buf.open_note(read.path.clone(), text);
                }
                self.restart_autosave(tx);
                OpenOutcome::Opened(read.path)
            }
            Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
                OpenOutcome::NotFound(read.path)
            }
            Err(e) => {
                tracing::error!("Failed to read note {}: {e}", read.path);
                let parent = read.path.get_parent_path().0;
                OpenOutcome::ReadFailed {
                    path: read.path,
                    parent,
                }
            }
        }
    }

    /// Open `path`: classify, save the current note, read, load. The
    /// composed form for hosts with no work between the steps.
    pub async fn open<B: NoteBuffer>(
        &mut self,
        path: VaultPath,
        mut buf: Option<&mut B>,
        tx: &AppTx,
    ) -> Opening {
        match self.open_kind(&path).await {
            OpenKind::Attachment => {
                return Opening {
                    saved: None,
                    outcome: OpenResult::Attachment(path),
                };
            }
            OpenKind::Directory => {
                return Opening {
                    saved: None,
                    outcome: OpenResult::Directory(path),
                };
            }
            OpenKind::Note => {}
        }
        let saved = self.save_now(buf.as_deref_mut()).await;
        let read = self.begin_open(path).run().await;
        let outcome = self.finish_open(read, buf, tx);
        Opening {
            saved: Some(saved),
            outcome: OpenResult::Note(outcome),
        }
    }

    // ---- Changes on disk ---------------------------------------------------

    /// Whether a delete / move / rename of `from` touches what is open: `from`
    /// is the open path itself (`is_like`) or a directory holding it. The GUI
    /// flushes before such an op and retargets or discards after it (module
    /// docs, hazards). The TUI keeps its `is_like`-only checks
    /// ([`Self::begin_rename`], [`Self::begin_remove`]).
    pub fn affects_open_note(&self, from: &VaultPath) -> bool {
        from.is_like(&self.path) || from.is_ancestor_of(&self.path)
    }

    /// A note was renamed `from` → `to`. When it is the open note, abort the
    /// background save still targeting the old path (it bakes the path in,
    /// and `save_note` writes unconditionally: it would recreate the old
    /// file) and return the read of the new path. No flush: the old path no
    /// longer exists on disk.
    ///
    /// `is_like`, so an absolute `from` (an index row) matches a relative
    /// open path.
    pub fn begin_rename(&mut self, from: &VaultPath, to: &VaultPath) -> Option<ReadRequest> {
        if !from.is_like(&self.path) {
            return None;
        }
        self.autosave_task.abort();
        Some(self.read(to.clone()))
    }

    /// Apply a rename's read: retarget and reload the buffer clean (the
    /// rename may have rewritten self-links), restart the tick. A failed
    /// read stops the tick: a dirty buffer must not clobber the rewrite.
    pub fn finish_rename<B: NoteBuffer>(
        &mut self,
        read: NoteRead,
        buf: Option<&mut B>,
        tx: &AppTx,
    ) -> RenameOutcome {
        match read.result {
            Ok(text) => {
                self.path = read.path.clone();
                if let Some(buf) = buf {
                    buf.renamed_to(read.path.clone());
                    buf.set_text(text.clone());
                    buf.mark_saved(text);
                }
                self.restart_autosave(tx);
                RenameOutcome::Retargeted(read.path)
            }
            Err(_) => {
                self.autosave.stop();
                RenameOutcome::ReloadFailed {
                    parent: read.path.get_parent_path().0,
                }
            }
        }
    }

    /// [`Self::begin_rename`] + read + [`Self::finish_rename`].
    pub async fn on_renamed<B: NoteBuffer>(
        &mut self,
        from: &VaultPath,
        to: &VaultPath,
        buf: Option<&mut B>,
        tx: &AppTx,
    ) -> RenameOutcome {
        let Some(req) = self.begin_rename(from, to) else {
            return RenameOutcome::NotOpen;
        };
        let read = req.run().await;
        self.finish_rename(read, buf, tx)
    }

    /// An entry at `from` was deleted or moved away. When it is the open
    /// note, stop the tick and return the flush to run (the host then
    /// browses the parent); `None` otherwise. `is_like` as in
    /// [`Self::begin_rename`]: a miss would leave the stale autosave to
    /// recreate the note. `Some(Err(FlushPending))` when another flush is
    /// outstanding (the tick is still stopped).
    ///
    /// The TUI flushes here, after the delete/move: see the module docs'
    /// hazards for the rule a GUI follows instead.
    pub fn begin_remove<B: NoteBuffer>(
        &mut self,
        from: &VaultPath,
        buf: Option<&B>,
    ) -> Option<Result<FlushRequest, FlushPending>> {
        if !from.is_like(&self.path) {
            return None;
        }
        self.autosave.stop();
        Some(self.prepare_flush(buf))
    }

    /// [`Self::begin_remove`] + run + [`Self::apply_flush`].
    pub async fn on_removed<B: NoteBuffer>(
        &mut self,
        from: &VaultPath,
        buf: Option<&mut B>,
    ) -> RemovedOutcome {
        let Some(req) = self.begin_remove(from, buf.as_deref()) else {
            return RemovedOutcome::NotOpen {
                parent: from.get_parent_path().0,
            };
        };
        let saved = match req {
            Ok(req) => {
                let result = req.run().await;
                self.apply_flush(result, buf)
            }
            Err(FlushPending) => SaveOutcome::FlushPending,
        };
        RemovedOutcome::OpenNoteRemoved {
            saved,
            parent: self.path.get_parent_path().0,
        }
    }

    /// The note at `path` changed on disk (a property write). Unsaved
    /// typing wins: a dirty open note keeps its buffer and its autosave.
    /// Otherwise abort the background save and read the note back.
    pub fn begin_reload<B: NoteBuffer>(&mut self, path: &VaultPath, buf: Option<&B>) -> ReloadStep {
        if !path.is_like(&self.path) {
            return ReloadStep::NotOpen;
        }
        if buf.is_some_and(|b| b.is_dirty()) {
            return ReloadStep::KeptDirty;
        }
        self.autosave_task.abort();
        ReloadStep::Read(self.read(self.path.clone()))
    }

    /// Apply a reload's read: the buffer takes the disk text, clean.
    pub fn finish_reload<B: NoteBuffer>(
        &mut self,
        read: NoteRead,
        buf: Option<&mut B>,
    ) -> ReloadOutcome {
        match read.result {
            Ok(text) => {
                if let Some(buf) = buf {
                    buf.set_text(text.clone());
                    buf.mark_saved(text);
                }
                ReloadOutcome::Reloaded
            }
            Err(_) => ReloadOutcome::Failed,
        }
    }

    /// [`Self::begin_reload`] + read + [`Self::finish_reload`].
    pub async fn on_reload_from_disk<B: NoteBuffer>(
        &mut self,
        path: &VaultPath,
        buf: Option<&mut B>,
    ) -> ReloadOutcome {
        match self.begin_reload(path, buf.as_deref()) {
            ReloadStep::NotOpen => ReloadOutcome::NotOpen,
            ReloadStep::KeptDirty => ReloadOutcome::KeptDirty,
            ReloadStep::Read(req) => {
                let read = req.run().await;
                self.finish_reload(read, buf)
            }
        }
    }
}

#[cfg(test)]
mod tests;
