mod content_extractor;

use std::fmt::Display;

use content_extractor::{extract_data, extract_title, get_markdown_and_links};

use crate::nfs::VaultPath;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoteDetails {
    pub path: VaultPath,
    pub data: NoteContentData,
    pub raw_text: String,
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

    // Returns the text and the links contained
    // The wikilinks are converted to markdown links, although only note links are allowed
    // External URLs needs to be created as markdown links. Always including the http(s)
    // Note links can be either Markdown or Wikilinks
    pub fn get_markdown_and_links(&self) -> MarkdownNote {
        let (text, links) = get_markdown_and_links(&self.raw_text);
        MarkdownNote { text, links }
    }

    pub fn get_title(&self) -> String {
        self.data.title.clone()
        // .unwrap_or_else(|| self.path.get_parent_path().1)
    }
}

pub struct MarkdownNote {
    pub text: String,
    pub links: Vec<Link>,
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
    Note(VaultPath),
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    ltype: LinkType,
    text: String,
}

impl Link {
    pub fn note<S: AsRef<str>>(path: VaultPath, text: S) -> Self {
        Self {
            ltype: LinkType::Note(path),
            text: text.as_ref().to_string(),
        }
    }
    pub fn url<S: AsRef<str>, T: AsRef<str>>(url: S, text: T) -> Self {
        Self {
            ltype: LinkType::Url(url.as_ref().to_string()),
            text: text.as_ref().to_string(),
        }
    }
}
