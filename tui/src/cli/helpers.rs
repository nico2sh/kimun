// tui/src/cli/helpers.rs
//
// Common helper functions for CLI operations to reduce code duplication.

use std::path::PathBuf;
use color_eyre::eyre::Result;
use kimun_core::NoteVault;
use crate::settings::AppSettings;

/// Load settings from either a specific config file path or the default location.
pub fn load_settings(config_path: Option<PathBuf>) -> Result<AppSettings> {
    match config_path {
        Some(path) => AppSettings::load_from_file(path),
        None => AppSettings::load_from_disk(),
    }
}

/// Resolve workspace configuration from settings, returning the workspace path and name.
///
/// Returns an error if no workspace is configured.
pub fn resolve_workspace_config(settings: &AppSettings) -> Result<(PathBuf, String)> {
    // Check legacy workspace_dir first (Phase 1 compatibility)
    if let Some(dir) = &settings.workspace_dir {
        return Ok((dir.clone(), "default".to_string()));
    }

    // Check Phase 2 workspace configuration
    if let Some(ref ws_config) = settings.workspace_config {
        if let Some(entry) = ws_config.get_current_workspace() {
            let name = ws_config.global.current_workspace.clone();
            return Ok((entry.path.clone(), name));
        }
    }

    Err(color_eyre::eyre::eyre!("No workspace configured. Run 'kimun' to set up a workspace."))
}

/// Load settings and resolve workspace configuration in one operation.
///
/// This is a convenience function that combines loading settings and resolving
/// the workspace configuration, which is a common pattern in CLI commands.
pub fn load_and_resolve_workspace(config_path: Option<PathBuf>) -> Result<(AppSettings, PathBuf, String)> {
    let settings = load_settings(config_path)?;
    let (workspace_path, workspace_name) = resolve_workspace_config(&settings)?;
    Ok((settings, workspace_path, workspace_name))
}

/// Create and initialize a vault from workspace configuration.
///
/// This handles the common pattern of creating a NoteVault from workspace settings
/// and initializing/validating its database.
pub async fn create_and_init_vault(
    config_path: Option<PathBuf>
) -> Result<(NoteVault, String)> {
    let (_settings, workspace_path, workspace_name) = load_and_resolve_workspace(config_path)?;

    let vault = NoteVault::new(&workspace_path).await?;
    vault.init_and_validate().await?;

    Ok((vault, workspace_name))
}