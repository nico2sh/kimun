// tui/src/cli/helpers.rs
//
// Common helper functions for CLI operations to reduce code duplication.

use std::path::PathBuf;
use color_eyre::eyre::Result;
use kimun_core::NoteVault;
use kimun_core::nfs::{VaultPath, PATH_SEPARATOR};
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

/// Returns the configured quick_note_path for the active workspace.
/// Falls back to VaultPath::root() for Phase 1 workspaces (no WorkspaceEntry) or if not configured.
pub fn resolve_quick_note_path(settings: &AppSettings) -> String {
    let root = kimun_core::nfs::VaultPath::root().to_string();
    // Phase 1 legacy: workspace_dir only, no WorkspaceEntry
    if settings.workspace_dir.is_some() {
        return root;
    }
    // Phase 2: workspace_config
    if let Some(ref ws_config) = settings.workspace_config {
        if let Some(entry) = ws_config.get_current_workspace() {
            return entry.effective_quick_note_path();
        }
    }
    root
}

/// Resolve a user-provided note path string into a VaultPath.
///
/// Rules:
/// - Empty or whitespace-only input → error
/// - Starts with PATH_SEPARATOR → absolute from vault root (quick_note_path ignored)
/// - Otherwise → relative, joined with quick_note_path using PATH_SEPARATOR
/// - VaultPath::note_path_from normalizes path and ensures .md extension
pub fn resolve_note_path(input: &str, quick_note_path: &str) -> Result<VaultPath> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(color_eyre::eyre::eyre!("Note path cannot be empty or whitespace-only"));
    }
    if trimmed.len() == 1 && trimmed.starts_with(PATH_SEPARATOR) {
        return Err(color_eyre::eyre::eyre!("Note path cannot be the root separator alone"));
    }
    let raw = if trimmed.starts_with(PATH_SEPARATOR) {
        trimmed.to_string()
    } else {
        let base = if quick_note_path.trim().is_empty() {
            VaultPath::root().to_string()
        } else {
            quick_note_path.trim_end_matches(PATH_SEPARATOR).to_string()
        };
        format!("{}{}{}", base, PATH_SEPARATOR, trimmed)
    };
    Ok(VaultPath::note_path_from(&raw))
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
    vault.validate_and_init().await?;

    Ok((vault, workspace_name))
}