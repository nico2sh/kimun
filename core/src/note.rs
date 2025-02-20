use std::{fmt::Display, path::Path};

use crate::{content_data, error::VaultError, nfs::VaultPath};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoteDetails {
    pub path: VaultPath,
    pub data: NoteContentData,
    // Content may be lazy fetched
    // if the details are taken from the DB, the content is
    // likely not going to be there, so the `get_content` function
    // will take it from disk, and store in the cache
    cached_text: Option<String>,
}

impl Display for NoteDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Path: {}, Data: {}, Has Text Cached: {}",
            self.path,
            self.data,
            self.cached_text.is_some()
        )
    }
}

impl NoteDetails {
    pub fn new(note_path: VaultPath, hash: u64, title: String, text: Option<String>) -> Self {
        let data = NoteContentData {
            hash,
            title,
            content_chunks: vec![],
        };
        Self {
            path: note_path,
            data,
            cached_text: text,
        }
    }

    pub fn from_content<S: AsRef<str>>(text: S, note_path: &VaultPath) -> Self {
        let data = content_data::extract_data(&text);
        Self {
            path: note_path.to_owned(),
            data,
            cached_text: Some(text.as_ref().to_owned()),
        }
    }

    pub fn get_text<P: AsRef<Path>>(&mut self, base_path: P) -> Result<String, VaultError> {
        let content = self.cached_text.as_ref();
        // Content may be lazy loaded from disk since it's
        // the only data that is not stored in the DB
        if let Some(content) = content {
            Ok(content.clone())
        } else {
            let content = crate::load_note(base_path, &self.path)?;
            self.cached_text = Some(content.clone());
            Ok(content)
        }
    }

    // pub fn get_markdown<P: AsRef<Path>>(&mut self, base_path: P) -> Result<String, VaultError> {
    //     let text = self.get_text(base_path)?;
    // }

    pub fn get_title(&self) -> String {
        self.data.title.clone()
        // .unwrap_or_else(|| self.path.get_parent_path().1)
    }
}

/// NoteContentData contains tha extracted data from the note
/// for comparison and search in the DB, it is expensive to get
/// so it is not a good idea to calculate it every time the content
/// has changed, but better lazy get it when needed and cache it somewhere
/// (like the DB) for search and access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteContentData {
    pub title: String,
    pub hash: u64,
    pub content_chunks: Vec<ContentChunk>,
}

impl NoteContentData {
    pub fn new(title: String, hash: u64, content_chunks: Vec<ContentChunk>) -> Self {
        Self {
            title,
            hash,
            content_chunks,
        }
    }
}

impl Display for NoteContentData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Title: {}, Hash: {}, Chunks: [{}]",
            self.title,
            self.hash,
            self.content_chunks
                .iter()
                .map(|chunk| format!("'{}'", chunk.get_breadcrumb()))
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentChunk {
    pub breadcrumb: Vec<String>,
    pub text: String,
}

impl ContentChunk {
    pub fn get_breadcrumb(&self) -> String {
        self.breadcrumb.join(">")
    }

    pub fn get_text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkType {
    Local,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    ltype: LinkType,
    url: String,
    text: String,
}
