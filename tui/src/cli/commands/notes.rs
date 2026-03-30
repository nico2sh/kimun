// tui/src/cli/commands/notes.rs
use color_eyre::eyre::Result;
use kimun_core::NoteVault;
use crate::cli::output::{OutputFormat, format_note_entries_text_with_journal};
use crate::cli::json_output::format_notes_as_json;

pub async fn run(
    vault: &NoteVault,
    path_filter: Option<&str>,
    format: OutputFormat,
    workspace_name: &str,
    _include_backlinks: bool,
) -> Result<()> {
    let mut results = vault.get_all_notes().await?;

    // Apply path filter if provided
    if let Some(prefix) = path_filter {
        results.retain(|(entry_data, _)| {
            entry_data.path.to_string().starts_with(prefix)
        });
    }

    match format {
        OutputFormat::Text => {
            let output = format_note_entries_text_with_journal(vault, &results);
            print!("{}", output);
        }
        OutputFormat::Paths => {
            for (entry_data, _) in &results {
                println!("{}", entry_data.path.to_bare_string());
            }
        }
        OutputFormat::Json => {
            let json_output = format_notes_as_json(
                vault,
                &results,
                workspace_name,
                None,
                true, // is_listing
            ).await
            .map_err(|e| color_eyre::eyre::eyre!("JSON formatting error: {}", e))?;
            print!("{}", json_output);
        }
    }

    Ok(())
}
