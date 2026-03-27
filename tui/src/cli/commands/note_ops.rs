// tui/src/cli/commands/note_ops.rs
//
// CLI commands for note create, append, and journal operations.

use clap::Subcommand;
use color_eyre::eyre::Result;
use kimun_core::{NoteVault, error::VaultError};

const NOTE_SEPARATOR: &str = "================================================================================";

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
    /// Show note content and metadata (read one or more notes)
    Show {
        /// One or more note paths (relative to quick_note_path or absolute from vault root)
        paths: Vec<String>,
        #[arg(long, value_enum, default_value = "text")]
        format: crate::cli::output::OutputFormat,
    },
}

pub async fn run(
    subcommand: NoteSubcommand,
    vault: &NoteVault,
    quick_note_path: &str,
    workspace_name: &str,
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
        NoteSubcommand::Show { paths, format } => {
            run_show(vault, &paths, quick_note_path, format, workspace_name).await
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
    let text = resolve_content(content)?;

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
    let text = resolve_content(content)?;

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
    let text = resolve_content(content)?;

    if text.is_empty() {
        return Ok(());
    }

    // journal_entry() handles create-if-absent internally, so no TOCTOU retry needed here.
    let (details, existing) = vault.journal_entry().await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    let combined = format!("{}\n{}", existing, text);
    vault.save_note(&details.path, &combined).await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    println!("Note saved: {}", details.path);
    Ok(())
}

fn format_note_show_text(
    path: &kimun_core::nfs::VaultPath,
    content: &str,
    title: &str,
    tags: &[String],
    links: &[String],
    backlinks: &[String],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Path:      {}\n", path));
    if !title.is_empty() {
        out.push_str(&format!("Title:     {}\n", title));
    }
    if !tags.is_empty() {
        out.push_str(&format!("Tags:      {}\n", tags.join(" ")));
    }
    if !links.is_empty() {
        out.push_str(&format!("Links:     {}\n", links.join(", ")));
    }
    if !backlinks.is_empty() {
        out.push_str(&format!("Backlinks: {}\n", backlinks.join(", ")));
    }
    out.push_str("---\n");
    out.push_str(content);
    out
}

async fn run_show(
    vault: &NoteVault,
    path_inputs: &[String],
    quick_note_path: &str,
    format: crate::cli::output::OutputFormat,
    workspace_name: &str,
) -> Result<()> {
    use crate::cli::helpers::resolve_note_path;
    use crate::cli::metadata_extractor::{extract_tags, extract_links, extract_headers};
    use crate::cli::json_output::{JsonNoteEntry, JsonNoteMetadata, JsonOutput, JsonOutputMetadata};
    use crate::cli::output::OutputFormat;
    use kimun_core::nfs::NoteEntryData;
    use kimun_core::error::{VaultError, FSError};
    use chrono::Utc;
    use std::time::UNIX_EPOCH;

    let mut text_entries: Vec<String> = Vec::new();
    let mut json_entries: Vec<JsonNoteEntry> = Vec::new();
    let mut had_errors = false;

    for input in path_inputs {
        let vault_path = match resolve_note_path(input, quick_note_path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Error: {}", e);
                had_errors = true;
                continue;
            }
        };

        let note_details = match vault.load_note(&vault_path).await {
            Ok(nd) => nd,
            Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {
                eprintln!("Error: Note not found: {}", vault_path);
                had_errors = true;
                continue;
            }
            Err(e) => return Err(color_eyre::eyre::eyre!("{}", e)),
        };

        let content = note_details.raw_text.clone();
        let content_data = note_details.get_content_data();

        let meta = tokio::fs::metadata(vault.path_to_pathbuf(&vault_path))
            .await
            .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        let modified_secs = meta
            .modified()
            .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
            .unwrap_or(0);
        let entry_data = NoteEntryData {
            path: vault_path.clone(),
            size: meta.len(),
            modified_secs,
        };

        let backlink_results = vault
            .get_backlinks(&vault_path)
            .await
            .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        let backlink_paths: Vec<String> = backlink_results
            .iter()
            .map(|(e, _)| e.path.to_string())
            .collect();

        match format {
            OutputFormat::Text => {
                let tags = extract_tags(&content);
                let links = extract_links(&content);
                let text = format_note_show_text(
                    &vault_path,
                    &content,
                    &content_data.title,
                    &tags,
                    &links,
                    &backlink_paths,
                );
                text_entries.push(text);
            }
            OutputFormat::Json => {
                let tags = extract_tags(&content);
                let links = extract_links(&content);
                let headers = extract_headers(&content);
                let journal_date = vault
                    .journal_date(&vault_path)
                    .map(|d| d.format("%Y-%m-%d").to_string());
                let path_str = vault_path.to_string();
                let path_with_ext = if path_str.ends_with(".md") {
                    path_str.clone()
                } else {
                    format!("{}.md", path_str)
                };
                json_entries.push(JsonNoteEntry {
                    path: path_with_ext,
                    title: content_data.title.clone(),
                    content: content.clone(),
                    size: entry_data.size,
                    modified: entry_data.modified_secs,
                    created: entry_data.modified_secs,
                    hash: format!("{:x}", content_data.hash),
                    journal_date,
                    metadata: JsonNoteMetadata { tags, links, headers },
                    backlinks: if backlink_paths.is_empty() {
                        None
                    } else {
                        Some(backlink_paths)
                    },
                });
            }
        }
    }

    if text_entries.is_empty() && json_entries.is_empty() {
        return Err(color_eyre::eyre::eyre!(
            "No notes found — all specified paths were missing"
        ));
    }

    match format {
        OutputFormat::Text => {
            let sep = format!("\n{}\n\n", NOTE_SEPARATOR);
            print!("{}", text_entries.join(&sep));
        }
        OutputFormat::Json => {
            let output = JsonOutput {
                metadata: JsonOutputMetadata {
                    workspace: workspace_name.to_string(),
                    workspace_path: vault.workspace_path.to_string_lossy().to_string(),
                    total_results: json_entries.len(),
                    query: None,
                    is_listing: false,
                    generated_at: Utc::now().to_rfc3339(),
                },
                notes: json_entries,
            };
            print!(
                "{}",
                serde_json::to_string(&output)
                    .map_err(|e| color_eyre::eyre::eyre!("{}", e))?
            );
        }
    }

    if had_errors {
        return Err(color_eyre::eyre::eyre!("One or more notes could not be found"));
    }

    Ok(())
}

/// Returns content from the Option, or reads from stdin if not a TTY.
/// Returns an empty string if content is None and stdin is a TTY.
/// Propagates I/O errors from stdin.
fn resolve_content(content: Option<String>) -> color_eyre::eyre::Result<String> {
    use std::io::IsTerminal;
    match content {
        Some(c) => Ok(c),
        None => {
            if std::io::stdin().is_terminal() {
                Ok(String::new())
            } else {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)
                    .map_err(|e| color_eyre::eyre::eyre!("Failed to read stdin: {}", e))?;
                Ok(buf.trim_end_matches(|c| c == '\n' || c == '\r').to_string())
            }
        }
    }
}
