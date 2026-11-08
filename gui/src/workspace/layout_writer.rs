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
//! key that changed the layout; [`LayoutWriter::threaded`] opts a test into
//! the real thing.

use std::sync::{Arc, Condvar, Mutex};

use kimun_notes::logic::layout_tree::LayoutTree;
use kimun_notes::logic::nav::DrawerView;

use super::reopen::SETTINGS_WRITE;
use super::{LayoutStore, PresetRecord};

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

#[derive(Default)]
struct State {
    /// The newest layout not written yet, and the preset record it carries.
    pending: Option<(LayoutTree<DrawerView>, Preset)>,
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

/// Saves the layout to `store`, the newest only.
#[derive(Clone)]
pub(crate) struct LayoutWriter {
    store: LayoutStore,
    shared: Arc<Shared>,
    inline: bool,
}

impl LayoutWriter {
    pub(crate) fn new(store: LayoutStore) -> Self {
        Self {
            store,
            shared: Arc::default(),
            inline: cfg!(test),
        }
    }

    /// A writer that writes in the background even under test.
    #[cfg(test)]
    pub(crate) fn threaded(store: LayoutStore) -> Self {
        Self {
            inline: false,
            ..Self::new(store)
        }
    }

    /// Save `layout`, recording `preset` as [`LayoutStore::save`] does. A
    /// `Keep` never undoes a preset record still waiting to be written.
    pub(crate) fn submit(&self, layout: LayoutTree<DrawerView>, preset: PresetRecord<'_>) {
        if self.inline {
            write(&self.store, &layout, preset);
            return;
        }
        let mut preset = Preset::from(preset);
        let mut state = self.shared.lock();
        if preset == Preset::Keep
            && let Some((_, waiting)) = &state.pending
        {
            preset = waiting.clone();
        }
        state.pending = Some((layout, preset));
        if state.busy {
            return;
        }
        state.busy = true;
        let (store, shared) = (self.store.clone(), self.shared.clone());
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
                write(&store, &job.0, job.1.as_record());
            }
        });
    }

    /// Wait until every layout submitted so far is written.
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

/// One write, in turn with the settings writes (`reopen::write_settings`).
fn write(store: &LayoutStore, layout: &LayoutTree<DrawerView>, preset: PresetRecord<'_>) {
    let _one_at_a_time = SETTINGS_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    store.save(layout, preset);
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
        writer.submit(tree_with(&[]), PresetRecord::Set("focus"));
        for _ in 0..50 {
            writer.submit(tree_with(&[DrawerView::Tags]), PresetRecord::Keep);
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
