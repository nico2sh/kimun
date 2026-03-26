// tui/src/cli/commands/search.rs
use color_eyre::eyre::Result;
use kimun_core::NoteVault;
use crate::cli::output::{OutputFormat, format_note_entries_text_with_journal};
use crate::cli::json_output::format_notes_as_json;

pub async fn run(vault: &NoteVault, query: &str, format: OutputFormat) -> Result<()> {
    let results = vault.search_notes(query).await?;

    match format {
        OutputFormat::Text => {
            let output = format_note_entries_text_with_journal(vault, &results);
            print!("{}", output);
        }
        OutputFormat::Json => {
            let workspace_path = vault.workspace_path.to_string_lossy().to_string();
            let json_output = format_notes_as_json(
                &results,
                "default",
                &workspace_path,
                Some(query),
                false
            )?;
            print!("{}", json_output);
        }
    }

    Ok(())
}
