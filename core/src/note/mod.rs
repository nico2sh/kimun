pub(crate) mod content_extractor;

use std::fmt::Display;

// Crate-internal whole-note operations (markdown pipeline, link rewriting).
// The note module is the only door to the extractor: nothing outside `note/`
// names `content_extractor` directly.
pub(crate) use content_extractor::{process_image_links, replace_note_links};

use crate::nfs::VaultPath;

/// Scan helpers — live text analysis over editor buffer fragments: link and
/// wikilink spans, exclusion zones (code/frontmatter/links), label tokens,
/// URL classification. The presentation layer uses these to drive WYSIWYG
/// behaviour on text that is *being edited*; whole-note extraction (title,
/// chunks, links) goes through [`NoteDetails`] instead.
pub mod scan {
    pub use super::content_extractor::{
        is_inside_code_link_or_frontmatter, is_inside_exclusion_zone, is_remote_url,
        link_char_spans, link_target_filename, target_looks_like_image, url_with_allowed_scheme,
        wikilink_char_spans, ExclusionZones, LinkSpan, LinkSpanKind,
    };

    /// A label token detected in note text, with byte-offset range and the
    /// label name (without leading `#`).
    #[derive(Debug, Clone, Copy)]
    pub struct LabelMatch<'a> {
        /// Byte offset of the leading `#` (inclusive).
        pub byte_start: usize,
        /// Byte offset just past the last label character (exclusive).
        pub byte_end: usize,
        /// The label name without the leading `#`.
        pub name: &'a str,
    }

    /// Yields every label token in `text` that satisfies the label rules:
    /// matches the label character set AND is preceded by a non-label
    /// character (or the start of input). Code-span / HTML / link-span
    /// exclusion is the caller's responsibility because those concerns are
    /// context-specific.
    pub fn label_matches(text: &str) -> impl Iterator<Item = LabelMatch<'_>> + '_ {
        super::content_extractor::label_matches_inner(text)
    }
}

/// Returns the deduplicated lowercase label names extracted from `text`
/// according to the same rules used by the indexer (skips frontmatter,
/// code, HTML, markdown links, wikilinks; applies word-boundary on both
/// sides of the match).
pub fn extract_labels(text: &str) -> Vec<String> {
    let path = crate::nfs::VaultPath::root();
    let (_md, links) = content_extractor::get_markdown_and_links(&path, text);
    let mut seen = std::collections::BTreeSet::new();
    for l in links {
        if let LinkType::Hashtag = l.ltype {
            seen.insert(l.text.to_lowercase());
        }
    }
    seen.into_iter().collect()
}

/// A note's vault path paired with its raw, unprocessed text.
///
/// This is the entry point for whole-note content extraction: title, hash,
/// heading chunks, and links are all derived on demand from [`raw_text`] via
/// the methods below — nothing is precomputed or cached here. The struct
/// performs no I/O; the caller is responsible for loading the text.
///
/// [`raw_text`]: Self::raw_text
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoteDetails {
    /// Vault-internal path of the note, flattened on construction.
    pub path: VaultPath,
    /// The note's verbatim Markdown source, including any frontmatter.
    pub raw_text: String,
}

impl Display for NoteDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Path: {}, Content: {}", self.path, self.raw_text)
    }
}

/// The one door to whole-note extraction. The borrowed-text associated
/// functions (`*_of`) exist for bulk paths (indexing) that hold many notes'
/// text as `&str` and must not clone it; the `get_*` methods are the same
/// operations over this note's owned text.
impl NoteDetails {
    /// Builds a [`NoteDetails`] from a vault path and the note's raw text.
    ///
    /// The path is flattened (`.`/`..` segments resolved) on the way in; the
    /// text is copied into the owned [`raw_text`] field unchanged.
    ///
    /// [`raw_text`]: Self::raw_text
    pub fn new<S: AsRef<str>>(note_path: &VaultPath, text: S) -> Self {
        Self {
            path: note_path.flatten(),
            raw_text: text.as_ref().to_owned(),
        }
    }

    /// Title of a note body, without constructing a `NoteDetails`.
    pub fn get_title_from_text<S: AsRef<str>>(text: S) -> String {
        content_extractor::extract_title(text)
    }

    /// Indexable content data (title + hash) of a note body, without
    /// constructing a `NoteDetails`.
    pub fn content_data_of<S: AsRef<str>>(text: S) -> NoteContentData {
        content_extractor::get_content_data(text)
    }

    /// Heading-chunked content of a note body, without constructing a
    /// `NoteDetails`.
    pub fn content_chunks_of<S: AsRef<str>>(text: S) -> Vec<ContentChunk> {
        content_extractor::get_content_chunks(text)
    }

    /// Heading chunks plus every link (note links, attachments, images,
    /// URLs, hashtags) of a note body at `path`, without constructing a
    /// `NoteDetails`.
    pub fn chunks_and_links_of<S: AsRef<str>>(
        path: &VaultPath,
        text: S,
    ) -> (Vec<ContentChunk>, Vec<NoteLink>) {
        content_extractor::get_chunks_and_links(path, text)
    }

    /// Title of this note (first non-empty line of the body, frontmatter
    /// skipped).
    pub fn get_title(&self) -> String {
        Self::get_title_from_text(&self.raw_text)
    }

    /// Indexable content data (title + content hash) of this note.
    pub fn get_content_data(&self) -> NoteContentData {
        Self::content_data_of(&self.raw_text)
    }

    /// Heading-chunked content of this note, one [`ContentChunk`] per
    /// heading section.
    pub fn get_content_chunks(&self) -> Vec<ContentChunk> {
        Self::content_chunks_of(&self.raw_text)
    }

    /// Heading chunks plus every link (note links, attachments, images,
    /// URLs, hashtags) of this note, resolved against its own [`path`].
    ///
    /// [`path`]: Self::path
    pub fn get_chunks_and_links(&self) -> (Vec<ContentChunk>, Vec<NoteLink>) {
        Self::chunks_and_links_of(&self.path, &self.raw_text)
    }

    /// Rendered Markdown of this note plus its extracted links: wikilinks
    /// become standard Markdown links, note links resolve to vault-relative
    /// absolute paths, hashtags become `[#tag](#tag)` links.
    pub fn get_markdown_and_links(&self) -> (String, Vec<NoteLink>) {
        content_extractor::get_markdown_and_links(&self.path, &self.raw_text)
    }
}

/// A note's rendered Markdown together with the links extracted from it.
///
/// The text is the result of the link-rewriting pipeline (wikilinks turned
/// into standard Markdown links, hashtags into anchor links), so it is ready
/// to hand to a Markdown renderer while [`links`] drives navigation.
///
/// [`links`]: Self::links
#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownNote {
    /// The rewritten Markdown source.
    pub text: String,
    /// Every link discovered in the note, in document order.
    pub links: Vec<NoteLink>,
}

/// NoteContentData contains the basic extracted data from the note
/// for comparison and search in the DB, it is expensive to get
/// so it is not a good idea to calculate it every time the content
/// has changed, but better lazy get it when needed and cache it somewhere
/// (like the DB) for search and access.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NoteContentData {
    /// The note's title (first non-empty body line, frontmatter skipped).
    pub title: String,
    /// XxHash64 digest of the note's full text, used to detect content
    /// changes cheaply during indexing.
    pub hash: u64,
}

impl NoteContentData {
    /// Builds a [`NoteContentData`] from a precomputed title and content
    /// hash.
    pub fn new(title: String, hash: u64) -> Self {
        Self { title, hash }
    }
}

impl Display for NoteContentData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Title: {}, Hash: {}", self.title, self.hash,)
    }
}

/// Separator used to flatten the heading hierarchy of a chunk into the single
/// `breadcrumb` string stored in the FTS column and in memory.
///
/// Uses ASCII Unit Separator (U+001F) so heading text containing visible
/// punctuation — including `>`, `/`, `|`, `:` — round-trips correctly through
/// `breadcrumb_parts()` / `breadcrumb_last()`. Not the `>` used as the search
/// query operator in `index::search_terms`.
pub const BREADCRUMB_SEP: &str = "\x1f";

/// A single heading section of a note: the text under one heading, tagged
/// with the heading hierarchy that leads to it.
///
/// Chunks are the unit of full-text search. The [`breadcrumb`] flattens the
/// nested heading path into one string (joined with [`BREADCRUMB_SEP`]); use
/// [`breadcrumb_parts`] / [`breadcrumb_last`] to read it back as segments.
///
/// [`breadcrumb`]: Self::breadcrumb
/// [`breadcrumb_parts`]: Self::breadcrumb_parts
/// [`breadcrumb_last`]: Self::breadcrumb_last
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentChunk {
    /// Heading hierarchy leading to this chunk, from outermost to innermost,
    /// joined with [`BREADCRUMB_SEP`]. Empty when the chunk has no heading.
    pub breadcrumb: String,
    /// The chunk's body text (the content under its innermost heading).
    pub text: String,
}

impl ContentChunk {
    /// Raw breadcrumb string, with segments joined by [`BREADCRUMB_SEP`].
    pub fn get_breadcrumb(&self) -> &str {
        &self.breadcrumb
    }

    /// Iterator over the heading components from outermost to innermost.
    /// Empty breadcrumb yields no items.
    pub fn breadcrumb_parts(&self) -> impl Iterator<Item = &str> {
        self.breadcrumb
            .split(BREADCRUMB_SEP)
            .filter(|s| !s.is_empty())
    }

    /// Last (innermost) heading in the breadcrumb, if any. O(last-segment-len)
    /// — scans backward from the end, short-circuiting at the first separator.
    pub fn breadcrumb_last(&self) -> Option<&str> {
        self.breadcrumb
            .rsplit(BREADCRUMB_SEP)
            .find(|s| !s.is_empty())
    }

    /// The chunk's body text.
    pub fn get_text(&self) -> &str {
        &self.text
    }
}

/// Classification of a link found in a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkType {
    /// A link to another note in the vault, by its resolved path.
    Note(VaultPath),
    /// A link to a non-note vault file (e.g. a PDF), by its resolved path.
    Attachment(VaultPath),
    /// Image link with its resolved path.
    /// For vault images: absolute OS path (e.g. `/home/user/vault/images/photo.png`).
    /// For external images: the original URL.
    Image(String),
    /// An external link to a remote `http`/`https` URL.
    Url,
    /// A `#hashtag` label.
    Hashtag,
}

/// A link extracted from a note: its [`LinkType`] classification, the display
/// text, and the original (uncleaned) link target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLink {
    /// What kind of link this is, with any resolved target.
    pub ltype: LinkType,
    /// The link's display text (alt text for images, tag name for hashtags).
    pub text: String,
    /// The link target exactly as written in the note, without any cleanup:
    /// it may contain invalid characters or uppercase letters (for note
    /// links these are normalized only when converting to a [`VaultPath`]).
    pub raw_link: String,
}

impl NoteLink {
    /// Builds a vault link, classifying it as [`LinkType::Note`] when `path`
    /// points at a note and [`LinkType::Attachment`] otherwise.
    pub fn vault_path<S: AsRef<str>>(path: &VaultPath, text: S) -> Self {
        let ltype = if path.is_note() {
            LinkType::Note(path.to_owned())
        } else {
            LinkType::Attachment(path.to_owned())
        };
        Self {
            ltype,
            text: text.as_ref().to_string(),
            raw_link: path.to_string(),
        }
    }
    /// Builds a [`LinkType::Note`] link to `path` with the given display
    /// `text`.
    pub fn note<S: AsRef<str>>(path: &VaultPath, text: S) -> Self {
        Self {
            ltype: LinkType::Note(path.to_owned()),
            text: text.as_ref().to_string(),
            raw_link: path.to_string(),
        }
    }
    /// Builds a [`LinkType::Url`] link to a remote `url` with the given
    /// display `text`.
    pub fn url<S: AsRef<str>, T: AsRef<str>>(url: S, text: T) -> Self {
        Self {
            ltype: LinkType::Url,
            text: text.as_ref().to_string(),
            raw_link: url.as_ref().to_string(),
        }
    }
    /// Builds a [`LinkType::Hashtag`] link from a bare tag name (no leading
    /// `#`). The display text is the tag name and `raw_link` is the tag with
    /// a `#` prefix restored.
    ///
    /// ```
    /// use kimun_core::note::{LinkType, NoteLink};
    ///
    /// let link = NoteLink::hashtag("rust");
    /// assert_eq!(link.ltype, LinkType::Hashtag);
    /// assert_eq!(link.text, "rust");
    /// assert_eq!(link.raw_link, "#rust");
    /// ```
    pub fn hashtag<S: AsRef<str>>(tag: S) -> Self {
        let tag_text = tag.as_ref().to_string();
        Self {
            ltype: LinkType::Hashtag,
            text: tag_text.clone(),
            raw_link: format!("#{}", tag_text),
        }
    }
    /// Image link.
    /// `resolved_path`: absolute OS path for vault images, original URL for external images.
    /// `alt_text`: the alt text from the markdown `![alt_text](...)`.
    /// `raw_link`: the original path/URL as written in the note.
    pub fn image<S: AsRef<str>, T: AsRef<str>, U: AsRef<str>>(
        resolved_path: S,
        alt_text: T,
        raw_link: U,
    ) -> Self {
        Self {
            ltype: LinkType::Image(resolved_path.as_ref().to_string()),
            text: alt_text.as_ref().to_string(),
            raw_link: raw_link.as_ref().to_string(),
        }
    }
}
