//! The workspace's **session file** (ADR-0058, CONTEXT.md): the tabs, the
//! arrangement of groups in each, the note in each group and the active tab
//! and group, recorded as the window changes and restored when the
//! workspace opens.
//!
//! **Saving.** Every change the window is told of (a notify) is checked
//! against the record last written: a note opened or replaced in a group, a
//! tab made, closed, switched or reordered, a group split, closed, moved or
//! swapped, a divider dragged, another active group. A change waits
//! [`SESSION_DEBOUNCE`] for the next one (a divider drag is many), then the
//! newest record goes to a background writer that keeps only the newest
//! ([`SessionWriter`], the layout writer's pattern). A quit or a window close
//! writes it at once and waits for the write. A group's note is the one it
//! holds or is opening ([`super::note::NoteHost::target`]); a group showing
//! an attachment holds none. Without a workspace nothing is written.
//!
//! **Restoring.** Once the vault indexed, a record that still has a note to
//! open wins over the history: the region is rebuilt from it
//! ([`EditorRegion::from_record`], a gone note dropped from its group), every
//! group gets a state ([`Workspace::replace_region`]) and each note opens
//! in its group the normal way. With no record, or none of its notes left,
//! the history's newest note opens as before. Until the restore lands
//! nothing is written, so a quit meanwhile keeps the record; a change the
//! user makes meanwhile wins over it (the restore is dropped).
//!
//! **A workspace switch** writes the old workspace's record under the old
//! workspace, resets the region to one empty group, and restores the new
//! one's.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use gpui_kit::{Context, Subscription, Task};
use kimun_notes::logic::editor_region::EditorRegion;
use kimun_notes::settings::session::{SessionFile, SessionRecord};

use super::Workspace;
use super::group::GroupState;
use super::note::NoteHost;
use crate::startup::SessionRestore;

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

/// The window's side of the session file.
#[derive(Default)]
pub(crate) struct SessionHost {
    /// The open workspace's session file; `None` without a workspace.
    file: Option<SessionFile>,
    writer: SessionWriter,
    /// The record last written, restored or scheduled: a change that
    /// leaves the record as it is writes nothing.
    last: Option<SessionRecord>,
    /// A restore is on its way: the record of what the window held when it
    /// started. Nothing is written meanwhile; a change from it drops the
    /// restore.
    restoring: Option<SessionRecord>,
    /// The restore's wait for the indexing and the record's checks.
    restore_task: Option<Task<()>>,
    /// The debounce of the next write.
    timer: Option<Task<()>>,
    /// The window's notifies, each a possible change.
    _watch: Option<Subscription>,
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

    /// Record to `file` from now on (the workspace was entered). With
    /// `restoring`, a restore is on its way and nothing is written until it
    /// lands ([`Self::session_restored`]).
    pub(super) fn begin_session(&mut self, file: SessionFile, restoring: bool) {
        self.session.file = Some(file);
        self.session.last = None;
        self.session.restoring = restoring.then(|| self.session_record());
    }

    /// Stop recording (the workspace is left): what is not written yet is
    /// dropped, as is a restore on its way. [`Self::write_session_now`]
    /// first to keep it.
    pub(super) fn end_session(&mut self) {
        self.session.file = None;
        self.session.last = None;
        self.session.restoring = None;
        self.session.restore_task = None;
        self.session.timer = None;
    }

    /// Wait for the restore `job` (the indexing, then the record's checks),
    /// then apply what it found.
    pub(super) fn await_session_restore(
        &mut self,
        job: tokio::task::JoinHandle<Option<SessionRestore>>,
        cx: &mut Context<Self>,
    ) {
        self.session.restore_task = Some(cx.spawn(async move |this, cx| {
            let restored = job.await.ok().flatten();
            this.update(cx, |workspace, cx| workspace.session_restored(restored, cx))
                .ok();
        }));
    }

    /// The window changed (or was only redrawn): when the record differs
    /// from the last, write it after the debounce.
    fn session_changed(&mut self, cx: &mut Context<Self>) {
        if self.session.file.is_none() {
            return;
        }
        let record = self.session_record();
        if let Some(baseline) = &self.session.restoring {
            if *baseline == record {
                return;
            }
            // The user changed something first: theirs wins.
            self.session.restoring = None;
        }
        if self.session.last.as_ref() == Some(&record) {
            return;
        }
        self.session.last = Some(record);
        self.session.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SESSION_DEBOUNCE).await;
            this.update(cx, |workspace, _| workspace.write_session())
                .ok();
        }));
    }

    /// Hand the record of now to the writer. Nothing without a workspace,
    /// nor while a restore is on its way.
    fn write_session(&mut self) {
        self.session.timer = None;
        if self.session.restoring.is_some() {
            return;
        }
        let Some(file) = self.session.file.clone() else {
            return;
        };
        let record = self.session_record();
        self.session.last = Some(record.clone());
        self.session.writer.submit(file, record);
    }

    /// Write the record now, without the debounce, and wait for it: the
    /// window is about to go, or to leave its workspace.
    pub(super) fn write_session_now(&mut self) {
        self.write_session();
        self.session.writer.flush();
    }

    /// The restore's answer: `None` when the history restores instead (or
    /// the job failed). Dropped when the user changed something meanwhile.
    fn session_restored(&mut self, restored: Option<SessionRestore>, cx: &mut Context<Self>) {
        self.session.restore_task = None;
        let Some(baseline) = self.session.restoring.take() else {
            return;
        };
        let Some(restored) = restored else {
            // The history's note is on its way: it clears the Panel's
            // "opening your last note" when it lands.
            return;
        };
        self.restore_pending = false;
        self.first_open_pending = false;
        if self.session_record() != baseline
            || self.groups_closing.is_some()
            || !self.replace_region(restored.region, cx)
        {
            self.ensure_files_listing(cx);
            cx.notify();
            return;
        }
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
        // until the user changes something.
        self.session.last = Some(self.session_record());
        self.ensure_files_listing(cx);
        cx.notify();
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
    use kimun_notes::settings::session::{GroupTreeRecord, TabRecord};

    fn record(note: &str) -> SessionRecord {
        SessionRecord {
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
