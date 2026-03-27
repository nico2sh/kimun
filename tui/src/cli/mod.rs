// tui/src/cli/mod.rs
pub mod commands;
pub mod output;
pub mod json_output;
pub mod metadata_extractor;

use clap::Subcommand;
use color_eyre::eyre::Result;
use crate::settings::AppSettings;
use kimun_core::NoteVault;
use output::OutputFormat;
use commands::workspace::WorkspaceSubcommand;

#[derive(Subcommand)]
pub enum CliCommand {
    /// Search notes by query
    Search {
        query: String,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// List all notes
    Notes {
        #[arg(long, help = "Filter notes by path prefix")]
        path: Option<String>,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Manage workspaces
    Workspace {
        #[command(subcommand)]
        subcommand: WorkspaceSubcommand,
    },
}

pub async fn run_cli(command: CliCommand, config_path: Option<std::path::PathBuf>) -> Result<()> {
    // For workspace commands we need mutable settings and handle them separately
    if let CliCommand::Workspace { subcommand } = command {
        let mut settings = match config_path {
            Some(path) => AppSettings::load_from_file(path)?,
            None => AppSettings::load_from_disk()?,
        };
        return commands::workspace::run(subcommand, &mut settings).await;
    }

    // Load settings to get workspace for search/notes commands
    let settings = match config_path {
        Some(path) => AppSettings::load_from_file(path)?,
        None => AppSettings::load_from_disk()?,
    };

    // Determine workspace path and name
    let (workspace, workspace_name) = if let Some(dir) = settings.workspace_dir {
        (dir, "default".to_string())
    } else if let Some(ref ws_config) = settings.workspace_config {
        if let Some(entry) = ws_config.get_current_workspace() {
            let name = ws_config.global.current_workspace.clone();
            (entry.path.clone(), name)
        } else {
            return Err(color_eyre::eyre::eyre!("No workspace configured. Run 'kimun' to set up a workspace."));
        }
    } else {
        return Err(color_eyre::eyre::eyre!("No workspace configured. Run 'kimun' to set up a workspace."));
    };

    // Create vault
    let vault = NoteVault::new(&workspace).await?;

    // Initialize and validate the vault database
    vault.init_and_validate().await?;

    match command {
        CliCommand::Search { query, format } => {
            commands::search::run(&vault, &query, format, &workspace_name, false).await
        }
        CliCommand::Notes { path, format } => {
            commands::notes::run(&vault, path.as_deref(), format, &workspace_name, false).await
        }
        CliCommand::Workspace { .. } => unreachable!("handled above"),
    }
}
