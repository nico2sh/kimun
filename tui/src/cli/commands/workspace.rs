// tui/src/cli/commands/workspace.rs
//
// Workspace management CLI commands: init, list, use, rename, remove, reindex.

use std::path::PathBuf;

use clap::Subcommand;
use color_eyre::eyre::{Result, eyre};
use kimun_core::error::VaultError;
use kimun_core::{NoteVault, SystemPath, VaultConfig};

use kimun_core::system;

use crate::settings::{
    AppSettings, config_migration::CURRENT_CONFIG_VERSION, history::HistoryFile,
    workspace_config::WorkspaceConfig,
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

pub async fn run(subcommand: WorkspaceSubcommand, settings: &mut AppSettings) -> Result<()> {
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

async fn run_init(settings: &mut AppSettings, name: Option<String>, path: PathBuf) -> Result<()> {
    // Ensure workspace_config exists
    if settings.workspace_config.is_none() {
        settings.workspace_config = Some(WorkspaceConfig::new_empty());
    }

    let ws_config = settings
        .workspace_config
        .as_ref()
        .expect("workspace_config must exist after init");

    // Workspace name is lowercased here because it backs case-insensitive
    // cache and history filenames; same lowering must apply to the DB path
    // computed below and the eventual add_workspace key.
    let workspace_name = match name {
        Some(n) => n.to_lowercase(),
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

    // Validate before anything derived from the name touches the filesystem.
    // `add_workspace` validates too, but only after the cache file below has
    // already been created at `<cache_dir>/<name>.kimuncache` — a name with
    // `..` or a separator in it puts that file (plus its -wal/-shm sidecars
    // and any parent directories) outside the cache directory entirely, and
    // the command then aborts having already written them.
    kimun_core::nfs::filename::validate_filename(&workspace_name).map_err(|e| eyre!("{}", e))?;

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
    let created = !path.exists();
    let canonical_path = system::create_dir(&path).map_err(|e| {
        eyre!(
            "Failed to create workspace directory {}: {}",
            path.display(),
            e
        )
    })?;
    if created {
        println!("Created directory: {}", path.display());
    }

    // The entry goes in BEFORE the index is named. `add_workspace` is what
    // mints the key the files are called after, so asking for the index first
    // would create it under the workspace's *name* and then look for it under
    // the key — a database written once and never found again. Nothing is
    // persisted until `save_to_disk` below, so bailing out after this point
    // leaves the config on disk untouched.
    let ws_config_mut = settings
        .workspace_config
        .as_mut()
        .expect("workspace_config must exist after init");
    ws_config_mut
        .add_workspace(
            workspace_name.clone(),
            canonical_path.clone().into_path_buf(),
        )
        .map_err(|e| eyre!("{}", e))?;

    println!("Initializing workspace database...");
    let cache_path = settings.index_for(&workspace_name);
    let vault = NoteVault::new(VaultConfig::new(canonical_path.clone()).with_index(cache_path))
        .await
        .map_err(|e| eyre!("Failed to create vault at {}: {}", canonical_path, e))?;
    let init_result = vault.validate_and_init().await;
    // This vault existed only to create the database; release its handle on the
    // cache file rather than leaving that to pool drop, which merely schedules
    // the close. A later `workspace remove` in the same process (the TUI, or a
    // test driving several commands) has to delete that file, and Windows will
    // not delete a file that is still open. Closed before the `?` too: a failed
    // init leaves the cache file on disk, so bailing out with it still open is
    // the same locked file with nobody left holding a handle to close it.
    vault.close().await;
    init_result.map_err(|e| eyre!("Failed to initialize vault database: {}", e))?;

    settings.config_version = CURRENT_CONFIG_VERSION;
    settings.save_to_disk()?;

    println!(
        "Workspace '{}' initialized at {}",
        workspace_name, canonical_path
    );

    let ws_config = settings
        .workspace_config
        .as_ref()
        .expect("workspace_config must exist after init");
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
                println!(
                    "No workspaces configured. Run 'kimun workspace init <path>' to create one."
                );
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

    let entry = ws_config.get_workspace(&name).ok_or_else(|| {
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
    if !entry.effective_path().exists() {
        return Err(eyre!(
            "Workspace '{}' path no longer exists: {}. \
             Update the path or remove this workspace.",
            name,
            entry.effective_path().display()
        ));
    }

    settings
        .workspace_config
        .as_mut()
        .expect("workspace_config must exist")
        .global
        .current_workspace = name.clone();
    settings.save_to_disk()?;

    println!("Switched to workspace '{}'.", name);
    Ok(())
}

fn run_rename(settings: &mut AppSettings, old_name: String, new_name: String) -> Result<()> {
    let new_name = new_name.to_lowercase();
    kimun_core::nfs::filename::validate_filename(&new_name).map_err(|e| eyre!("{}", e))?;

    let ws_config = settings
        .workspace_config
        .as_ref()
        .ok_or_else(|| eyre!("No workspaces configured."))?;

    if !ws_config.workspaces.contains_key(&old_name) {
        return Err(eyre!("Workspace '{}' not found.", old_name));
    }

    if ws_config.workspaces.contains_key(&new_name) {
        return Err(eyre!(
            "Workspace '{}' already exists. Choose a different name.",
            new_name
        ));
    }

    // Nothing on disk moves. The index and history keep the name they were
    // created under, which `rename_workspace` pins into the entry — see
    // `WorkspaceEntry::file_key`. Renaming used to move an open SQLite
    // database, and Windows will not move a file any handle still holds, so
    // this is the one rename that cannot fail halfway.
    let ws_config_mut = settings
        .workspace_config
        .as_mut()
        .expect("workspace_config must exist after init");
    let renamed = ws_config_mut.rename_workspace(&old_name, new_name.clone());
    debug_assert!(renamed, "entry existence was checked above");

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

    if ws_config.global.current_workspace == name {
        return Err(eyre!(
            "Cannot remove the current workspace '{}'. \
             Switch to a different workspace first with: kimun workspace use <name>",
            name
        ));
    }

    let index = settings.index_for(&name);
    let history = settings.history_for(&name);

    settings
        .workspace_config
        .as_mut()
        .expect("workspace_config must exist")
        .workspaces
        .remove(&name);

    settings.save_to_disk()?;

    let leftovers = delete_artifacts(&index, &history);

    println!("Workspace '{}' removed.", name);
    if let Some(report) = leftover_report(&leftovers) {
        // stderr, not stdout: the removal did happen, and a script reading
        // stdout should not have to parse this out of the success line.
        eprintln!("{report}");
    }
    Ok(())
}

/// Deletes a removed workspace's artifacts, returning the ones that would not
/// go, each with the reason.
///
/// Best-effort by design: the workspace is already out of the config, so a
/// stuck file is a leftover rather than a reason to fail the command. Returned
/// rather than only logged because `tracing::warn!` in a CLI with no
/// subscriber attached goes nowhere — on Windows, where a handle still open on
/// the index blocks the delete, `remove` printed plain success over an index it
/// had not deleted.
fn delete_artifacts(index: &kimun_core::IndexFile, history: &HistoryFile) -> Vec<String> {
    let mut leftovers = Vec::new();
    // Every file the index is made of, not just the first one that failed: a
    // held handle is normally on a sidecar, and naming only that one would send
    // the user to delete one file out of three.
    let stuck = index.remove();
    if stuck.is_empty() {
        tracing::info!("removed index {}", index);
    }
    for (path, e) in stuck {
        tracing::warn!("failed to remove {}: {}", path, e);
        leftovers.push(format!("  {path}\n    {e}"));
    }
    if let Err(e) = history.remove() {
        tracing::warn!("failed to remove history {}: {}", history, e);
        leftovers.push(format!("  {history}\n    {e}"));
    } else {
        tracing::info!("removed history {}", history);
    }
    leftovers
}

/// What to tell the user about files that would not delete, or `None` when
/// everything went.
fn leftover_report(leftovers: &[String]) -> Option<String> {
    (!leftovers.is_empty()).then(|| {
        format!(
            "\nWarning: these files could not be deleted and are safe to \
             delete by hand:\n{}",
            leftovers.join("\n")
        )
    })
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

    if !entry.effective_path().exists() {
        return Err(eyre!(
            "Workspace '{}' path no longer exists: {}",
            workspace_name,
            entry.effective_path().display()
        ));
    }

    println!("Reindexing workspace '{}'...", workspace_name);

    let cache_path = settings.index_for(&workspace_name);
    let workspace_path = SystemPath::try_absolute(entry.effective_path())
        .map_err(|e| eyre!("Workspace '{}' has an unusable path: {}", workspace_name, e))?;
    let vault = NoteVault::new(VaultConfig::new(workspace_path.clone()).with_index(cache_path))
        .await
        .map_err(|e| eyre!("Failed to open vault at {}: {}", workspace_path, e))?;

    let index_result = vault.recreate_index().await;
    // Reindexing is done with the vault; close it (on the error paths too)
    // instead of letting the process hold the cache file open, which blocks a
    // later rename or remove of that workspace on Windows.
    vault.close().await;

    let report = match index_result {
        Ok(r) => r,
        Err(VaultError::CaseConflict { conflicts }) => {
            eprintln!(
                "Error: vault '{}' has case-sensitivity conflicts:",
                workspace_name
            );
            for c in &conflicts {
                eprintln!("  {}", c);
            }
            eprintln!(
                "\nResolve the conflicts on disk, then run `kimun workspace use {}` to re-select the vault.",
                workspace_name
            );
            return Err(eyre!(
                "Vault '{}' has case-sensitivity conflicts",
                workspace_name
            ));
        }
        Err(e) => {
            return Err(eyre!(
                "Failed to reindex workspace '{}': {}",
                workspace_name,
                e
            ));
        }
    };

    let _ = report; // IndexReport only contains timing info
    println!("Reindex complete for workspace '{}'.", workspace_name);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::IndexFile;
    use kimun_core::system::SystemPath;

    fn sys(path: &std::path::Path) -> SystemPath {
        SystemPath::try_absolute(path).unwrap()
    }

    /// The whole point of returning the leftovers: a delete that fails has to
    /// reach the user. It only ever went to `tracing::warn!` before, which in a
    /// CLI with no subscriber attached is the same as silence — and the command
    /// printed success over an index it had not deleted.
    #[test]
    fn a_file_that_will_not_delete_is_reported() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = IndexFile::in_dir(&sys(dir.path()), "stuck");
        let history = HistoryFile::in_dir(&sys(dir.path()), "stuck");
        // A non-empty directory where the index file should be: `remove_file`
        // cannot delete it. A stand-in for the Windows lock, which cannot be
        // provoked on demand — the reporting is what is under test.
        std::fs::create_dir(index.path().as_path()).unwrap();
        std::fs::write(index.path().as_path().join("occupied"), b"x").unwrap();

        let leftovers = delete_artifacts(&index, &history);

        assert_eq!(leftovers.len(), 1, "got {leftovers:?}");
        assert!(leftovers[0].contains("stuck.kimuncache"), "{leftovers:?}");
        let report = leftover_report(&leftovers).expect("a leftover must be reported");
        assert!(report.contains("could not be deleted"), "{report}");
        assert!(report.contains("stuck.kimuncache"), "{report}");
    }

    /// The ordinary case stays quiet: nothing left behind, nothing printed.
    #[test]
    fn a_clean_delete_reports_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let index = IndexFile::in_dir(&sys(dir.path()), "gone");
        let history = HistoryFile::in_dir(&sys(dir.path()), "gone");
        std::fs::write(index.path().as_path(), b"index").unwrap();
        std::fs::write(history.path().as_path(), b"a.md\n").unwrap();

        let leftovers = delete_artifacts(&index, &history);

        assert!(leftovers.is_empty(), "got {leftovers:?}");
        assert!(leftover_report(&leftovers).is_none());
        assert!(!index.exists());
    }
}
