//! The wiki declaration: `.kimun/wiki.toml`, the strict file that turns a
//! vault into a wiki. All filesystem access lives here per the project rule
//! that fs ops belong in `nfs`.

use serde::{Deserialize, Serialize};

use crate::error::FSError;
use crate::nfs::VaultPath;
use crate::system::SystemPath;

/// The wiki declaration. Paths are strings, as written in the file. Use the
/// `*_path` methods to get `VaultPath` values. They resolve `.` and `..`, and
/// a relative path counts from the vault root. Unknown keys are an error, so
/// a typo never silently turns a rule off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WikiDeclaration {
    /// Folder of raw source notes.
    pub sources: String,
    /// Folder of wiki notes.
    pub wiki: String,
    /// The log note.
    pub log: String,
    /// The schema note.
    pub schema: String,
    /// Properties every wiki note must carry.
    pub required_properties: Vec<String>,
}

impl WikiDeclaration {
    /// The default layout: `/sources`, `/wiki`, `/wiki/log.md`, `/wiki/schema.md`.
    pub fn defaults() -> Self {
        Self {
            sources: "/sources".to_string(),
            wiki: "/wiki".to_string(),
            log: "/wiki/log.md".to_string(),
            schema: "/wiki/schema.md".to_string(),
            required_properties: ["summary", "sources", "updated"]
                .into_iter()
                .map(String::from)
                .collect(),
        }
    }

    /// The default declaration with other folders. The log and schema notes
    /// live inside the wiki folder. Each path is stored flat and absolute, so
    /// `./raw` is written as `/raw`.
    pub fn with_folders(sources: &VaultPath, wiki: &VaultPath) -> Self {
        let wiki = wiki.flatten().absolute();
        let mut d = Self::defaults();
        d.sources = sources.flatten().absolute().to_string();
        d.log = wiki.append(&VaultPath::note_path_from("log")).to_string();
        d.schema = wiki
            .append(&VaultPath::note_path_from("schema"))
            .to_string();
        d.wiki = wiki.to_string();
        d
    }

    /// The sources folder.
    pub fn sources_path(&self) -> VaultPath {
        normalized(&self.sources)
    }

    /// The wiki folder.
    pub fn wiki_path(&self) -> VaultPath {
        normalized(&self.wiki)
    }

    /// The log note.
    pub fn log_path(&self) -> VaultPath {
        normalized(&self.log)
    }

    /// The schema note.
    pub fn schema_path(&self) -> VaultPath {
        normalized(&self.schema)
    }

    /// Check the declaration rules. The error text says what to change.
    pub fn validate(&self) -> Result<(), String> {
        let fields = [
            ("sources", &self.sources),
            ("wiki", &self.wiki),
            ("log", &self.log),
            ("schema", &self.schema),
        ];
        for (name, raw) in fields {
            if !VaultPath::is_valid(raw) {
                return Err(format!(
                    "{name} has a character or name that is not allowed in a path, got '{raw}'"
                ));
            }
        }
        let sources = self.sources_path();
        let wiki = self.wiki_path();
        if sources.is_root_or_empty() || wiki.is_root_or_empty() {
            return Err("sources and wiki must be folders, not the vault root".to_string());
        }
        if sources.is_note() {
            return Err(format!("sources must be a folder, got '{}'", self.sources));
        }
        if wiki.is_note() {
            return Err(format!("wiki must be a folder, got '{}'", self.wiki));
        }
        if sources.is_like(&wiki) {
            return Err("sources and wiki must be different folders".to_string());
        }
        if sources.is_ancestor_of(&wiki) || wiki.is_ancestor_of(&sources) {
            return Err("sources and wiki folders must not contain each other".to_string());
        }
        let log = self.log_path();
        let schema = self.schema_path();
        if !log.is_note() {
            return Err(format!("log must be a note path, got '{}'", self.log));
        }
        if !schema.is_note() {
            return Err(format!("schema must be a note path, got '{}'", self.schema));
        }
        if sources.is_ancestor_of(&log) || sources.is_ancestor_of(&schema) {
            return Err("log and schema must not be inside the sources folder".to_string());
        }
        Ok(())
    }
}

/// A declared path as a flat, absolute `VaultPath`: `.` and `..` are
/// resolved and a relative path counts from the vault root.
fn normalized(raw: &str) -> VaultPath {
    VaultPath::new(raw).flatten().absolute()
}

fn wiki_declaration_path(workspace_path: &SystemPath) -> std::path::PathBuf {
    workspace_path.as_path().join(".kimun").join("wiki.toml")
}

/// Read the declaration. A missing file gives `None`.
pub async fn read_wiki_declaration(
    workspace_path: &SystemPath,
) -> Result<Option<WikiDeclaration>, FSError> {
    let path = wiki_declaration_path(workspace_path);
    match tokio::fs::read_to_string(&path).await {
        Ok(body) => toml::from_str(&body)
            .map(Some)
            .map_err(|e| FSError::SerializationError(e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(FSError::ReadFileError(e)),
    }
}

/// Write the declaration, creating `.kimun/` if needed.
pub async fn write_wiki_declaration(
    workspace_path: &SystemPath,
    declaration: &WikiDeclaration,
) -> Result<(), FSError> {
    let path = wiki_declaration_path(workspace_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let body = toml::to_string_pretty(declaration)
        .map_err(|e| FSError::SerializationError(e.to_string()))?;
    tokio::fs::write(&path, body).await?;
    Ok(())
}
