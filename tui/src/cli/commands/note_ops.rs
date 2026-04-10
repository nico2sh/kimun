// tui/src/cli/commands/note_ops.rs
//
// CLI commands for note create, append, and show operations.

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
    /// Quickly capture a thought into a timestamped inbox note
    Quick {
        /// Text content (reads from stdin if omitted and stdin is not a TTY)
        content: Option<String>,
    },
    /// List inbox notes for triage
    Triage,
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
        NoteSubcommand::Quick { content } => run_quick(vault, content).await,
        NoteSubcommand::Triage => run_triage(vault).await,
        NoteSubcommand::Show { paths, format } => {
            use std::io::IsTerminal;
            let reader = if std::io::stdin().is_terminal() {
                None
            } else {
                Some(std::io::BufReader::new(std::io::stdin().lock()))
            };
            let resolved = resolve_show_paths(paths, reader)?;
            run_show(vault, &resolved, quick_note_path, format, workspace_name).await
        }
    }
}

async fn run_create(
    vault: &NoteVault,
    path_input: &str,
    content: Option<String>,
    quick_note_path: &str,
) -> Result<()> {
    use crate::cli::helpers::{resolve_note_path, resolve_content};

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
    use crate::cli::helpers::{resolve_note_path, resolve_content};
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

pub(crate) fn format_note_show_text(
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

/// Resolves the effective path list for `note show`.
/// - If `args` is non-empty, returns it directly (reader is ignored).
/// - If `args` is empty and `reader` is `Some`, reads non-blank trimmed lines from it.
/// - If `args` is empty and `reader` is `None` (TTY), returns an error.
fn resolve_show_paths<R: std::io::BufRead>(
    args: Vec<String>,
    reader: Option<R>,
) -> color_eyre::eyre::Result<Vec<String>> {
    if !args.is_empty() {
        return Ok(args);
    }
    match reader {
        Some(r) => {
            let paths: Result<Vec<String>, _> = r
                .lines()
                .filter(|l| l.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(true))
                .map(|l| l.map(|s| s.trim().split('\t').next().unwrap_or("").to_owned()))
                .collect();
            let paths = paths.map_err(|e| color_eyre::eyre::eyre!("Failed to read stdin: {}", e))?;
            if paths.is_empty() {
                return Err(color_eyre::eyre::eyre!(
                    "No paths provided — pass paths as arguments or pipe from stdin"
                ));
            }
            Ok(paths)
        }
        None => Err(color_eyre::eyre::eyre!(
            "No paths provided — pass paths as arguments or pipe from stdin"
        )),
    }
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

    if matches!(format, OutputFormat::Paths) {
        return Err(color_eyre::eyre::eyre!(
            "--format paths is not valid for note show; use 'text' or 'json'"
        ));
    }

    // One accumulator per format — only the active one is ever populated.
    enum Accumulator {
        Text(Vec<String>),
        Json(Vec<JsonNoteEntry>),
    }

    let mut acc = match format {
        OutputFormat::Text => Accumulator::Text(Vec::new()),
        OutputFormat::Json => Accumulator::Json(Vec::new()),
        OutputFormat::Paths => unreachable!("guarded above"),
    };
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

        let content = &note_details.raw_text;
        let content_data = note_details.get_content_data();

        let backlink_results = vault
            .get_backlinks(&vault_path)
            .await
            .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        let backlink_paths: Vec<String> = backlink_results
            .iter()
            .map(|(e, _)| e.path.to_string())
            .collect();

        match &mut acc {
            Accumulator::Text(entries) => {
                let tags = extract_tags(content);
                let links = extract_links(content);
                entries.push(format_note_show_text(
                    &vault_path,
                    content,
                    &content_data.title,
                    &tags,
                    &links,
                    &backlink_paths,
                ));
            }
            Accumulator::Json(entries) => {
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
                let tags = extract_tags(content);
                let links = extract_links(content);
                let headers = extract_headers(content);
                let journal_date = vault
                    .journal_date(&vault_path)
                    .map(|d| d.format("%Y-%m-%d").to_string());
                entries.push(JsonNoteEntry {
                    path: vault_path.to_string_with_ext(),
                    title: content_data.title.clone(),
                    content: content.clone(),
                    size: entry_data.size,
                    modified: entry_data.modified_secs,
                    created: entry_data.modified_secs, // TODO: track actual creation time
                    hash: format!("{:x}", content_data.hash),
                    journal_date,
                    metadata: JsonNoteMetadata { tags, links, headers },
                    backlinks: if backlink_paths.is_empty() { None } else { Some(backlink_paths) },
                });
            }
        }
    }

    let is_empty = match &acc {
        Accumulator::Text(v) => v.is_empty(),
        Accumulator::Json(v) => v.is_empty(),
    };
    if is_empty {
        return Err(color_eyre::eyre::eyre!(
            "No notes found — all specified paths were missing"
        ));
    }

    // Output whatever was found — the JSON/text is valid for the notes that succeeded.
    // had_errors (non-zero exit) signals that some notes were missing; those were
    // already reported to stderr in the loop above.
    match acc {
        Accumulator::Text(entries) => {
            let sep = format!("\n{}\n\n", NOTE_SEPARATOR);
            print!("{}", entries.join(&sep));
        }
        Accumulator::Json(notes) => {
            let output = JsonOutput {
                metadata: JsonOutputMetadata {
                    workspace: workspace_name.to_string(),
                    workspace_path: vault.workspace_path.to_string_lossy().to_string(),
                    total_results: notes.len(),
                    query: None,
                    is_listing: false,
                    generated_at: Utc::now().to_rfc3339(),
                },
                notes,
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

async fn run_triage(vault: &NoteVault) -> Result<()> {
    let inbox = vault.inbox_path().clone();
    let all_notes = vault
        .get_all_notes()
        .await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    let inbox_notes: Vec<_> = all_notes
        .into_iter()
        .filter(|(entry, _)| {
            let (parent, _) = entry.path.get_parent_path();
            parent.is_like(&inbox)
                || parent.to_string().starts_with(&inbox.to_string())
        })
        .collect();

    if inbox_notes.is_empty() {
        println!("Inbox is empty.");
        return Ok(());
    }

    println!("Inbox notes ({}):\n", inbox_notes.len());
    for (entry, content_data) in &inbox_notes {
        let title = if content_data.title.trim().is_empty() {
            "<no title>"
        } else {
            &content_data.title
        };
        println!("  {} — {}", entry.path, title);
    }

    Ok(())
}

async fn run_quick(vault: &NoteVault, content: Option<String>) -> Result<()> {
    use crate::cli::helpers::resolve_content;

    let text = resolve_content(content)?;
    if text.is_empty() {
        return Ok(());
    }

    let details = vault
        .quick_note(&text)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    println!("Note saved: {}", details.path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_show_paths;
    use std::io::Cursor;

    #[test]
    fn test_resolve_show_paths_uses_args_when_given() {
        let args = vec!["projects/foo".to_string(), "inbox/bar".to_string()];
        let result = resolve_show_paths(args.clone(), None::<Cursor<&[u8]>>).unwrap();
        assert_eq!(result, args);
    }

    #[test]
    fn test_resolve_show_paths_reads_from_reader() {
        let input = b"projects/foo\ninbox/bar\n";
        let reader = Cursor::new(input.as_ref());
        let result = resolve_show_paths(vec![], Some(reader)).unwrap();
        assert_eq!(result, vec!["projects/foo", "inbox/bar"]);
    }

    #[test]
    fn test_resolve_show_paths_skips_blank_lines() {
        let input = b"projects/foo\n\n  \ninbox/bar\n";
        let reader = Cursor::new(input.as_ref());
        let result = resolve_show_paths(vec![], Some(reader)).unwrap();
        assert_eq!(result, vec!["projects/foo", "inbox/bar"]);
    }

    #[test]
    fn test_resolve_show_paths_all_blank_stdin_returns_empty() {
        let input = b"\n  \n\t\n";
        let reader = Cursor::new(input.as_ref());
        let result = resolve_show_paths(vec![], Some(reader));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No paths provided"), "got: {}", msg);
    }

    #[test]
    fn test_resolve_show_paths_strips_tab_separated_fields() {
        // kimun notes outputs tab-separated lines: path\ttitle\tsize\ttimestamp
        let input = b"projects/foo\tFoo Note\t1234\t1700000000\ninbox/bar\tBar\t42\t1700000001\n";
        let reader = Cursor::new(input.as_ref());
        let result = resolve_show_paths(vec![], Some(reader)).unwrap();
        assert_eq!(result, vec!["projects/foo", "inbox/bar"]);
    }

    #[test]
    fn test_resolve_show_paths_no_args_no_reader_errors() {
        let result = resolve_show_paths(vec![], None::<Cursor<&[u8]>>);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No paths provided"), "got: {}", msg);
    }
}
