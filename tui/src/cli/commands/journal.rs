// tui/src/cli/commands/journal.rs
//
// Top-level `kimun journal` command: append to and show journal entries.

use clap::Subcommand;
use color_eyre::eyre::Result;
use kimun_core::{NoteVault, nfs::VaultPath};

use crate::cli::output::OutputFormat;

#[derive(clap::Args, Debug)]
pub struct JournalArgs {
    /// Date in YYYY-MM-DD format (defaults to today)
    #[arg(long)]
    pub date: Option<String>,
    /// Text to append (reads from stdin if omitted and stdin is not a TTY)
    pub content: Option<String>,
    #[command(subcommand)]
    pub subcommand: Option<JournalSubcommand>,
}

#[derive(Subcommand, Debug)]
pub enum JournalSubcommand {
    /// Show a journal entry
    Show {
        /// Date in YYYY-MM-DD format (defaults to today)
        #[arg(long)]
        date: Option<String>,
        /// Output format
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
}

pub async fn run(args: JournalArgs, vault: &NoteVault, workspace_name: &str) -> Result<()> {
    match args.subcommand {
        Some(JournalSubcommand::Show { date, format }) => {
            run_show(vault, date.as_deref(), format, workspace_name).await
        }
        None => run_append(vault, args.date.as_deref(), args.content).await,
    }
}

/// Validate and return a `YYYY-MM-DD` date string. Defaults to today when `None`.
fn resolve_date(date: Option<&str>) -> Result<String> {
    match date {
        None => Ok(chrono::Utc::now().format("%Y-%m-%d").to_string()),
        Some(d) => {
            chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| {
                color_eyre::eyre::eyre!("Invalid date '{}' — expected format YYYY-MM-DD", d)
            })?;
            Ok(d.to_string())
        }
    }
}

/// Build the vault path for a journal entry using the vault's configured journal path.
fn journal_entry_path(vault: &NoteVault, date_str: &str) -> VaultPath {
    vault
        .journal_path()
        .append(&VaultPath::note_path_from(date_str))
        .absolute()
}

async fn run_append(vault: &NoteVault, date: Option<&str>, content: Option<String>) -> Result<()> {
    use crate::cli::helpers::resolve_content;

    let text = resolve_content(content)?;
    if text.is_empty() {
        return Ok(());
    }

    let (vault_path, existing) = match date {
        None => {
            // Today — journal_entry() handles create-if-absent internally.
            let (details, existing) = vault
                .journal_entry()
                .await
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            (details.path, existing)
        }
        Some(d) => {
            let date_str = resolve_date(Some(d))?;
            let vault_path = journal_entry_path(vault, &date_str);
            let existing = vault
                .load_or_create_note(&vault_path, Some(format!("# {}\n\n", date_str)))
                .await
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            (vault_path, existing)
        }
    };

    let combined = format!("{}\n{}", existing, text);
    vault
        .save_note(&vault_path, &combined)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    println!("Note saved: {}", vault_path);
    Ok(())
}

async fn run_show(
    vault: &NoteVault,
    date: Option<&str>,
    format: OutputFormat,
    workspace_name: &str,
) -> Result<()> {
    use crate::cli::commands::note_ops::format_note_show_text;
    use crate::cli::json_output::{JsonNoteEntry, JsonNoteMetadata, JsonOutput, JsonOutputMetadata};
    use crate::cli::metadata_extractor::{extract_tags, extract_links, extract_headers};
    use kimun_core::error::{VaultError, FSError};
    use chrono::Utc;
    use std::time::UNIX_EPOCH;

    if matches!(format, OutputFormat::Paths) {
        return Err(color_eyre::eyre::eyre!(
            "--format paths is not valid for journal show; use 'text' or 'json'"
        ));
    }

    let date_str = resolve_date(date)?;
    let vault_path = journal_entry_path(vault, &date_str);

    let note_details = vault.load_note(&vault_path).await.map_err(|e| match e {
        VaultError::FSError(FSError::VaultPathNotFound { .. }) => {
            color_eyre::eyre::eyre!("No journal entry found for {}", date_str)
        }
        _ => color_eyre::eyre::eyre!("{}", e),
    })?;

    let content = &note_details.raw_text;
    let content_data = note_details.get_content_data();

    let backlinks = vault
        .get_backlinks(&vault_path)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
    let backlink_paths: Vec<String> = backlinks.iter().map(|(e, _)| e.path.to_string()).collect();

    match format {
        OutputFormat::Text => {
            let tags = extract_tags(content);
            let links = extract_links(content);
            print!(
                "{}",
                format_note_show_text(
                    &vault_path,
                    content,
                    &content_data.title,
                    &tags,
                    &links,
                    &backlink_paths,
                )
            );
        }
        OutputFormat::Json => {
            let meta = tokio::fs::metadata(vault.path_to_pathbuf(&vault_path))
                .await
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            let modified_secs = meta
                .modified()
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
                .unwrap_or(0);
            let tags = extract_tags(content);
            let links = extract_links(content);
            let headers = extract_headers(content);
            let journal_date = vault
                .journal_date(&vault_path)
                .map(|d| d.format("%Y-%m-%d").to_string());
            let entry = JsonNoteEntry {
                path: vault_path.to_string_with_ext(),
                title: content_data.title.clone(),
                content: content.clone(),
                size: meta.len(),
                modified: modified_secs,
                created: modified_secs,
                hash: format!("{:x}", content_data.hash),
                journal_date,
                metadata: JsonNoteMetadata { tags, links, headers },
                backlinks: if backlink_paths.is_empty() {
                    None
                } else {
                    Some(backlink_paths)
                },
            };
            let output = JsonOutput {
                metadata: JsonOutputMetadata {
                    workspace: workspace_name.to_string(),
                    workspace_path: vault.workspace_path.to_string_lossy().to_string(),
                    total_results: 1,
                    query: None,
                    is_listing: false,
                    generated_at: Utc::now().to_rfc3339(),
                },
                notes: vec![entry],
            };
            print!(
                "{}",
                serde_json::to_string(&output)
                    .map_err(|e| color_eyre::eyre::eyre!("{}", e))?
            );
        }
        OutputFormat::Paths => unreachable!("guarded above"),
    }

    Ok(())
}
