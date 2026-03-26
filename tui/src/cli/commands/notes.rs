// tui/src/cli/commands/notes.rs
use color_eyre::eyre::Result;
use kimun_core::NoteVault;
use crate::cli::output::{OutputFormat, format_note_entries_text_with_journal};
use crate::cli::json_output::format_notes_as_json;

pub async fn run(vault: &NoteVault, path_filter: Option<&str>, format: OutputFormat) -> Result<()> {
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
        OutputFormat::Json => {
            let workspace_path = vault.workspace_path.to_string_lossy().to_string();
            let json_output = format_notes_as_json(
                &results,
                "default",
                &workspace_path,
                None,
                true
            )?;
            print!("{}", json_output);
        }
    }

    Ok(())
}
