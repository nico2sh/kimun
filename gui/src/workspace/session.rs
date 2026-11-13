//! The workspace's **session file** (ADR-0058, CONTEXT.md): the tabs, the
//! arrangement of groups in each, the note in each group and the active tab
//! and group, recorded as the window changes and restored when the
//! workspace opens.
//!
//! **Saving.** Every change the window is told of (a notify) is checked
//! against the record the file holds (last written, restored, or a
//! launch's baseline): a note opened or replaced in a group, a tab made,
//! closed, switched or reordered, a group split, closed, moved or swapped, a
//! divider dragged, another active group. A change waits
//! [`SESSION_DEBOUNCE`] for the next one (a divider drag is many), then the
//! newest record goes to a background writer that keeps only the newest
//! ([`SessionWriter`], the layout writer's pattern). A quit or a window close
//! writes it at once and waits for the write, when it differs from what the
//! file holds. A group's note is the one it holds or is opening
//! ([`super::note::NoteHost::target`]); a group showing an attachment holds
//! none. Without a workspace nothing is written.
//!
//! **Restoring.** A start with no note on the command line, or with one
//! that was refused, restores: once the vault indexed, a record that still
//! has a note to open wins over the history. The region is rebuilt from it
//! ([`EditorRegion::from_record`], a gone note dropped from its group), every
//! group gets a state ([`Workspace::replace_region`]) and each note opens
//! in its group the normal way. With no record, or none of its notes left,
//! the history's newest note opens as before (not after a refused
//! argument: the editor stays empty, as it did). A restore whose job fails
//! lands on the history too, and the stored record is kept (only a real
//! change writes). Only a stored record that loads makes a restore: with
//! none, opens go through at once as they always did. A stored file that
//! exists but does not load (a newer build's format, or a read that failed)
//! is kept the same way: the history's note (or the empty group) is no
//! change, only a real one writes. Until the restore
//! lands nothing is written, so a quit meanwhile keeps the record, and an
//! open asked for meanwhile waits:
//! the newest one opens afterwards, in the restored active group by the
//! plain open's rules (a note a restored group holds is focused there), or
//! in a new tab or split when that is what was asked for.
//!
//! **A note on the command line** opens alone in one group. Until the
//! first structural change (a tab or group added, removed, moved or
//! swapped) one tab of one group is no change, whatever note it holds:
//! opening another note there keeps the stored arrangement. After a
//! structural change every change is written as usual.
//!
//! **A workspace switch** writes the old workspace's record under the old
//! workspace, resets the region to one empty group, and restores the new
//! one's.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use gpui_kit::{Context, Subscription, Task};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor_region::EditorRegion;
use kimun_notes::logic::layout_tree::Axis;
use kimun_notes::logic::startup::RestoreTarget;
use kimun_notes::settings::session::{GroupTreeRecord, SessionFile, SessionRecord, TabRecord};

use super::Workspace;
use super::group::GroupState;
use super::note::NoteHost;
use crate::startup::{self, Landing, SessionRestore};

/// How long a change waits for the next before the record is written.
pub(crate) const SESSION_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Default)]
struct State {
    /// The newest record not written yet, and the file it goes to.
    pending: Option<(SessionFile, SessionRecord)>,
    /// A write thread is running (and will see `pending`).
    busy: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    idle: Condvar,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Writes session records off the UI thread, the newest only: records
/// that pile up while one is written collapse into one write of the
/// newest. In tests the write is inline.
#[derive(Clone)]
pub(crate) struct SessionWriter {
    shared: Arc<Shared>,
    inline: bool,
}

/// Tests write inline, so a test reads the file right after the write.
const WRITE_INLINE: bool = cfg!(test);

impl Default for SessionWriter {
    fn default() -> Self {
        Self {
            shared: Arc::default(),
            inline: WRITE_INLINE,
        }
    }
}

impl SessionWriter {
    /// A writer that writes in the background even under test.
    #[cfg(test)]
    fn threaded() -> Self {
        Self {
            inline: false,
            ..Self::default()
        }
    }

    /// Write `record` to `file`.
    pub(crate) fn submit(&self, file: SessionFile, record: SessionRecord) {
        if self.inline {
            write(&file, &record);
            return;
        }
        let mut state = self.shared.lock();
        state.pending = Some((file, record));
        if state.busy {
            return;
        }
        state.busy = true;
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            loop {
                let (file, record) = {
                    let mut state = shared.lock();
                    match state.pending.take() {
                        Some(job) => job,
                        None => {
                            state.busy = false;
                            shared.idle.notify_all();
                            return;
                        }
                    }
                };
                write(&file, &record);
            }
        });
    }

    /// Wait until every record submitted so far is written.
    pub(crate) fn flush(&self) {
        let mut state = self.shared.lock();
        while state.busy {
            state = self
                .shared
                .idle
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// One write. The record is non-critical: a failure is logged, never shown.
fn write(file: &SessionFile, record: &SessionRecord) {
    if let Err(e) = file.save(record) {
        tracing::warn!("could not save the session to {file}: {e}");
    }
}

/// How the window entered the workspace, for the record.
pub(crate) enum SessionStart {
    /// Land on the session record once the vault indexed; with `history`,
    /// on the history's newest note when the record has no note left.
    Restore { history: bool },
    /// The note named on the command line opens alone: the record stays as
    /// it is until the first structural change.
    Launch,
    /// A stored file exists but does not load (another build's format, a
    /// read that failed): nothing restores from it and opens go through at
    /// once, but it is kept until a real change, as after a failed restore.
    /// With `history`, the history's newest note lands.
    Unreadable { history: bool },
    /// No stored record: nothing restores from it, and opens go through at
    /// once.
    Fresh,
}

/// Where an open asked for during a restore goes once it landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueuedOpen {
    /// The active group, by the plain open's rules.
    Plain,
    /// A new tab ([`Workspace::open_note_in_new_tab`]).
    NewTab,
    /// A new group split off the active one ([`Workspace::open_note_in_split`]).
    Split(Axis),
}

/// The window's side of the session file.
#[derive(Default)]
pub(crate) struct SessionHost {
    /// The open workspace's session file; `None` without a workspace.
    file: Option<SessionFile>,
    writer: SessionWriter,
    /// The record the file holds as far as the window knows: the one last
    /// written or restored, or a launch's baseline. A record equal to it is
    /// not a change.
    last: Option<SessionRecord>,
    /// An empty start (one empty group, before the history's note lands, or
    /// for good when it will not open): not a change either, until the
    /// first real change.
    launch: Option<SessionRecord>,
    /// A command-line launch: until the first structural change, one tab
    /// of one group is no change, whatever note it holds.
    structure_only: bool,
    /// The record last seen at a notify: a notify that leaves it as it is
    /// does not restart the debounce.
    seen: Option<SessionRecord>,
    /// A restore is on its way: nothing is written, and an open asked for
    /// meanwhile waits for it (`queued`).
    restoring: bool,
    /// The restore falls back to the history's newest note.
    history: bool,
    /// The newest open asked for while the restore was on its way, its
    /// search needles and where it goes: it opens once the restore landed.
    queued: Option<(VaultPath, Option<Vec<String>>, QueuedOpen)>,
    /// The restore's wait for the indexing and the record's checks.
    restore_task: Option<Task<()>>,
    /// The debounce of the next write.
    timer: Option<Task<()>>,
    /// The window's notifies, each a possible change.
    _watch: Option<Subscription>,
}

/// Whether `record` is one tab of one group, whatever note it holds: the
/// structure a command-line launch starts with.
fn is_one_group(record: &SessionRecord) -> bool {
    matches!(
        record.tabs.as_slice(),
        [TabRecord {
            tree: GroupTreeRecord::Leaf { .. },
            ..
        }]
    )
}

/// The record of one group holding `note`: what a start landing on `note`
/// shows.
fn one_group(note: Option<VaultPath>) -> SessionRecord {
    SessionRecord::new(
        vec![TabRecord {
            tree: GroupTreeRecord::Leaf { note },
            active: 0,
        }],
        0,
    )
}

impl Workspace {
    /// Watch the window's own changes for the record ([module docs](self)).
    pub(super) fn watch_session(&mut self, cx: &mut Context<Self>) {
        // A weak observer: the window must not keep itself alive.
        let this = cx.entity();
        self.session._watch =
            Some(cx.observe(&this, |workspace, _, cx| workspace.session_changed(cx)));
    }

    /// What the window shows now, as a record.
    pub(crate) fn session_record(&self) -> SessionRecord {
        self.layout.region().to_record(|group| {
            self.state(group)
                .and_then(|s| s.notes.as_ref())
                .and_then(NoteHost::target)
        })
    }

    /// Record to `file` from now on (the workspace was entered), as `start`
    /// says.
    pub(super) fn begin_session(&mut self, file: SessionFile, start: SessionStart) {
        self.session.file = Some(file);
        self.session.seen = None;
        self.session.queued = None;
        self.session.last = None;
        self.session.launch = None;
        self.session.structure_only = false;
        self.session.restoring = false;
        self.session.history = false;
        match start {
            SessionStart::Restore { history } => {
                self.session.restoring = true;
                self.session.history = history;
            }
            SessionStart::Launch => self.session.structure_only = true,
            SessionStart::Unreadable { history } => {
                let landed = history.then(|| self.history_newest()).flatten();
                self.keep_stored_record(landed);
            }
            SessionStart::Fresh => {}
        }
    }

    /// The history's newest note, unchecked: where a start with no record
    /// to restore lands (as with a vault that did not index).
    fn history_newest(&self) -> Option<VaultPath> {
        self.settings.as_ref().and_then(|settings| {
            settings
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .current_last_paths()
                .first()
                .cloned()
        })
    }

    /// The stored file was never applied: the one group the start lands on
    /// (`landed` when it is a note, or empty before it lands or when it
    /// will not) must not overwrite it. Only a real change does.
    fn keep_stored_record(&mut self, landed: Option<VaultPath>) {
        let landed = landed.filter(VaultPath::is_note);
        self.session.last = Some(one_group(landed));
        self.session.launch = Some(one_group(None));
        self.session.seen = None;
    }

    /// Stop recording (the workspace is left): what is not written yet is
    /// dropped, as is a restore on its way and an open waiting for it.
    /// [`Self::write_session_now`] first to keep it.
    pub(super) fn end_session(&mut self) {
        self.session.file = None;
        self.session.last = None;
        self.session.launch = None;
        self.session.structure_only = false;
        self.session.seen = None;
        self.session.restoring = false;
        self.session.queued = None;
        self.session.restore_task = None;
        self.session.timer = None;
    }

    /// Whether a restore is on its way: an open asked for now waits for it
    /// ([`Self::queue_open`]).
    pub(super) fn session_restoring(&self) -> bool {
        self.session.restoring
    }

    /// Keep `path` (with its search needles) to open where `kind` says
    /// once the restore landed. A newer open replaces it.
    pub(super) fn queue_open(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        kind: QueuedOpen,
    ) {
        self.session.queued = Some((path, emphasis, kind));
    }

    /// Wait for the restore `job` (the indexing, then the record's checks),
    /// then apply what it found. A job that failed (it panicked) lands on
    /// the history, as a start without a record does.
    pub(super) fn await_session_restore(
        &mut self,
        job: tokio::task::JoinHandle<Option<Landing>>,
        cx: &mut Context<Self>,
    ) {
        self.session.restore_task = Some(cx.spawn(async move |this, cx| {
            let landing = job.await;
            this.update(cx, |workspace, cx| match landing {
                Ok(landing) => workspace.session_restored(landing, cx),
                Err(e) => {
                    tracing::warn!("the session restore failed: {e}");
                    workspace.session_restore_failed(cx)
                }
            })
            .ok();
        }));
    }

    /// Whether `record` is what the file holds already.
    fn session_unchanged(&self, record: &SessionRecord) -> bool {
        self.session.last.as_ref() == Some(record)
            || self.session.launch.as_ref() == Some(record)
            || (self.session.structure_only && is_one_group(record))
    }

    /// The window changed (or was only redrawn): when the record differs
    /// from what the file holds, write it after the debounce.
    fn session_changed(&mut self, cx: &mut Context<Self>) {
        if self.session.file.is_none() || self.session.restoring {
            return;
        }
        let record = self.session_record();
        if self.session.seen.as_ref() == Some(&record) {
            return;
        }
        let unchanged = self.session_unchanged(&record);
        self.session.seen = Some(record);
        if unchanged {
            // Back to what the file holds: nothing to write.
            self.session.timer = None;
            return;
        }
        self.session.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SESSION_DEBOUNCE).await;
            this.update(cx, |workspace, _| workspace.write_session())
                .ok();
        }));
    }

    /// Hand the record of now to the writer when it differs from what the
    /// file holds. Nothing without a workspace, nor while a restore is on
    /// its way.
    fn write_session(&mut self) {
        self.session.timer = None;
        if self.session.restoring {
            return;
        }
        let Some(file) = self.session.file.clone() else {
            return;
        };
        let record = self.session_record();
        if self.session_unchanged(&record) {
            return;
        }
        self.session.last = Some(record.clone());
        // A real change: an empty start, and a launch's one group, are
        // states like any other from now on.
        self.session.launch = None;
        self.session.structure_only = false;
        self.session.writer.submit(file, record);
    }

    /// [`Self::write_session`] now, without the debounce, and wait for it:
    /// the window is about to go, or to leave its workspace.
    pub(super) fn write_session_now(&mut self) {
        self.write_session();
        self.session.writer.flush();
    }

    /// The restore's answer ([`Landing`]); `None` when nothing restores
    /// (a refused argument with no record left).
    fn session_restored(&mut self, landing: Option<Landing>, cx: &mut Context<Self>) {
        self.session.restore_task = None;
        if !self.session.restoring {
            return;
        }
        match landing {
            Some(Landing::Session(restored)) => self.land_on_record(restored, cx),
            Some(Landing::History(target)) => {
                self.session.restoring = false;
                // As before the record: the open asked for meanwhile goes
                // first, then the history's note.
                self.open_queued(cx);
                if let Some(tx) = self.tx.clone() {
                    startup::send_history(target, &tx);
                }
            }
            None => {
                self.session.restoring = false;
                self.restore_pending = false;
                self.first_open_pending = false;
                self.open_queued(cx);
            }
        }
        self.ensure_files_listing(cx);
        cx.notify();
    }

    /// The restore failed: land on the history's newest note (unchecked,
    /// as with a vault that did not index), or nowhere for a refused
    /// argument.
    fn session_restore_failed(&mut self, cx: &mut Context<Self>) {
        self.session.restore_task = None;
        if !self.session.restoring {
            return;
        }
        let newest = self
            .session
            .history
            .then(|| self.history_newest())
            .flatten();
        let target: Option<RestoreTarget> = self.session.history.then(|| RestoreTarget {
            path: newest.clone().unwrap_or_else(VaultPath::root),
            gone: None,
        });
        self.session_restored(target.map(Landing::History), cx);
        self.keep_stored_record(newest);
    }

    /// The open asked for while the restore was on its way, in the active
    /// group by the plain open's rules (a note another group holds is
    /// focused there).
    fn open_queued(&mut self, cx: &mut Context<Self>) {
        let Some((path, emphasis, kind)) = self.session.queued.take() else {
            return;
        };
        match kind {
            QueuedOpen::Plain => self.open_note_with(path, emphasis, cx),
            QueuedOpen::NewTab => self.open_note_in_new_tab(path, cx),
            QueuedOpen::Split(axis) => self.open_note_in_split(path, axis, cx),
        }
    }

    /// The record wins: its region replaces the window's (one empty group
    /// at most, as every open waited), each note opens in its group, then
    /// the open asked for meanwhile in the active group.
    fn land_on_record(&mut self, restored: SessionRestore, cx: &mut Context<Self>) {
        self.restore_pending = false;
        self.first_open_pending = false;
        if self.groups_closing.is_some() || !self.replace_region(restored.region, cx) {
            // Not applied: the stored record is still the better one, so
            // what the window holds now is no change; only the user's own
            // is (the open asked for meanwhile, for one).
            self.session.restoring = false;
            let now = self.session_record();
            self.session.seen = Some(now.clone());
            self.session.last = Some(now);
            self.session.launch = None;
            self.open_queued(cx);
            return;
        }
        // Lifted before the record's own notes open, so they are not
        // queued as the user's would be.
        self.session.restoring = false;
        for (group, note) in restored.notes {
            self.open_note_in(group, note, cx);
        }
        if let Some(first) = restored.gone.first() {
            let more = match restored.gone.len() - 1 {
                0 => String::new(),
                1 => " (and 1 more note)".to_string(),
                n => format!(" (and {n} more notes)"),
            };
            self.flash(
                format!("{first} is gone (moved or deleted outside kimün){more}"),
                cx,
            );
        }
        // What the restore opened is what the file says: nothing to write
        // until something changes. The open asked for meanwhile is such a
        // change.
        let restored = self.session_record();
        self.session.seen = Some(restored.clone());
        self.session.last = Some(restored);
        self.open_queued(cx);
    }

    /// Put `region` in place of the live editor region: every group of it
    /// gets a state, the states of the groups before carried over in order
    /// (the active one first, so the window's first editor stays in use)
    /// and new ones made for the rest; states left over go. The one place a
    /// whole region is replaced, keeping `groups` in step with it (the
    /// invariant [`Self::replace_layout`] keeps for the slots). The groups
    /// before must hold no note. False (nothing changes) when a new state
    /// cannot be made (no vault).
    pub(super) fn replace_region(&mut self, region: EditorRegion, cx: &mut Context<Self>) -> bool {
        let wanted = region.all_groups();
        let mut made = Vec::new();
        for _ in self.groups.len()..wanted.len() {
            let Some(state) = self.new_group_state(cx) else {
                return false;
            };
            made.push(state);
        }
        let mut carried: VecDeque<GroupState> = self
            .group_ids_active_first()
            .into_iter()
            .filter_map(|group| self.groups.remove(&group))
            .collect();
        // Anything else (there should be nothing) goes too.
        self.groups.clear();
        *self.layout.region_mut() = region;
        let mut made = made.into_iter();
        for group in wanted {
            match carried.pop_front() {
                Some(state) => {
                    self.groups.insert(group, state);
                }
                None => {
                    let state = made.next().expect("a state was made for every new group");
                    self.insert_group(group, state, cx);
                }
            }
        }
        // Ids of the region before mean nothing now (a new region numbers
        // its groups afresh).
        self.note_gone = None;
        self.tab_hover = None;
        self.groups_closing = None;
        self.with_window(cx, |workspace, window, cx| {
            workspace.end_group_drag(window, cx)
        });
        debug_assert!(
            self.layout
                .region()
                .all_groups()
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                == self.groups.keys().copied().collect(),
            "groups out of step with the region"
        );
        self.sync_active_status(cx);
        self.focus_shown_content(cx);
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::nfs::VaultPath;
    use kimun_core::system::SystemPath;
    use kimun_notes::settings::session::{GroupTreeRecord, SESSION_VERSION, TabRecord};

    fn record(note: &str) -> SessionRecord {
        SessionRecord {
            version: SESSION_VERSION,
            tabs: vec![TabRecord {
                tree: GroupTreeRecord::Leaf {
                    note: Some(VaultPath::new(note)),
                },
                active: 0,
            }],
            shown: 0,
        }
    }

    #[test]
    fn a_burst_ends_with_the_newest_record_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = SystemPath::try_absolute(dir.path()).unwrap();
        let file = SessionFile::in_dir(&root, "ws");
        let writer = SessionWriter::threaded();
        for i in 0..50 {
            writer.submit(file.clone(), record(&format!("n{i}.md")));
        }
        writer.flush();
        assert_eq!(file.load(), Some(record("n49.md")));
    }

    #[test]
    fn flush_with_nothing_pending_returns() {
        SessionWriter::threaded().flush();
    }
}
