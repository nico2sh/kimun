// tui/src/cli/commands/search.rs
use crate::cli::json_output::format_notes_as_json;
use crate::cli::output::{OutputFormat, format_note_entries_text_with_journal};
use color_eyre::eyre::Result;
use kimun_core::NoteVault;

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
                println!("{}", entry_data.path.to_bare_string());
            }
        }
        OutputFormat::Json => {
            let json_output = format_notes_as_json(
                vault,
                &results,
                workspace_name,
                Some(query),
                false, // is_listing
            )
            .await
            .map_err(|e| color_eyre::eyre::eyre!("JSON formatting error: {}", e))?;
            print!("{}", json_output);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use kimun_core::nfs::VaultPath;

    #[test]
    fn test_paths_strip_md_suffix() {
        let path = VaultPath::new("projects/my-note.md");
        assert_eq!(path.to_bare_string(), "projects/my-note");
    }

    #[test]
    fn test_paths_no_md_suffix_unchanged() {
        let path = VaultPath::new("projects/my-note");
        assert_eq!(path.to_bare_string(), "projects/my-note");
    }
}
