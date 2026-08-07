//! Config migration — upgrades settings from older versions to the current format.
//!
//! All migration logic lives here so there is a single place to manage
//! version transitions. `ConfigMigration::run` is called once during
//! `AppSettings::load_from_file` after deserialization.

use kimun_core::IndexFile;
use kimun_core::nfs::VaultPath;
use kimun_core::system::{self, SystemPath};

use super::history::HistoryFile;

use super::AppSettings;
use super::SettingsError;
use super::workspace_config::{WorkspaceConfig, WorkspaceEntry};

/// Current config version. Bump this when adding a new migration step.
pub const CURRENT_CONFIG_VERSION: u32 = 6;

/// Runs all necessary migrations on `settings`, mutating it in place.
/// Returns `true` if any migration was applied (caller should persist).
pub struct ConfigMigration;

impl ConfigMigration {
    /// Apply all pending migrations to bring `settings` up to
    /// `CURRENT_CONFIG_VERSION`. Returns `true` if any migration ran.
    pub fn run(settings: &mut AppSettings) -> Result<bool, SettingsError> {
        let mut migrated = false;

        // v1 → v2: workspace_dir → workspace_config
        if settings.workspace_dir.is_some() {
            Self::migrate_workspace_dir(settings)?;
            migrated = true;
        }

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

        // v2 → v3: move per-workspace SQLite cache + extract last_paths history.
        if settings.config_version < 3 {
            Self::migrate_to_v3(settings)?;
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

        // Future migrations go here, gated on config_version:
        // if settings.config_version < 7 { ... migrated = true; }

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

    /// v2 → v3: move `<workspace>/kimun.sqlite` to
    /// `<cache_dir>/<workspace>.kimuncache` and extract per-workspace
    /// `last_paths` to `<history_dir>/<workspace>.txt`. Then clear the
    /// in-memory `last_paths` so the next save does not re-write them.
    ///
    /// Pre-flight: validates every workspace name; aborts with a single
    /// error listing every bad name. Idempotent: skips any step whose
    /// destination already exists.
    fn migrate_to_v3(settings: &mut AppSettings) -> Result<(), SettingsError> {
        let Some(ref wc) = settings.workspace_config else {
            return Ok(());
        };

        let mut invalid = Vec::new();
        for name in wc.workspaces.keys() {
            if let Err(e) = kimun_core::nfs::filename::validate_filename(name) {
                invalid.push(format!("{e}"));
            }
        }
        if !invalid.is_empty() {
            return Err(SettingsError::Migration(format!(
                "Cannot migrate to v3: invalid workspace names:\n  - {}",
                invalid.join("\n  - ")
            )));
        }

        if let Some(ref cfg_path) = settings.config_file {
            let bak_path = cfg_path.with_extension("toml.bak.v2");
            if !bak_path.exists() {
                let current = std::fs::read(cfg_path)?;
                system::replace_atomically(&bak_path, &current).map_err(|e| {
                    SettingsError::Migration(format!(
                        "failed to back up config to {bak_path:?}: {e}"
                    ))
                })?;
                tracing::info!("backed up v2 config to {:?}", bak_path);
            }
        }

        let cache_dir = settings.cache_dir_resolved().clone();
        let history_dir = settings.history_dir_resolved().clone();

        let work: Vec<(String, std::path::PathBuf, Vec<String>)> = wc
            .workspaces
            .iter()
            .map(|(name, entry)| {
                (
                    name.clone(),
                    entry.effective_path().clone(),
                    entry.last_paths.clone(),
                )
            })
            .collect();

        // Workspaces whose history could not be written out, so their
        // `last_paths` must survive rather than be cleared as migrated.
        let mut unwritten: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (name, ws_path, last_paths) in work {
            // A workspace path that is not absolute cannot name an index on
            // this machine — but the rest of this entry's migration still
            // applies, and `last_paths` is cleared below whatever happens
            // here, so skipping the history write would destroy it.
            match SystemPath::try_absolute(&ws_path) {
                Ok(path) => {
                    let old_index = IndexFile::legacy_in_workspace(&path);
                    let new_index = IndexFile::in_dir(&cache_dir, &name);
                    // Moving the index is best-effort, and deliberately so: it
                    // is a rebuildable cache, and this runs inside
                    // `load_from_file` on the first launch after an upgrade.
                    // Failing here used to propagate out and stop the app from
                    // starting at all — on Windows, where a handle on the index
                    // blocks the move, that turned a stale cache into "kimün
                    // will not open". The worst case of leaving it behind is
                    // one reindex.
                    if old_index.exists() {
                        if new_index.exists() {
                            tracing::warn!(
                                "destination index {new_index} already exists, leaving the old one at {old_index}"
                            );
                        } else if let Err(e) = system::ensure_dir(&cache_dir)
                            .and_then(|()| old_index.move_to(&new_index))
                        {
                            // Vault directory and cache directory are routinely
                            // on different volumes, and the index brings its
                            // sidecars.
                            tracing::warn!(
                                "could not move {old_index} to {new_index}: {e}. \
                                 Leaving it in place; the workspace will reindex."
                            );
                        } else {
                            tracing::info!("migrated {old_index} -> {new_index}");
                        }
                    }
                }
                Err(e) => tracing::warn!("skipping index migration for {name}: {e}"),
            }

            if !last_paths.is_empty() {
                let history = HistoryFile::in_dir(&history_dir, &name);
                if !history.exists() {
                    // Through `VaultPath`, which is the form the history file
                    // is read back as anyway — so a v2 entry written in some
                    // other form lands here already normalized.
                    let paths: Vec<VaultPath> = last_paths.iter().map(VaultPath::new).collect();
                    // Non-fatal like the index move above, for the same reason:
                    // a recent-notes list is not worth refusing to start over.
                    // But the entry is then left alone below, because clearing
                    // it after a failed write is how the list gets destroyed
                    // rather than postponed.
                    if let Err(e) = history.write(&paths) {
                        tracing::warn!("could not write {history}: {e}");
                        unwritten.insert(name.clone());
                    }
                }
            }
        }

        if let Some(ref mut wc) = settings.workspace_config {
            for (name, entry) in wc.workspaces.iter_mut() {
                if !unwritten.contains(name) {
                    entry.last_paths.clear();
                }
            }
        }

        Ok(())
    }

    /// Migrate the legacy `workspace_dir` field into `workspace_config`.
    ///
    /// Two sub-cases:
    /// 1. No `workspace_config` exists — full migration: create one with a
    ///    "default" workspace from the legacy fields.
    /// 2. `workspace_config` already exists — the legacy field is orphaned
    ///    (e.g. from a partial earlier migration). Add it as "default" if no
    ///    workspace already points to the same path.
    fn migrate_workspace_dir(settings: &mut AppSettings) -> Result<(), SettingsError> {
        let Some(workspace_dir) = settings.workspace_dir.take() else {
            return Ok(());
        };

        if settings.workspace_config.is_none() {
            // Full Phase 1 → Phase 2 migration.
            if !workspace_dir.exists() {
                return Err(SettingsError::Migration(format!(
                    "Cannot migrate: workspace directory {} no longer exists",
                    workspace_dir.display()
                )));
            }
            tracing::info!("Migrating Phase 1 config to Phase 2 format");
            let last_paths: Vec<String> =
                settings.last_paths.iter().map(|p| p.to_string()).collect();

            settings.workspace_config = Some(WorkspaceConfig::from_phase1_migration(
                workspace_dir,
                last_paths,
            ));
            // Theme stays as the top-level field — no duplication.
        } else if let Some(ref mut wc) = settings.workspace_config {
            // Phase 2 config exists but legacy workspace_dir was still present.
            let already_exists = wc
                .workspaces
                .values()
                .any(|e| *e.effective_path() == workspace_dir);
            if !already_exists && !workspace_dir.exists() {
                tracing::warn!(
                    "Dropping orphaned workspace_dir {:?} (directory no longer exists)",
                    workspace_dir
                );
            } else if !already_exists && workspace_dir.exists() {
                tracing::info!(
                    "Migrating orphaned workspace_dir into workspace_config as 'default'"
                );
                let name = Self::unique_workspace_name(wc, "default");
                let last_paths: Vec<String> =
                    settings.last_paths.iter().map(|p| p.to_string()).collect();
                let entry = WorkspaceEntry {
                    path: workspace_dir,
                    last_paths,
                    created: chrono::Utc::now(),
                    quick_note_path: None,
                    inbox_path: None,
                    resolved_path: None,
                    file_key: None,
                };
                wc.workspaces.insert(name, entry);
            }
        }

        settings.last_paths.clear();
        Ok(())
    }

    /// Find a unique workspace name starting from `base`. If `base` is taken,
    /// tries `base-2`, `base-3`, etc.
    fn unique_workspace_name(wc: &WorkspaceConfig, base: &str) -> String {
        if !wc.workspaces.contains_key(base) {
            return base.to_string();
        }
        let mut n = 2;
        loop {
            let candidate = format!("{}-{}", base, n);
            if !wc.workspaces.contains_key(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn settings_with_workspace_dir(path: &str) -> AppSettings {
        let mut s = AppSettings::default();
        s.workspace_dir = Some(PathBuf::from(path));
        s.theme = "gruvbox_dark".to_string();
        s
    }

    #[test]
    fn full_phase1_migration_creates_default_workspace() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut settings = settings_with_workspace_dir(dir.path().to_str().unwrap());

        let migrated = ConfigMigration::run(&mut settings).unwrap();

        assert!(migrated);
        assert!(settings.workspace_dir.is_none());
        assert!(settings.last_paths.is_empty());
        assert_eq!(settings.config_version, CURRENT_CONFIG_VERSION);
        let wc = settings.workspace_config.as_ref().unwrap();
        assert!(wc.workspaces.contains_key("default"));
        assert_eq!(wc.global.current_workspace, "default");
    }

    #[test]
    fn full_phase1_migration_fails_for_missing_dir() {
        let mut settings = settings_with_workspace_dir("/nonexistent/path/that/does/not/exist");
        let result = ConfigMigration::run(&mut settings);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot migrate"));
    }

    #[test]
    fn orphaned_workspace_dir_migrated_into_existing_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut settings = settings_with_workspace_dir(dir.path().to_str().unwrap());

        // Pre-existing Phase 2 config with a different workspace.
        let other_dir = tempfile::TempDir::new().unwrap();
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("production".to_string(), other_dir.path().to_path_buf())
            .unwrap();
        wc.global.current_workspace = "production".to_string();
        settings.workspace_config = Some(wc);

        let migrated = ConfigMigration::run(&mut settings).unwrap();

        assert!(migrated);
        assert!(settings.workspace_dir.is_none());
        let wc = settings.workspace_config.as_ref().unwrap();
        assert!(wc.workspaces.contains_key("default"));
        assert!(wc.workspaces.contains_key("production"));
        assert_eq!(wc.global.current_workspace, "production"); // unchanged
    }

    /// A workspace path this machine cannot turn into an index path must still
    /// An index that cannot be moved must not stop the app from starting.
    ///
    /// This migration runs inside `load_from_file`, on the first launch after
    /// an upgrade. The index is a rebuildable cache, so the worst case of
    /// leaving it where it is amounts to one reindex — whereas propagating the
    /// error means kimün does not open at all, which is what a Windows handle
    /// still on the index used to cause.
    #[test]
    fn v2_migration_survives_an_index_it_cannot_move() {
        let dir = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let legacy = workspace.path().join("kimun.sqlite");
        std::fs::write(&legacy, b"an index").unwrap();

        // A *file* where the cache directory should be, so the move cannot
        // land. A stand-in for the Windows lock, which cannot be provoked on
        // demand; what is under test is the error handling, not the cause.
        let cache = dir.path().join("cache");
        std::fs::write(&cache, b"not a directory").unwrap();

        let mut settings = AppSettings::default();
        settings.config_version = 2;
        settings.cache_dir_resolved = kimun_core::system::SystemPath::try_absolute(&cache).unwrap();
        settings.history_dir_resolved =
            kimun_core::system::SystemPath::try_absolute(dir.path()).unwrap();

        let mut wc = WorkspaceConfig::new_empty();
        wc.workspaces.insert(
            "notes".to_string(),
            WorkspaceEntry {
                path: workspace.path().to_path_buf(),
                last_paths: Vec::new(),
                created: chrono::Utc::now(),
                quick_note_path: None,
                inbox_path: None,
                resolved_path: None,
                file_key: None,
            },
        );
        wc.global.current_workspace = "notes".to_string();
        settings.workspace_config = Some(wc);

        ConfigMigration::run(&mut settings)
            .expect("a stuck index must not fail the migration, and so the launch");

        assert_eq!(settings.config_version, CURRENT_CONFIG_VERSION);
        // The failure really happened — the index is still where it was, and
        // the workspace will simply reindex.
        assert_eq!(
            std::fs::read(&legacy).unwrap(),
            b"an index",
            "the index must be left in place, not lost"
        );
    }

    /// have its history extracted. `last_paths` is cleared for *every* entry
    /// once the loop finishes, so skipping the rest of an entry's migration
    /// does not postpone that work — it destroys it.
    #[test]
    fn v2_migration_keeps_the_history_of_a_workspace_with_an_unusable_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = kimun_core::system::SystemPath::try_absolute(dir.path()).unwrap();

        let mut settings = AppSettings::default();
        settings.config_version = 2;
        settings.cache_dir_resolved = base.clone();
        settings.history_dir_resolved = base.join("history");

        let mut wc = WorkspaceConfig::new_empty();
        wc.workspaces.insert(
            "notes".to_string(),
            WorkspaceEntry {
                // Relative and unresolved — no index on this machine can be
                // named from it, which is the branch under test.
                path: PathBuf::from("relative/notes"),
                last_paths: vec!["a.md".to_string(), "b.md".to_string()],
                created: chrono::Utc::now(),
                quick_note_path: None,
                inbox_path: None,
                resolved_path: None,
                file_key: None,
            },
        );
        wc.global.current_workspace = "notes".to_string();
        settings.workspace_config = Some(wc);

        ConfigMigration::run(&mut settings).unwrap();

        let body = std::fs::read_to_string(dir.path().join("history").join("notes.txt"))
            .expect("the history file must be written even though the index was skipped");
        assert_eq!(body.lines().collect::<Vec<_>>(), ["a.md", "b.md"]);

        let wc = settings.workspace_config.as_ref().unwrap();
        assert!(
            wc.workspaces["notes"].last_paths.is_empty(),
            "the in-memory copy is dropped once it lives in the history file"
        );
    }

    #[test]
    fn orphaned_workspace_dir_skipped_if_same_path_exists() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut settings = settings_with_workspace_dir(dir.path().to_str().unwrap());

        // Pre-existing config already has a workspace at the same path.
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("existing".to_string(), dir.path().to_path_buf())
            .unwrap();
        wc.global.current_workspace = "existing".to_string();
        settings.workspace_config = Some(wc);

        ConfigMigration::run(&mut settings).unwrap();

        let wc = settings.workspace_config.as_ref().unwrap();
        assert_eq!(wc.workspaces.len(), 1); // not duplicated
        assert!(wc.workspaces.contains_key("existing"));
    }

    #[test]
    fn unique_name_avoids_collision() {
        let mut wc = WorkspaceConfig::new_empty();
        let dir = tempfile::TempDir::new().unwrap();
        wc.add_workspace("default".to_string(), dir.path().to_path_buf())
            .unwrap();

        let name = ConfigMigration::unique_workspace_name(&wc, "default");
        assert_eq!(name, "default-2");
    }

    #[test]
    fn no_migration_when_no_legacy_fields() {
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
