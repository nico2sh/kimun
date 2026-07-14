//! The editor screen's input classifier: resolves one raw input event into an
//! **Intent** — what the event *means* under the screen's input precedence —
//! before anything mutates. `classify` is pure over an [`InputCtx`] snapshot,
//! so the precedence (leader → paste intercepts → shortcuts → overlay →
//! mouse → panels) is table-tested here instead of living as statement order
//! inside `EditorScreen::handle_input`. The screen builds the snapshot,
//! classifies, then *executes* the intent.
//!
//! A pending leader sequence owns the input first (spec §8a) — including
//! ahead of the paste intercepts, which in the pre-extraction ladder sat
//! above it (the old quirk let a leader-pending Ctrl+V paste an image and
//! leave the sequence pending). Ctrl-chords and paste payloads cancel the
//! sequence and dispatch normally; every other key feeds it.
//!
//! Two decided collaterals of that reorder, both asserted by tests below:
//! Ctrl+Enter mid-sequence now feeds the leader instead of following the
//! link (Enter is not a `Char` chord, so no exception applies — §8a wins
//! over the old follow-link-first order), and a paste arriving mid-sequence
//! under an open overlay cancels the leader where the old ladder left it
//! pending (the paste rule applies regardless of who receives the paste).

use ratatui::crossterm::event::KeyEvent;

use crate::components::drawer::DrawerView;
use crate::components::events::InputEvent;
use crate::components::overlay::OverlayKind;
use crate::components::panel::PanelKind;
use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
use crate::keys::key_strike::KeyStrike;
use crate::keys::{KeyBindings, key_event_to_combo};

/// Snapshot of the screen state the classifier needs. Built per event by
/// `EditorScreen::handle_input`; keep it minimal — every field is a reason
/// the classification can change.
#[derive(Debug, Clone, PartialEq)]
pub struct InputCtx {
    /// The open overlay's kind, `None` when no overlay is open.
    pub overlay: Option<OverlayKind>,
    /// A leader key sequence is pending.
    pub leader_pending: bool,
    /// The focused panel.
    pub focused: PanelKind,
    /// The active drawer view (regardless of drawer visibility).
    pub drawer_view: DrawerView,
    /// Bare Space starts the leader (vim Normal mode, empty pending state).
    pub vim_space_leads: bool,
}

impl InputCtx {
    /// The editor owns key input: it is focused and no overlay sits over it.
    /// Mirrors `EditorScreen::editor_active`.
    fn editor_active(&self) -> bool {
        self.focused == PanelKind::Editor && self.overlay.is_none()
    }

    fn find_panel_focused(&self) -> bool {
        self.focused == PanelKind::Drawer && self.drawer_view == DrawerView::Find
    }
}

/// The classifier's full verdict: pre-effects plus the intent. The executor
/// applies `flash` and `cancel_leader` first, then runs the intent.
#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    /// Footer chord flash (F-keys and Ctrl/Alt+letter chords, except the
    /// leader gateway, whose affordance is the pending sequence).
    pub flash: Option<String>,
    /// A pending leader sequence must be cancelled before the intent runs
    /// (an overlay opened underneath, or a Ctrl-chord dispatches normally).
    pub cancel_leader: bool,
    pub intent: EditorIntent,
}

/// What one raw input event means in the editor screen. Execution lives in
/// `EditorScreen`; anything that depends on a runtime outcome (the clipboard
/// image probe, a panel's first crack at a key) carries its fallback as data.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorIntent {
    /// Swallow the event (guarded no-ops, the F-key sink).
    Consume,
    /// Bracketed paste into the editor: try an image paste first, fall
    /// back to pasting the event's text payload when the clipboard holds
    /// no image. Carries nothing — the executor reads the payload back
    /// from the event it already holds, so large pastes are never cloned.
    EditorPaste,
    /// Ctrl+V in the editor: probe the clipboard for an image. When there
    /// is none, the executor reclassifies the tail of the ladder
    /// ([`classify_tail`] with `cancel_leader = false` — the probe
    /// classification already owned any leader cancel) against fresh
    /// screen state. Lazy on purpose: the common with-image press must not
    /// pay for a discarded fallback classification.
    ImageProbe,
    /// Follow the note link under the editor cursor.
    FollowLink,
    /// Feed the key to the pending leader sequence.
    LeaderKey(KeyEvent),
    /// Start a leader sequence and schedule the which-key reveal.
    LeaderStart,
    /// A screen operation with no classification-time policy beyond its
    /// guard (see CONTEXT.md **Intent** — "action" is reserved for
    /// `ActionShortcuts`, the classifier's *input*).
    Op(EditorOp),
    /// Dismiss the overlay when one of `kind` is already open, else `open`.
    ToggleOverlay {
        kind: OverlayKind,
        open: OverlayOpen,
    },
    /// Present a dialog overlay (the executor reads its seed state).
    OpenDialog(DialogRequest),
    /// Route the event to the open overlay.
    Overlay,
    /// Route the mouse event through the `PanelSet` hit-test path.
    Mouse,
    /// Route the event to the focused panel; on `NotConsumed` apply the
    /// fallback.
    Panel { fallback: PanelFallback },
}

/// Which opener a [`EditorIntent::ToggleOverlay`] uses — two actions share
/// `OverlayKind::NoteBrowser` (search browser vs file finder), so the kind
/// alone cannot pick the opener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayOpen {
    SearchBrowser,
    FileFinder,
    SavedSearches,
    RagAnswer,
    CommandPalette,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DialogRequest {
    SortQuery,
    SortSidebar,
    QuickNote,
}

/// Fallback applied when the focused panel does not consume the event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelFallback {
    None,
    /// Tab / Shift-Tab cycle panel focus when the panel passes.
    FocusCycle(CycleDir),
    /// The FIND view yields focus back to the editor on an unhandled Esc.
    FocusEditor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CycleDir {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditorOp {
    ToggleDrawer,
    FocusLeft,
    FocusRight,
    OpenJournal,
    ShowFileOps,
    ToggleQueryPanel,
    /// Open (or switch the drawer to) FILES and reveal the open note's
    /// directory — never hides the drawer.
    OpenFileBrowserReveal,
    SaveCurrentQuery,
    OpenWorkspaceSwitcher,
    FindInBuffer,
    ApplyText(TextAction),
    OpenHelp,
    OpenQueryHelp,
}

/// Resolve one raw input event into its [`Classification`] under the editor
/// screen's input precedence. Pure: same event + bindings + ctx, same verdict.
pub fn classify(event: &InputEvent, bindings: &KeyBindings, ctx: &InputCtx) -> Classification {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    // A pending leader sequence owns the input ahead of everything else
    // (spec §8a), including the paste intercepts below. Exceptions: an
    // overlay that opened underneath wins, a Ctrl-chord cancels the sequence
    // then dispatches normally, and a paste payload — not a key the sequence
    // can consume — gets the same cancel-then-dispatch treatment.
    let mut cancel_leader = false;
    if ctx.leader_pending {
        match event {
            InputEvent::Key(key) => {
                if ctx.overlay.is_some() {
                    cancel_leader = true;
                } else if matches!(key.code, KeyCode::Char(_))
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    cancel_leader = true;
                    // fall through to normal dispatch below
                } else {
                    return Classification {
                        flash: None,
                        cancel_leader: false,
                        intent: EditorIntent::LeaderKey(*key),
                    };
                }
            }
            InputEvent::Paste(_) => cancel_leader = true,
            // Mouse input never fed the sequence; it routes below unchanged.
            InputEvent::Mouse(_) => {}
        }
    }

    // Bracketed paste (terminal-level). The executor tries an image paste
    // first and falls back to the text payload.
    if ctx.editor_active() && matches!(event, InputEvent::Paste(_)) {
        return Classification {
            flash: None,
            cancel_leader,
            intent: EditorIntent::EditorPaste,
        };
    }

    // Ctrl+V: probe the clipboard for an image ahead of the editor's own
    // text paste. No image → the executor reclassifies the tail against
    // fresh state. The leader cancel rides on this classification: the
    // sequence dies whether or not the clipboard holds an image.
    if ctx.editor_active()
        && let InputEvent::Key(key) = event
        && key.modifiers == KeyModifiers::CONTROL
        && key.code == KeyCode::Char('v')
    {
        return Classification {
            flash: None,
            cancel_leader,
            intent: EditorIntent::ImageProbe,
        };
    }

    classify_tail(event, bindings, ctx, cancel_leader)
}

/// The ladder below the leader tier and the paste intercepts: shortcuts →
/// overlay → mouse → vim Space leader → Tab cycle → FIND Esc → focused
/// panel. `cancel_leader` carries the leader tier's verdict into the
/// returned classification. `pub(crate)` for one caller besides
/// [`classify`]: the executor's no-image [`EditorIntent::ImageProbe`]
/// path, which runs the tail against post-probe state.
pub(crate) fn classify_tail(
    event: &InputEvent,
    bindings: &KeyBindings,
    ctx: &InputCtx,
    cancel_leader: bool,
) -> Classification {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    // Ctrl+Enter follows the link under the cursor on kitty-protocol
    // terminals (legacy terminals can't tell it from Enter).
    if ctx.editor_active()
        && let InputEvent::Key(key) = event
        && key.code == KeyCode::Enter
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return Classification {
            flash: None,
            cancel_leader,
            intent: EditorIntent::FollowLink,
        };
    }

    // Shortcut tier: resolve the bound action (if any) to an intent, and
    // compute the footer chord flash. The match *yields* instead of
    // returning so `flash` has a single owner: the one `done` constructor
    // below moves it into whichever classification wins.
    let mut flash = None;
    let mut shortcut_intent = None;
    if let InputEvent::Key(key) = event
        && let Some(combo) = key_event_to_combo(key)
    {
        let is_fkey = combo.key.is_fkey();
        let action = bindings.get_action(&combo);
        // Flash the raw chord — except for the leader gateway, whose
        // affordance is the pending sequence (and the which-key overlay).
        if action != Some(ActionShortcuts::Leader) && (is_fkey || combo.is_letter_chord()) {
            flash = Some(combo.to_string());
        }
        shortcut_intent = match action {
            Some(ActionShortcuts::OpenCommandPalette) => Some(EditorIntent::ToggleOverlay {
                kind: OverlayKind::CommandPalette,
                open: OverlayOpen::CommandPalette,
            }),
            Some(ActionShortcuts::Leader) => {
                // The gateway works in every context, including mid-typing —
                // but not while an overlay owns input.
                Some(if ctx.overlay.is_none() {
                    EditorIntent::LeaderStart
                } else {
                    EditorIntent::Consume
                })
            }
            Some(ActionShortcuts::ToggleSidebar) => Some(EditorIntent::Op(EditorOp::ToggleDrawer)),
            Some(ActionShortcuts::FocusSidebar) => {
                // No-op while an overlay owns input, but still consume the key.
                Some(if ctx.overlay.is_none() {
                    EditorIntent::Op(EditorOp::FocusLeft)
                } else {
                    EditorIntent::Consume
                })
            }
            Some(ActionShortcuts::FocusEditor) => Some(if ctx.overlay.is_none() {
                EditorIntent::Op(EditorOp::FocusRight)
            } else {
                EditorIntent::Consume
            }),
            Some(ActionShortcuts::NewJournal) => Some(EditorIntent::Op(EditorOp::OpenJournal)),
            Some(ActionShortcuts::SearchNotes) => Some(EditorIntent::ToggleOverlay {
                kind: OverlayKind::NoteBrowser,
                open: OverlayOpen::SearchBrowser,
            }),
            Some(ActionShortcuts::OpenNote) => Some(EditorIntent::ToggleOverlay {
                kind: OverlayKind::NoteBrowser,
                open: OverlayOpen::FileFinder,
            }),
            Some(ActionShortcuts::FileOperations) if ctx.editor_active() => {
                Some(EditorIntent::Op(EditorOp::ShowFileOps))
            }
            Some(ActionShortcuts::FollowLink) if ctx.editor_active() => {
                Some(EditorIntent::FollowLink)
            }
            Some(ActionShortcuts::ToggleQueryPanel) => {
                Some(EditorIntent::Op(EditorOp::ToggleQueryPanel))
            }
            Some(ActionShortcuts::OpenFileBrowser) => {
                // Open (or switch to) the FILES view — never hides the drawer;
                // ToggleSidebar is the on/off switch. Always reveal: with FILES
                // already open this is the "where is my note" gesture.
                Some(EditorIntent::Op(EditorOp::OpenFileBrowserReveal))
            }
            Some(ActionShortcuts::OpenSavedSearches) => Some(EditorIntent::ToggleOverlay {
                kind: OverlayKind::SavedSearches,
                open: OverlayOpen::SavedSearches,
            }),
            Some(ActionShortcuts::OpenRagAnswer) => Some(EditorIntent::ToggleOverlay {
                kind: OverlayKind::RagAnswer,
                open: OverlayOpen::RagAnswer,
            }),
            Some(ActionShortcuts::OpenSortDialog) => {
                // Sort applies only when a list is focused (the drawer's
                // Find / Files views). When the editor is focused, do NOT
                // consume — fall through (`None`) so the key reaches it
                // (e.g. Ctrl+R is redo in the nvim editor).
                if ctx.focused == PanelKind::Drawer && ctx.overlay.is_none() {
                    Some(match ctx.drawer_view {
                        DrawerView::Find => EditorIntent::OpenDialog(DialogRequest::SortQuery),
                        DrawerView::Files => EditorIntent::OpenDialog(DialogRequest::SortSidebar),
                        _ => EditorIntent::Consume,
                    })
                } else {
                    None
                }
            }
            Some(ActionShortcuts::SaveCurrentQuery) => {
                // Whether there is anything to save is executor state (the
                // live query text); the key is consumed either way.
                Some(EditorIntent::Op(EditorOp::SaveCurrentQuery))
            }
            Some(ActionShortcuts::SwitchWorkspace) => {
                Some(EditorIntent::Op(EditorOp::OpenWorkspaceSwitcher))
            }
            Some(ActionShortcuts::QuickNote) => Some(if ctx.overlay.is_none() {
                EditorIntent::OpenDialog(DialogRequest::QuickNote)
            } else {
                EditorIntent::Consume
            }),
            Some(ActionShortcuts::FindInBuffer) if ctx.editor_active() => {
                Some(EditorIntent::Op(EditorOp::FindInBuffer))
            }
            Some(ActionShortcuts::Text(
                action @ (TextAction::Bold | TextAction::Italic | TextAction::Strikethrough),
            )) if ctx.editor_active() => Some(EditorIntent::Op(EditorOp::ApplyText(action))),
            _ => {
                if is_fkey {
                    // F1 opens the help modal. Over the Find panel it surfaces
                    // query syntax instead of the flat key-bindings help. All
                    // F-keys are consumed and never forwarded to the editor.
                    if combo.key == KeyStrike::F1 && combo.modifiers.is_empty() {
                        Some(EditorIntent::Op(if ctx.find_panel_focused() {
                            EditorOp::OpenQueryHelp
                        } else {
                            EditorOp::OpenHelp
                        }))
                    } else {
                        Some(EditorIntent::Consume)
                    }
                } else {
                    None
                }
            }
        };
    }

    // Single constructor: moves `flash` into the winning classification.
    // Every call site diverges, so the FnOnce closure is used at most once.
    let done = move |intent| Classification {
        flash,
        cancel_leader,
        intent,
    };

    if let Some(intent) = shortcut_intent {
        return done(intent);
    }

    // An open overlay intercepts all remaining input ahead of the panels.
    if ctx.overlay.is_some() {
        return done(EditorIntent::Overlay);
    }

    if matches!(event, InputEvent::Mouse(_)) {
        return done(EditorIntent::Mouse);
    }

    // Vim Normal mode: bare Space is a second leader gateway, but only with
    // an empty pending state so it never shadows Space as a motion/operator
    // argument. Insert/Visual and the other backends keep Space typing a
    // space (`vim_space_leads` is false for those states).
    if ctx.editor_active()
        && (!ctx.leader_pending || cancel_leader)
        && let InputEvent::Key(key) = event
        && key.code == KeyCode::Char(' ')
        && key.modifiers.is_empty()
        && ctx.vim_space_leads
    {
        return done(EditorIntent::LeaderStart);
    }

    // Tab / Shift-Tab cycle panel focus (spec §2). The focused panel gets
    // first crack — the Query panel's autocomplete accepts on Tab — and the
    // editor keeps Tab for indentation.
    if ctx.focused != PanelKind::Editor
        && let InputEvent::Key(key) = event
        && matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
    {
        return done(EditorIntent::Panel {
            fallback: PanelFallback::FocusCycle(if key.code == KeyCode::Tab {
                CycleDir::Right
            } else {
                CycleDir::Left
            }),
        });
    }

    // The drawer's FIND view gets first crack (its autocomplete popup may
    // consume Esc); on an unhandled Esc it yields focus back to the editor.
    if ctx.find_panel_focused()
        && let InputEvent::Key(key) = event
        && key.code == KeyCode::Esc
    {
        return done(EditorIntent::Panel {
            fallback: PanelFallback::FocusEditor,
        });
    }

    done(EditorIntent::Panel {
        fallback: PanelFallback::None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

    fn bindings() -> KeyBindings {
        let mut kb = KeyBindings::empty();
        kb.batch_add()
            .with_ctrl()
            .add(KeyStrike::KeyP, ActionShortcuts::OpenCommandPalette)
            .add(KeyStrike::KeyK, ActionShortcuts::SearchNotes)
            .add(KeyStrike::KeyG, ActionShortcuts::Leader)
            .add(KeyStrike::KeyT, ActionShortcuts::ToggleSidebar)
            .add(KeyStrike::KeyR, ActionShortcuts::OpenSortDialog)
            .add(KeyStrike::KeyH, ActionShortcuts::FocusSidebar)
            .add(KeyStrike::KeyB, ActionShortcuts::Text(TextAction::Bold))
            .add(KeyStrike::KeyN, ActionShortcuts::FollowLink)
            .add(KeyStrike::KeyW, ActionShortcuts::QuickNote);
        kb.batch_add()
            .add(KeyStrike::F2, ActionShortcuts::FileOperations);
        kb
    }

    /// Editor focused, nothing pending — the common ctx.
    fn ctx() -> InputCtx {
        InputCtx {
            overlay: None,
            leader_pending: false,
            focused: PanelKind::Editor,
            drawer_view: DrawerView::Files,
            vim_space_leads: false,
        }
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> InputEvent {
        InputEvent::Key(KeyEvent::new(code, mods))
    }

    fn ctrl(c: char) -> InputEvent {
        key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn plain(c: char) -> InputEvent {
        key(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn classify_it(event: &InputEvent, ctx: &InputCtx) -> Classification {
        classify(event, &bindings(), ctx)
    }

    // ---- paste tiers -----------------------------------------------------

    #[test]
    fn bracketed_paste_in_editor_is_editor_paste() {
        let c = classify_it(&InputEvent::Paste("hi".into()), &ctx());
        assert_eq!(c.intent, EditorIntent::EditorPaste);
    }

    #[test]
    fn bracketed_paste_with_overlay_routes_to_overlay() {
        let mut cx = ctx();
        cx.overlay = Some(OverlayKind::NoteBrowser);
        let c = classify_it(&InputEvent::Paste("hi".into()), &cx);
        assert_eq!(c.intent, EditorIntent::Overlay);
    }

    #[test]
    fn bracketed_paste_drawer_focused_goes_to_panel() {
        let mut cx = ctx();
        cx.focused = PanelKind::Drawer;
        let c = classify_it(&InputEvent::Paste("hi".into()), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::None
            }
        );
    }

    #[test]
    fn ctrl_v_in_editor_is_a_bare_image_probe() {
        // The probe carries no precomputed fallback: the executor
        // reclassifies the tail only when the clipboard holds no image.
        let c = classify_it(&ctrl('v'), &ctx());
        assert_eq!(c.intent, EditorIntent::ImageProbe);
        assert_eq!(c.flash, None, "flash belongs to the no-image tail only");
    }

    #[test]
    fn ctrl_v_no_image_tail_reaches_panel_with_a_flash() {
        // The executor's no-image path: classify_tail against post-probe
        // state (leader already cancelled → cancel_leader = false). Ctrl+V
        // is unbound, so it reaches the focused panel (the editor's own
        // paste), flashing the chord on the way through the shortcut tier.
        let c = classify_tail(&ctrl('v'), &bindings(), &ctx(), false);
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::None
            }
        );
        assert!(c.flash.is_some());
        assert!(!c.cancel_leader, "the outer probe already owned the cancel");
    }

    #[test]
    fn leader_pending_ctrl_v_cancels_leader_then_probes_image() {
        // §8a ctrl-chord exception: the chord cancels the pending sequence
        // and dispatches normally — for Ctrl+V that is the image probe, so
        // the cancel rides on the probe classification (the leader must die
        // whether or not the clipboard holds an image), and exactly once:
        // the no-image reclassification must not re-cancel.
        let mut cx = ctx();
        cx.leader_pending = true;
        let c = classify_it(&ctrl('v'), &cx);
        assert!(c.cancel_leader);
        assert_eq!(c.intent, EditorIntent::ImageProbe);
    }

    #[test]
    fn leader_pending_bracketed_paste_cancels_leader_then_pastes() {
        // A paste payload is not a key the sequence can consume: the
        // ctrl-chord rule applied to a non-key event — cancel, then paste.
        let mut cx = ctx();
        cx.leader_pending = true;
        let c = classify_it(&InputEvent::Paste("hi".into()), &cx);
        assert!(c.cancel_leader);
        assert_eq!(c.intent, EditorIntent::EditorPaste);
    }

    #[test]
    fn leader_pending_ctrl_enter_feeds_leader() {
        // Enter is not a Char ctrl-chord, so no exception applies: the
        // pending sequence owns the key (§8a) ahead of follow-link.
        let mut cx = ctx();
        cx.leader_pending = true;
        let c = classify_it(&key(KeyCode::Enter, KeyModifiers::CONTROL), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::LeaderKey(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL))
        );
        assert!(!c.cancel_leader);
    }

    #[test]
    fn ctrl_enter_in_editor_follows_link() {
        let c = classify_it(&key(KeyCode::Enter, KeyModifiers::CONTROL), &ctx());
        assert_eq!(c.intent, EditorIntent::FollowLink);
    }

    // ---- leader tier -----------------------------------------------------

    #[test]
    fn leader_pending_plain_key_feeds_leader() {
        let mut cx = ctx();
        cx.leader_pending = true;
        let c = classify_it(&plain('f'), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::LeaderKey(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        );
        assert!(!c.cancel_leader);
    }

    #[test]
    fn leader_pending_with_overlay_cancels_and_routes_to_overlay() {
        let mut cx = ctx();
        cx.leader_pending = true;
        cx.overlay = Some(OverlayKind::Dialog);
        let c = classify_it(&plain('f'), &cx);
        assert!(c.cancel_leader);
        assert_eq!(c.intent, EditorIntent::Overlay);
    }

    #[test]
    fn leader_pending_ctrl_chord_cancels_then_dispatches() {
        let mut cx = ctx();
        cx.leader_pending = true;
        let c = classify_it(&ctrl('p'), &cx);
        assert!(c.cancel_leader);
        assert_eq!(
            c.intent,
            EditorIntent::ToggleOverlay {
                kind: OverlayKind::CommandPalette,
                open: OverlayOpen::CommandPalette
            }
        );
    }

    // ---- shortcut tier ---------------------------------------------------

    #[test]
    fn command_palette_chord_toggles_regardless_of_open_state() {
        // Same intent open or closed — the executor dismisses when the kind
        // is already active.
        let c = classify_it(&ctrl('p'), &ctx());
        assert_eq!(
            c.intent,
            EditorIntent::ToggleOverlay {
                kind: OverlayKind::CommandPalette,
                open: OverlayOpen::CommandPalette
            }
        );
        assert!(c.flash.is_some());

        let mut cx = ctx();
        cx.overlay = Some(OverlayKind::CommandPalette);
        let c = classify_it(&ctrl('p'), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::ToggleOverlay {
                kind: OverlayKind::CommandPalette,
                open: OverlayOpen::CommandPalette
            }
        );
    }

    #[test]
    fn leader_gateway_starts_sequence_without_flash() {
        let c = classify_it(&ctrl('g'), &ctx());
        assert_eq!(c.intent, EditorIntent::LeaderStart);
        assert_eq!(c.flash, None);
    }

    #[test]
    fn leader_gateway_with_overlay_is_consumed_noop() {
        let mut cx = ctx();
        cx.overlay = Some(OverlayKind::NoteBrowser);
        let c = classify_it(&ctrl('g'), &cx);
        assert_eq!(c.intent, EditorIntent::Consume);
    }

    #[test]
    fn focus_sidebar_with_overlay_is_consumed_noop() {
        let mut cx = ctx();
        cx.overlay = Some(OverlayKind::NoteBrowser);
        let c = classify_it(&ctrl('h'), &cx);
        assert_eq!(c.intent, EditorIntent::Consume);
    }

    #[test]
    fn toggle_drawer_chord_is_action() {
        let c = classify_it(&ctrl('t'), &ctx());
        assert_eq!(c.intent, EditorIntent::Op(EditorOp::ToggleDrawer));
    }

    #[test]
    fn quick_note_opens_dialog_only_without_overlay() {
        let c = classify_it(&ctrl('w'), &ctx());
        assert_eq!(c.intent, EditorIntent::OpenDialog(DialogRequest::QuickNote));

        let mut cx = ctx();
        cx.overlay = Some(OverlayKind::Dialog);
        let c = classify_it(&ctrl('w'), &cx);
        assert_eq!(c.intent, EditorIntent::Consume);
    }

    #[test]
    fn text_style_chord_needs_active_editor() {
        let c = classify_it(&ctrl('b'), &ctx());
        assert_eq!(
            c.intent,
            EditorIntent::Op(EditorOp::ApplyText(TextAction::Bold))
        );

        // Drawer focused: the guard fails and the chord falls through the
        // match to the focused panel.
        let mut cx = ctx();
        cx.focused = PanelKind::Drawer;
        let c = classify_it(&ctrl('b'), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::None
            }
        );
    }

    #[test]
    fn sort_dialog_targets_the_focused_drawer_view() {
        let mut cx = ctx();
        cx.focused = PanelKind::Drawer;
        cx.drawer_view = DrawerView::Find;
        let c = classify_it(&ctrl('r'), &cx);
        assert_eq!(c.intent, EditorIntent::OpenDialog(DialogRequest::SortQuery));

        cx.drawer_view = DrawerView::Files;
        let c = classify_it(&ctrl('r'), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::OpenDialog(DialogRequest::SortSidebar)
        );

        // A drawer view without a sortable list consumes the chord.
        cx.drawer_view = DrawerView::Tags;
        let c = classify_it(&ctrl('r'), &cx);
        assert_eq!(c.intent, EditorIntent::Consume);
    }

    #[test]
    fn sort_dialog_chord_falls_through_when_editor_focused() {
        // Ctrl+R must reach the editor (redo in the nvim backend).
        let c = classify_it(&ctrl('r'), &ctx());
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::None
            }
        );
    }

    #[test]
    fn f1_opens_help_or_query_help_by_focus() {
        let c = classify_it(&key(KeyCode::F(1), KeyModifiers::NONE), &ctx());
        assert_eq!(c.intent, EditorIntent::Op(EditorOp::OpenHelp));

        let mut cx = ctx();
        cx.focused = PanelKind::Drawer;
        cx.drawer_view = DrawerView::Find;
        let c = classify_it(&key(KeyCode::F(1), KeyModifiers::NONE), &cx);
        assert_eq!(c.intent, EditorIntent::Op(EditorOp::OpenQueryHelp));
    }

    #[test]
    fn unbound_fkeys_are_sunk_with_a_flash() {
        let c = classify_it(&key(KeyCode::F(9), KeyModifiers::NONE), &ctx());
        assert_eq!(c.intent, EditorIntent::Consume);
        assert!(c.flash.is_some());
    }

    #[test]
    fn high_fkeys_are_sunk_like_the_rest() {
        // "All F-keys are consumed and never forwarded to the editor" —
        // including F13+ (KeyStrike defines up to F25); the shared
        // KeyStrike::is_fkey predicate keeps this in step with the
        // binding validator.
        let c = classify_it(&key(KeyCode::F(13), KeyModifiers::NONE), &ctx());
        assert_eq!(c.intent, EditorIntent::Consume);
    }

    #[test]
    fn bound_fkey_dispatches_when_guard_holds_and_sinks_when_not() {
        // F2 = FileOperations, guarded on the active editor.
        let c = classify_it(&key(KeyCode::F(2), KeyModifiers::NONE), &ctx());
        assert_eq!(c.intent, EditorIntent::Op(EditorOp::ShowFileOps));

        let mut cx = ctx();
        cx.focused = PanelKind::Drawer;
        let c = classify_it(&key(KeyCode::F(2), KeyModifiers::NONE), &cx);
        assert_eq!(c.intent, EditorIntent::Consume);
    }

    // ---- lower tiers -----------------------------------------------------

    #[test]
    fn overlay_intercepts_unbound_keys() {
        let mut cx = ctx();
        cx.overlay = Some(OverlayKind::SavedSearches);
        let c = classify_it(&plain('x'), &cx);
        assert_eq!(c.intent, EditorIntent::Overlay);
    }

    #[test]
    fn mouse_events_take_the_hit_test_path() {
        let ev = InputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 3,
            row: 4,
            modifiers: KeyModifiers::NONE,
        });
        let c = classify_it(&ev, &ctx());
        assert_eq!(c.intent, EditorIntent::Mouse);
    }

    #[test]
    fn vim_space_starts_leader_only_when_it_leads() {
        let mut cx = ctx();
        cx.vim_space_leads = true;
        let c = classify_it(&plain(' '), &cx);
        assert_eq!(c.intent, EditorIntent::LeaderStart);

        cx.vim_space_leads = false;
        let c = classify_it(&plain(' '), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::None
            }
        );
    }

    #[test]
    fn tab_cycles_focus_when_a_non_editor_panel_passes() {
        let mut cx = ctx();
        cx.focused = PanelKind::Drawer;
        let c = classify_it(&key(KeyCode::Tab, KeyModifiers::NONE), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::FocusCycle(CycleDir::Right)
            }
        );

        let c = classify_it(&key(KeyCode::BackTab, KeyModifiers::SHIFT), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::FocusCycle(CycleDir::Left)
            }
        );
    }

    #[test]
    fn editor_keeps_tab_for_indentation() {
        let c = classify_it(&key(KeyCode::Tab, KeyModifiers::NONE), &ctx());
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::None
            }
        );
    }

    #[test]
    fn find_view_yields_focus_to_editor_on_unhandled_esc() {
        let mut cx = ctx();
        cx.focused = PanelKind::Drawer;
        cx.drawer_view = DrawerView::Find;
        let c = classify_it(&key(KeyCode::Esc, KeyModifiers::NONE), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::FocusEditor
            }
        );

        // Any other unhandled key propagates as-is.
        let c = classify_it(&plain('x'), &cx);
        assert_eq!(
            c.intent,
            EditorIntent::Panel {
                fallback: PanelFallback::None
            }
        );
    }
}
