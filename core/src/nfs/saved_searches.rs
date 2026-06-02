//! Saved searches: named queries persisted in the vault under
//! `.kimun/saved-searches.toml`, so they travel with the notes (see
//! `adr/0004-saved-searches-stored-in-vault.md`). All filesystem access
//! lives here per the project rule that fs ops belong in `nfs`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::FSError;

/// A named query. `query` is stored verbatim, including any TUI query
/// variable such as `{note}`; resolution happens in the presentation layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
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
