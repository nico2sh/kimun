// tui/src/cli/commands/search.rs
use color_eyre::eyre::Result;
use kimun_core::NoteVault;
use crate::cli::output::{OutputFormat, format_note_entries_text_with_journal};
use crate::cli::json_output::format_notes_as_json;

pub async fn run(
    vault: &NoteVault,
    query: &str,
    format: OutputFormat,
    workspace_name: &str,
    _include_backlinks: bool,
) -> Result<()> {
    let results = vault.search_notes(query).await?;

    match format {
        OutputFormat::Text => {
            let output = format_note_entries_text_with_journal(vault, &results);
            print!("{}", output);
        }
        OutputFormat::Json => {
            let json_output = format_notes_as_json(
                vault,
                &results,
                workspace_name,
                Some(query),
                false, // is_listing
            ).await
            .map_err(|e| color_eyre::eyre::eyre!("JSON formatting error: {}", e))?;
            print!("{}", json_output);
        }
    }

    Ok(())
}
