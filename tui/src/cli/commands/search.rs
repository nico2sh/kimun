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
        OutputFormat::Paths => {
            for (entry_data, _) in &results {
                let s = entry_data.path.to_string();
                let bare = s.strip_suffix(".md").unwrap_or(&s);
                println!("{}", bare);
            }
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_paths_strip_md_suffix() {
        let with_ext = "projects/my-note.md";
        let bare = with_ext.strip_suffix(".md").unwrap_or(with_ext);
        assert_eq!(bare, "projects/my-note");
    }

    #[test]
    fn test_paths_no_md_suffix_unchanged() {
        let no_ext = "projects/my-note";
        let bare = no_ext.strip_suffix(".md").unwrap_or(no_ext);
        assert_eq!(bare, "projects/my-note");
    }
}
