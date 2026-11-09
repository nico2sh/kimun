//! The Ask workspace's show/hide and Sources-sync rules, toolkit-free (see
//! CONTEXT.md: **Ask workspace**, **Sources view**). [`AskFlow`] is plain state
//! — whether the Thread is showing in the editor area, and which turn the
//! Sources view last mirrored — with transitions that answer what the host must
//! do. The host owns the panels and executes the answers; the thread itself
//! lives on in the host whether or not it is shown.

/// What a drawer-view switch asks of the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// Nothing to do.
    Stay,
    /// Show the Thread in the editor area, then point the Sources view at the
    /// selected turn (unconditionally — see [`AskFlow::synced`]).
    Enter,
    /// Return the editor area to the note editor. The thread survives.
    Leave,
}

/// What the host does after an input event on the Thread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfterInput {
    /// Point the Sources view at the selected turn (it changed since the last
    /// sync).
    pub sync_sources: bool,
    /// Focus the source a clicked citation `[n]` names (an ordinal, not a row).
    pub focus_citation: Option<usize>,
}

/// Show/hide state and the Sources-sync memory of the Ask workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskFlow {
    showing: bool,
    /// The turn id last mirrored into the Sources view. Turn ids are never
    /// reused, so a stale value can never falsely match a fresh selection.
    /// Reset by [`AskFlow::hide_if_shown`] so re-entering always re-syncs.
    last_synced_turn: Option<u64>,
}

impl AskFlow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the Thread is the editor area's content.
    pub fn showing(&self) -> bool {
        self.showing
    }

    /// The turn the Sources view last mirrored.
    pub fn last_synced_turn(&self) -> Option<u64> {
        self.last_synced_turn
    }

    /// Adopt the host's own record of whether the Thread is shown — for a host
    /// (the TUI) whose panel set also tracks the editor-area content.
    pub fn sync_showing(&mut self, showing: bool) {
        self.showing = showing;
    }

    /// A drawer-view switch: to ASK enters, to any other view leaves.
    pub fn switch_to(&mut self, ask: bool) -> Transition {
        if ask { self.enter() } else { self.leave() }
    }

    /// Enter the workspace: [`Transition::Enter`] unless already showing.
    pub fn enter(&mut self) -> Transition {
        if self.showing {
            return Transition::Stay;
        }
        self.showing = true;
        Transition::Enter
    }

    /// Leave for another drawer view: [`Transition::Leave`] when showing. The
    /// sync memory is kept (an `Enter` re-syncs regardless).
    pub fn leave(&mut self) -> Transition {
        if !self.showing {
            return Transition::Stay;
        }
        self.showing = false;
        Transition::Leave
    }

    /// Something else (a note, an attachment) takes the editor area. `true` =
    /// the Thread was showing: hide it **and** switch the drawer off ASK to
    /// FILES without revealing a hidden drawer (a hidden thread's sources
    /// shouldn't linger, and a drawer left on ASK would read the next ASK click
    /// as a toggle-off). Forgets the synced turn so re-entering re-syncs.
    pub fn hide_if_shown(&mut self) -> bool {
        if !self.showing {
            return false;
        }
        self.showing = false;
        self.last_synced_turn = None;
        true
    }

    /// The host pointed the Sources view at turn `turn_id`.
    pub fn synced(&mut self, turn_id: u64) {
        self.last_synced_turn = Some(turn_id);
    }

    /// After an input event on the Thread: re-sync the Sources view when the
    /// selected turn changed since the last sync, and focus a clicked
    /// citation. Nothing while the Thread is not showing — the host then
    /// leaves the citation signal untaken.
    pub fn after_input(&self, selected: Option<u64>, citation: Option<usize>) -> AfterInput {
        if !self.showing {
            return AfterInput::default();
        }
        AfterInput {
            sync_sources: selected.is_some() && selected != self.last_synced_turn,
            focus_citation: citation,
        }
    }

    /// An answer for `turn_id` landed (the host always hands it to the
    /// thread). `true` = refresh the Sources view with that turn's new
    /// sources: the Thread is showing and the turn is the selected one.
    pub fn answer_ready(&self, turn_id: u64, selected: Option<u64>) -> bool {
        self.showing && selected == Some(turn_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_and_leave_flip_only_on_change() {
        let mut flow = AskFlow::new();
        assert_eq!(flow.leave(), Transition::Stay);
        assert_eq!(flow.enter(), Transition::Enter);
        assert!(flow.showing());
        assert_eq!(flow.enter(), Transition::Stay);
        assert_eq!(flow.switch_to(true), Transition::Stay);
        assert_eq!(flow.switch_to(false), Transition::Leave);
        assert!(!flow.showing());
        assert_eq!(flow.switch_to(false), Transition::Stay);
        assert_eq!(flow.switch_to(true), Transition::Enter);
    }

    #[test]
    fn leave_keeps_the_synced_turn_but_hide_forgets_it() {
        let mut flow = AskFlow::new();
        flow.enter();
        flow.synced(4);
        flow.leave();
        assert_eq!(flow.last_synced_turn(), Some(4));

        flow.enter();
        assert!(flow.hide_if_shown());
        assert!(!flow.showing());
        assert_eq!(flow.last_synced_turn(), None);
        assert!(!flow.hide_if_shown(), "nothing to hide twice");
    }

    #[test]
    fn input_syncs_only_a_changed_selection_while_showing() {
        let mut flow = AskFlow::new();
        // Not showing: nothing, not even the citation.
        assert_eq!(flow.after_input(Some(1), Some(2)), AfterInput::default());

        flow.enter();
        assert_eq!(
            flow.after_input(Some(1), None),
            AfterInput {
                sync_sources: true,
                focus_citation: None
            }
        );
        flow.synced(1);
        assert_eq!(flow.after_input(Some(1), None), AfterInput::default());
        assert_eq!(
            flow.after_input(None, None),
            AfterInput::default(),
            "no selected turn"
        );
        assert_eq!(
            flow.after_input(Some(1), Some(3)),
            AfterInput {
                sync_sources: false,
                focus_citation: Some(3)
            }
        );
        assert!(flow.after_input(Some(2), None).sync_sources);
    }

    #[test]
    fn an_answer_refreshes_sources_only_for_the_shown_selected_turn() {
        let mut flow = AskFlow::new();
        assert!(!flow.answer_ready(1, Some(1)), "not showing");
        flow.enter();
        assert!(flow.answer_ready(1, Some(1)));
        assert!(!flow.answer_ready(1, Some(2)), "another turn is selected");
        assert!(!flow.answer_ready(1, None));
    }

    #[test]
    fn re_entering_after_hide_resyncs_the_same_turn() {
        let mut flow = AskFlow::new();
        flow.enter();
        flow.synced(7);
        assert!(!flow.after_input(Some(7), None).sync_sources);
        flow.hide_if_shown();
        assert_eq!(flow.enter(), Transition::Enter);
        assert!(
            flow.after_input(Some(7), None).sync_sources,
            "the synced turn was forgotten"
        );
    }
}
