use std::{cmp::min, fmt::Display};

use log::error;
use pulldown_cmark::{Event, Parser, Tag};

use crate::nfs;

const MAX_TITLE_LENGTH: usize = 40;

pub fn extract_data<S: AsRef<str>>(md_text: S) -> NoteContentData {
    let (frontmatter, text) = remove_frontmatter(md_text.as_ref());

    let mut note_content = parse_text(&text);
    if !frontmatter.is_empty() {
        note_content.content_chunks.push(ContentChunk {
            breadcrumb: vec!["FrontMatter".to_string()],
            text: frontmatter,
        })
    };
    note_content
}

fn parse_text(md_text: &str) -> NoteContentData {
    let hash = nfs::hash_text(md_text);
    let mut title = None;
    let mut content_chunks = vec![];
    let mut current_breadcrumb: Vec<(u8, String)> = vec![];
    let mut current_content = vec![];

    let mut parser = pulldown_cmark::Parser::new(md_text);
    while let Some(event) = parser.next() {
        let tt = match event {
            Event::Start(tag) => parse_start_tag(tag, &mut parser),
            Event::End(_tag_end) => {
                panic!("Non Matching Tags")
            }
            Event::Text(cow_str) => TextType::Text(cow_str.to_string()),
            Event::Code(cow_str) => TextType::Text(cow_str.to_string()),
            Event::InlineMath(cow_str) => TextType::Text(cow_str.to_string()),
            Event::DisplayMath(cow_str) => TextType::Text(cow_str.to_string()),
            Event::Html(cow_str) => TextType::Text(cow_str.to_string()),
            Event::InlineHtml(cow_str) => TextType::Text(cow_str.to_string()),
            Event::FootnoteReference(cow_str) => TextType::Text(cow_str.to_string()),
            Event::SoftBreak => TextType::None,
            Event::HardBreak => TextType::None,
            Event::Rule => TextType::None,
            Event::TaskListMarker(result) => TextType::Text(result.to_string()),
        };

        if title.is_none() {
            let title_cand = match &tt {
                TextType::Header(_, text) => text.to_owned(),
                TextType::Text(text) => text.to_owned(),
                TextType::None => String::new(),
            };
            title = title_cand.lines().next().map(|t| {
                let title_length = min(MAX_TITLE_LENGTH, t.len());
                t.chars().take(title_length).collect()
            });
        }

        match tt {
            TextType::Header(level, text) => {
                if !current_breadcrumb.is_empty() || !current_content.is_empty() {
                    let breadcrumb = current_breadcrumb.clone();
                    let content =
                        super::utilities::remove_diacritics(&current_content.clone().join("\n"));
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
            TextType::Text(text) => {
                current_content.push(text);
            }
            TextType::None => {
                // Don't do anything
            }
        }
    }

    if !current_breadcrumb.is_empty() || !current_content.is_empty() {
        let content = super::utilities::remove_diacritics(&current_content.clone().join("\n"));
        content_chunks.push(ContentChunk {
            breadcrumb: current_breadcrumb
                .into_iter()
                .map(|c| c.1.clone())
                .collect(),
            text: content,
        });
    }
    NoteContentData {
        title,
        hash,
        content_chunks,
    }
}

fn remove_frontmatter(text: &str) -> (String, String) {
    let mut lines = text.lines();
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
            ("".to_string(), text.to_string())
        }
    } else {
        ("".to_string(), "".to_string())
    }
}

enum TextType {
    None,
    Header(u8, String),
    Text(String),
}

fn parse_start_tag(tag: Tag, parser: &mut Parser) -> TextType {
    match tag {
        Tag::Heading {
            level,
            id: _,
            classes: _,
            attrs: _,
        } => {
            let level = match level {
                pulldown_cmark::HeadingLevel::H1 => 1,
                pulldown_cmark::HeadingLevel::H2 => 2,
                pulldown_cmark::HeadingLevel::H3 => 3,
                pulldown_cmark::HeadingLevel::H4 => 4,
                pulldown_cmark::HeadingLevel::H5 => 5,
                pulldown_cmark::HeadingLevel::H6 => 6,
            };
            let text = get_text_till_end(parser);
            TextType::Header(level, text)
        }
        Tag::Link {
            link_type: _,
            dest_url: _,
            title,
            id: _,
        } => {
            let mut text = if title.is_empty() {
                vec![]
            } else {
                vec![title.to_string()]
            };
            text.push(get_text_till_end(parser));
            TextType::Text(text.join(" "))
        }
        Tag::Image {
            link_type: _,
            dest_url: _,
            title,
            id: _,
        } => {
            let mut text = if title.is_empty() {
                vec![]
            } else {
                vec![title.to_string()]
            };
            text.push(get_text_till_end(parser));
            TextType::Text(text.join(" "))
        }
        _ => {
            let text = get_text_till_end(parser);
            TextType::Text(text)
        }
    }
}

fn get_text_till_end(parser: &mut Parser) -> String {
    let mut open_tags = 1;
    let mut text_vec = vec![];
    let mut current_text = String::new();
    while open_tags > 0 {
        let event = &parser.next();
        if let Some(event) = event {
            match event {
                Event::Start(tag) => {
                    let breaks = !matches!(
                        tag,
                        Tag::Emphasis
                            | Tag::Strong
                            | Tag::Link {
                                link_type: _,
                                dest_url: _,
                                title: _,
                                id: _,
                            }
                    );
                    open_tags += 1;
                    if !current_text.is_empty() && breaks {
                        text_vec.push(current_text);
                        current_text = String::new();
                    }
                }
                Event::End(_tag) => {
                    open_tags -= 1;
                }
                Event::Text(cow_str) => current_text.push_str(cow_str.as_ref()),
                Event::Code(cow_str) => current_text.push_str(cow_str.as_ref()),
                Event::InlineMath(cow_str) => current_text.push_str(cow_str.as_ref()),
                Event::DisplayMath(cow_str) => current_text.push_str(cow_str.as_ref()),
                Event::Html(cow_str) => current_text.push_str(cow_str.as_ref()),
                Event::InlineHtml(cow_str) => current_text.push_str(cow_str.as_ref()),
                Event::FootnoteReference(cow_str) => current_text.push_str(cow_str.as_ref()),
                Event::SoftBreak => current_text.push('\n'),
                Event::HardBreak => current_text.push('\n'),
                Event::Rule => current_text.push('\n'),
                Event::TaskListMarker(_) => current_text.push('\n'),
            }
        } else {
            error!("Error parsing markdown");
            open_tags = 0;
        }
    }
    if !current_text.is_empty() {
        text_vec.push(current_text);
    }
    text_vec.join("\n")
}

/// NoteContentData contains tha extracted data from the note
/// for comparison and search in the DB, it is expensive to get
/// so it is not a good idea to calculate it every time the content
/// has changed, but better lazy get it when needed and cache it somewhere
/// (like the DB) for search and access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteContentData {
    pub(super) title: Option<String>,
    pub hash: u64,
    pub content_chunks: Vec<ContentChunk>,
}

impl Display for NoteContentData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Title: {}, Hash: {}, Chunks: [{}]",
            self.title.clone().unwrap_or("<None>".to_string()),
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
    breadcrumb: Vec<String>,
    pub text: String,
}

impl ContentChunk {
    pub fn get_breadcrumb(&self) -> String {
        self.breadcrumb.join(">")
    }

    fn get_text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod test {
    use crate::content_data::extract_data;

    #[test]
    fn check_title_yaml_frontmatter() {
        let markdown = r#"---
something: nice
other: else
---

title"#;
        let ch = extract_data(markdown);

        assert_eq!(2, ch.content_chunks.len());
        assert_eq!(Some("title".to_string()), ch.title);
        assert_eq!("", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("title", ch.content_chunks[0].get_text());
        assert_eq!("FrontMatter", ch.content_chunks[1].get_breadcrumb());
        assert_eq!(
            "something: nice\nother: else",
            ch.content_chunks[1].get_text()
        );
    }

    #[test]
    fn check_title_toml_frontmatter() {
        let markdown = r#"+++
something: nice
other: else
+++

title"#;
        let ch = extract_data(markdown);

        assert_eq!(2, ch.content_chunks.len());
        assert_eq!(Some("title".to_string()), ch.title);
        assert_eq!("", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("title", ch.content_chunks[0].get_text());
        assert_eq!("FrontMatter", ch.content_chunks[1].get_breadcrumb());
        assert_eq!(
            "something: nice\nother: else",
            ch.content_chunks[1].get_text()
        );
    }

    #[test]
    fn check_title_in_list() {
        let markdown = r#"- First Item
- Second Item

Some text"#;
        let ch = extract_data(markdown);

        assert_eq!(1, ch.content_chunks.len());
        assert_eq!(Some("First Item".to_string()), ch.title);
        assert_eq!("", ch.content_chunks[0].get_breadcrumb());
        assert_eq!(
            "First Item\nSecond Item\nSome text",
            ch.content_chunks[0].get_text()
        );
    }

    #[test]
    fn check_title_no_header() {
        let markdown = r#"[No header](https://example.com)

Some text"#;
        let ch = extract_data(markdown);

        assert_eq!(1, ch.content_chunks.len());
        assert_eq!(Some("No header".to_string()), ch.title);
        assert_eq!("", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("No header\nSome text", ch.content_chunks[0].get_text());
    }

    #[test]
    fn check_hierarchy_one() {
        let markdown = r#"# Title
Some text"#;
        let ch = extract_data(markdown);

        assert_eq!(1, ch.content_chunks.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[0].get_text());
    }

    #[test]
    fn check_hierarchy_two() {
        let markdown = r#"# Title
Some text

## Subtitle
More text"#;
        let ch = extract_data(markdown);

        assert_eq!(2, ch.content_chunks.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", ch.content_chunks[1].get_breadcrumb());
        assert_eq!("More text", ch.content_chunks[1].get_text());
    }

    #[test]
    fn check_hierarchy_three() {
        let markdown = r#"# Title
Some text

## Subtitle
More text

### Subsubtitle
Even more text"#;
        let ch = extract_data(markdown);

        assert_eq!(3, ch.content_chunks.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", ch.content_chunks[1].get_breadcrumb());
        assert_eq!("More text", ch.content_chunks[1].get_text());
        assert_eq!(
            "Title>Subtitle>Subsubtitle",
            ch.content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", ch.content_chunks[2].get_text());
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
        let ch = extract_data(markdown);

        assert_eq!(4, ch.content_chunks.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", ch.content_chunks[1].get_breadcrumb());
        assert_eq!("More text", ch.content_chunks[1].get_text());
        assert_eq!(
            "Title>Subtitle>Subsubtitle",
            ch.content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", ch.content_chunks[2].get_text());
        assert_eq!("Title>Level 2 Title", ch.content_chunks[3].get_breadcrumb());
        assert_eq!("There is text here", ch.content_chunks[3].get_text());
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
        let ch = extract_data(markdown);

        assert_eq!(6, ch.content_chunks.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", ch.content_chunks[1].get_breadcrumb());
        assert_eq!("More text", ch.content_chunks[1].get_text());
        assert_eq!(
            "Title>Subtitle>Subsubtitle",
            ch.content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", ch.content_chunks[2].get_text());
        assert_eq!("Title>Level 2 Title", ch.content_chunks[3].get_breadcrumb());
        assert_eq!("There is text here", ch.content_chunks[3].get_text());
        assert_eq!(
            "Title>Level 2 Title>Fourth Subsubtitle",
            ch.content_chunks[4].get_breadcrumb()
        );
        assert_eq!("Before last text", ch.content_chunks[4].get_text());
        assert_eq!("Main Title", ch.content_chunks[5].get_breadcrumb());
        assert_eq!("Another main content", ch.content_chunks[5].get_text());
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
        let ch = extract_data(markdown);

        assert_eq!(6, ch.content_chunks.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[0].get_text());
        assert_eq!("Title>Subtitle", ch.content_chunks[1].get_breadcrumb());
        assert_eq!("More text", ch.content_chunks[1].get_text());
        assert_eq!("Subsubtitle", ch.content_chunks[2].get_breadcrumb());
        assert_eq!("Even more text", ch.content_chunks[2].get_text());
        assert_eq!(
            "Subsubtitle>Level 2 Title",
            ch.content_chunks[3].get_breadcrumb()
        );
        assert_eq!("There is text here", ch.content_chunks[3].get_text());
        assert_eq!(
            "Subsubtitle>Fourth Subsubtitle",
            ch.content_chunks[4].get_breadcrumb()
        );
        assert_eq!("Before last text", ch.content_chunks[4].get_text());
        assert_eq!("Main Title", ch.content_chunks[5].get_breadcrumb());
        assert_eq!("Another main content", ch.content_chunks[5].get_text());
    }

    #[test]
    fn check_title_with_link() {
        let markdown = r#"# [Title link](https://nico.red)
Some text"#;
        let ch = extract_data(markdown);

        assert_eq!(1, ch.content_chunks.len());
        assert_eq!(Some("Title link".to_string()), ch.title);
        assert_eq!("Title link", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[0].get_text());
    }

    #[test]
    fn check_title_with_style() {
        let markdown = r#"# Title **bold** *italic*
Some text"#;
        let ch = extract_data(markdown);

        assert_eq!(1, ch.content_chunks.len());
        assert_eq!(Some("Title bold italic".to_string()), ch.title);
        assert_eq!("Title bold italic", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[0].get_text());
    }

    #[test]
    fn check_content_without_title() {
        let markdown = r#"Intro text

# Title

Some text"#;
        let ch = extract_data(markdown);

        assert_eq!(2, ch.content_chunks.len());
        assert_eq!(Some("Intro text".to_string()), ch.title);
        assert_eq!("", ch.content_chunks[0].get_breadcrumb());
        assert_eq!("Intro text", ch.content_chunks[0].get_text());
        assert_eq!("Title", ch.content_chunks[1].get_breadcrumb());
        assert_eq!("Some text", ch.content_chunks[1].get_text());
    }
}
