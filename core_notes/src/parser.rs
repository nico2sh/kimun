use std::{cmp::min, fmt::Display};

const MAX_TITLE_LENGTH: usize = 20;

struct Parser {}

impl Parser {
    fn new() -> Self {
        Self {}
    }

    pub fn parse(&self, md_text: String) -> NoteContent {
        let md_tree = markdown::to_mdast(&md_text, &markdown::ParseOptions::default()).unwrap();
        let title = self.extract_title(&md_tree);
        let ch = if let Some(nodes) = md_tree.children() {
            self.parse_nodes(nodes)
        } else {
            vec![]
        };
        NoteContent { title, content: ch }
    }

    fn extract_title(&self, node: &markdown::mdast::Node) -> Option<String> {
        let node_to_check = match node {
            markdown::mdast::Node::Root(root) => root.children.first(),
            _ => Some(node),
        };
        if let Some(node) = node_to_check {
            match node {
                markdown::mdast::Node::Paragraph(_)
                | markdown::mdast::Node::Heading(_)
                | markdown::mdast::Node::Emphasis(_)
                | markdown::mdast::Node::Strong(_)
                | markdown::mdast::Node::Text(_)
                | markdown::mdast::Node::Link(_) => {
                    let text = &node.to_string();
                    let length = text.len();
                    Some(text[..min(length, MAX_TITLE_LENGTH)].to_string())
                }
                _ => None,
            }
        } else {
            None
        }
    }

    fn parse_nodes(&self, nodes: &Vec<markdown::mdast::Node>) -> Vec<ContentHierarchy> {
        let mut ch = vec![];
        let mut current_breadcrumb: Vec<(u8, String)> = vec![];
        let mut current_content = vec![];
        println!("NODES: {:?}", nodes);
        for node in nodes {
            match node {
                markdown::mdast::Node::Heading(heading) => {
                    if !current_content.is_empty() || !current_breadcrumb.is_empty() {
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
                // We add anything else as content
                node => {
                    let mut content_string = node.to_string();
                    if matches!(node, markdown::mdast::Node::Paragraph(_)) {
                        content_string.push(' ');
                    }
                    current_content.push(content_string);
                }
            }
        }
        ch.push(ContentHierarchy {
            breadcrumb: current_breadcrumb,
            content: current_content
                .into_iter()
                .collect::<String>()
                .trim()
                .to_string(),
        });
        ch
    }
}

#[derive(Debug)]
struct NoteContent {
    title: Option<String>,
    content: Vec<ContentHierarchy>,
}

#[derive(Debug)]
struct ContentHierarchy {
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
    use super::Parser;

    #[test]
    fn check_title_no_header() {
        let markdown = r#"[No header](https://example.com)

Some text"#;
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

        assert_eq!(1, ch.content.len());
        assert_eq!(Some("No header".to_string()), ch.title);
        assert_eq!("", ch.content[0].get_breadcrumb());
        assert_eq!("No header Some text", ch.content[0].get_content());
    }

    #[test]
    fn check_hierarchy_one() {
        let markdown = r#"# Title
Some text"#;
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

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
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

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
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

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
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

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
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

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
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

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
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

        assert_eq!(1, ch.content.len());
        assert_eq!(Some("Title link".to_string()), ch.title);
        assert_eq!("Title link", ch.content[0].get_breadcrumb());
        assert_eq!("Some text", ch.content[0].get_content());
    }

    #[test]
    fn check_title_with_style() {
        let markdown = r#"# Title **bold** *italic*
Some text"#;
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

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
        let parser = Parser::new();
        let ch = parser.parse(markdown.to_string());
        println!("{:?}", ch);

        assert_eq!(2, ch.content.len());
        assert_eq!(Some("Intro text".to_string()), ch.title);
        assert_eq!("", ch.content[0].get_breadcrumb());
        assert_eq!("Intro text", ch.content[0].get_content());
        assert_eq!("Title", ch.content[1].get_breadcrumb());
        assert_eq!("Some text", ch.content[1].get_content());
    }
}
