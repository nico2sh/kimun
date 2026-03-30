use log::debug;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use regex::{Captures, Regex};
use std::sync::LazyLock;

use crate::{
    nfs::{self, VaultPath},
    note::{ContentChunk, NoteContentData},
};

use super::NoteLink;

const _MAX_TITLE_LENGTH: usize = 40;

// Compile regexes once at startup
static WIKILINK_RX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:\[\[(?P<link_text>[^\]]+)\]\])"#).unwrap()
});

static HASHTAG_RX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"#(?P<ht_text>[A-Za-z0-9_]+)"#).unwrap()
});

static MD_LINK_RX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?P<bang>!?)(?:\[(?P<text>[^\]]+)\])\((?P<link>[^\)]+?)\)"#).unwrap()
});

static URL_RX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^https?:\/\/[\w\d]+\.[\w\d]+(?:(?:\.[\w\d]+)|(?:[\w\d\/?=#]+))+$"#).unwrap()
});

/// Discriminates the type of an inline link found by [`link_char_spans`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSpanKind {
    /// A `[[page]]` or `[[page|display]]` wikilink.
    WikiLink,
    /// A `[text](url)` or `![alt](url)` markdown link.
    Markdown,
}

/// A resolved inline link span within a text string.
///
/// `start` and `end` are char-index offsets covering the full token
/// (including delimiters such as `[[`/`]]` or `[`/`)`).
/// `target` holds the link destination — the wiki page name for
/// [`LinkSpanKind::WikiLink`] (before any `|` separator), or the URL/path
/// for [`LinkSpanKind::Markdown`].
#[derive(Debug, Clone)]
pub struct LinkSpan {
    pub start: usize,
    pub end: usize,
    pub kind: LinkSpanKind,
    pub target: String,
}

/// Returns every inline link span in `text`, covering both `[[wikilinks]]`
/// and `[markdown](links)`, sorted by document order.
///
/// Suitable for syntax highlighting, editor decoration, and lightweight
/// link extraction without full vault-path resolution.
pub fn link_char_spans(text: &str) -> Vec<LinkSpan> {
    let mut spans: Vec<LinkSpan> = Vec::new();

    for caps in WIKILINK_RX.captures_iter(text) {
        let m = caps.get(0).unwrap();
        let start = text[..m.start()].chars().count();
        let end = text[..m.end()].chars().count();
        let inner = &caps["link_text"];
        let target = inner.split('|').next().unwrap_or(inner).to_string();
        spans.push(LinkSpan { start, end, kind: LinkSpanKind::WikiLink, target });
    }

    for caps in MD_LINK_RX.captures_iter(text) {
        let m = caps.get(0).unwrap();
        let start = text[..m.start()].chars().count();
        let end = text[..m.end()].chars().count();
        let target = caps["link"].trim().to_string();
        spans.push(LinkSpan { start, end, kind: LinkSpanKind::Markdown, target });
    }

    spans.sort_by_key(|s| s.start);
    spans
}

/// Returns chunks and links in a single pass, avoiding double markdown parsing.
pub fn get_chunks_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (Vec<ContentChunk>, Vec<super::NoteLink>) {
    let chunks = get_content_chunks(md_text.as_ref());
    let (_text, links) = get_markdown_and_links(reference_path, md_text.as_ref());
    (chunks, links)
}

pub fn get_content_data<S: AsRef<str>>(md_text: S) -> NoteContentData {
    let hash = nfs::hash_text(md_text.as_ref());
    let title = extract_title(md_text);

    NoteContentData { title, hash }
}

pub fn get_content_chunks<S: AsRef<str>>(md_text: S) -> Vec<ContentChunk> {
    let (frontmatter, text) = remove_frontmatter(md_text.as_ref());

    // Clean up wikilinks and hashtags for indexing
    let text = process_wikilinks(&text, |_link, _text| None);
    let text = cleanup_hashtags(&text);

    let mut content_chunks = parse_text(&text);

    if !frontmatter.is_empty() {
        content_chunks.push(ContentChunk {
            breadcrumb: vec!["FrontMatter".to_string()],
            text: frontmatter,
        })
    }

    content_chunks
}

/// Process wikilinks with a custom handler function
/// Handler returns None to remove the wikilink (keep only text), or Some(String) to replace it
fn process_wikilinks<F>(md_text: &str, handler: F) -> String
where
    F: Fn(&str, &str) -> Option<String>,
{
    WIKILINK_RX
        .replace_all(md_text, |caps: &Captures| {
            let items = &caps["link_text"];
            let parts: Vec<&str> = items.split('|').collect();

            let (link, text) = match parts.len() {
                1 => (parts[0], parts[0]),
                2 => (parts[0], parts[1]),
                // Extra pipes: use first part as link, second as display text, ignore rest
                _ => (parts[0], parts[1]),
            };

            handler(link, text).unwrap_or_else(|| text.to_string())
        })
        .into_owned()
}

fn cleanup_hashtags(md_text: &str) -> String {
    HASHTAG_RX
        .replace_all(md_text, |caps: &Captures| caps["ht_text"].to_string())
        .into_owned()
}

/// Returns the converted text into Markdown (replacing note wikilinks to markdown links)
/// Normalizes the links urls when needed (lowercasing the path for vault paths)
/// And a list of the links existing in the note, relative links are transformed to absolute links.
/// Hashtags are converted to markdown links and added to the links list.
pub(crate) fn get_markdown_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (String, Vec<NoteLink>) {
    let mut links = vec![];

    // Convert wikilinks to markdown links
    let md_text = process_wikilinks(md_text.as_ref(), |link, text| {
        if VaultPath::is_valid(link) {
            let link_path = VaultPath::note_path_from(link);
            Some(format!("[{}]({})", text, link_path))
        } else {
            // Keep invalid wikilinks as-is
            Some(format!("[[{}]]", if link == text {
                link.to_string()
            } else {
                format!("{}|{}", link, text)
            }))
        }
    });

    // Process markdown links and extract them
    let md_text = MD_LINK_RX.replace_all(&md_text, |caps: &Captures| {
        let bang = &caps["bang"];
        let text = &caps["text"];
        let link = caps["link"].trim();

        // Ignore image links
        if !bang.is_empty() {
            return format!("![{}]({})", text, link);
        }

        debug!("checking link {}", link);

        let clean_link = if URL_RX.is_match(link) {
            // URL link
            links.push(NoteLink::url(link, text));
            link.to_string()
        } else if VaultPath::is_valid(link) {
            // Vault path link
            let path = VaultPath::new(link);

            if path.is_note_file() {
                // Absolute note path
                links.push(NoteLink::note(&path, text));
                path.to_string()
            } else {
                // Relative path - resolve it
                let ref_path = if reference_path.is_note() {
                    reference_path.get_parent_path().0
                } else {
                    reference_path.to_owned()
                };

                let abs_path = ref_path.append(&path).flatten();

                if abs_path.is_note() {
                    links.push(NoteLink::note(&abs_path, text));
                } else {
                    links.push(NoteLink::vault_path(&abs_path, text));
                }

                abs_path.to_string()
            }
        } else {
            debug!("link not counting {}", link);
            link.to_string()
        };

        format!("[{}]({})", text, clean_link)
    });

    // Process hashtags and convert them to links
    let clean_md_text = HASHTAG_RX.replace_all(&md_text, |caps: &Captures| {
        let tag = &caps["ht_text"];
        links.push(NoteLink::hashtag(tag));
        format!("[#{}](#{})", tag, tag)
    });

    (clean_md_text.to_string(), links)
}

/// Rewrites all links in `md_text` that target `old_path` so they target `new_path` instead.
///
/// Handles three link forms:
/// - WikiLinks: `[[old-name]]` → `[[new-name]]`, `[[old-name|display]]` → `[[new-name|display]]`
/// - Markdown links by full vault path: `[text](/old/path.md)` → `[text](/new/path.md)`
/// - Markdown links by filename: `[text](old-name.md)` → `[text](new-name.md)`
///
/// Returns `(updated_text, changed)` where `changed` is true when at least one replacement was made.
pub(crate) fn replace_note_links(
    md_text: &str,
    old_path: &VaultPath,
    new_path: &VaultPath,
) -> (String, bool) {
    let old_name = old_path.get_name(); // e.g. "old-title.md"
    let old_full = old_path.to_string(); // e.g. "/notes/old-title.md"
    let new_clean = new_path.get_clean_name(); // e.g. "new-title" (no extension)
    let new_name = new_path.get_name(); // e.g. "new-title.md"
    let new_full = new_path.to_string(); // e.g. "/notes/new-title.md"

    // Step 1: rewrite wikilinks whose resolved name matches old_path
    let after_wikilinks = WIKILINK_RX.replace_all(md_text, |caps: &Captures| {
        let items = &caps["link_text"];
        let parts: Vec<&str> = items.split('|').collect();
        let (link, display) = match parts.len() {
            1 => (parts[0], parts[0]),
            _ => (parts[0], parts[1]),
        };
        if VaultPath::note_path_from(link).get_name() == old_name {
            if link == display {
                format!("[[{}]]", new_clean)
            } else {
                format!("[[{}|{}]]", new_clean, display)
            }
        } else {
            // Keep unchanged — reconstruct the original form
            format!("[[{}]]", items)
        }
    });

    // Step 2: rewrite markdown links by full vault path or bare filename
    let after_links = MD_LINK_RX.replace_all(&after_wikilinks, |caps: &Captures| {
        let bang = &caps["bang"];
        let text = &caps["text"];
        let link = caps["link"].trim();
        if !bang.is_empty() {
            return format!("![{}]({})", text, link); // image — skip
        }
        if link == old_full {
            format!("[{}]({})", text, new_full)
        } else if link == old_name {
            format!("[{}]({})", text, new_name)
        } else {
            format!("[{}]({})", text, link)
        }
    });

    let result = after_links.to_string();
    let changed = result != md_text;
    (result, changed)
}

/// Process image links in already-converted markdown, calling `resolver` for each image.
///
/// `resolver(alt_text, raw_path) -> (resolved_path_in_markdown, NoteLink)`:
/// - receives the alt text and the raw path/URL from the markdown
/// - returns the path string to embed in the output markdown, and the link to record
///
/// Non-image links pass through unchanged.
pub(crate) fn process_image_links<F>(md_text: &str, mut resolver: F) -> (String, Vec<super::NoteLink>)
where
    F: FnMut(&str, &str) -> (String, super::NoteLink),
{
    let mut image_links = vec![];
    let result = MD_LINK_RX.replace_all(md_text, |caps: &Captures| {
        let bang = &caps["bang"];
        let text = &caps["text"];
        let link = caps["link"].trim();

        if bang.is_empty() {
            // Not an image — pass through unchanged
            return format!("[{}]({})", text, link);
        }

        let (resolved_path, note_link) = resolver(text, link);
        image_links.push(note_link);
        format!("![{}]({})", text, resolved_path)
    });
    (result.to_string(), image_links)
}

pub fn extract_title<S: AsRef<str>>(md_text: S) -> String {
    let (_frontmatter, md_text) = remove_frontmatter(md_text);
    let mut parser = Parser::new(md_text.as_ref());
    let result = loop_events(&mut parser);

    result
        .iter()
        .find_map(|tt| match tt {
            TextLine::Empty => None,
            TextLine::Header(_level, text) => Some(text.to_owned()),
            TextLine::Text(text) => Some(text.to_owned()),
            TextLine::ListItem(_level, text) => Some(text.to_owned()),
        })
        .unwrap_or_default()
}

fn parse_text(md_text: &str) -> Vec<ContentChunk> {
    let mut content_chunks = vec![];
    let mut current_breadcrumb: Vec<(u8, String)> = vec![];
    let mut current_content = vec![];

    let mut parser = Parser::new(md_text);
    let result = loop_events(&mut parser);

    for text_line in result {
        match text_line {
            TextLine::Header(level, text) => {
                // Save current chunk if we have content
                if !current_breadcrumb.is_empty() || !current_content.is_empty() {
                    let content = crate::utilities::remove_diacritics(&current_content.join("\n"));
                    if !content.trim().is_empty() {
                        content_chunks.push(ContentChunk {
                            breadcrumb: current_breadcrumb.iter().map(|(_, t)| t.clone()).collect(),
                            text: content,
                        });
                    }
                }

                // Update breadcrumb for new header
                current_breadcrumb.retain(|(lvl, _)| *lvl < level);
                current_breadcrumb.push((level, text));
                current_content.clear();
            }
            TextLine::Empty => {
                // Skip empty lines
            }
            _ => {
                current_content.push(text_line.to_text());
            }
        }
    }

    // Save final chunk
    if !current_breadcrumb.is_empty() || !current_content.is_empty() {
        let content = crate::utilities::remove_diacritics(&current_content.join("\n"));
        if !content.trim().is_empty() {
            content_chunks.push(ContentChunk {
                breadcrumb: current_breadcrumb.iter().map(|(_, t)| t.clone()).collect(),
                text: content,
            });
        }
    }

    content_chunks
}

fn remove_frontmatter<S: AsRef<str>>(text: S) -> (String, String) {
    let mut lines = text.as_ref().lines();

    let Some(first_line) = lines.next() else {
        return (String::new(), String::new());
    };

    if first_line != "---" && first_line != "+++" {
        return (String::new(), text.as_ref().to_string());
    }

    let delimiter = first_line;
    let mut frontmatter = vec![];
    let mut content = vec![];
    let mut closed_fm = false;

    for line in lines {
        if line == delimiter && !closed_fm {
            closed_fm = true;
        } else if closed_fm {
            content.push(line);
        } else {
            frontmatter.push(line);
        }
    }

    if closed_fm {
        (frontmatter.join("\n"), content.join("\n"))
    } else {
        (String::new(), frontmatter.join("\n"))
    }
}

#[derive(Debug, Default, Clone)]
enum TextLine {
    #[default]
    Empty,
    Header(u8, String),
    Text(String),
    ListItem(u8, String),
}

impl TextLine {
    fn append_text(&self, text: String) -> TextLine {
        match self {
            TextLine::Empty => TextLine::Text(text),
            TextLine::Header(level, header_text) => {
                TextLine::Header(*level, format!("{}{}", header_text, text))
            }
            TextLine::Text(line_text) => TextLine::Text(format!("{}{}", line_text, text)),
            TextLine::ListItem(level, item_text) => {
                TextLine::ListItem(*level, format!("{}{}", item_text, text))
            }
        }
    }

    fn to_text(&self) -> String {
        match self {
            TextLine::Empty => String::new(),
            TextLine::Header(level, text) => {
                format!("{} {}", "#".repeat(*level as usize), text)
            }
            TextLine::Text(text) => text.to_owned(),
            TextLine::ListItem(level, text) => {
                format!("{}* {}", " ".repeat((*level as usize) * 4), text)
            }
        }
    }

    fn trim(&self) -> Self {
        match self {
            TextLine::Empty => TextLine::Empty,
            TextLine::Header(level, text) => {
                TextLine::Header(*level, text.trim().to_string())
            }
            TextLine::Text(text) => TextLine::Text(text.trim().to_string()),
            TextLine::ListItem(level, text) => {
                TextLine::ListItem(*level, text.trim().to_string())
            }
        }
    }
}

fn loop_events(parser: &mut Parser) -> Vec<TextLine> {
    let mut text_lines: Vec<TextLine> = vec![];
    let mut tag_stack = vec![];

    for event in parser.by_ref() {
        match event {
            Event::Start(tag) => {
                let current_line = text_lines.pop().unwrap_or_default();
                let new_lines = parse_tag(&tag, current_line);
                text_lines.extend(new_lines);
                tag_stack.push(tag);
            }
            Event::End(tag_end) => {
                let Some(start_tag) = tag_stack.pop() else {
                    debug!("Non matching tag end (empty stack): {:?}", tag_end);
                    continue;
                };

                if tag_end != start_tag.to_end() {
                    debug!("Non matching tags: expected {:?}, got {:?}", start_tag.to_end(), tag_end);
                    tag_stack.push(start_tag);
                    continue;
                }

                let current_line = text_lines.pop().unwrap_or_default();
                let new_lines = parse_tag_end(&tag_end, current_line);
                text_lines.extend(new_lines);
            }
            Event::Text(cow_str) => {
                let last_text = text_lines.pop().unwrap_or_default();
                text_lines.push(last_text.append_text(cow_str.to_string()));
            }
            Event::Code(cow_str) => {
                let current_line = text_lines.pop().unwrap_or_default();
                text_lines.push(current_line.append_text(format!("`{}`", cow_str)));
            }
            Event::InlineMath(cow_str)
            | Event::DisplayMath(cow_str)
            | Event::Html(cow_str)
            | Event::InlineHtml(cow_str)
            | Event::FootnoteReference(cow_str) => {
                text_lines.push(TextLine::Text(cow_str.to_string()));
            }
            Event::SoftBreak => {
                text_lines.push(TextLine::Empty);
            }
            Event::HardBreak => {
                text_lines.push(TextLine::Empty);
                text_lines.push(TextLine::Empty);
            }
            Event::Rule => {
                text_lines.push(TextLine::Empty);
            }
            Event::TaskListMarker(result) => {
                text_lines.push(TextLine::Text(result.to_string()));
            }
        }
    }

    text_lines
}

fn parse_tag(tag: &Tag, current_line: TextLine) -> Vec<TextLine> {
    match tag {
        Tag::Heading { level, .. } => {
            let level = match level {
                pulldown_cmark::HeadingLevel::H1 => 1,
                pulldown_cmark::HeadingLevel::H2 => 2,
                pulldown_cmark::HeadingLevel::H3 => 3,
                pulldown_cmark::HeadingLevel::H4 => 4,
                pulldown_cmark::HeadingLevel::H5 => 5,
                pulldown_cmark::HeadingLevel::H6 => 6,
            };
            vec![current_line, TextLine::Header(level, String::new())]
        }
        Tag::Link { .. } => {
            // Link text arrives via Event::Text; nothing to prepend here.
            vec![current_line]
        }
        Tag::Image { .. } => {
            // Alt text arrives via Event::Text; nothing to prepend here.
            vec![current_line]
        }
        Tag::CodeBlock(kind) => {
            let open = match kind {
                pulldown_cmark::CodeBlockKind::Indented => "```".to_string(),
                pulldown_cmark::CodeBlockKind::Fenced(lang) => format!("```{}", lang),
            };
            vec![TextLine::Text(open), TextLine::Empty]
        }
        Tag::List(_) => {
            let line = if let TextLine::ListItem(lvl, _) = current_line {
                TextLine::ListItem(lvl + 1, String::new())
            } else {
                TextLine::ListItem(0, String::new())
            };
            vec![current_line, line]
        }
        Tag::Item => {
            match &current_line {
                TextLine::ListItem(lvl, text) => {
                    let lvl = *lvl;
                    if text.is_empty() {
                        vec![current_line]
                    } else {
                        vec![current_line, TextLine::ListItem(lvl, String::new())]
                    }
                }
                _ => vec![TextLine::ListItem(0, String::new())],
            }
        }
        Tag::Paragraph => {
            vec![current_line, TextLine::Empty]
        }
        Tag::Strong | Tag::Emphasis | Tag::Strikethrough | Tag::Subscript | Tag::Superscript => {
            vec![current_line]
        }
        Tag::BlockQuote(_) => {
            vec![current_line]
        }
        _ => {
            vec![current_line]
        }
    }
}

fn parse_tag_end(tag_end: &TagEnd, current_line: TextLine) -> Vec<TextLine> {
    match tag_end {
        TagEnd::CodeBlock => {
            vec![current_line.trim(), TextLine::Text("```".to_string())]
        }
        TagEnd::List(_) => {
            if let TextLine::ListItem(lvl, text) = &current_line {
                let last_line = if *lvl > 0 {
                    TextLine::ListItem(lvl - 1, String::new())
                } else {
                    TextLine::Empty
                };

                if text.is_empty() {
                    vec![last_line]
                } else {
                    vec![current_line, last_line]
                }
            } else {
                vec![current_line]
            }
        }
        TagEnd::Paragraph => {
            vec![current_line, TextLine::Empty]
        }
        _ => {
            vec![current_line]
        }
    }
}

#[cfg(test)]
mod test {
    use log::debug;

    use crate::{
        nfs::VaultPath,
        note::{
            content_extractor::{get_content_chunks, get_content_data},
            LinkType,
        },
    };

    use super::{get_markdown_and_links, replace_note_links};

    // ---- replace_note_links tests ----

    #[test]
    fn replace_wikilink_no_display() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("See [[old-note]].", &old, &new);
        assert!(changed);
        assert_eq!(result, "See [[new-note]].");
    }

    #[test]
    fn replace_wikilink_with_display_text() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("See [[old-note|my note]].", &old, &new);
        assert!(changed);
        assert_eq!(result, "See [[new-note|my note]].");
    }

    #[test]
    fn replace_markdown_link_full_path() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) =
            replace_note_links("[click](/notes/old-note.md)", &old, &new);
        assert!(changed);
        assert_eq!(result, "[click](/notes/new-note.md)");
    }

    #[test]
    fn replace_markdown_link_filename_only() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("[click](old-note.md)", &old, &new);
        assert!(changed);
        assert_eq!(result, "[click](new-note.md)");
    }

    #[test]
    fn replace_does_not_touch_unrelated_links() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let text = "[[other-note]] [x](/notes/unrelated.md) [y](unrelated.md)";
        let (result, changed) = replace_note_links(text, &old, &new);
        assert!(!changed);
        assert_eq!(result, text);
    }

    #[test]
    fn replace_does_not_touch_images() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        // Images that happen to match the name must not be touched
        let text = "![old-note.md](old-note.md)";
        let (result, changed) = replace_note_links(text, &old, &new);
        assert!(!changed);
        assert_eq!(result, text);
    }

    #[test]
    fn replace_mixed_content() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/archive/new-note.md");
        let text =
            "[[old-note]] and [[old-note|read this]] plus [link](/notes/old-note.md) end.";
        let (result, changed) = replace_note_links(text, &old, &new);
        assert!(changed);
        assert_eq!(
            result,
            "[[new-note]] and [[new-note|read this]] plus [link](/archive/new-note.md) end."
        );
    }

    #[test]
    fn replace_returns_unchanged_false_when_no_match() {
        let old = VaultPath::new("/notes/missing.md");
        let new = VaultPath::new("/notes/also-missing.md");
        let text = "No references here at all.";
        let (result, changed) = replace_note_links(text, &old, &new);
        assert!(!changed);
        assert_eq!(result, text);
    }

    #[test]
    fn convert_wiki_link() {
        let markdown = r#"Here is a [[Wikilink|text with link]]"#;

        let (md, _) = get_markdown_and_links(&VaultPath::root(), markdown);

        assert_eq!(md, "Here is a [text with link](wikilink.md)");
    }

    #[test]
    fn convert_many_wiki_links() {
        let markdown = r#"Here is a [[Wikilink|text with link]], and another [[Link]] this time without text.

    And a [[https://example.com|url link]]"#;

        let (md, _) = get_markdown_and_links(&VaultPath::root(), markdown);

        assert_eq!(
            md,
            r#"Here is a [text with link](wikilink.md), and another [Link](link.md) this time without text.

    And a [[https://example.com|url link]]"#
        );
    }

    #[test]
    fn ignore_image_links() {
        let markdown = r#"This is an ![image](image.png)"#;

        let (_md, links) = get_markdown_and_links(&VaultPath::root(), markdown);

        assert!(links.is_empty());
    }

    #[test]
    fn extract_relative_link_from_text() {
        let markdown =
            r#"This is a [link](../main.md) to a note, this is a [non](:caca) valid link"#;
        let note_path = VaultPath::new("/directory/test_note.md");

        let (_md, links) = get_markdown_and_links(&note_path, markdown);

        assert_eq!(1, links.len());
        let link = links.first().unwrap();
        assert_eq!("link", link.text);
        assert_eq!(LinkType::Note(VaultPath::new("/main.md")), link.ltype);
    }

    #[test]
    fn extract_link_from_text() {
        let markdown =
            r#"This is a [link](notes/main.md) to a note, this is a [non](:caca) valid link"#;

        let note_path = VaultPath::new("/test_note.md");
        let (_md, links) = get_markdown_and_links(&note_path, markdown);

        assert_eq!(1, links.len());
        let link = links.first().unwrap();
        assert_eq!("link", link.text);
        assert_eq!(LinkType::Note(VaultPath::new("/notes/main.md")), link.ltype);
    }

    #[test]
    fn extract_many_links_from_text() {
        let markdown = r#"This is a [link](notes/main.md) to a note, this is a [[note.md]]] valid link

    Here's a [url](https://www.example.com)"#;

        let note_path = VaultPath::new("/test_note.md");
        let (_md, links) = get_markdown_and_links(&note_path, markdown);

        assert_eq!(3, links.len());
        // Now has an absolute path
        assert!(links.iter().any(|link| {
            let path = VaultPath::new("/notes/main.md");
            link.text.eq("link") && link.ltype.eq(&LinkType::Note(path))
        }));
        assert!(links.iter().any(|link| {
            let path = VaultPath::new("note.md");
            link.text.eq("note.md") && link.ltype.eq(&LinkType::Note(path))
        }));
        assert!(links.iter().any(|link| {
            debug!("{:?}", link);
            let url = "https://www.example.com".to_string();
            link.text.eq("url") && link.ltype.eq(&LinkType::Url) && link.raw_link.eq(&url)
        }));
    }

    #[test]
    fn check_title_yaml_frontmatter() {
        let markdown = r#"---
something: nice
other: else
---

title"#;
        let content_chunks = get_content_chunks(markdown);

        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!("title", content_chunks[0].get_text());
        assert_eq!("FrontMatter", content_chunks[1].get_breadcrumb());
        assert_eq!("something: nice\nother: else", content_chunks[1].get_text());
    }

    #[test]
    fn check_title_toml_frontmatter() {
        let markdown = r#"+++
something: nice
other: else
+++

title"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(2, content_chunks.len());
        assert_eq!("title".to_string(), data.title);
        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!("title", content_chunks[0].get_text());
        assert_eq!("FrontMatter", content_chunks[1].get_breadcrumb());
        assert_eq!("something: nice\nother: else", content_chunks[1].get_text());
    }

    #[test]
    fn check_title_in_list() {
        let markdown = r#"- First Item
- Second Item

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("First Item".to_string(), data.title);
        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "* First Item\n* Second Item\nSome text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn convert_list() {
        let markdown = r#"# Title

- First *Item*
- Second Item

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "* First Item\n* Second Item\nSome text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn convert_list_two_level() {
        let markdown = r#"# Title

- First Item
    - First subitem
    - Second subitem
- Second Item

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "* First Item\n    * First subitem\n    * Second subitem\n* Second Item\nSome text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn convert_list_empty_item() {
        let markdown = r#"# Title

- First Item
- Second Item
-

"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("* First Item\n* Second Item", content_chunks[0].get_text());
    }

    #[test]
    fn check_title_no_header() {
        let markdown = r#"[No header](https://example.com)

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("No header".to_string(), data.title);
        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!("No header\nSome text", content_chunks[0].get_text());
    }

    #[test]
    fn check_hierarchy_one() {
        let markdown = r#"# Title
Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
    }

    #[test]
    fn check_hierarchy_two() {
        let markdown = r#"# Title
Some text

## Subtitle
More text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(2, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", content_chunks[1].get_breadcrumb());
        assert_eq!("More text", content_chunks[1].get_text());
    }

    #[test]
    fn check_hierarchy_three() {
        let markdown = r#"# Title
Some text

## Subtitle
More text

### Subsubtitle
Even more text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(3, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", content_chunks[1].get_breadcrumb());
        assert_eq!("More text", content_chunks[1].get_text());
        assert_eq!(
            "Title>Subtitle>Subsubtitle",
            content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", content_chunks[2].get_text());
    }

    #[test]
    fn check_nested_hierarchy_three() {
        let markdown = r#"# Title
Some text

## Subtitle
More text

### Subsubtitle
Even more text

## Level 2 Title
There is text here"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(4, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", content_chunks[1].get_breadcrumb());
        assert_eq!("More text", content_chunks[1].get_text());
        assert_eq!(
            "Title>Subtitle>Subsubtitle",
            content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", content_chunks[2].get_text());
        assert_eq!("Title>Level 2 Title", content_chunks[3].get_breadcrumb());
        assert_eq!("There is text here", content_chunks[3].get_text());
    }

    #[test]
    fn check_nested_hierarchy_four() {
        let markdown = r#"# Title
Some text

## Subtitle
More text

### Subsubtitle
Even more text

## Level 2 Title
There is text here

### Fourth Subsubtitle
Before last text

# Main Title
Another main content
"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(6, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", content_chunks[1].get_breadcrumb());
        assert_eq!("More text", content_chunks[1].get_text());
        assert_eq!(
            "Title>Subtitle>Subsubtitle",
            content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", content_chunks[2].get_text());
        assert_eq!("Title>Level 2 Title", content_chunks[3].get_breadcrumb());
        assert_eq!("There is text here", content_chunks[3].get_text());
        assert_eq!(
            "Title>Level 2 Title>Fourth Subsubtitle",
            content_chunks[4].get_breadcrumb()
        );
        assert_eq!("Before last text", content_chunks[4].get_text());
        assert_eq!("Main Title", content_chunks[5].get_breadcrumb());
        assert_eq!("Another main content", content_chunks[5].get_text());
    }

    #[test]
    fn check_nested_hierarchy_four_jump() {
        let markdown = r#"# Title
Some text

### Subtitle
More text

# Subsubtitle
Even more text

#### Level 2 Title
There is text here

## Fourth Subsubtitle
Before last text

# Main Title
Another main content
"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(6, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", content_chunks[1].get_breadcrumb());
        assert_eq!("More text", content_chunks[1].get_text());
        assert_eq!("Subsubtitle", content_chunks[2].get_breadcrumb());
        assert_eq!("Even more text", content_chunks[2].get_text());
        assert_eq!(
            "Subsubtitle>Level 2 Title",
            content_chunks[3].get_breadcrumb()
        );
        assert_eq!("There is text here", content_chunks[3].get_text());
        assert_eq!(
            "Subsubtitle>Fourth Subsubtitle",
            content_chunks[4].get_breadcrumb()
        );
        assert_eq!("Before last text", content_chunks[4].get_text());
        assert_eq!("Main Title", content_chunks[5].get_breadcrumb());
        assert_eq!("Another main content", content_chunks[5].get_text());
    }

    #[test]
    fn check_title_with_link() {
        let markdown = r#"# [Title link](https://nico.red)
Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title link".to_string(), data.title);
        assert_eq!("Title link", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
    }

    #[test]
    fn check_title_with_style() {
        let markdown = r#"# Title **bold** *italic*
Some text"#;
        let content_chunks = get_content_chunks(markdown);
        debug!("===================================");
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title bold italic".to_string(), data.title);
        assert_eq!("Title bold italic", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
    }

    #[test]
    fn check_content_without_title() {
        let markdown = r#"Intro text

# Title

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(2, content_chunks.len());
        assert_eq!("Intro text".to_string(), data.title);
        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!("Intro text", content_chunks[0].get_text());
        assert_eq!("Title", content_chunks[1].get_breadcrumb());
        assert_eq!("Some text", content_chunks[1].get_text());
    }

    #[test]
    fn check_content_with_link() {
        let markdown = r#"# Title

[Some text linking](www.example.com)"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text linking", content_chunks[0].get_text());
    }

    #[test]
    fn check_content_with_wikilink() {
        let markdown = r#"# Title

[[Some text linking]]"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text linking", content_chunks[0].get_text());
    }

    #[test]
    fn check_content_with_hashtags() {
        let markdown = r#"# Title

Some text, #hashtag and more text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "Some text, hashtag and more text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn check_code() {
        let markdown = r#"# Title

Some text, `code` and more text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "Some text, `code` and more text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn check_code_block() {
        let markdown = r#"# Title

Some text

```bash
mkdir test
ls -la ./test
```"#;

        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "Some text\n```bash\nmkdir test\nls -la ./test\n```",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn extract_hashtags_as_links() {
        let markdown = r#"Some text with #hashtag and another #tag123"#;

        let (md, links) = get_markdown_and_links(&VaultPath::root(), markdown);

        assert_eq!(2, links.len());
        assert!(links.iter().any(|link| {
            link.text.eq("hashtag")
                && link.ltype.eq(&LinkType::Hashtag)
                && link.raw_link.eq("#hashtag")
        }));
        assert!(links.iter().any(|link| {
            link.text.eq("tag123")
                && link.ltype.eq(&LinkType::Hashtag)
                && link.raw_link.eq("#tag123")
        }));
        assert_eq!(md, "Some text with [#hashtag](#hashtag) and another [#tag123](#tag123)");
    }

    // --- get_content_chunks: new / regression tests ---

    #[test]
    fn empty_note_produces_no_chunks() {
        let chunks = get_content_chunks("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn only_frontmatter_produces_one_chunk() {
        let markdown = "---\ntitle: Hello\n---";
        let chunks = get_content_chunks(markdown);
        // Only the FrontMatter chunk; no body content.
        assert_eq!(1, chunks.len());
        assert_eq!("FrontMatter", chunks[0].get_breadcrumb());
    }

    #[test]
    fn adjacent_headers_no_empty_chunks() {
        // When two headers are back-to-back the first produces no body text,
        // so no empty chunk should be emitted.
        let markdown = "# Title\n## Subtitle\nSome text";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!("Title>Subtitle", chunks[0].get_breadcrumb());
        assert_eq!("Some text", chunks[0].get_text());
    }

    #[test]
    fn link_with_title_attribute_keeps_only_link_text() {
        // [text](url "title") — "title" must NOT appear in the chunk content.
        let markdown = "# Section\n[visit here](https://example.com \"My Site\")";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!("visit here", chunks[0].get_text());
    }

    #[test]
    fn image_alt_text_is_kept_not_title() {
        // ![alt](img.png "tooltip") — only alt text should appear.
        let markdown = "# Section\n![an image](photo.png \"Photo title\")";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!("an image", chunks[0].get_text());
    }

    #[test]
    fn wikilink_multi_pipe_uses_display_text() {
        // [[link|display|extra]] — display text should be kept, extra part ignored.
        let markdown = "# Section\n[[note|display text|ignored extra]]";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!("display text", chunks[0].get_text());
    }

    #[test]
    fn header_only_note_no_body_chunk() {
        // A note that is only a header line with no following text.
        let markdown = "# Just a title";
        let chunks = get_content_chunks(markdown);
        // No body text → no chunk should be emitted.
        assert!(chunks.is_empty());
    }

    #[test]
    fn deeply_nested_headers() {
        let markdown = "# H1\n## H2\n### H3\n#### H4\ntext";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!("H1>H2>H3>H4", chunks[0].get_breadcrumb());
        assert_eq!("text", chunks[0].get_text());
    }

    #[test]
    fn unclosed_frontmatter_treated_as_body() {
        // An unclosed frontmatter block should not swallow the whole document.
        let markdown = "---\ntitle: Hello\nSome actual content";
        let chunks = get_content_chunks(markdown);
        // No FrontMatter chunk (delimiter never closed),
        // body should contain the remaining lines.
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.get_breadcrumb() != "FrontMatter"));
    }

    #[test]
    fn extract_mixed_links_and_hashtags() {
        let markdown = r#"This is a [link](note.md) and #hashtag with [[wikilink]] and #another_tag"#;

        let note_path = VaultPath::new("/test_note.md");
        let (_md, links) = get_markdown_and_links(&note_path, markdown);

        assert_eq!(4, links.len());
        // Check for note links
        assert_eq!(
            2,
            links
                .iter()
                .filter(|l| matches!(l.ltype, LinkType::Note(_)))
                .count()
        );
        // Check for hashtags
        assert_eq!(
            2,
            links
                .iter()
                .filter(|l| matches!(l.ltype, LinkType::Hashtag))
                .count()
        );
        assert!(links.iter().any(|link| link.text.eq("hashtag")
            && link.ltype.eq(&LinkType::Hashtag)));
        assert!(links.iter().any(|link| link.text.eq("another_tag")
            && link.ltype.eq(&LinkType::Hashtag)));
    }
}
