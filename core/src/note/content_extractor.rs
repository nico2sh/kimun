use std::cmp::min;

use log::{debug, error};
use pulldown_cmark::{Event, Parser, Tag};
use regex::{Captures, Regex};

use crate::{
    nfs::{self, VaultPath},
    note::{ContentChunk, NoteContentData},
};

use super::Link;

const MAX_TITLE_LENGTH: usize = 40;

pub fn get_content_data<S: AsRef<str>>(md_text: S) -> NoteContentData {
    let hash = nfs::hash_text(md_text.as_ref());
    let title = extract_title(md_text);

    NoteContentData { title, hash }
}

pub fn get_content_chunks<S: AsRef<str>>(md_text: S) -> Vec<ContentChunk> {
    let (frontmatter, text) = remove_frontmatter(md_text.as_ref());

    let mut content_chunks = parse_text(&text);
    if !frontmatter.is_empty() {
        content_chunks.push(ContentChunk {
            breadcrumb: vec!["FrontMatter".to_string()],
            text: frontmatter,
        })
    };
    content_chunks
}

// Convert any wikilink into a link to a note
fn convert_wikilinks<S: AsRef<str>>(md_text: S) -> (String, Vec<Link>) {
    let wiki_link_regex = r#"(?:\[\[(?P<link_text>[^\]]+)\]\])"#; // Remember to check the pipe `|`
    let rx = Regex::new(wiki_link_regex).unwrap();
    let mut note_links = vec![];
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
                note_links.push(Link::note(link, text));
                format!("[{}]({})", text, link)
            } else {
                format!("[[{}]]", items)
            }
        })
        .into_owned();
    (text, note_links)
}

pub fn get_markdown_and_links<S: AsRef<str>>(md_text: S) -> (String, Vec<Link>) {
    let mut links = vec![];
    let md_link_regex = r#"(?P<bang>!?)(?:\[(?P<text>[^\]]+)\])\((?P<link>[^\)]+?)\)"#;
    let url_regex = r#"^https?:\/\/[\w\d]+\.[\w\d]+(?:(?:\.[\w\d]+)|(?:[\w\d\/?=#]+))+$"#;
    let rx = Regex::new(md_link_regex).unwrap();
    rx.captures_iter(md_text.as_ref()).for_each(|caps| {
        let bang = &caps["bang"];
        let text = &caps["text"];
        let link = &caps["link"];
        // We ignore links that start with a `!`, since these are images
        if bang.is_empty() {
            debug!("checking link {}", link);
            if VaultPath::is_valid(link) {
                links.push(Link::vault_path(link, text));
            } else {
                let rxurl = Regex::new(url_regex).unwrap();
                if rxurl.is_match(link) {
                    links.push(Link::url(link, text));
                } else {
                    debug!("link not counting {}", link);
                }
            }
        }
    });
    let (md_text, mut note_links) = convert_wikilinks(md_text);
    links.append(&mut note_links);
    (md_text, links)
}

pub fn extract_title<S: AsRef<str>>(md_text: S) -> String {
    let (_frontmatter, md_text) = remove_frontmatter(md_text);
    let mut title = String::new();
    let mut parser = pulldown_cmark::Parser::new(md_text.as_ref());
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

        if title.is_empty() {
            let title_cand = match &tt {
                TextType::Header(_, text) => text.to_owned(),
                TextType::Text(text) => text.to_owned(),
                TextType::None => String::new(),
            };
            title = title_cand
                .lines()
                .next()
                .map(|t| {
                    let title_length = min(MAX_TITLE_LENGTH, t.len());
                    t.chars().take(title_length).collect()
                })
                .unwrap_or_default();
            return title;
        }
    }

    "<None>".to_string()
}

fn parse_text(md_text: &str) -> Vec<ContentChunk> {
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

        // if title.is_empty() {
        //     let title_cand = match &tt {
        //         TextType::Header(_, text) => text.to_owned(),
        //         TextType::Text(text) => text.to_owned(),
        //         TextType::None => String::new(),
        //     };
        //     title = title_cand
        //         .lines()
        //         .next()
        //         .map(|t| {
        //             let title_length = min(MAX_TITLE_LENGTH, t.len());
        //             t.chars().take(title_length).collect()
        //         })
        //         .unwrap_or_default();
        // }

        match tt {
            TextType::Header(level, text) => {
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
            TextType::Text(text) => {
                current_content.push(text);
            }
            TextType::None => {
                // Don't do anything
            }
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

#[cfg(test)]
mod test {
    use crate::{
        nfs::VaultPath,
        note::{
            content_extractor::{get_content_chunks, get_content_data},
            Link, LinkType,
        },
    };

    use super::{convert_wikilinks, get_markdown_and_links};

    #[test]
    fn convert_wiki_link() {
        let markdown = r#"Here is a [[Wikilink|text with link]]"#;

        let (md, links) = convert_wikilinks(markdown);

        assert_eq!(md, "Here is a [text with link](Wikilink)");
        assert_eq!(1, links.len());
        assert!(links
            .iter()
            .any(|link| { link.eq(&Link::note("Wikilink", "text with link")) }));
    }

    #[test]
    fn convert_many_wiki_links() {
        let markdown = r#"Here is a [[Wikilink|text with link]], and another [[Link]] this time without text.

    And a [[https://example.com|url link]]"#;

        let (md, links) = convert_wikilinks(markdown);

        assert_eq!(
            md,
            r#"Here is a [text with link](Wikilink), and another [Link](Link) this time without text.

    And a [[https://example.com|url link]]"#
        );
        assert_eq!(2, links.len());
        assert!(links
            .iter()
            .any(|link| { link.eq(&Link::note("Wikilink", "text with link")) }));
        assert!(links
            .iter()
            .any(|link| { link.eq(&Link::note("Link", "Link")) }))
    }

    #[test]
    fn ignore_image_links() {
        let markdown = r#"This is an ![image](image.png)"#;

        let (_md, links) = get_markdown_and_links(markdown);

        assert!(links.is_empty());
    }

    #[test]
    fn extract_link_from_text() {
        let markdown =
            r#"This is a [link](notes/main.md) to a note, this is a [non](:caca) valid link"#;

        let (_md, links) = get_markdown_and_links(markdown);

        assert_eq!(1, links.len());
        let link = links.first().unwrap();
        assert_eq!("link", link.text);
        assert_eq!(LinkType::Note(VaultPath::new("notes/main.md")), link.ltype);
    }

    #[test]
    fn extract_many_links_from_text() {
        let markdown = r#"This is a [link](notes/main.md) to a note, this is a [[note.md]]] valid link

    Here's a [url](https://www.example.com)"#;

        let (_md, links) = get_markdown_and_links(markdown);

        assert_eq!(3, links.len());
        assert!(links.iter().any(|link| {
            let path = VaultPath::new("notes/main.md");
            link.text.eq("link") && link.ltype.eq(&LinkType::Note(path))
        }));
        assert!(links.iter().any(|link| {
            let path = VaultPath::new("note.md");
            link.text.eq("note.md") && link.ltype.eq(&LinkType::Note(path))
        }));
        assert!(links.iter().any(|link| {
            println!("{:?}", link);
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
            "First Item\nSecond Item\nSome text",
            content_chunks[0].get_text()
        );
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
}
