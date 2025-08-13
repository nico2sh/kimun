use log::{debug, warn};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use regex::{Captures, Regex};

use crate::{
    nfs::{self, VaultPath},
    note::{ContentChunk, NoteContentData},
};

use super::NoteLink;

const _MAX_TITLE_LENGTH: usize = 40;
const REGEX_WIKILINK: &str = r#"(?:\[\[(?P<link_text>[^\]]+)\]\])"#;
const REGEX_HASHTAG: &str = r#"#(?P<ht_text>[A-Za-z0-9_]+)"#;

pub fn get_content_data<S: AsRef<str>>(md_text: S) -> NoteContentData {
    let hash = nfs::hash_text(md_text.as_ref());
    let title = extract_title(md_text);

    NoteContentData { title, hash }
}

pub fn get_content_chunks<S: AsRef<str>>(md_text: S) -> Vec<ContentChunk> {
    let (frontmatter, text) = remove_frontmatter(md_text.as_ref());

    let text = cleanup_hashtags(cleanup_wikilinks(text));

    let mut content_chunks = parse_text(&text);
    if !frontmatter.is_empty() {
        content_chunks.push(ContentChunk {
            breadcrumb: vec!["FrontMatter".to_string()],
            text: frontmatter,
        })
    };
    content_chunks
}

fn cleanup_wikilinks<S: AsRef<str>>(md_text: S) -> String {
    let rx = Regex::new(REGEX_WIKILINK).unwrap();
    let text = rx
        .replace_all(md_text.as_ref(), |caps: &Captures| {
            let items = &caps["link_text"];
            let link_text = items.split("|").collect::<Vec<&str>>();
            let text = match link_text.len() {
                1 => link_text[0],
                2 => link_text[1],
                _ => "",
            };
            text.to_string()
        })
        .into_owned();
    text
}

fn cleanup_hashtags<S: AsRef<str>>(md_text: S) -> String {
    let rx = Regex::new(REGEX_HASHTAG).unwrap();
    let text = rx
        .replace_all(md_text.as_ref(), |caps: &Captures| {
            let text = &caps["ht_text"];
            text.to_string()
        })
        .into_owned();
    text
}

// Convert any wikilink into a link to a note, only note links
fn convert_wikilinks<S: AsRef<str>>(md_text: S) -> String {
    let rx = Regex::new(REGEX_WIKILINK).unwrap();
    let text = rx
        .replace_all(md_text.as_ref(), |caps: &Captures| {
            let items = &caps["link_text"];
            let link_text = items.split("|").collect::<Vec<&str>>();
            let (link, text) = match link_text.len() {
                1 => (link_text[0], link_text[0]),
                2 => (link_text[0], link_text[1]),
                _ => ("", ""),
            };
            if !link.is_empty() && VaultPath::is_valid(link) {
                let link = VaultPath::note_path_from(link);
                format!("[{}]({})", text, link)
            } else {
                format!("[[{}]]", items)
            }
        })
        .into_owned();
    text
}

/// Returns the converted text into Markdown (replacing note wikilinks to markdown links)
/// Normalizes the links urls when needed (lowercasing the path for vault paths)
/// And a list of the links existing in the note, relative links are transformed to absolute links.
pub fn get_markdown_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (String, Vec<NoteLink>) {
    let md_text = convert_wikilinks(md_text);
    let mut links = vec![];
    let md_link_regex = r#"(?P<bang>!?)(?:\[(?P<text>[^\]]+)\])\((?P<link>[^\)]+?)\)"#;
    let url_regex = r#"^https?:\/\/[\w\d]+\.[\w\d]+(?:(?:\.[\w\d]+)|(?:[\w\d\/?=#]+))+$"#;

    let rx = Regex::new(md_link_regex).unwrap();
    let clean_md_text = rx.replace_all(md_text.as_ref(), |caps: &Captures| {
        let bang = &caps["bang"];
        let text = &caps["text"];
        let link = &caps["link"].trim();
        // We ignore links that start with a `!`, since these are images
        if bang.is_empty() {
            debug!("checking link {}", link);
            // Is it a URL or a local path?
            let rxurl = Regex::new(url_regex).unwrap();
            let clean_link = if rxurl.is_match(link) {
                let url_link = NoteLink::url(link, text);
                links.push(url_link);
                link.to_string()
            } else if VaultPath::is_valid(link) {
                // It is a local path
                let path = VaultPath::new(link);
                if path.is_note_file() {
                    // A single note
                    links.push(NoteLink::note(&path, text));
                    // We return the path as we found it
                    path.to_string()
                } else {
                    // A path to content, we resolve the relative path
                    let ref_path = if reference_path.is_note() {
                        reference_path.get_parent_path().0
                    } else {
                        reference_path.to_owned()
                    };
                    let abs_path = ref_path.append(&path).flatten();
                    if abs_path.is_note() {
                        // Note
                        links.push(NoteLink::note(&abs_path, text));
                    } else {
                        // Attachment
                        links.push(NoteLink::vault_path(&abs_path, text));
                    }
                    // We return the resolved absolute path
                    abs_path.to_string()
                }
            } else {
                debug!("link not counting {}", link);
                link.to_string()
            };
            format!("[{}]({})", text, clean_link)
        } else {
            format!("![{}]({})", text, link)
        }
    });

    (clean_md_text.to_string(), links)
}

pub fn extract_title<S: AsRef<str>>(md_text: S) -> String {
    let (_frontmatter, md_text) = remove_frontmatter(md_text);
    let mut parser = pulldown_cmark::Parser::new(md_text.as_ref());
    let result = loop_events(&mut parser);
    // debug!("{:?}", result);
    let title = result
        .iter()
        .find_map(|tt| match tt {
            TextLine::Empty => None,
            TextLine::Header(_level, text) => Some(text.to_owned()),
            TextLine::Text(text) => Some(text.to_owned()),
            TextLine::ListItem(_level, text) => Some(text.to_owned()),
        })
        .unwrap_or_default();
    title
}

fn parse_text(md_text: &str) -> Vec<ContentChunk> {
    let mut content_chunks = vec![];
    let mut current_breadcrumb: Vec<(u8, String)> = vec![];
    let mut current_content = vec![];

    let mut parser = pulldown_cmark::Parser::new(md_text);
    let result = loop_events(&mut parser);
    for text_line in result {
        match text_line {
            TextLine::Header(level, text) => {
                if !current_breadcrumb.is_empty() || !current_content.is_empty() {
                    let breadcrumb = current_breadcrumb.clone();
                    let content =
                        crate::utilities::remove_diacritics(&current_content.clone().join("\n"));
                    content_chunks.push(ContentChunk {
                        breadcrumb: breadcrumb.into_iter().map(|c| c.1).collect(),
                        text: content,
                    });
                }
                while !current_breadcrumb.is_empty()
                    && current_breadcrumb.last().unwrap().0 >= level
                {
                    current_breadcrumb.remove(current_breadcrumb.len() - 1);
                }
                current_breadcrumb.push((level, text));
                current_content.clear();
            }
            TextLine::Empty => {
                // We do nothing
            }
            _ => current_content.push(text_line.to_text()),
        }
    }

    if !current_breadcrumb.is_empty() || !current_content.is_empty() {
        let content = crate::utilities::remove_diacritics(&current_content.clone().join("\n"));
        content_chunks.push(ContentChunk {
            breadcrumb: current_breadcrumb
                .into_iter()
                .map(|c| c.1.clone())
                .collect(),
            text: content,
        });
    }

    content_chunks
}

fn remove_frontmatter<S: AsRef<str>>(text: S) -> (String, String) {
    let mut lines = text.as_ref().lines();
    let first_line = lines.next();
    if let Some(line) = first_line {
        if line == "---" || line == "+++" {
            let close = line;
            let mut frontmatter = vec![];
            let mut content = vec![];
            let mut closed_fm = false;
            for next_line in lines {
                if next_line == close {
                    closed_fm = true;
                } else if closed_fm {
                    content.push(next_line);
                } else {
                    frontmatter.push(next_line);
                }
            }
            if closed_fm {
                (frontmatter.join("\n"), content.join("\n"))
            } else {
                ("".to_string(), frontmatter.join("\n"))
            }
        } else {
            ("".to_string(), text.as_ref().to_string())
        }
    } else {
        ("".to_string(), "".to_string())
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
                TextLine::Header(level.to_owned(), format!("{}{}", header_text, text))
            }
            TextLine::Text(line_text) => TextLine::Text(format!("{}{}", line_text, text)),
            TextLine::ListItem(level, item_text) => {
                TextLine::ListItem(level.to_owned(), format!("{}{}", item_text, text))
            }
        }
    }

    fn to_text(&self) -> String {
        match self {
            TextLine::Empty => "".to_string(),
            TextLine::Header(level, text) => {
                format!("{} {}", "#".repeat(level.to_owned().into()), text)
            }
            TextLine::Text(text) => text.to_owned(),
            TextLine::ListItem(level, text) => {
                format!("{}* {}", " ".repeat((level.to_owned() * 4).into()), text)
            }
        }
    }

    fn trim(&self) -> Self {
        match self {
            TextLine::Empty => TextLine::Empty,
            TextLine::Header(level, text) => {
                TextLine::Header(level.to_owned(), text.trim().to_string())
            }
            TextLine::Text(text) => TextLine::Text(text.trim().to_string()),
            TextLine::ListItem(level, text) => {
                TextLine::ListItem(level.to_owned(), text.trim().to_string())
            }
        }
    }
}

fn loop_events(parser: &mut Parser) -> Vec<TextLine> {
    let mut text_lines: Vec<TextLine> = vec![];
    let mut tag_stack = vec![];
    for event in parser.by_ref() {
        // debug!("TEXT LINES BEFORE: {:?}", text_lines);
        // debug!("EVENT: {:?}", event);
        match event {
            Event::Start(tag) => {
                // debug!(
                //     "FOUND TAG: {:?}\n -> CURRENT ELEMENT LIST: {:?}",
                //     tag, text_lines
                // );
                let tag = tag.to_owned();
                let current_line = text_lines.pop().unwrap_or_default();
                let new_lines = parse_tag(&tag, current_line);
                for l in new_lines {
                    text_lines.push(l);
                }
                tag_stack.push(tag);
                // We get the current text
                // let last_line = text_lines.pop().unwrap_or_default();
                // let mut tag_text = parse_tag(&tag, parser, last_line);
                // text_lines.append(&mut tag_text);
            }
            Event::End(tag_end) => {
                if let Some(end) = tag_stack.pop() {
                    if tag_end != end.to_end() {
                        panic!("Non Matching Tags: {:?}", tag_end);
                    } else {
                        let current_line = text_lines.pop().unwrap_or_default();
                        let new_lines = parse_tag_end(&tag_end, current_line);
                        for l in new_lines {
                            text_lines.push(l);
                        }
                    }
                } else {
                    panic!("Non Matching Tags: {:?}", tag_end);
                }
            }
            Event::Text(cow_str) => {
                let last_text = text_lines.pop().unwrap_or_default();
                text_lines.push(last_text.append_text(cow_str.to_string()));
            }
            Event::Code(cow_str) => {
                let current_line = text_lines.pop().unwrap_or_default();
                text_lines.push(current_line.append_text(format!("`{}`", cow_str)));
            }
            Event::InlineMath(cow_str) => {
                text_lines.push(TextLine::Text(cow_str.to_string()));
            }
            Event::DisplayMath(cow_str) => {
                text_lines.push(TextLine::Text(cow_str.to_string()));
            }
            Event::Html(cow_str) => {
                text_lines.push(TextLine::Text(cow_str.to_string()));
            }
            Event::InlineHtml(cow_str) => {
                text_lines.push(TextLine::Text(cow_str.to_string()));
            }
            Event::FootnoteReference(cow_str) => {
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
        // debug!("TEXT LINES AFTER: {:?}", text_lines);
    }
    text_lines
}

fn parse_tag(tag: &Tag, current_line: TextLine) -> Vec<TextLine> {
    match tag {
        Tag::Heading {
            level,
            id: _,
            classes: _,
            attrs: _,
        } => {
            // debug!("TEXT LINE: {:?}", text_type);
            let level = match level {
                pulldown_cmark::HeadingLevel::H1 => 1,
                pulldown_cmark::HeadingLevel::H2 => 2,
                pulldown_cmark::HeadingLevel::H3 => 3,
                pulldown_cmark::HeadingLevel::H4 => 4,
                pulldown_cmark::HeadingLevel::H5 => 5,
                pulldown_cmark::HeadingLevel::H6 => 6,
            };
            vec![current_line, TextLine::Header(level, "".to_string())]
        }
        Tag::Link {
            link_type: _,
            dest_url: _,
            title,
            id: _,
        } => {
            vec![current_line.append_text(title.to_string())]
        }
        Tag::Image {
            link_type: _,
            dest_url: _,
            title,
            id: _,
        } => {
            vec![current_line.append_text(title.to_string())]
        }
        Tag::CodeBlock(kind) => {
            let open = match kind {
                pulldown_cmark::CodeBlockKind::Indented => "```".to_string(),
                pulldown_cmark::CodeBlockKind::Fenced(cow_str) => {
                    format!("```{}", cow_str)
                }
            };
            vec![TextLine::Text(open), TextLine::Empty]
        }
        Tag::List(_number) => {
            let line = if let TextLine::ListItem(lvl, _) = current_line {
                TextLine::ListItem(lvl + 1, "".to_string())
            } else {
                TextLine::ListItem(0, "".to_string())
            };
            vec![current_line, line]
        }
        Tag::Item => {
            if let TextLine::ListItem(lvl, text) = &current_line {
                let lvl = lvl.to_owned();
                if text.is_empty() {
                    vec![current_line]
                } else {
                    vec![
                        current_line,
                        TextLine::ListItem(lvl.to_owned(), "".to_string()),
                    ]
                }
            } else {
                vec![TextLine::ListItem(0, "".to_string())]
            }
        }
        Tag::Paragraph => {
            vec![current_line, TextLine::Empty]
        }
        Tag::Strong | Tag::Emphasis | Tag::Strikethrough | Tag::Subscript | Tag::Superscript => {
            // We ignore format
            vec![current_line]
        }
        Tag::BlockQuote(_kind) => {
            vec![current_line]
        }
        _ => {
            // nada
            // debug!("LOOPING IN TAG: {:?}", tag);
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
                let last_line = if lvl > &0 {
                    TextLine::ListItem(lvl - 1, "".to_string())
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
            // nada
            // debug!("LOOPING IN TAG: {:?}", tag_end);
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

    use super::{convert_wikilinks, get_markdown_and_links};

    #[test]
    fn convert_wiki_link() {
        let markdown = r#"Here is a [[Wikilink|text with link]]"#;

        let md = convert_wikilinks(markdown);

        assert_eq!(md, "Here is a [text with link](Wikilink)");
    }

    #[test]
    fn convert_many_wiki_links() {
        let markdown = r#"Here is a [[Wikilink|text with link]], and another [[Link]] this time without text.

    And a [[https://example.com|url link]]"#;

        let md = convert_wikilinks(markdown);

        assert_eq!(
            md,
            r#"Here is a [text with link](Wikilink), and another [Link](Link) this time without text.

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
}
