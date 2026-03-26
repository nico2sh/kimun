// tui/src/cli/commands/search.rs
use color_eyre::eyre::Result;
use kimun_core::NoteVault;
use crate::cli::output::{OutputFormat, format_note_entries_text_with_journal};

pub async fn run(vault: &NoteVault, query: &str, format: OutputFormat) -> Result<()> {
    let results = vault.search_notes(query).await?;

    match format {
        OutputFormat::Text => {
            let output = format_note_entries_text_with_journal(vault, &results);
            print!("{}", output);
        }
    }

    Ok(())
}
