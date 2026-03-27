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
use helpers::{create_and_init_vault, load_settings};

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
        let mut settings = load_settings(config_path)?;
        return commands::workspace::run(subcommand, &mut settings).await;
    }

    // Create and initialize vault using helper function
    let (vault, workspace_name) = create_and_init_vault(config_path).await?;

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
