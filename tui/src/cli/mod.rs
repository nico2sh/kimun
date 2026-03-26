// tui/src/cli/mod.rs
pub mod commands;
pub mod output;

use clap::Subcommand;
use color_eyre::eyre::Result;
use crate::settings::AppSettings;
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
}

pub async fn run_cli(command: CliCommand) -> Result<()> {
    // Load settings to get workspace
    let settings = AppSettings::load_from_disk()?;

    let workspace = match settings.workspace_dir {
        Some(dir) => dir,
        None => {
            eprintln!("Error: No workspace configured. Run 'kimun' to set up a workspace.");
            std::process::exit(1);
        }
    };

    // Create vault
    let vault = NoteVault::new(&workspace).await?;

    match command {
        CliCommand::Search { query, format } => {
            commands::search::run(&vault, &query, format).await
        }
        CliCommand::Notes { path, format } => {
            commands::notes::run(&vault, path.as_deref(), format).await
        }
    }
}
