//! The editor's background work: the full parse and the full wrap of a large
//! buffer, run off the typing thread.
//!
//! [`EditorDisplay`] installs a placeholder parse (or an unwrapped layout stub)
//! when a buffer is too big to process inside a frame, and flags that a real
//! one is wanted. [`BackgroundJobs::spawn_pending`] starts it;
//! [`BackgroundJobs::pump`] installs whatever has finished. Both are called
//! once per frame from `EditorSession::prepare_frame`: drain first, spawn last.

use crate::logic::editor::display::EditorDisplay;
use crate::logic::editor::markdown::ParsedBuffer;
use crate::logic::message::Notifier;
use crate::util::single_slot_task::SingleSlotTask;

/// The background parse and layout slots, and the channels their results come
/// back on.
///
/// `SingleSlotTask` aborts the previous spawn on a fresh edit, so a burst of
/// edits resolves against the latest content.
pub struct BackgroundJobs {
    /// Background full-parse fallback for large buffers (perf #9). The display
    /// installs a placeholder `ParsedBuffer` and signals pending; this slot owns
    /// the spawned tokio task that runs the real `ParsedBuffer::parse`.
    full_parse_task: SingleSlotTask<()>,
    /// Background full-wrap fallback for large buffers, the layout-side twin of
    /// `full_parse_task`. The display installs a `Layout::unwrapped` stub and
    /// signals pending; this slot owns the spawned task that runs the real
    /// `Layout::compute`.
    layout_task: SingleSlotTask<()>,
    full_parse_tx: tokio::sync::mpsc::UnboundedSender<(u64, ParsedBuffer)>,
    full_parse_rx: tokio::sync::mpsc::UnboundedReceiver<(u64, ParsedBuffer)>,
    layout_tx: tokio::sync::mpsc::UnboundedSender<(u64, crate::ropetext::Layout)>,
    layout_rx: tokio::sync::mpsc::UnboundedReceiver<(u64, crate::ropetext::Layout)>,
}

impl BackgroundJobs {
    pub fn new() -> Self {
        let (full_parse_tx, full_parse_rx) = tokio::sync::mpsc::unbounded_channel();
        let (layout_tx, layout_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            full_parse_task: SingleSlotTask::empty(),
            layout_task: SingleSlotTask::empty(),
            full_parse_tx,
            full_parse_rx,
            layout_tx,
            layout_rx,
        }
    }

    /// Drain completed background results into the display.
    ///
    /// Call BEFORE the display's `update_sized`, so a just-finished async parse
    /// lands before Gate 1 has a chance to install another placeholder.
    /// Generation mismatches drop silently (the spawned task's input is older
    /// than the current buffer).
    pub fn pump(&mut self, display: &mut EditorDisplay) {
        while let Ok((generation, buf)) = self.full_parse_rx.try_recv() {
            display.install_full_parse(generation, buf);
        }
        // Same drain, for a just-finished background full wrap. Order
        // relative to the parse drain above does not matter — the two
        // are staleness-gated independently on `generation`.
        while let Ok((generation, layout)) = self.layout_rx.try_recv() {
            display.install_full_layout(generation, layout);
        }
    }

    /// Spawn the real parse / layout if the display asked for one.
    ///
    /// `parse_text` yields the text the display is DRAWING — the replace
    /// preview's when one is showing, the buffer's otherwise. It is only called
    /// when a parse is actually spawned. Parsing the buffer's text under a
    /// preview would install a parse of text that is not on screen: the
    /// placeholder this generation came from was keyed on the preview's
    /// synthetic revision, so the result would sail through
    /// `install_full_parse`'s staleness check.
    ///
    /// The task gets the text itself. A clone shares its structure, so handing
    /// a 5000-row note to a background parse costs a pointer rather than a copy.
    /// `notifier` wakes the host on completion so the rich result lands without
    /// waiting for the next keystroke.
    pub fn spawn_pending(
        &mut self,
        display: &mut EditorDisplay,
        parse_text: impl FnOnce() -> crate::ropetext::Text,
        notifier: Option<Notifier>,
    ) {
        if let Some(generation) = display.take_pending_full_parse() {
            let text = parse_text();
            let tx = self.full_parse_tx.clone();
            let redraw = notifier.clone();
            self.full_parse_task.spawn(async move {
                let buf = ParsedBuffer::parse(&text);
                let _ = tx.send((generation, buf));
                if let Some(redraw) = redraw {
                    redraw();
                }
            });
        }
        // Same shape, for the layout side. The job already carries its own
        // `Text`/`rendered_cache`/`gutter_insets` clones.
        if let Some(job) = display.take_pending_full_layout() {
            let tx = self.layout_tx.clone();
            let redraw = notifier;
            self.layout_task.spawn(async move {
                let hints = crate::logic::editor::display::row_hints(
                    &job.rendered_cache,
                    &job.gutter_insets,
                );
                let layout = crate::ropetext::Layout::compute(
                    &job.text,
                    job.width,
                    crate::ropetext::Metrics::default(),
                    &hints,
                );
                let _ = tx.send((job.generation, layout));
                if let Some(redraw) = redraw {
                    redraw();
                }
            });
        }
    }
}

impl Default for BackgroundJobs {
    fn default() -> Self {
        Self::new()
    }
}
