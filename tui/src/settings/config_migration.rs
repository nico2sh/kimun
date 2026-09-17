//! Config migration — upgrades settings from older versions to the current format.
//!
//! All migration logic lives here so there is a single place to manage
//! version transitions. `ConfigMigration::run` is called once during
//! `AppSettings::load_from_file` after deserialization.

use super::AppSettings;
use super::SettingsError;

/// Current config version. Bump this when adding a new migration step.
///
/// Migrations below v3 have been removed: they upgraded the pre-`workspace_config`
/// layout (`workspace_dir` + top-level `last_paths`) and moved the index out of
/// the vault, and every installation has long since passed through them. The
/// oldest config this build understands is a v3 one.
pub const CURRENT_CONFIG_VERSION: u32 = 7;

/// Runs all necessary migrations on `settings`, mutating it in place.
/// Returns `true` if any migration was applied (caller should persist).
pub struct ConfigMigration;

impl ConfigMigration {
    /// Apply all pending migrations to bring `settings` up to
    /// `CURRENT_CONFIG_VERSION`. Returns `true` if any migration ran.
    pub fn run(settings: &mut AppSettings) -> Result<bool, SettingsError> {
        let mut migrated = false;

        // Validate current_workspace points to an existing entry.
        if let Some(ref mut wc) = settings.workspace_config
            && !wc.global.current_workspace.is_empty()
            && !wc.workspaces.contains_key(&wc.global.current_workspace)
        {
            let first = wc.workspaces.keys().next().cloned().unwrap_or_default();
            tracing::warn!(
                "current_workspace '{}' does not exist, resetting to '{}'",
                wc.global.current_workspace,
                first
            );
            wc.global.current_workspace = first;
            migrated = true;
        }

        // v3 → v4: the leader gateway takes Ctrl-G; FollowLink moves to
        // Ctrl-N (plus the hardcoded Ctrl+Enter on kitty-protocol terminals).
        if settings.config_version < 4 {
            Self::migrate_to_v4(settings);
            migrated = true;
        }

        // v4 → v5: Ctrl-P becomes the command palette; settings move to
        // Ctrl+Shift+P.
        if settings.config_version < 5 {
            Self::migrate_to_v5(settings);
            migrated = true;
        }

        // v5 → v6: settings move from Ctrl+Shift+P (kitty chord-prefix
        // collision) to Ctrl+,.
        if settings.config_version < 6 {
            Self::migrate_to_v6(settings);
            migrated = true;
        }

        // v6 → v7: the formatting chords retire to the leader's `+text` group.
        if settings.config_version < 7 {
            Self::migrate_to_v7(settings);
            migrated = true;
        }

        // Future migrations go here, gated on config_version:
        // if settings.config_version < 8 { ... migrated = true; }

        if migrated {
            settings.config_version = CURRENT_CONFIG_VERSION;
        }

        Ok(migrated)
    }

    /// v5 → v6: settings move from Ctrl+Shift+P to Ctrl+, — Ctrl+Shift+P is
    /// kitty's default hints-kitten chord prefix, which holds the screen
    /// mid-chord and made the binding look broken there. Only applies when
    /// the binding is still at the v5 default.
    fn migrate_to_v6(settings: &mut AppSettings) {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::ActionShortcuts;
        use crate::keys::key_combo::KeyCombo;
        use crate::keys::key_strike::KeyStrike;

        let ctrl = crate::keys::key_combo::KeyModifiers::new().and_ctrl();
        let ctrl_shift_p = KeyCombo::new(ctrl.and_shift(), KeyStrike::KeyP);
        let ctrl_comma = KeyCombo::new(ctrl, KeyStrike::Comma);

        let mut map = settings.key_bindings.to_hashmap();
        let at_old_default = map
            .get(&ActionShortcuts::OpenPreferences)
            .is_some_and(|v| v.as_slice() == [ctrl_shift_p]);
        let comma_free = !map.values().flatten().any(|c| *c == ctrl_comma);
        if at_old_default && comma_free {
            map.insert(ActionShortcuts::OpenPreferences, vec![ctrl_comma]);
        }
        settings.key_bindings = KeyBindings::from_hashmap(map);
    }

    /// v6 → v7: drop the retired formatting chords, which moved to the
    /// leader's `+text` group (`<leader> t b` / `t i` / `t s`).
    ///
    /// Needed because `key_bindings` is serialized: every config written
    /// before this release carries the old defaults — `TextEditor-Bold =
    /// ["ctrl&B"]` and friends — and `merge_missing_default_bindings` only
    /// ever *adds*. Without this step the change reaches new installs only.
    /// Existing ones would keep the chords *and* gain the leader group, which
    /// is more inconsistent than before it, and two of those chords
    /// (`Ctrl+I`, `Ctrl+Shift+L`) cannot arrive on a legacy terminal at all —
    /// so the startup warning would fire about bindings kimün itself wrote.
    ///
    /// Each entry goes only if it still holds exactly the combo that was its
    /// default; an edited one is left alone. Someone who typed the old
    /// default by hand is indistinguishable from someone who inherited it —
    /// unavoidable once defaults are serialized — and re-adding the line is
    /// how they get it back.
    fn migrate_to_v7(settings: &mut AppSettings) {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let ctrl = KeyModifiers::new().and_ctrl();
        let retired = [
            (TextAction::Bold, KeyCombo::new(ctrl, KeyStrike::KeyB)),
            (TextAction::Italic, KeyCombo::new(ctrl, KeyStrike::KeyI)),
            (TextAction::Underline, KeyCombo::new(ctrl, KeyStrike::KeyU)),
            (
                TextAction::Strikethrough,
                KeyCombo::new(ctrl, KeyStrike::KeyS),
            ),
            (
                TextAction::Image,
                KeyCombo::new(ctrl.and_shift(), KeyStrike::KeyL),
            ),
            // These two never reached a config file — they were overwritten
            // inside `default_keybindings` before it was ever serialized —
            // but they were defaults on paper, so retire them for the same
            // reason and in the same place.
            (TextAction::Link, KeyCombo::new(ctrl, KeyStrike::KeyL)),
            (
                TextAction::ToggleHeader,
                KeyCombo::new(ctrl, KeyStrike::KeyT),
            ),
        ];

        let mut map = settings.key_bindings.to_hashmap();
        for (action, old_default) in retired {
            let action = ActionShortcuts::Text(action);
            if map
                .get(&action)
                .is_some_and(|combos| combos.as_slice() == [old_default])
            {
                map.remove(&action);
            }
        }
        settings.key_bindings = KeyBindings::from_hashmap(map);
    }

    /// v4 → v5: swap the palette onto Ctrl-P and settings onto Ctrl+Shift+P —
    /// only for bindings still at their previous defaults; customised ones
    /// are left untouched.
    fn migrate_to_v5(settings: &mut AppSettings) {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::ActionShortcuts;
        use crate::keys::key_combo::KeyCombo;
        use crate::keys::key_strike::KeyStrike;

        let ctrl = crate::keys::key_combo::KeyModifiers::new().and_ctrl();
        let ctrl_shift = ctrl.and_shift();
        let ctrl_p = KeyCombo::new(ctrl, KeyStrike::KeyP);
        let ctrl_shift_p = KeyCombo::new(ctrl_shift, KeyStrike::KeyP);

        let mut map = settings.key_bindings.to_hashmap();
        let settings_is_old_default = map
            .get(&ActionShortcuts::OpenPreferences)
            .is_some_and(|v| v.as_slice() == [ctrl_p]);
        let palette_unset_or_old_default = map
            .get(&ActionShortcuts::OpenCommandPalette)
            .is_none_or(|v| v.is_empty() || v.as_slice() == [ctrl_shift_p]);
        if settings_is_old_default && palette_unset_or_old_default {
            map.insert(ActionShortcuts::OpenPreferences, vec![ctrl_shift_p]);
            map.insert(ActionShortcuts::OpenCommandPalette, vec![ctrl_p]);
        }
        settings.key_bindings = KeyBindings::from_hashmap(map);
    }

    /// v3 → v4: move Ctrl-G from FollowLink to the new Leader gateway —
    /// but only when the user still had the old default (FollowLink bound
    /// to exactly Ctrl-G); customised bindings are left untouched, and the
    /// leader is then inserted only if Ctrl-G is free.
    fn migrate_to_v4(settings: &mut AppSettings) {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::ActionShortcuts;
        use crate::keys::key_combo::KeyCombo;
        use crate::keys::key_strike::KeyStrike;

        let ctrl = crate::keys::key_combo::KeyModifiers::new().and_ctrl();
        let ctrl_g = KeyCombo::new(ctrl, KeyStrike::KeyG);
        let ctrl_n = KeyCombo::new(ctrl, KeyStrike::KeyN);

        let mut map = settings.key_bindings.to_hashmap();
        let follow_is_old_default = map
            .get(&ActionShortcuts::FollowLink)
            .is_some_and(|v| v.as_slice() == [ctrl_g]);
        if follow_is_old_default {
            // Old default: hand Ctrl-G to the leader, FollowLink → Ctrl-N.
            map.insert(ActionShortcuts::FollowLink, vec![ctrl_n]);
            map.entry(ActionShortcuts::Leader).or_default().push(ctrl_g);
        }
        settings.key_bindings = KeyBindings::from_hashmap(map);
        // (If the user had customised FollowLink, the leader simply stays
        // unbound until `merge_missing_default_bindings` finds Ctrl-G free
        // or the user binds it explicitly.)
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::settings::workspace_config::WorkspaceConfig;

    #[test]
    fn a_config_already_at_the_current_version_is_left_alone() {
        let mut settings = AppSettings::default();
        settings.config_version = CURRENT_CONFIG_VERSION;
        settings.workspace_config = Some(WorkspaceConfig::new_empty());

        let migrated = ConfigMigration::run(&mut settings).unwrap();
        assert!(!migrated);
    }

    #[test]
    fn v4_moves_ctrl_g_from_followlink_to_leader() {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::ActionShortcuts;
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let ctrl = KeyModifiers::new().and_ctrl();
        let ctrl_g = KeyCombo::new(ctrl, KeyStrike::KeyG);
        let ctrl_n = KeyCombo::new(ctrl, KeyStrike::KeyN);

        // Old default: FollowLink bound to exactly Ctrl-G.
        let mut settings = AppSettings::default();
        let mut map = std::collections::HashMap::new();
        map.insert(ActionShortcuts::FollowLink, vec![ctrl_g]);
        settings.key_bindings = KeyBindings::from_hashmap(map);
        settings.config_version = 3;

        assert!(ConfigMigration::run(&mut settings).unwrap());
        let map = settings.key_bindings.to_hashmap();
        assert_eq!(map.get(&ActionShortcuts::Leader), Some(&vec![ctrl_g]));
        assert_eq!(map.get(&ActionShortcuts::FollowLink), Some(&vec![ctrl_n]));
        assert_eq!(settings.config_version, CURRENT_CONFIG_VERSION);
    }

    /// The retired formatting chords leave existing configs, so the leader
    /// really is the only route — for upgraders as well as new installs.
    #[test]
    fn v7_retires_the_formatting_chords() {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let ctrl = KeyModifiers::new().and_ctrl();
        let mut settings = AppSettings::default();
        let mut map = std::collections::HashMap::new();
        // Exactly what a pre-v7 config.toml carries.
        for (action, key) in [
            (TextAction::Bold, KeyStrike::KeyB),
            (TextAction::Italic, KeyStrike::KeyI),
            (TextAction::Underline, KeyStrike::KeyU),
            (TextAction::Strikethrough, KeyStrike::KeyS),
        ] {
            map.insert(
                ActionShortcuts::Text(action),
                vec![KeyCombo::new(ctrl, key)],
            );
        }
        map.insert(
            ActionShortcuts::Text(TextAction::Image),
            vec![KeyCombo::new(ctrl.and_shift(), KeyStrike::KeyL)],
        );
        // A chord the user chose themselves, which must survive.
        let ctrl_y = KeyCombo::new(ctrl, KeyStrike::KeyY);
        map.insert(ActionShortcuts::Text(TextAction::Header(1)), vec![ctrl_y]);
        settings.key_bindings = KeyBindings::from_hashmap(map);
        settings.config_version = 6;

        assert!(ConfigMigration::run(&mut settings).unwrap());
        let map = settings.key_bindings.to_hashmap();
        for action in [
            TextAction::Bold,
            TextAction::Italic,
            TextAction::Underline,
            TextAction::Strikethrough,
            TextAction::Image,
        ] {
            let name = format!("{action:?}");
            assert!(
                !map.contains_key(&ActionShortcuts::Text(action)),
                "{name} should have retired to the leader"
            );
        }
        assert_eq!(
            map.get(&ActionShortcuts::Text(TextAction::Header(1))),
            Some(&vec![ctrl_y]),
            "a chord the user chose is not ours to retire"
        );
    }

    /// An edited formatting chord is the user's, not an inherited default, so
    /// the migration leaves it where it is.
    #[test]
    fn v7_leaves_a_customised_formatting_chord_alone() {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let ctrl = KeyModifiers::new().and_ctrl();
        let moved = KeyCombo::new(ctrl.and_alt(), KeyStrike::KeyB);
        let mut settings = AppSettings::default();
        settings.key_bindings = KeyBindings::from_hashmap(std::collections::HashMap::from([(
            ActionShortcuts::Text(TextAction::Bold),
            vec![moved],
        )]));
        settings.config_version = 6;

        ConfigMigration::run(&mut settings).unwrap();
        assert_eq!(
            settings
                .key_bindings
                .to_hashmap()
                .get(&ActionShortcuts::Text(TextAction::Bold)),
            Some(&vec![moved])
        );
    }

    #[test]
    fn v6_moves_settings_to_ctrl_comma() {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::ActionShortcuts;
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let ctrl = KeyModifiers::new().and_ctrl();
        let ctrl_shift_p = KeyCombo::new(ctrl.and_shift(), KeyStrike::KeyP);
        let ctrl_comma = KeyCombo::new(ctrl, KeyStrike::Comma);

        let mut settings = AppSettings::default();
        let mut map = std::collections::HashMap::new();
        map.insert(ActionShortcuts::OpenPreferences, vec![ctrl_shift_p]);
        settings.key_bindings = KeyBindings::from_hashmap(map);
        settings.config_version = 5;

        assert!(ConfigMigration::run(&mut settings).unwrap());
        let map = settings.key_bindings.to_hashmap();
        assert_eq!(
            map.get(&ActionShortcuts::OpenPreferences),
            Some(&vec![ctrl_comma])
        );
    }

    #[test]
    fn v5_swaps_palette_onto_ctrl_p() {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::ActionShortcuts;
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let ctrl = KeyModifiers::new().and_ctrl();
        let ctrl_p = KeyCombo::new(ctrl, KeyStrike::KeyP);
        let ctrl_shift_p = KeyCombo::new(ctrl.and_shift(), KeyStrike::KeyP);

        let mut settings = AppSettings::default();
        let mut map = std::collections::HashMap::new();
        map.insert(ActionShortcuts::OpenPreferences, vec![ctrl_p]);
        settings.key_bindings = KeyBindings::from_hashmap(map);
        settings.config_version = 4;

        assert!(ConfigMigration::run(&mut settings).unwrap());
        let map = settings.key_bindings.to_hashmap();
        assert_eq!(
            map.get(&ActionShortcuts::OpenCommandPalette),
            Some(&vec![ctrl_p])
        );
        // v6 chains after v5: settings end on Ctrl+, (kitty collision).
        let ctrl_comma = KeyCombo::new(ctrl, KeyStrike::Comma);
        assert_eq!(
            map.get(&ActionShortcuts::OpenPreferences),
            Some(&vec![ctrl_comma])
        );
        let _ = ctrl_shift_p;
    }

    #[test]
    fn v5_leaves_customised_settings_binding_alone() {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::ActionShortcuts;
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let ctrl = KeyModifiers::new().and_ctrl();
        let ctrl_x = KeyCombo::new(ctrl, KeyStrike::KeyX);

        let mut settings = AppSettings::default();
        let mut map = std::collections::HashMap::new();
        map.insert(ActionShortcuts::OpenPreferences, vec![ctrl_x]);
        settings.key_bindings = KeyBindings::from_hashmap(map);
        settings.config_version = 4;

        ConfigMigration::run(&mut settings).unwrap();
        let map = settings.key_bindings.to_hashmap();
        assert_eq!(
            map.get(&ActionShortcuts::OpenPreferences),
            Some(&vec![ctrl_x])
        );
    }

    #[test]
    fn v4_leaves_customised_followlink_alone() {
        use crate::keys::KeyBindings;
        use crate::keys::action_shortcuts::ActionShortcuts;
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let ctrl = KeyModifiers::new().and_ctrl();
        let ctrl_x = KeyCombo::new(ctrl, KeyStrike::KeyX);

        let mut settings = AppSettings::default();
        let mut map = std::collections::HashMap::new();
        map.insert(ActionShortcuts::FollowLink, vec![ctrl_x]);
        settings.key_bindings = KeyBindings::from_hashmap(map);
        settings.config_version = 3;

        ConfigMigration::run(&mut settings).unwrap();
        let map = settings.key_bindings.to_hashmap();
        // Customised binding untouched; the leader is not force-bound.
        assert_eq!(map.get(&ActionShortcuts::FollowLink), Some(&vec![ctrl_x]));
        assert!(
            map.get(&ActionShortcuts::Leader)
                .is_none_or(|v| v.is_empty())
        );
    }
}
