//! What any editor engine needs per frame, terminal-free: the open note's
//! identity, the revision clock, search needles, the [`EditorDisplay`], the
//! background parse/layout jobs, and the host's wake-up hook.
//!
//! [`EditorSession::prepare_frame`] is the logic half of drawing the editor:
//! it brings the display up to date with the engine's snapshot and decides
//! what is highlighted. A front end then paints `display.frame(..)`.

use crate::logic::editor::display::EditorDisplay;
use crate::logic::editor::jobs::BackgroundJobs;
use crate::logic::editor::overlay::{Overlay, OverlayKind};
use crate::logic::editor::revisions::Revisions;
use crate::logic::editor::snapshot::EditorSnapshot;
use crate::logic::message::Notifier;
use kimun_core::nfs::VaultPath;
use std::num::NonZeroU64;

/// Everything the editor shows and tracks across frames, apart from the
/// engine's buffer.
pub struct EditorSession {
    /// The note on screen. Whether the next text is a *reload* of this note or
    /// *another* note is decided by this, never by comparing texts: two notes
    /// can hold the same text, and state that belongs to one — undo history,
    /// selection, `gv` marks — must not leak into the other.
    pub note: Option<VaultPath>,
    /// The one revision clock plus its comparison snapshots (saved, needles) —
    /// see `Revisions`. `revs.current()` advances iff the buffer text
    /// changes. Cursor moves never touch it, so an in-flight autosave's
    /// revision token survives navigation, and the display reuses its parse
    /// cache.
    pub revs: Revisions,
    /// Lowercased needles to emphasize in the rendered buffer — set when the
    /// note was opened from a query result ("search match"), and dropped on the
    /// first edit (`revs.needles_stale()`).
    pub needles: Vec<String>,
    pub display: EditorDisplay,
    pub jobs: BackgroundJobs,
    /// The host's wake-up hook. The background parse/layout tasks call it on
    /// completion so the host redraws without waiting for the next keystroke.
    pub notifier: Option<Notifier>,
    /// The last frame drew a **replace preview** instead of the buffer.
    showed_preview: bool,
}

/// What the caller supplies for one frame.
pub struct FrameInput {
    /// The single producer of the frame's text, cursor and content revision.
    pub snapshot: EditorSnapshot,
    pub width: u16,
    pub height: u16,
    /// What is painted as selection (the find bar's current match, else the
    /// editor's own selection).
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// The **replace preview**'s lines, when one is showing. The buffer is
    /// never touched — only this frame's view of it is substituted.
    pub preview: Option<Vec<String>>,
    /// The preview's spans, as `Preview` / `PreviewCurrent` overlays. Used only
    /// with `preview`.
    pub preview_overlays: Vec<Overlay>,
    /// Find-pattern matches as `Match` overlays. Skipped while a preview is
    /// showing (decided here, on `preview`): those columns already carry the
    /// preview colour, which is the more important fact.
    pub matches: Vec<Overlay>,
}

/// Identity for a **replace preview**'s snapshot: the real content revision
/// folded together with the previewed text.
///
/// The display gates parse-cache rebuilds on `content_revision`, so a preview
/// must not reuse the buffer's — it would show a parse of text that is not on
/// screen. Deriving it from the previewed lines means an unchanged preview
/// keeps its cache entry across frames, and any change to the pattern, the
/// replacement, or the buffer produces a new one.
pub(crate) fn preview_revision(base: NonZeroU64, lines: &[String]) -> NonZeroU64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    base.get().hash(&mut h);
    lines.hash(&mut h);
    NonZeroU64::new(h.finish()).unwrap_or(NonZeroU64::MIN)
}

/// A selection as overlays: one per row, the middle rows running to their full
/// width, which `mark_overlay` clamps.
fn selection_overlays(selection: Option<((usize, usize), (usize, usize))>) -> Vec<Overlay> {
    let mut overlays = Vec::new();
    if let Some(((sr, sc), (er, ec))) = selection {
        for row in sr..=er {
            let start = if row == sr { sc } else { 0 };
            let end = if row == er { ec } else { usize::MAX };
            overlays.push(Overlay::new(row, start, end, OverlayKind::Selection));
        }
    }
    overlays
}

impl EditorSession {
    pub fn new() -> Self {
        Self {
            note: None,
            revs: Revisions::new(),
            needles: Vec::new(),
            display: EditorDisplay::new(),
            jobs: BackgroundJobs::new(),
            notifier: None,
            showed_preview: false,
        }
    }

    /// Bring the display up to date for one frame. The order is the contract:
    ///
    /// 1. drain completed background parse/layout results, so a just-finished
    ///    parse lands before the update can install another placeholder;
    /// 2. adopt the snapshot's revision (never the preview's synthetic one, or
    ///    dirty tracking would follow the preview);
    /// 3. update the display (against the preview's lines when one is showing);
    /// 4. drop stale needles, then hand the rest to the display;
    /// 5. set the overlays: selection, preview spans, and (no preview only)
    ///    find matches;
    /// 6. spawn the real parse / layout if the update deferred one.
    pub fn prepare_frame(&mut self, input: FrameInput) {
        let FrameInput {
            snapshot: snap,
            width,
            height,
            selection,
            preview,
            preview_overlays,
            matches,
        } = input;
        self.jobs.pump(&mut self.display);

        self.revs.adopt(snap.content_revision);
        // A preview swapping in, changing or out is no edit of the text the
        // display last drew (see `note_swapped_snapshot`).
        if preview.is_some() || self.showed_preview {
            self.display.note_swapped_snapshot();
        }
        self.showed_preview = preview.is_some();
        match &preview {
            None => self.display.update_sized(&snap, width, height),
            Some(lines) => {
                // The parse cache keys on `content_revision`, so the preview
                // carries an identity of its own.
                let rev = preview_revision(snap.content_revision, lines);
                let view_snap = EditorSnapshot::borrowed(lines, snap.cursor, rev);
                self.display.update_sized(&view_snap, width, height);
            }
        }
        // Needles reach the display as **overlays**; the display appends the two
        // kinds it derives from content (tasks, needles) for the visible rows.
        if self.revs.needles_stale() {
            self.needles.clear();
            self.revs.disarm_needles();
        }
        self.display.set_needles(self.needles.clone());

        let mut overlays = selection_overlays(selection);
        if preview.is_some() {
            overlays.extend(preview_overlays);
        } else {
            overlays.extend(matches);
        }
        self.display.set_overlays(overlays);

        // Parse the lines the display is DRAWING, not the buffer's.
        self.jobs.spawn_pending(
            &mut self.display,
            || match &preview {
                Some(lines) => crate::ropetext::Text::from(lines.join("\n").as_str()),
                None => snap.text.clone(),
            },
            self.notifier.clone(),
        );
    }
}

impl Default for EditorSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::editor::display::tests::frame_row_text;
    use crate::logic::editor::overlay::OverlayKind;
    use crate::logic::editor::snapshot::EditorSnapshot;
    use std::num::NonZeroU64;

    fn rev(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).unwrap()
    }

    fn input(lines: &[&str], revision: u64) -> FrameInput {
        let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        FrameInput {
            snapshot: EditorSnapshot::owned(lines, (0, 0), rev(revision)),
            width: 40,
            height: 10,
            selection: None,
            preview: None,
            preview_overlays: Vec::new(),
            matches: Vec::new(),
        }
    }

    #[test]
    fn prepare_frame_adopts_the_snapshot_revision() {
        let mut session = EditorSession::new();
        session.prepare_frame(input(&["hello"], 41));
        assert_eq!(session.revs.current(), rev(41));
        let f = session.display.frame(40, true);
        assert_eq!(frame_row_text(&f.rows[0]), "hello");
    }

    #[test]
    fn prepare_frame_clears_needles_once_the_revision_moves_on() {
        let mut session = EditorSession::new();
        session.prepare_frame(input(&["hello"], 1));
        session.needles = vec!["hello".to_string()];
        session.revs.arm_needles();
        // Same revision: the needles survive the frame.
        session.prepare_frame(input(&["hello"], 1));
        assert_eq!(session.needles, vec!["hello".to_string()]);
        // The text moved on.
        session.prepare_frame(input(&["hello!"], 2));
        assert!(session.needles.is_empty());
        assert!(!session.revs.needles_stale());
    }

    #[test]
    fn prepare_frame_turns_a_multirow_selection_into_one_overlay_per_row() {
        let overlays = selection_overlays(Some(((0, 2), (2, 1))));
        assert_eq!(
            overlays,
            vec![
                Overlay::new(0, 2, usize::MAX, OverlayKind::Selection),
                Overlay::new(1, 0, usize::MAX, OverlayKind::Selection),
                Overlay::new(2, 0, 1, OverlayKind::Selection),
            ]
        );
        assert!(selection_overlays(None).is_empty());
    }

    #[test]
    fn prepare_frame_paints_the_selection_it_is_given() {
        let mut session = EditorSession::new();
        let mut inp = input(&["abcdef"], 1);
        inp.selection = Some(((0, 1), (0, 3)));
        session.prepare_frame(inp);
        let f = session.display.frame(40, true);
        let selected: String = f.rows[0]
            .runs
            .iter()
            .filter(|r| r.overlays.iter().any(|k| k == OverlayKind::Selection))
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(selected, "bc");
    }

    #[test]
    fn find_matches_are_skipped_while_a_preview_is_showing() {
        let mut session = EditorSession::new();
        let mut inp = input(&["abcdef"], 1);
        inp.matches = vec![Overlay::new(0, 0, 2, OverlayKind::Match)];
        session.prepare_frame(inp);
        let f = session.display.frame(40, true);
        assert!(
            f.rows[0]
                .runs
                .iter()
                .any(|r| r.overlays.iter().any(|k| k == OverlayKind::Match))
        );

        let mut inp = input(&["abcdef"], 1);
        inp.matches = vec![Overlay::new(0, 0, 2, OverlayKind::Match)];
        inp.preview = Some(vec!["XXcdef".to_string()]);
        inp.preview_overlays = vec![Overlay::new(0, 0, 2, OverlayKind::PreviewCurrent)];
        session.prepare_frame(inp);
        let f = session.display.frame(40, true);
        assert_eq!(frame_row_text(&f.rows[0]), "XXcdef");
        let kinds: Vec<_> = f.rows[0]
            .runs
            .iter()
            .flat_map(|r| r.overlays.iter())
            .collect();
        assert!(kinds.contains(&OverlayKind::PreviewCurrent));
        assert!(!kinds.contains(&OverlayKind::Match));
    }

    /// A large note parses in the background; `pump` installs the result on a
    /// later frame, and the notifier is what tells the host to come back.
    #[tokio::test]
    async fn prepare_frame_spawns_the_parse_job_for_a_large_buffer_and_pump_installs_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let woken = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = woken.clone();
        let mut session = EditorSession::new();
        session.notifier = Some(std::sync::Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let mut lines: Vec<String> = vec!["# heading".to_string()];
        lines.extend((0..1200).map(|i| format!("paragraph {i}")));
        let big = |lines: &[String]| FrameInput {
            snapshot: EditorSnapshot::owned(lines.to_vec(), (0, 0), rev(1)),
            width: 40,
            height: 10,
            selection: None,
            preview: None,
            preview_overlays: Vec::new(),
            matches: Vec::new(),
        };
        session.prepare_frame(big(&lines));
        // Placeholder: every row is plain until the real parse lands.
        assert!(
            session
                .display
                .parsed_buffer_kinds()
                .iter()
                .all(|k| *k == session.display.parsed_buffer_kinds()[0]),
            "the first frame is a placeholder"
        );
        for _ in 0..200 {
            if woken.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(woken.load(Ordering::SeqCst) >= 1, "host was never woken");
        // The next frame drains the result first.
        session.prepare_frame(big(&lines));
        let kinds = session.display.parsed_buffer_kinds();
        assert!(
            kinds.iter().any(|k| *k != kinds[0]),
            "the real parse was installed (the heading differs from the paragraphs)"
        );
    }
}
