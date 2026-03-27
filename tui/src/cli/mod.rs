// tui/src/cli/mod.rs
pub mod commands;
pub mod helpers;
pub mod json_output;
pub mod metadata_extractor;
pub mod output;

use clap::Subcommand;
use color_eyre::eyre::{Result, eyre};
use commands::note_ops::NoteSubcommand;
use commands::workspace::WorkspaceSubcommand;
use helpers::{
    create_and_init_vault, load_and_resolve_workspace, load_settings, resolve_quick_note_path,
};
use kimun_core::NoteVault;
use output::OutputFormat;

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
    match command {
        CliCommand::Workspace { subcommand } => {
            let mut settings = load_settings(config_path)?;
            commands::workspace::run(subcommand, &mut settings).await
        }
        CliCommand::Note { subcommand } => {
            let (settings, workspace_path, workspace_name) =
                load_and_resolve_workspace(config_path)?;
            let quick_note_path = resolve_quick_note_path(&settings);
            let vault = NoteVault::new(&workspace_path).await?;
            match vault.validate().await? {
                kimun_core::db::DBStatus::Ready => {
                    commands::note_ops::run(subcommand, &vault, &quick_note_path, &workspace_name)
                        .await
                }
                status => Err(eyre!("{}", status)),
            }
        }
        CliCommand::Search { query, format } => {
            let (vault, workspace_name) = create_and_init_vault(config_path).await?;
            commands::search::run(&vault, &query, format, &workspace_name, false).await
        }
        CliCommand::Notes { path, format } => {
            let (vault, workspace_name) = create_and_init_vault(config_path).await?;
            commands::notes::run(&vault, path.as_deref(), format, &workspace_name, false).await
        }
    }
}
