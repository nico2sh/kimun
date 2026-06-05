//! The **hint registry** — the one place that turns a focus/cursor context
//! into the key hints the status bar shows. Each surface still declares its
//! own context hints (`hint_shortcuts`); this module owns the *global* hints
//! (always-on actions, right-aligned on status line 1) and the shared `Hint`
//! shape. The leader engine and which-key overlay (phases 05/06) will read
//! from here so hint text never forks from the actual bindings.

use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;

/// One key hint: `(key combo label, action label)`. An empty key renders the
/// label alone, emphasized (used by the nvim backend's mode line).
pub type Hint = (String, String);

/// Build hints for `(action, label)` pairs, dropping actions with no binding.
pub fn hints_for(kb: &KeyBindings, actions: &[(ActionShortcuts, &str)]) -> Vec<Hint> {
    actions
        .iter()
        .filter_map(|(action, label)| {
            kb.first_combo_for(action)
                .map(|combo| (combo, label.to_string()))
        })
        .collect()
}

/// The always-on global hints, right-aligned on status line 1: the actions a
/// user must always be able to find regardless of focus.
pub fn global_hints(kb: &KeyBindings) -> Vec<Hint> {
    hints_for(
        kb,
        &[
            (ActionShortcuts::SearchNotes, "search"),
            (ActionShortcuts::OpenPreferences, "prefs"),
            (ActionShortcuts::Quit, "quit"),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_hints_resolve_from_default_bindings() {
        let settings = crate::settings::AppSettings::default();
        let hints = global_hints(&settings.key_bindings);
        let labels: Vec<&str> = hints.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, vec!["search", "prefs", "quit"]);
        // Keys come from the real bindings, not hard-coded strings.
        assert!(hints.iter().all(|(k, _)| !k.is_empty()));
    }

    #[test]
    fn unbound_actions_are_dropped() {
        let kb = KeyBindings::empty();
        assert!(global_hints(&kb).is_empty());
    }
}
