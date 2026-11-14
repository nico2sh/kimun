//! The workspace's **session file** (ADR-0058, CONTEXT.md): the tabs, the
//! arrangement of groups in each, the note in each group and the active tab
//! and group, recorded as the window changes and restored when the
//! workspace opens.
//!
//! **Saving.** Every change the window is told of (a notify) is checked,
//! without building the record, against what the record was built from at
//! the last change (the region and each group's note, [`Seen`]): a note
//! opened or replaced in a group, a tab made, closed, switched or
//! reordered, a group split, closed, moved or swapped, a divider dragged,
//! another active group. A hover, a flash or a redraw is none. A change
//! waits [`SESSION_DEBOUNCE`] for the next one (a divider drag is many),
//! then the record is built and, when it differs from the one the file
//! holds (last written, restored, or a launch's baseline), goes to a
//! background writer that keeps only the newest ([`SessionWriter`]). A
//! quit or a window close writes it at once and waits for the write, when
//! it differs from what the file holds. A group's note is the one it holds
//! or is opening ([`super::note::NoteHost::target`]); a group showing an
//! attachment holds none. Without a workspace nothing is written.
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
//! in a new tab or split when that is what was asked for. A new tab, a
//! split, a tab step and a group close asked for meanwhile wait the same
//! way (the restore would replace what they made): once it landed they run
//! in the order they were asked for, the open among them, on the restored
//! region, each on its active group then ([`Queued`]).
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

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use gpui_kit::{Context, Subscription, Task};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor_region::EditorRegion;
use kimun_notes::logic::layout_tree::{Axis, GroupId};
use kimun_notes::logic::startup::RestoreTarget;
use kimun_notes::settings::session::{GroupTreeRecord, SessionFile, SessionRecord, TabRecord};

use super::Workspace;
use super::group::GroupState;
use super::layout_writer::LatestWriter;
use super::note::NoteHost;
use crate::startup::{self, Landing, SessionRestore};

/// How long a change waits for the next before the record is written.
pub(crate) const SESSION_DEBOUNCE: Duration = Duration::from_millis(500);

/// Writes session records off the UI thread, the newest only: records
/// that pile up while one is written collapse into one write of the
/// newest ([`LatestWriter`], shared with the layout's writes). In tests the
/// write is inline.
#[derive(Clone)]
pub(crate) struct SessionWriter(LatestWriter<(SessionFile, SessionRecord)>);

impl Default for SessionWriter {
    fn default() -> Self {
        Self(LatestWriter::new(
            |(file, record): (SessionFile, SessionRecord)| write(&file, &record),
        ))
    }
}

impl SessionWriter {
    /// A writer that writes in the background even under test.
    #[cfg(test)]
    fn threaded() -> Self {
        Self(Self::default().0.threaded())
    }

    /// Write `record` to `file`.
    pub(crate) fn submit(&self, file: SessionFile, record: SessionRecord) {
        self.0.submit((file, record));
    }

    /// Wait until every record submitted so far is written.
    pub(crate) fn flush(&self) {
        self.0.flush();
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

/// What was asked for while a restore was on its way, run in order once
/// it landed ([`Workspace::run_queued`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Queued {
    /// An open of the note, with its search needles, where the kind says.
    /// Only the newest open is kept.
    Open(VaultPath, Option<Vec<String>>, QueuedOpen),
    /// [`Workspace::new_tab`].
    NewTab,
    /// A split of the active group ([`Workspace::split_focused`]).
    Split(Axis),
    /// [`Workspace::step_tab`].
    StepTab { forward: bool },
    /// [`Workspace::close_editor_group`] on the active group (the only
    /// group there is while a restore is on its way).
    CloseGroup,
}

/// What a record is built from, kept to tell a notify that changed the
/// record from one that did not without building it: the region (its tabs,
/// their groups, splits and weights, the shown tab and each tab's active
/// group) and the note each group is on ([`NoteHost::target`]). The record
/// is a function of these two, so an equal `Seen` is an equal record.
struct Seen {
    region: EditorRegion,
    targets: HashMap<GroupId, Option<VaultPath>>,
}

/// The note `state`'s group is on, as the record keeps it.
fn target_of(state: &GroupState) -> Option<&VaultPath> {
    state.notes.as_ref().and_then(NoteHost::target_ref)
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
    /// What the record was built from at the last notify that changed it
    /// ([`Seen`]): a notify that leaves it as it is (a hover, a flash, a
    /// redraw) builds nothing and does not restart the debounce.
    seen: Option<Seen>,
    /// A restore is on its way: nothing is written, and an open or a
    /// structural command asked for meanwhile waits for it (`queued`).
    restoring: bool,
    /// The restore falls back to the history's newest note.
    history: bool,
    /// What was asked for while the restore was on its way, in order (the
    /// newest open only): it runs once the restore landed.
    queued: Vec<Queued>,
    /// The restore's wait for the indexing and the record's checks.
    restore_task: Option<Task<()>>,
    /// The debounce of the next write.
    timer: Option<Task<()>>,
    /// The window's notifies, each a possible change.
    _watch: Option<Subscription>,
    /// Test-only: how many records were built ([`Workspace::session_record`]).
    #[cfg(test)]
    pub(super) builds: std::cell::Cell<usize>,
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
        #[cfg(test)]
        self.session.builds.set(self.session.builds.get() + 1);
        self.layout
            .region()
            .to_record(|group| self.host_in(group).and_then(NoteHost::target))
    }

    /// Record to `file` from now on (the workspace was entered), as `start`
    /// says.
    pub(super) fn begin_session(&mut self, file: SessionFile, start: SessionStart) {
        self.session.file = Some(file);
        self.session.seen = None;
        self.session.queued.clear();
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
        self.session.queued.clear();
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
        self.session
            .queued
            .retain(|queued| !matches!(queued, Queued::Open(..)));
        self.session.queued.push(Queued::Open(path, emphasis, kind));
    }

    /// Hold the structural `command` while a restore is on its way, to run
    /// after the ones before it once it landed. False (nothing held) when
    /// no restore is on its way: the caller runs it now.
    pub(super) fn hold_while_restoring(&mut self, command: Queued) -> bool {
        if !self.session.restoring {
            return false;
        }
        self.session.queued.push(command);
        true
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

    /// What the record would be built from now.
    fn seen_now(&self) -> Seen {
        Seen {
            region: self.layout.region().clone(),
            targets: self
                .groups
                .iter()
                .map(|(group, state)| (*group, target_of(state).cloned()))
                .collect(),
        }
    }

    /// Whether what the record is built from is as it was at the last
    /// notify that changed it: a compare, no record built, nothing cloned.
    fn seen_unchanged(&self) -> bool {
        self.session.seen.as_ref().is_some_and(|seen| {
            seen.region == *self.layout.region()
                && seen.targets.len() == self.groups.len()
                && self.groups.iter().all(|(group, state)| {
                    seen.targets
                        .get(group)
                        .is_some_and(|target| target.as_ref() == target_of(state))
                })
        })
    }

    /// Whether `record` is what the file holds already.
    fn session_unchanged(&self, record: &SessionRecord) -> bool {
        self.session.last.as_ref() == Some(record)
            || self.session.launch.as_ref() == Some(record)
            || (self.session.structure_only && is_one_group(record))
    }

    /// The window changed (or was only redrawn): when what the record is
    /// built from changed ([`Seen`]), (re)start the debounce. The record is
    /// built once, when it is over ([`Self::write_session`]), and written
    /// only when it differs from what the file holds: a change undone
    /// meanwhile writes nothing.
    fn session_changed(&mut self, cx: &mut Context<Self>) {
        if self.session.file.is_none() || self.session.restoring || self.seen_unchanged() {
            return;
        }
        self.session.seen = Some(self.seen_now());
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
                self.run_queued(cx);
                if let Some(tx) = self.tx.clone() {
                    startup::send_history(target, &tx);
                }
            }
            None => {
                self.session.restoring = false;
                self.restore_pending = false;
                self.first_open_pending = false;
                self.run_queued(cx);
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

    /// What was asked for while the restore was on its way, in order: the
    /// open in the active group by the plain open's rules (a note another
    /// group holds is focused there), or in a new tab or split; each
    /// structural command on the active group of then.
    fn run_queued(&mut self, cx: &mut Context<Self>) {
        // A close names the group the user saw while the restore was on its
        // way, the empty one the record replaced. Only a group a held new
        // tab, split or open made since is still there to close; before
        // that, the close would hit a restored group nobody asked about.
        let mut made_a_group = false;
        for queued in std::mem::take(&mut self.session.queued) {
            made_a_group |= matches!(
                queued,
                Queued::NewTab
                    | Queued::Split(_)
                    | Queued::Open(_, _, QueuedOpen::NewTab | QueuedOpen::Split(_))
            );
            match queued {
                Queued::Open(path, emphasis, QueuedOpen::Plain) => {
                    self.open_note_with(path, emphasis, cx)
                }
                Queued::Open(path, _, QueuedOpen::NewTab) => self.open_note_in_new_tab(path, cx),
                Queued::Open(path, _, QueuedOpen::Split(axis)) => {
                    self.open_note_in_split(path, axis, cx)
                }
                Queued::NewTab => self.new_tab(cx),
                Queued::Split(axis) => self.split_active_group(axis, cx),
                Queued::StepTab { forward } => self.step_tab(forward, cx),
                Queued::CloseGroup if !made_a_group => {}
                Queued::CloseGroup => {
                    let active = self.layout.active_group();
                    self.close_editor_group(active, cx);
                }
            }
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
            self.session.seen = Some(self.seen_now());
            self.session.last = Some(self.session_record());
            self.session.launch = None;
            self.run_queued(cx);
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
        self.session.seen = Some(self.seen_now());
        self.session.last = Some(self.session_record());
        self.run_queued(cx);
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
        // The open menu's group may be gone, or its id name another group.
        self.forget_note_menu(cx);
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
