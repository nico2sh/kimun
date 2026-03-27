// tui/src/cli/mod.rs
pub mod commands;
pub mod output;
pub mod json_output;
pub mod metadata_extractor;
pub mod helpers;

use clap::Subcommand;
use color_eyre::eyre::Result;
use output::OutputFormat;
use commands::workspace::WorkspaceSubcommand;
use commands::note_ops::NoteSubcommand;
use helpers::{create_and_init_vault, load_settings, load_and_resolve_workspace, resolve_quick_note_path};

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
    /// Note operations (create, append, journal)
    Note {
        #[command(subcommand)]
        subcommand: NoteSubcommand,
    },
}

pub async fn run_cli(command: CliCommand, config_path: Option<std::path::PathBuf>) -> Result<()> {
    // Workspace commands need mutable settings
    if let CliCommand::Workspace { subcommand } = command {
        let mut settings = load_settings(config_path)?;
        return commands::workspace::run(subcommand, &mut settings).await;
    }

    // Note commands need settings for quick_note_path
    if let CliCommand::Note { subcommand } = command {
        let (settings, workspace_path, _workspace_name) = load_and_resolve_workspace(config_path)?;
        let quick_note_path = resolve_quick_note_path(&settings);
        let vault = kimun_core::NoteVault::new(&workspace_path).await?;
        vault.init_and_validate().await?;
        return commands::note_ops::run(subcommand, &vault, &quick_note_path).await;
    }

    // Search and Notes commands
    let (vault, workspace_name) = create_and_init_vault(config_path).await?;

    match command {
        CliCommand::Search { query, format } => {
            commands::search::run(&vault, &query, format, &workspace_name, false).await
        }
        CliCommand::Notes { path, format } => {
            commands::notes::run(&vault, path.as_deref(), format, &workspace_name, false).await
        }
        CliCommand::Workspace { .. } => unreachable!("handled above"),
        CliCommand::Note { .. } => unreachable!("handled above"),
    }
}
