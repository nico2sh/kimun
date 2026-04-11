//! Config migration — upgrades settings from older versions to the current format.
//!
//! All migration logic lives here so there is a single place to manage
//! version transitions. `ConfigMigration::run` is called once during
//! `AppSettings::load_from_file` after deserialization.

use color_eyre::eyre;

use super::workspace_config::{WorkspaceConfig, WorkspaceEntry};
use super::AppSettings;

/// Current config version. Bump this when adding a new migration step.
pub const CURRENT_CONFIG_VERSION: u32 = 2;

/// Runs all necessary migrations on `settings`, mutating it in place.
/// Returns `true` if any migration was applied (caller should persist).
pub struct ConfigMigration;

impl ConfigMigration {
    /// Apply all pending migrations to bring `settings` up to
    /// `CURRENT_CONFIG_VERSION`. Returns `true` if any migration ran.
    pub fn run(settings: &mut AppSettings) -> eyre::Result<bool> {
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

        // Future migrations go here, gated on config_version:
        // if settings.config_version < 3 { ... migrated = true; }

        if migrated {
            settings.config_version = CURRENT_CONFIG_VERSION;
        }

        Ok(migrated)
    }

    /// Migrate the legacy `workspace_dir` field into `workspace_config`.
    ///
    /// Two sub-cases:
    /// 1. No `workspace_config` exists — full migration: create one with a
    ///    "default" workspace from the legacy fields.
    /// 2. `workspace_config` already exists — the legacy field is orphaned
    ///    (e.g. from a partial earlier migration). Add it as "default" if no
    ///    workspace already points to the same path.
    fn migrate_workspace_dir(settings: &mut AppSettings) -> eyre::Result<()> {
        let Some(workspace_dir) = settings.workspace_dir.take() else {
            return Ok(());
        };

        if settings.workspace_config.is_none() {
            // Full Phase 1 → Phase 2 migration.
            if !workspace_dir.exists() {
                return Err(eyre::eyre!(
                    "Cannot migrate: workspace directory {} no longer exists",
                    workspace_dir.display()
                ));
            }
            tracing::info!("Migrating Phase 1 config to Phase 2 format");
            let last_paths: Vec<String> = settings
                .last_paths
                .iter()
                .map(|p| p.to_string())
                .collect();

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
                let last_paths: Vec<String> = settings
                    .last_paths
                    .iter()
                    .map(|p| p.to_string())
                    .collect();
                let entry = WorkspaceEntry {
                    path: workspace_dir,
                    last_paths,
                    created: chrono::Utc::now(),
                    quick_note_path: None,
                    inbox_path: None,
                    resolved_path: None,
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
        settings.workspace_config = Some(WorkspaceConfig::new_empty());

        let migrated = ConfigMigration::run(&mut settings).unwrap();
        assert!(!migrated);
    }
}
