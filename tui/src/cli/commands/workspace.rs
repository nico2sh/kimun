// tui/src/cli/commands/workspace.rs
//
// Workspace management CLI commands: init, list, use, rename, remove, reindex.

use std::path::PathBuf;

use clap::Subcommand;
use color_eyre::eyre::{eyre, Result};
use kimun_core::NoteVault;
use kimun_core::error::VaultError;

use crate::settings::{
    workspace_config::WorkspaceConfig,
    AppSettings,
};

#[derive(Subcommand, Debug)]
pub enum WorkspaceSubcommand {
    /// Initialize a new workspace
    Init {
        /// Name for the workspace (defaults to "default" for first workspace)
        #[arg(long)]
        name: Option<String>,
        /// Path to the workspace directory
        path: PathBuf,
    },
    /// List all configured workspaces
    List,
    /// Switch to a different workspace
    Use {
        /// Name of the workspace to switch to
        name: String,
    },
    /// Rename a workspace
    Rename {
        /// Current workspace name
        old_name: String,
        /// New workspace name
        new_name: String,
    },
    /// Remove a workspace from the configuration
    Remove {
        /// Name of the workspace to remove
        name: String,
    },
    /// Reindex a workspace
    Reindex {
        /// Workspace name (defaults to current workspace)
        #[arg(long)]
        name: Option<String>,
    },
}

pub async fn run(
    subcommand: WorkspaceSubcommand,
    settings: &mut AppSettings,
) -> Result<()> {
    match subcommand {
        WorkspaceSubcommand::Init { name, path } => run_init(settings, name, path).await,
        WorkspaceSubcommand::List => run_list(settings),
        WorkspaceSubcommand::Use { name } => run_use(settings, name),
        WorkspaceSubcommand::Rename { old_name, new_name } => {
            run_rename(settings, old_name, new_name)
        }
        WorkspaceSubcommand::Remove { name } => run_remove(settings, name),
        WorkspaceSubcommand::Reindex { name } => run_reindex(settings, name).await,
    }
}

async fn run_init(
    settings: &mut AppSettings,
    name: Option<String>,
    path: PathBuf,
) -> Result<()> {
    // Ensure workspace_config exists
    if settings.workspace_config.is_none() {
        settings.workspace_config = Some(WorkspaceConfig::new_empty());
    }

    let ws_config = settings.workspace_config.as_ref().unwrap();

    // Determine workspace name
    let workspace_name = match name {
        Some(n) => n,
        None => {
            if ws_config.workspaces.is_empty() {
                "default".to_string()
            } else {
                return Err(eyre!(
                    "A workspace name is required when other workspaces already exist. \
                     Use: kimun workspace init --name <name> <path>"
                ));
            }
        }
    };

    // Check for duplicates
    if ws_config.workspaces.contains_key(&workspace_name) {
        let existing_path = &ws_config.workspaces[&workspace_name].path;
        return Err(eyre!(
            "Workspace '{}' already exists at {}. \
             Use a different name or remove the existing workspace first.",
            workspace_name,
            existing_path.display()
        ));
    }

    // Validate/create the target path
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| {
            eyre!(
                "Failed to create workspace directory {}: {}",
                path.display(),
                e
            )
        })?;
        println!("Created directory: {}", path.display());
    }

    let canonical_path = path.canonicalize().map_err(|e| {
        eyre!(
            "Failed to resolve workspace path {}: {}",
            path.display(),
            e
        )
    })?;

    // Initialize NoteVault database (creates kimun.sqlite)
    println!("Initializing workspace database...");
    let vault = NoteVault::new(&canonical_path).await.map_err(|e| {
        eyre!("Failed to create vault at {}: {}", canonical_path.display(), e)
    })?;
    vault.validate_and_init().await.map_err(|e| {
        eyre!("Failed to initialize vault database: {}", e)
    })?;

    // Add workspace to config and save
    let ws_config_mut = settings.workspace_config.as_mut().unwrap();
    ws_config_mut
        .add_workspace(workspace_name.clone(), canonical_path.clone())
        .map_err(|e| eyre!("{}", e))?;

    settings.config_version = 2;
    settings.save_to_disk()?;

    println!(
        "Workspace '{}' initialized at {}",
        workspace_name,
        canonical_path.display()
    );

    let ws_config = settings.workspace_config.as_ref().unwrap();
    if ws_config.global.current_workspace == workspace_name {
        println!("Set as current workspace.");
    }

    Ok(())
}

fn run_list(settings: &AppSettings) -> Result<()> {
    match &settings.workspace_config {
        None => {
            println!("No workspaces configured. Run 'kimun workspace init <path>' to create one.");
        }
        Some(ws_config) => {
            if ws_config.workspaces.is_empty() {
                println!("No workspaces configured. Run 'kimun workspace init <path>' to create one.");
            } else {
                println!("Configured workspaces:");
                let mut names: Vec<&String> = ws_config.workspaces.keys().collect();
                names.sort();
                for name in names {
                    let entry = &ws_config.workspaces[name];
                    let marker = if name == &ws_config.global.current_workspace {
                        "* "
                    } else {
                        "  "
                    };
                    println!("{}{}  ({})", marker, name, entry.path.display());
                }
            }
        }
    }
    Ok(())
}

fn run_use(settings: &mut AppSettings, name: String) -> Result<()> {
    let ws_config = settings
        .workspace_config
        .as_ref()
        .ok_or_else(|| eyre!("No workspaces configured."))?;

    let entry = ws_config
        .get_workspace(&name)
        .ok_or_else(|| {
            let available: Vec<&String> = ws_config.workspaces.keys().collect();
            eyre!(
                "Workspace '{}' not found. Available workspaces: {}",
                name,
                available
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    // Validate workspace path still exists
    if !entry.path.exists() {
        return Err(eyre!(
            "Workspace '{}' path no longer exists: {}. \
             Update the path or remove this workspace.",
            name,
            entry.path.display()
        ));
    }

    settings.workspace_config.as_mut().unwrap().global.current_workspace = name.clone();
    settings.save_to_disk()?;

    println!("Switched to workspace '{}'.", name);
    Ok(())
}

fn run_rename(
    settings: &mut AppSettings,
    old_name: String,
    new_name: String,
) -> Result<()> {
    let ws_config = settings
        .workspace_config
        .as_ref()
        .ok_or_else(|| eyre!("No workspaces configured."))?;

    if !ws_config.workspaces.contains_key(&old_name) {
        return Err(eyre!(
            "Workspace '{}' not found.",
            old_name
        ));
    }

    if ws_config.workspaces.contains_key(&new_name) {
        return Err(eyre!(
            "Workspace '{}' already exists. Choose a different name.",
            new_name
        ));
    }

    let ws_config_mut = settings.workspace_config.as_mut().unwrap();

    // Move entry to new key
    let entry = ws_config_mut
        .workspaces
        .remove(&old_name)
        .expect("entry must exist (checked above)");
    ws_config_mut.workspaces.insert(new_name.clone(), entry);

    // Update current_workspace reference if needed
    if ws_config_mut.global.current_workspace == old_name {
        ws_config_mut.global.current_workspace = new_name.clone();
    }

    settings.save_to_disk()?;

    println!("Workspace '{}' renamed to '{}'.", old_name, new_name);
    Ok(())
}

fn run_remove(settings: &mut AppSettings, name: String) -> Result<()> {
    let ws_config = settings
        .workspace_config
        .as_ref()
        .ok_or_else(|| eyre!("No workspaces configured."))?;

    if !ws_config.workspaces.contains_key(&name) {
        return Err(eyre!("Workspace '{}' not found.", name));
    }

    // Prevent removing the current workspace
    if ws_config.global.current_workspace == name {
        return Err(eyre!(
            "Cannot remove the current workspace '{}'. \
             Switch to a different workspace first with: kimun workspace use <name>",
            name
        ));
    }

    settings
        .workspace_config
        .as_mut()
        .unwrap()
        .workspaces
        .remove(&name);

    settings.save_to_disk()?;

    println!("Workspace '{}' removed.", name);
    Ok(())
}

async fn run_reindex(settings: &AppSettings, name: Option<String>) -> Result<()> {
    let ws_config = settings
        .workspace_config
        .as_ref()
        .ok_or_else(|| eyre!("No workspaces configured."))?;

    let workspace_name = match name {
        Some(n) => n,
        None => ws_config.global.current_workspace.clone(),
    };

    if workspace_name.is_empty() {
        return Err(eyre!("No current workspace set. Specify a workspace name."));
    }

    let entry = ws_config
        .get_workspace(&workspace_name)
        .ok_or_else(|| eyre!("Workspace '{}' not found.", workspace_name))?;

    if !entry.path.exists() {
        return Err(eyre!(
            "Workspace '{}' path no longer exists: {}",
            workspace_name,
            entry.path.display()
        ));
    }

    println!("Reindexing workspace '{}'...", workspace_name);

    let vault = NoteVault::new(&entry.path).await.map_err(|e| {
        eyre!("Failed to open vault at {}: {}", entry.path.display(), e)
    })?;

    let report = match vault.recreate_index().await {
        Ok(r) => r,
        Err(VaultError::CaseConflict { conflicts }) => {
            eprintln!("Error: vault '{}' has case-sensitivity conflicts:", workspace_name);
            for c in &conflicts {
                eprintln!("  {}", c);
            }
            eprintln!(
                "\nResolve the conflicts on disk, then run `kimun workspace use {}` to re-select the vault.",
                workspace_name
            );
            return Err(eyre!("Vault '{}' has case-sensitivity conflicts", workspace_name));
        }
        Err(e) => return Err(eyre!("Failed to reindex workspace '{}': {}", workspace_name, e)),
    };

    let _ = report; // IndexReport only contains timing info
    println!(
        "Reindex complete for workspace '{}'.",
        workspace_name
    );

    Ok(())
}
