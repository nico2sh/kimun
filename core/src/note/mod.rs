mod content_data;

use std::fmt::Display;

use content_data::{extract_data, extract_title};

use crate::nfs::VaultPath;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoteDetails {
    pub path: VaultPath,
    pub data: NoteContentData,
    pub text: String,
    pub content_chunks: Vec<ContentChunk>,
}

impl Display for NoteDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Path: {}, Data: {}", self.path, self.data,)?;
        write!(
            f,
            "Chunks: [{}]",
            self.content_chunks
                .iter()
                .map(|chunk| format!("'{}'", chunk.get_breadcrumb()))
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}

impl NoteDetails {
    pub fn new<S: AsRef<str>>(note_path: &VaultPath, text: S) -> Self {
        extract_data(note_path, text)
    }

    pub fn get_title_from_text<S: AsRef<str>>(text: S) -> String {
        extract_title(text)
    }
    // pub fn new(note_path: VaultPath, hash: u64, title: String, text: Option<String>) -> Self {
    //     let data = NoteContentData {
    //         hash,
    //         title,
    //         content_chunks: vec![],
    //     };
    //     Self {
    //         path: note_path,
    //         data,
    //         text,
    //     }
    // }

    // pub fn get_markdown<P: AsRef<Path>>(&mut self, base_path: P) -> Result<String, VaultError> {
    //     let text = self.get_text(base_path)?;
    // }

    pub fn get_title(&self) -> String {
        self.data.title.clone()
        // .unwrap_or_else(|| self.path.get_parent_path().1)
    }
}

/// NoteContentData contains the basic extracted data from the note
/// for comparison and search in the DB, it is expensive to get
/// so it is not a good idea to calculate it every time the content
/// has changed, but better lazy get it when needed and cache it somewhere
/// (like the DB) for search and access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteContentData {
    pub title: String,
    pub hash: u64,
}

impl NoteContentData {
    pub fn new(title: String, hash: u64) -> Self {
        Self { title, hash }
    }
}

impl Display for NoteContentData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Title: {}, Hash: {}", self.title, self.hash,)
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
