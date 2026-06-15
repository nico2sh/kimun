//! Host-side glue for the Neovim backend.
//!
//! The backend (`NvimBackend`) owns the nvim process and its snapshot. This
//! module owns the *host policy* that sits between that backend and the app:
//! the `ZZ`/`ZQ` and `:wq`/`:q` quit intercepts, and the per-frame
//! `content_gen` → `content_revision` mirror.
//!
//! As with the [decode seam](super::nvim_decode), the fragile part is pulled
//! out as a pure decision ([`classify_nvim_key`]) that is fully testable with
//! no nvim process: given the pending-Z state, the key, the mode and the
//! command line, it returns *what to do*. [`NvimHost`] is the thin stateful
//! shell that applies the decision — forwarding to nvim and emitting app events.

use std::num::NonZeroU64;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::backend::NvimBackend;
use super::snapshot::EditorMode;
use crate::components::events::{AppEvent, AppTx};

/// Logical (row, char-col) selection span, as carried on the snapshot.
type Selection = ((usize, usize), (usize, usize));

/// Where a quit came from. Both side effects the host performs — whether to
/// autosave, and whether to `<Esc>` nvim out of its current mode first — are
/// *derived* from this, so a caller never has to coordinate two booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitKind {
    /// `ZZ` from Normal mode: write + quit. No `<Esc>` (nvim never left Normal —
    /// the first `Z` was buffered, not forwarded).
    WriteQuit,
    /// `ZQ` from Normal mode: quit without saving. No `<Esc>`.
    DiscardQuit,
    /// A `:`-command quit (`:wq`, `:q`, …). Nvim is in command-line mode, so it
    /// must be `<Esc>`-ed out first. `save` is whether the command writes.
    Command { save: bool },
}

impl QuitKind {
    /// Whether to autosave before leaving the editor.
    pub fn saves(self) -> bool {
        match self {
            QuitKind::WriteQuit => true,
            QuitKind::DiscardQuit => false,
            QuitKind::Command { save } => save,
        }
    }

    /// Whether nvim must be `<Esc>`-ed out of its current mode before quitting.
    pub fn needs_escape(self) -> bool {
        matches!(self, QuitKind::Command { .. })
    }
}

/// What a single key should do on the Nvim backend. Pure data — no I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvimKeyDecision {
    /// First `Z` of a possible `ZZ`/`ZQ` in Normal mode: swallow and wait.
    BufferZ,
    /// A quit/write-quit. The side effects are derived from the [`QuitKind`].
    Quit(QuitKind),
    /// A buffered `Z` was not followed by `Z`/`Q`: replay the `Z`, then
    /// forward the current key.
    ReplayZThenForward,
    /// Nothing special — forward the key to nvim.
    Forward,
}

/// Decide what a key does on the Nvim backend. Pure: depends only on the
/// pending-Z flag, the key, the current mode, and the command line.
pub fn classify_nvim_key(
    pending_z: bool,
    key: &KeyEvent,
    mode: &EditorMode,
    cmdline: Option<&str>,
) -> NvimKeyDecision {
    // Second key after a buffered `Z`.
    if pending_z {
        return match key.code {
            KeyCode::Char('Z') => NvimKeyDecision::Quit(QuitKind::WriteQuit),
            KeyCode::Char('Q') => NvimKeyDecision::Quit(QuitKind::DiscardQuit),
            _ => NvimKeyDecision::ReplayZThenForward,
        };
    }

    // First `Z` in Normal mode — buffer it.
    if key.code == KeyCode::Char('Z') && *mode == EditorMode::Normal {
        return NvimKeyDecision::BufferZ;
    }

    // `<CR>` while in command-line mode: intercept quit/write-quit so they
    // don't kill the embedded nvim process. Match the leading command *word*
    // so `:w report.md`, `:wq | echo`, `: wq` and trailing whitespace are all
    // recognised. The app has no save-as, so any write/quit verb — with or
    // without arguments — means "save and leave"; the arguments are ignored.
    if key.code == KeyCode::Enter && *mode == EditorMode::Command {
        let cmd = cmdline.unwrap_or("").trim_start_matches(':').trim();
        let word = cmd.split([' ', '\t', '|']).next().unwrap_or("");
        let saves = matches!(
            word,
            "w" | "wq" | "wq!" | "wqa" | "wqa!" | "x" | "xa" | "x!"
        );
        let quits = saves || matches!(word, "q" | "q!" | "qa" | "qa!" | "cq" | "cq!");
        if quits {
            return NvimKeyDecision::Quit(QuitKind::Command { save: saves });
        }
    }

    NvimKeyDecision::Forward
}

/// Whether [`classify_nvim_key`] consults `mode`/`cmdline` for this input. When
/// `false`, the caller may skip locking the snapshot entirely: the pending-Z
/// branch decides on `key.code` alone, and any non-`Z`/non-`Enter` key in the
/// non-pending case short-circuits to `Forward` before `mode` is read.
fn needs_snapshot(pending_z: bool, key: &KeyEvent) -> bool {
    !pending_z && matches!(key.code, KeyCode::Char('Z') | KeyCode::Enter)
}

/// Outcome of applying a key on the Nvim backend. The host bumps the cursor
/// generation only when a key was actually forwarded to nvim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvimKeyResult {
    /// Key was handled without forwarding (buffered Z, or a quit intercept).
    Consumed,
    /// Key (and possibly a replayed `Z`) was forwarded to nvim.
    Forwarded,
}

/// Values the render loop needs from the Nvim backend each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSync {
    /// `content_revision` to mirror, if the refresh task saw a content change.
    /// `None` leaves the host's revision untouched.
    pub rev: Option<NonZeroU64>,
    /// Active visual selection to render.
    pub selection: Option<Selection>,
}

/// Host-side Nvim state: the only thing the host must track itself is the
/// pending-`Z` flag for the `ZZ`/`ZQ` two-key sequence.
#[derive(Debug, Default)]
pub struct NvimHost {
    pending_z: bool,
}

impl NvimHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one key to the Nvim backend: classify, update the pending-Z flag,
    /// then forward / emit as the decision dictates.
    ///
    /// The snapshot Mutex (shared with the reverse-refresh task) is locked only
    /// when the decision actually consults mode/cmdline — see [`needs_snapshot`].
    /// Ordinary keystrokes (insert-mode typing, the pending-Z second key) take
    /// the lock-free path: no lock, no clone.
    pub fn handle_key(&mut self, nvim: &NvimBackend, key: &KeyEvent, tx: &AppTx) -> NvimKeyResult {
        let decision = if needs_snapshot(self.pending_z, key) {
            let snap = nvim.snapshot();
            classify_nvim_key(self.pending_z, key, &snap.mode, snap.cmdline.as_deref())
        } else {
            // classify ignores mode/cmdline on this path (that is exactly what
            // `needs_snapshot` returning false means), so the placeholders are
            // never read.
            classify_nvim_key(self.pending_z, key, &EditorMode::Normal, None)
        };
        self.pending_z = matches!(decision, NvimKeyDecision::BufferZ);

        match decision {
            NvimKeyDecision::BufferZ => NvimKeyResult::Consumed,
            NvimKeyDecision::Quit(kind) => {
                if kind.needs_escape() {
                    // Leave command-line mode so the intercept doesn't strand
                    // nvim mid-command.
                    nvim.handle_key(
                        &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                        tx.clone(),
                    );
                }
                if kind.saves() {
                    tx.send(AppEvent::Autosave).ok();
                }
                tx.send(AppEvent::FocusSidebar).ok();
                NvimKeyResult::Consumed
            }
            NvimKeyDecision::ReplayZThenForward => {
                nvim.handle_key(
                    &KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::NONE),
                    tx.clone(),
                );
                nvim.handle_key(key, tx.clone());
                NvimKeyResult::Forwarded
            }
            NvimKeyDecision::Forward => {
                nvim.handle_key(key, tx.clone());
                NvimKeyResult::Forwarded
            }
        }
    }

    /// Per-frame sync: resize nvim to the editor area, then read the snapshot's
    /// `content_gen` and active visual selection.
    ///
    /// Canonical explanation of the revision mirror (referenced from the host's
    /// key handler): the editor tracks two independent counters. `edit_generation`
    /// (bumped by every forwarded key) invalidates the view cache; `content_gen`
    /// is owned by the reverse-refresh task in `backend.rs`, which bumps it *only*
    /// when `snap.lines` actually diffs. We mirror `content_gen` into the host's
    /// `content_revision` here. Because navigation keystrokes don't change `lines`,
    /// they don't bump `content_gen`, so an in-flight autosave's revision token
    /// stays valid across cursor movement. `NonZeroU64::new(gen + 1)` maps the
    /// initial `gen == 0` to "no change yet" (`None` leaves the host's revision
    /// untouched).
    pub fn frame_sync(&self, nvim: &NvimBackend, width: u16, height: u16) -> FrameSync {
        nvim.maybe_resize(width, height);
        let snap = nvim.snapshot();
        let selection = snap.visual_selection;
        let content_gen = snap.content_gen;
        drop(snap);
        FrameSync {
            rev: NonZeroU64::new(content_gen.saturating_add(1)),
            selection,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    #[test]
    fn pending_z_then_z_is_write_quit_no_esc() {
        assert_eq!(
            classify_nvim_key(true, &key('Z'), &EditorMode::Normal, None),
            NvimKeyDecision::Quit(QuitKind::WriteQuit)
        );
    }

    #[test]
    fn pending_z_then_q_is_quit_no_save() {
        assert_eq!(
            classify_nvim_key(true, &key('Q'), &EditorMode::Normal, None),
            NvimKeyDecision::Quit(QuitKind::DiscardQuit)
        );
    }

    #[test]
    fn pending_z_then_other_replays() {
        assert_eq!(
            classify_nvim_key(true, &key('x'), &EditorMode::Normal, None),
            NvimKeyDecision::ReplayZThenForward
        );
    }

    #[test]
    fn z_in_normal_buffers() {
        assert_eq!(
            classify_nvim_key(false, &key('Z'), &EditorMode::Normal, None),
            NvimKeyDecision::BufferZ
        );
    }

    #[test]
    fn z_in_insert_forwards() {
        assert_eq!(
            classify_nvim_key(false, &key('Z'), &EditorMode::Insert, None),
            NvimKeyDecision::Forward
        );
    }

    #[test]
    fn command_wq_saves_and_quits_with_esc() {
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(":wq")),
            NvimKeyDecision::Quit(QuitKind::Command { save: true })
        );
    }

    #[test]
    fn command_q_quits_no_save_with_esc() {
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(":q")),
            NvimKeyDecision::Quit(QuitKind::Command { save: false })
        );
    }

    #[test]
    fn command_q_bang_quits() {
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(":q!")),
            NvimKeyDecision::Quit(QuitKind::Command { save: false })
        );
    }

    #[test]
    fn command_bare_w_saves_and_quits() {
        // Characterises current behaviour: `:w` is in the saves set, and the
        // quit set is a superset of saves, so `:w<CR>` saves *and* leaves the
        // editor. (Not changed by this refactor.)
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(":w")),
            NvimKeyDecision::Quit(QuitKind::Command { save: true })
        );
    }

    #[test]
    fn command_write_with_filename_saves_and_quits() {
        // `:w report.md` — leading verb `w` is matched, the argument ignored.
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(":w report.md")),
            NvimKeyDecision::Quit(QuitKind::Command { save: true })
        );
    }

    #[test]
    fn command_wq_with_bar_and_trailing_space() {
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(":wq | echo hi")),
            NvimKeyDecision::Quit(QuitKind::Command { save: true })
        );
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(":q  ")),
            NvimKeyDecision::Quit(QuitKind::Command { save: false })
        );
    }

    #[test]
    fn command_space_after_colon() {
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(": wq")),
            NvimKeyDecision::Quit(QuitKind::Command { save: true })
        );
    }

    #[test]
    fn command_unknown_forwards() {
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Command, Some(":noh")),
            NvimKeyDecision::Forward
        );
    }

    #[test]
    fn enter_in_normal_forwards() {
        assert_eq!(
            classify_nvim_key(false, &enter(), &EditorMode::Normal, None),
            NvimKeyDecision::Forward
        );
    }

    #[test]
    fn needs_snapshot_only_for_z_and_enter_when_not_pending() {
        assert!(needs_snapshot(false, &key('Z')));
        assert!(needs_snapshot(false, &enter()));
        // Ordinary keys: lock-free path.
        assert!(!needs_snapshot(false, &key('a')));
        assert!(!needs_snapshot(false, &key('Q')));
        // Pending-Z second key never needs the snapshot.
        assert!(!needs_snapshot(true, &key('Z')));
        assert!(!needs_snapshot(true, &enter()));
        assert!(!needs_snapshot(true, &key('x')));
    }

    #[test]
    fn regular_char_forwards() {
        assert_eq!(
            classify_nvim_key(false, &key('a'), &EditorMode::Insert, None),
            NvimKeyDecision::Forward
        );
    }
}
