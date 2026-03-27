// tui/src/cli/commands/note_ops.rs
//
// CLI commands for note create, append, and journal operations.

use clap::Subcommand;
use color_eyre::eyre::Result;
use kimun_core::{NoteVault, error::VaultError};
use kimun_core::nfs::VaultPath;

#[derive(Subcommand, Debug)]
pub enum NoteSubcommand {
    /// Create a new note (fails if the note already exists)
    Create {
        /// Note path, relative to quick_note_path or absolute from vault root
        path: String,
        /// Note content (reads from stdin if omitted and stdin is not a TTY)
        content: Option<String>,
    },
    /// Append text to a note (creates the note if it does not exist)
    Append {
        /// Note path, relative to quick_note_path or absolute from vault root
        path: String,
        /// Text to append (reads from stdin if omitted and stdin is not a TTY)
        content: Option<String>,
    },
    /// Append text to today's journal entry (creates it if it does not exist)
    Journal {
        /// Text to append (reads from stdin if omitted and stdin is not a TTY)
        content: Option<String>,
    },
}

pub async fn run(
    subcommand: NoteSubcommand,
    vault: &NoteVault,
    quick_note_path: &str,
) -> Result<()> {
    match subcommand {
        NoteSubcommand::Create { path, content } => {
            run_create(vault, &path, content, quick_note_path).await
        }
        NoteSubcommand::Append { path, content } => {
            run_append(vault, &path, content, quick_note_path).await
        }
        NoteSubcommand::Journal { content } => {
            run_journal(vault, content).await
        }
    }
}

async fn run_create(
    vault: &NoteVault,
    path_input: &str,
    content: Option<String>,
    quick_note_path: &str,
) -> Result<()> {
    use crate::cli::helpers::resolve_note_path;

    let vault_path = resolve_note_path(path_input, quick_note_path)?;
    let text = resolve_content(content);

    vault.create_note(&vault_path, &text).await.map_err(|e| {
        match &e {
            VaultError::NoteExists { path } => {
                color_eyre::eyre::eyre!("Note already exists: {}", path)
            }
            _ => color_eyre::eyre::eyre!("{}", e),
        }
    })?;

    println!("Note saved: {}", vault_path);
    Ok(())
}

async fn run_append(
    vault: &NoteVault,
    path_input: &str,
    content: Option<String>,
    quick_note_path: &str,
) -> Result<()> {
    use crate::cli::helpers::resolve_note_path;
    use kimun_core::error::FSError;

    let vault_path = resolve_note_path(path_input, quick_note_path)?;
    let text = resolve_content(content);

    if text.is_empty() {
        return Ok(());
    }

    match vault.get_note_text(&vault_path).await {
        Ok(existing) => {
            let combined = format!("{}\n{}", existing, text);
            vault.save_note(&vault_path, &combined).await
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        }
        Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
            match vault.create_note(&vault_path, &text).await {
                Ok(_) => {}
                Err(VaultError::NoteExists { .. }) => {
                    // Race: note created between our get and create — re-read and save
                    let existing = vault.get_note_text(&vault_path).await
                        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
                    let combined = format!("{}\n{}", existing, text);
                    vault.save_note(&vault_path, &combined).await
                        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
                }
                Err(e) => return Err(color_eyre::eyre::eyre!("{}", e)),
            }
        }
        Err(e) => return Err(color_eyre::eyre::eyre!("{}", e)),
    }

    println!("Note saved: {}", vault_path);
    Ok(())
}

async fn run_journal(vault: &NoteVault, content: Option<String>) -> Result<()> {
    let text = resolve_content(content);

    if text.is_empty() {
        return Ok(());
    }

    let (details, existing) = vault.journal_entry().await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    let combined = format!("{}\n{}", existing, text);
    vault.save_note(&details.path, &combined).await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    println!("Note saved: {}", details.path);
    Ok(())
}

/// Returns content from the Option, or reads from stdin if not a TTY.
/// Returns an empty string if content is None and stdin is a TTY.
fn resolve_content(content: Option<String>) -> String {
    use std::io::IsTerminal;
    match content {
        Some(c) => c,
        None => {
            if std::io::stdin().is_terminal() {
                String::new()
            } else {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf).unwrap_or(0);
                buf.trim_end_matches('\n').to_string()
            }
        }
    }
}
