use super::state::AutocompleteState;

/// A toolkit-free popup input. A front end maps its own keys and clicks onto
/// one of these; [`apply_command`] holds the rules (what moves the highlight,
/// what accepts, what dismisses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupCommand {
    Down,
    Up,
    PageDown,
    PageUp,
    Home,
    End,
    /// Tab / Enter: accept the highlighted suggestion.
    Accept,
    /// Esc: close the popup, buffer untouched.
    Dismiss,
    /// A click on the visible row at this offset from the window start:
    /// highlights and accepts it.
    ClickRow(usize),
    /// An event over the popup that does nothing but must not fall through
    /// (and so dismiss it): a move, a drag, another button.
    Swallow,
}

/// Result of forwarding an input to the popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupOutcome {
    Consumed(PopupAction),
    NotHandled,
}

/// What the host should do after a `Consumed` outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction {
    /// Internal navigation, no buffer change.
    None,
    /// User accepted the highlighted suggestion. Host pulls
    /// `state.selected()` and writes the replacement.
    Accept,
    /// User dismissed: close the popup without changing the buffer.
    Dismiss,
}

/// Apply `command` to the popup state. Accept on an empty list is
/// `NotHandled`, so the host processes the key as usual (Tab indents, Enter
/// breaks the line).
pub fn apply_command(state: &mut AutocompleteState, command: PopupCommand) -> PopupOutcome {
    let none = PopupOutcome::Consumed(PopupAction::None);
    match command {
        PopupCommand::Down => {
            state.move_highlight_down();
            none
        }
        PopupCommand::Up => {
            state.move_highlight_up();
            none
        }
        PopupCommand::PageDown => {
            state.page_down();
            none
        }
        PopupCommand::PageUp => {
            state.page_up();
            none
        }
        PopupCommand::Home => {
            state.home();
            none
        }
        PopupCommand::End => {
            state.end();
            none
        }
        PopupCommand::Accept => {
            if state.selected().is_some() {
                PopupOutcome::Consumed(PopupAction::Accept)
            } else {
                PopupOutcome::NotHandled
            }
        }
        PopupCommand::Dismiss => PopupOutcome::Consumed(PopupAction::Dismiss),
        PopupCommand::ClickRow(offset) => {
            let (start, end) = state.visible_window();
            let idx = start + offset;
            if idx < end {
                state.highlighted = idx;
                PopupOutcome::Consumed(PopupAction::Accept)
            } else {
                none
            }
        }
        PopupCommand::Swallow => none,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::autocomplete::{Suggestion, TriggerKind};

    fn state(n: usize) -> AutocompleteState {
        let mut st = AutocompleteState::new(TriggerKind::Hashtag, (0, 0));
        st.max_visible_rows = 3;
        st.set_items(
            (0..n)
                .map(|i| Suggestion {
                    display: format!("t{i}"),
                    secondary: None,
                })
                .collect(),
        );
        st
    }

    #[test]
    fn navigation_is_consumed_without_an_action() {
        let mut st = state(10);
        for c in [
            PopupCommand::Down,
            PopupCommand::Up,
            PopupCommand::PageDown,
            PopupCommand::PageUp,
            PopupCommand::End,
            PopupCommand::Home,
            PopupCommand::Swallow,
        ] {
            assert_eq!(
                apply_command(&mut st, c),
                PopupOutcome::Consumed(PopupAction::None)
            );
        }
    }

    #[test]
    fn accept_needs_a_selection() {
        assert_eq!(
            apply_command(&mut state(0), PopupCommand::Accept),
            PopupOutcome::NotHandled
        );
        assert_eq!(
            apply_command(&mut state(2), PopupCommand::Accept),
            PopupOutcome::Consumed(PopupAction::Accept)
        );
    }

    #[test]
    fn dismiss_is_consumed_as_dismiss() {
        assert_eq!(
            apply_command(&mut state(0), PopupCommand::Dismiss),
            PopupOutcome::Consumed(PopupAction::Dismiss)
        );
    }

    #[test]
    fn click_highlights_and_accepts_a_visible_row() {
        let mut st = state(10);
        st.end(); // window slides: rows 7..10
        assert_eq!(
            apply_command(&mut st, PopupCommand::ClickRow(1)),
            PopupOutcome::Consumed(PopupAction::Accept)
        );
        assert_eq!(st.highlighted, 8);
    }

    #[test]
    fn click_below_the_last_row_does_nothing() {
        let mut st = state(2);
        assert_eq!(
            apply_command(&mut st, PopupCommand::ClickRow(2)),
            PopupOutcome::Consumed(PopupAction::None)
        );
        assert_eq!(st.highlighted, 0);
    }
}
