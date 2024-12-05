const MAX_TITLE_LENGTH: usize = 20;

pub fn parse(md_text: String) -> NoteContent {
    let title = extract_title(&md_text);
    let content = super::utilities::remove_diacritics(&md_text);
    let md_tree = markdown::to_mdast(&content, &parse_options()).unwrap();
    let ch = if let Some(nodes) = md_tree.children() {
        parse_nodes(nodes)
    } else {
        vec![]
    };
    NoteContent { title, content: ch }
}

pub fn extract_title(text: &str) -> Option<String> {
    let root_node = markdown::to_mdast(text, &parse_options()).unwrap();
    root_node.children().and_then(|children| {
        children.iter().find_map(|n| {
            if matches!(
                n,
                markdown::mdast::Node::Yaml(_) | markdown::mdast::Node::Toml(_)
            ) {
                None
            } else if let markdown::mdast::Node::List(list) = n {
                list.children.iter().find_map(|e| {
                    let title = e.to_string();
                    if title.is_empty() {
                        None
                    } else {
                        Some(title)
                    }
                })
            } else {
                let title = n.to_string();
                if title.is_empty() {
                    None
                } else {
                    Some(title)
                }
            }
        })
    })
}

fn parse_options() -> markdown::ParseOptions {
    let constructs = markdown::Constructs {
        frontmatter: true,
        ..Default::default()
    };

    markdown::ParseOptions {
        constructs,
        ..Default::default()
    }
}

fn parse_nodes(nodes: &Vec<markdown::mdast::Node>) -> Vec<ContentHierarchy> {
    let mut ch = vec![];
    let mut current_breadcrumb: Vec<(u8, String)> = vec![];
    let mut current_content = vec![];
    for node in nodes {
        match node {
            markdown::mdast::Node::Heading(heading) => {
                if !current_breadcrumb.is_empty() || !current_content.is_empty() {
                    let breadcrumb = current_breadcrumb.clone();
                    ch.push(ContentHierarchy {
                        breadcrumb,
                        content: current_content
                            .clone()
                            .into_iter()
                            .collect::<String>()
                            .trim()
                            .to_string(),
                    });
                }
                let children = &heading.children;
                let head = children.iter().map(|n| n.to_string()).collect::<String>();
                while !current_breadcrumb.is_empty()
                    && current_breadcrumb.last().unwrap().0 >= heading.depth
                {
                    current_breadcrumb.remove(current_breadcrumb.len() - 1);
                }
                current_breadcrumb.push((heading.depth, head));
                current_content.clear();
            }
            markdown::mdast::Node::Yaml(yaml) => {
                // We add frontmatters in its special section
                ch.push(ContentHierarchy {
                    breadcrumb: vec![(0, "FrontMatter".to_owned())],
                    content: yaml.value.clone(),
                })
            }
            markdown::mdast::Node::Toml(toml) => {
                // We add frontmatters in its special section
                ch.push(ContentHierarchy {
                    breadcrumb: vec![(0, "FrontMatter".to_owned())],
                    content: toml.value.clone(),
                })
            }
            markdown::mdast::Node::List(list) => {
                for list_node in &list.children {
                    add_node_to_content_string(list_node, &mut current_content);
                }
            }
            markdown::mdast::Node::Table(table) => {
                for list_node in &table.children {
                    add_node_to_content_string(list_node, &mut current_content);
                }
            }
            // We add anything else as content
            node => {
                add_node_to_content_string(node, &mut current_content);
            }
        }
    }
    if !current_breadcrumb.is_empty() || !current_content.is_empty() {
        ch.push(ContentHierarchy {
            breadcrumb: current_breadcrumb,
            content: current_content
                .into_iter()
                .collect::<String>()
                .trim()
                .to_string(),
        });
    }
    ch
}

fn add_node_to_content_string(node: &markdown::mdast::Node, current_content: &mut Vec<String>) {
    let content_string = node.to_string().trim().to_owned();
    println!("{:?}", node);
    if !content_string.is_empty() {
        current_content.push(content_string);
        if matches!(
            node,
            markdown::mdast::Node::Paragraph(_)
                | markdown::mdast::Node::ListItem(_)
                | markdown::mdast::Node::Break(_)
                | markdown::mdast::Node::Code(_)
                | markdown::mdast::Node::InlineCode(_)
                | markdown::mdast::Node::Math(_)
                | markdown::mdast::Node::InlineMath(_)
                | markdown::mdast::Node::TableRow(_)
                | markdown::mdast::Node::TableCell(_)
        ) {
            // We add an extra space
            current_content.push(" ".to_string());
        }
    }
}

#[derive(Debug)]
pub struct NoteContent {
    title: Option<String>,
    content: Vec<ContentHierarchy>,
}

#[derive(Debug)]
pub struct ContentHierarchy {
    breadcrumb: Vec<(u8, String)>,
    content: String,
}

impl ContentHierarchy {
    pub fn get_breadcrumb(&self) -> String {
        self.breadcrumb
            .iter()
            .map(|b| b.1.clone())
            .collect::<Vec<String>>()
            .join(">")
    }

    fn get_content(&self) -> &str {
        &self.content
    }
}

#[cfg(test)]
mod test {
    use crate::parser::parse;

    #[test]
    fn check_title_yaml_frontmatter() {
        let markdown = r#"---
something: nice
other: else
---

title"#;
        let ch = parse(markdown.to_string());

        assert_eq!(2, ch.content.len());
        assert_eq!(Some("title".to_string()), ch.title);
        assert_eq!("FrontMatter", ch.content[0].get_breadcrumb());
        assert_eq!("something: nice\nother: else", ch.content[0].get_content());
        assert_eq!("", ch.content[1].get_breadcrumb());
        assert_eq!("title", ch.content[1].get_content());
    }

    #[test]
    fn check_title_toml_frontmatter() {
        let markdown = r#"+++
something: nice
other: else
+++

title"#;
        let ch = parse(markdown.to_string());

        assert_eq!(2, ch.content.len());
        assert_eq!(Some("title".to_string()), ch.title);
        assert_eq!("FrontMatter", ch.content[0].get_breadcrumb());
        assert_eq!("something: nice\nother: else", ch.content[0].get_content());
        assert_eq!("", ch.content[1].get_breadcrumb());
        assert_eq!("title", ch.content[1].get_content());
    }

    #[test]
    fn check_title_in_list() {
        let markdown = r#"- First Item
- Second Item

Some text"#;
        let ch = parse(markdown.to_string());

        assert_eq!(1, ch.content.len());
        assert_eq!(Some("First Item".to_string()), ch.title);
        assert_eq!("", ch.content[0].get_breadcrumb());
        assert_eq!(
            "First Item Second Item Some text",
            ch.content[0].get_content()
        );
    }

    #[test]
    fn check_title_no_header() {
        let markdown = r#"[No header](https://example.com)

Some text"#;
        let ch = parse(markdown.to_string());

        assert_eq!(1, ch.content.len());
        assert_eq!(Some("No header".to_string()), ch.title);
        assert_eq!("", ch.content[0].get_breadcrumb());
        assert_eq!("No header Some text", ch.content[0].get_content());
    }

    #[test]
    fn check_hierarchy_one() {
        let markdown = r#"# Title
Some text"#;
        let ch = parse(markdown.to_string());

        assert_eq!(1, ch.content.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
    }

    #[test]
    fn check_hierarchy_two() {
        let markdown = r#"# Title
Some text

## Subtitle
More text"#;
        let ch = parse(markdown.to_string());

        assert_eq!(2, ch.content.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
        assert_eq!("Title>Subtitle", ch.content[1].get_breadcrumb());
        assert_eq!("More text", ch.content[1].get_content());
    }

    #[test]
    fn check_hierarchy_three() {
        let markdown = r#"# Title
Some text

## Subtitle
More text

### Subsubtitle
Even more text"#;
        let ch = parse(markdown.to_string());

        assert_eq!(3, ch.content.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
        assert_eq!("Title>Subtitle", ch.content[1].get_breadcrumb());
        assert_eq!("More text", ch.content[1].get_content());
        assert_eq!("Title>Subtitle>Subsubtitle", ch.content[2].get_breadcrumb());
        assert_eq!("Even more text", ch.content[2].get_content());
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
        let ch = parse(markdown.to_string());

        assert_eq!(4, ch.content.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
        assert_eq!("Title>Subtitle", ch.content[1].get_breadcrumb());
        assert_eq!("More text", ch.content[1].get_content());
        assert_eq!("Title>Subtitle>Subsubtitle", ch.content[2].get_breadcrumb());
        assert_eq!("Even more text", ch.content[2].get_content());
        assert_eq!("Title>Level 2 Title", ch.content[3].get_breadcrumb());
        assert_eq!("There is text here", ch.content[3].get_content());
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
        let ch = parse(markdown.to_string());

        assert_eq!(6, ch.content.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
        assert_eq!("Title>Subtitle", ch.content[1].get_breadcrumb());
        assert_eq!("More text", ch.content[1].get_content());
        assert_eq!("Title>Subtitle>Subsubtitle", ch.content[2].get_breadcrumb());
        assert_eq!("Even more text", ch.content[2].get_content());
        assert_eq!("Title>Level 2 Title", ch.content[3].get_breadcrumb());
        assert_eq!("There is text here", ch.content[3].get_content());
        assert_eq!(
            "Title>Level 2 Title>Fourth Subsubtitle",
            ch.content[4].get_breadcrumb()
        );
        assert_eq!("Before last text", ch.content[4].get_content());
        assert_eq!("Main Title", ch.content[5].get_breadcrumb());
        assert_eq!("Another main content", ch.content[5].get_content());
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
        let ch = parse(markdown.to_string());

        assert_eq!(6, ch.content.len());
        assert_eq!(Some("Title".to_string()), ch.title);
        assert_eq!("Title", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
        assert_eq!("Title>Subtitle", ch.content[1].get_breadcrumb());
        assert_eq!("More text", ch.content[1].get_content());
        assert_eq!("Subsubtitle", ch.content[2].get_breadcrumb());
        assert_eq!("Even more text", ch.content[2].get_content());
        assert_eq!("Subsubtitle>Level 2 Title", ch.content[3].get_breadcrumb());
        assert_eq!("There is text here", ch.content[3].get_content());
        assert_eq!(
            "Subsubtitle>Fourth Subsubtitle",
            ch.content[4].get_breadcrumb()
        );
        assert_eq!("Before last text", ch.content[4].get_content());
        assert_eq!("Main Title", ch.content[5].get_breadcrumb());
        assert_eq!("Another main content", ch.content[5].get_content());
    }

    #[test]
    fn check_title_with_link() {
        let markdown = r#"# [Title link](https://nico.red)
Some text"#;
        let ch = parse(markdown.to_string());

        assert_eq!(1, ch.content.len());
        assert_eq!(Some("Title link".to_string()), ch.title);
        assert_eq!("Title link", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
    }

    #[test]
    fn check_title_with_style() {
        let markdown = r#"# Title **bold** *italic*
Some text"#;
        let ch = parse(markdown.to_string());

        assert_eq!(1, ch.content.len());
        assert_eq!(Some("Title bold italic".to_string()), ch.title);
        assert_eq!("Title bold italic", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
    }

    #[test]
    fn check_content_without_title() {
        let markdown = r#"Intro text

# Title

Some text"#;
        let ch = parse(markdown.to_string());

        assert_eq!(2, ch.content.len());
        assert_eq!(Some("Intro text".to_string()), ch.title);
        assert_eq!("", ch.content[0].get_breadcrumb());
        assert_eq!("Intro text", ch.content[0].get_content());
        assert_eq!("Title", ch.content[1].get_breadcrumb());
        assert_eq!("Some text", ch.content[1].get_content());
    }
}
