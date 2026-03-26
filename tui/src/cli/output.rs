// tui/src/cli/output.rs
use clap::ValueEnum;
use kimun_core::nfs::NoteEntryData;
use kimun_core::note::NoteContentData;

#[derive(ValueEnum, Clone)]
pub enum OutputFormat {
    Text,
    Json,
}

/// Format note entries as text with journal date detection
pub fn format_note_entries_text_with_journal(
    vault: &kimun_core::NoteVault,
    entries: &[(NoteEntryData, NoteContentData)]
) -> String {
    let mut output = String::new();

    for (entry_data, content_data) in entries {
        let path = entry_data.path.to_string();
        let title = format!("\"{}\"", content_data.title);
        let size = entry_data.size;
        let modified_secs = entry_data.modified_secs;

        let mut line = format!("{}\t{}\t{}\t{}", path, title, size, modified_secs);

        // Add journal date if this is a journal note
        if let Some(journal_date) = vault.journal_date(&entry_data.path) {
            line.push_str(&format!("\tjournal:{}", journal_date.format("%Y-%m-%d")));
        }

        output.push_str(&line);
        output.push('\n');
    }

    output
}
