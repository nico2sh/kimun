//! Saved searches: named queries persisted in the vault under
//! `.kimun/saved-searches.toml`, so they travel with the notes.
//! All filesystem access lives here per the project rule that
//! fs ops belong in `nfs`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::FSError;

/// A named query. `query` is stored verbatim, including any TUI query
/// variable such as `{note}`; resolution happens in the presentation layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    /// User-facing label shown in the saved-searches list.
    pub name: String,
    /// The query string, stored verbatim including any unresolved TUI query
    /// variable (e.g. `{note}`).
    pub query: String,
}

/// On-disk wrapper: TOML needs a named array-of-tables at the top level.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedSearchFile {
    #[serde(default)]
    search: Vec<SavedSearch>,
}

fn saved_searches_path(workspace_path: &Path) -> std::path::PathBuf {
    workspace_path.join(".kimun").join("saved-searches.toml")
}

/// The one name-match rule for saved searches: ASCII case-insensitive.
/// Every comparison against a saved-search name — core's save/delete/rename
/// upsert lookups and any UI preview of what a save will do — must go
/// through this, so a future change to the rule happens in exactly one place.
pub fn saved_search_name_matches(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Read all saved searches. Returns an empty list if the file does not
/// exist yet (a fresh vault has none).
pub async fn read_saved_searches(workspace_path: &Path) -> Result<Vec<SavedSearch>, FSError> {
    let path = saved_searches_path(workspace_path);
    match tokio::fs::read_to_string(&path).await {
        Ok(body) => {
            let parsed: SavedSearchFile =
                toml::from_str(&body).map_err(|e| FSError::SerializationError(e.to_string()))?;
            Ok(parsed.search)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(FSError::ReadFileError(e)),
    }
}

/// Write the full saved-search list, creating `.kimun/` if needed.
pub async fn write_saved_searches(
    workspace_path: &Path,
    searches: &[SavedSearch],
) -> Result<(), FSError> {
    let path = saved_searches_path(workspace_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file = SavedSearchFile {
        search: searches.to_vec(),
    };
    let body =
        toml::to_string_pretty(&file).map_err(|e| FSError::SerializationError(e.to_string()))?;
    tokio::fs::write(&path, body).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_match_is_ascii_case_insensitive() {
        assert!(saved_search_name_matches("todo", "TODO"));
        assert!(saved_search_name_matches("Todo", "toDo"));
        assert!(!saved_search_name_matches("todo", "todos"));
        // ASCII-only folding: accented characters compare byte-equal.
        assert!(!saved_search_name_matches("naïve", "NAÏVE"));
        assert!(saved_search_name_matches("Naïve", "naïve"));
    }

    #[tokio::test]
    async fn read_missing_file_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let got = read_saved_searches(dir.path()).await.unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn write_then_read_roundtrips() {
        let dir = tempfile::TempDir::new().unwrap();
        let searches = vec![
            SavedSearch {
                name: "todo".into(),
                query: "#todo".into(),
            },
            SavedSearch {
                name: "backlinks".into(),
                query: ">{note}".into(),
            },
        ];
        write_saved_searches(dir.path(), &searches).await.unwrap();
        let got = read_saved_searches(dir.path()).await.unwrap();
        assert_eq!(got, searches);
    }

    #[tokio::test]
    async fn write_creates_kimun_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        write_saved_searches(dir.path(), &[]).await.unwrap();
        assert!(dir
            .path()
            .join(".kimun")
            .join("saved-searches.toml")
            .exists());
    }
}
