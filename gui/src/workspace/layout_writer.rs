//! The layout's writes to the config, off the UI thread.
//!
//! Every arrangement change saves the layout, and a held grow key or a
//! divider drag saves it many times a second. Each save is a blocking read-
//! modify-write of the config, so they go to a background thread that keeps
//! only the **latest** layout: saves that pile up while one is writing
//! collapse into a single write of the newest. [`LayoutWriter::flush`] waits
//! for the writes in flight (the app's quit does).
//!
//! In tests the write is inline, so a test reads the file right after the
//! key that changed the layout; `LayoutWriter::threaded` opts a test into
//! the real thing. The newest-only writer itself ([`LatestWriter`]) is
//! shared with the session file's writer.

use std::sync::{Arc, Condvar, Mutex};

use kimun_notes::logic::layout_tree::LayoutTree;
use kimun_notes::logic::nav::DrawerView;

use super::reopen::SETTINGS_WRITE;
use super::{LayoutStore, PresetRecord, RailState};

/// [`PresetRecord`] owned, to wait in the queue.
#[derive(Clone, Debug, PartialEq)]
enum Preset {
    Keep,
    Set(String),
    Clear,
}

impl Preset {
    fn from(record: PresetRecord<'_>) -> Self {
        match record {
            PresetRecord::Keep => Preset::Keep,
            PresetRecord::Set(name) => Preset::Set(name.to_string()),
            PresetRecord::Clear => Preset::Clear,
        }
    }

    fn as_record(&self) -> PresetRecord<'_> {
        match self {
            Preset::Keep => PresetRecord::Keep,
            Preset::Set(name) => PresetRecord::Set(name),
            Preset::Clear => PresetRecord::Clear,
        }
    }
}

/// The job waiting for the write thread, and whether one runs.
struct State<J> {
    /// The newest job not written yet.
    pending: Option<J>,
    /// A write thread is running (and will see `pending`).
    busy: bool,
}

struct Shared<J> {
    state: Mutex<State<J>>,
    idle: Condvar,
}

impl<J> Shared<J> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<J>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Writes jobs off the UI thread, the newest only: jobs that pile up while
/// one is written collapse into one write of the newest. In tests the write
/// is inline, so a test reads the file right after the submit;
/// [`Self::threaded`] opts a test into the real thing. The layout's writes
/// and the session file's ([`super::session::SessionWriter`]) both go
/// through one.
pub(crate) struct LatestWriter<J> {
    shared: Arc<Shared<J>>,
    write: Arc<dyn Fn(J) + Send + Sync>,
    inline: bool,
}

impl<J> Clone for LatestWriter<J> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            write: self.write.clone(),
            inline: self.inline,
        }
    }
}

impl<J: Send + 'static> LatestWriter<J> {
    /// A writer that hands every job it writes to `write`.
    pub(crate) fn new(write: impl Fn(J) + Send + Sync + 'static) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    pending: None,
                    busy: false,
                }),
                idle: Condvar::new(),
            }),
            write: Arc::new(write),
            inline: cfg!(test),
        }
    }

    /// This writer, writing in the background even under test.
    #[cfg(test)]
    pub(crate) fn threaded(self) -> Self {
        Self {
            inline: false,
            ..self
        }
    }

    /// Write `job`, replacing one still waiting.
    pub(crate) fn submit(&self, job: J) {
        self.submit_merged(job, |_, job| job);
    }

    /// Write `job`; a job still waiting is replaced by `merge(waiting,
    /// job)`.
    pub(crate) fn submit_merged(&self, job: J, merge: impl FnOnce(&J, J) -> J) {
        if self.inline {
            (self.write)(job);
            return;
        }
        let mut state = self.shared.lock();
        let job = match &state.pending {
            Some(waiting) => merge(waiting, job),
            None => job,
        };
        state.pending = Some(job);
        if state.busy {
            return;
        }
        state.busy = true;
        let (shared, write) = (self.shared.clone(), self.write.clone());
        std::thread::spawn(move || {
            loop {
                let job = {
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
                write(job);
            }
        });
    }

    /// Wait until every job submitted so far is written.
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

/// A layout waiting to be written, with the preset record and rail state
/// it carries.
type LayoutJob = (LayoutTree<DrawerView>, Preset, RailState);

/// Saves the layout to `store`, the newest only.
#[derive(Clone)]
pub(crate) struct LayoutWriter(LatestWriter<LayoutJob>);

impl LayoutWriter {
    pub(crate) fn new(store: LayoutStore) -> Self {
        Self(LatestWriter::new(
            move |(layout, preset, rail): LayoutJob| {
                write(&store, &layout, preset.as_record(), rail)
            },
        ))
    }

    /// A writer that writes in the background even under test.
    #[cfg(test)]
    pub(crate) fn threaded(store: LayoutStore) -> Self {
        Self(Self::new(store).0.threaded())
    }

    /// Save `layout`, recording `preset` as [`LayoutStore::save`] does. A
    /// `Keep` never undoes a preset record still waiting to be written.
    pub(crate) fn submit(
        &self,
        layout: LayoutTree<DrawerView>,
        preset: PresetRecord<'_>,
        rail: RailState,
    ) {
        self.0
            .submit_merged((layout, Preset::from(preset), rail), |waiting, mut job| {
                if job.1 == Preset::Keep {
                    job.1 = waiting.1.clone();
                }
                job
            });
    }

    /// Wait until every layout submitted so far is written.
    pub(crate) fn flush(&self) {
        self.0.flush();
    }
}

/// One write, in turn with the settings writes (`reopen::write_settings`).
fn write(
    store: &LayoutStore,
    layout: &LayoutTree<DrawerView>,
    preset: PresetRecord<'_>,
    rail: RailState,
) {
    let _one_at_a_time = SETTINGS_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    store.save(layout, preset, rail);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_with(views: &[DrawerView]) -> LayoutTree<DrawerView> {
        let mut tree = LayoutTree::default_layout(DrawerView::Files);
        for view in views {
            tree.show_view(view, false);
        }
        tree
    }

    fn saved(path: &std::path::Path) -> kimun_notes::settings::AppSettings {
        toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_burst_ends_with_the_newest_layout_and_its_preset_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        let writer = LayoutWriter::threaded(LayoutStore::File(path.clone()));
        let rail = RailState {
            position: kimun_notes::logic::layout_tree::RailPosition::Left,
            hidden: false,
        };
        writer.submit(tree_with(&[]), PresetRecord::Set("focus"), rail);
        for _ in 0..50 {
            writer.submit(tree_with(&[DrawerView::Tags]), PresetRecord::Keep, rail);
        }
        writer.flush();
        let settings = saved(&path);
        assert_eq!(settings.gui.layout_preset().as_deref(), Some("focus"));
        let restored = crate::shell_tree::restore(&settings.gui, crate::shell_tree::RESTORE_CAPS);
        assert_eq!(restored, tree_with(&[DrawerView::Tags]));
    }

    #[test]
    fn flush_with_nothing_pending_returns() {
        let writer = LayoutWriter::threaded(LayoutStore::Off);
        writer.flush();
    }
}
