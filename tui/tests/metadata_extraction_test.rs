use kimun_notes::cli::metadata_extractor::{extract_headers, extract_links, extract_tags};

#[test]
fn extract_tags_from_hashtags_and_frontmatter() {
    let content = r#"---
tags: ["project", "urgent"]
tag: meeting
---
# Meeting Notes

This is #important and #todo items.
Also #project-related stuff.
"#;

    let tags = extract_tags(content);
    let mut expected = vec![
        "project",
        "urgent",
        "meeting",
        "important",
        "todo",
        "project-related",
    ];
    expected.sort();
    let mut actual = tags;
    actual.sort();

    assert_eq!(actual, expected);
}

#[test]
fn extract_markdown_links() {
    let content = r#"
# Notes

See [other note](other-note.md) and [external](https://example.com).
Also check [[wikilink]] and ![image](image.png).
"#;

    let links = extract_links(content);
    let expected = vec![
        "other-note.md",
        "https://example.com",
        "wikilink",
        "image.png",
    ];
    assert_eq!(links, expected);
}

#[test]
fn extract_markdown_headers() {
    let content = r#"# Main Title

## Section 1

### Subsection A

## Section 2
"#;

    let headers = extract_headers(content);
    assert_eq!(headers.len(), 4);
    assert_eq!(headers[0].text, "Main Title");
    assert_eq!(headers[0].level, 1);
    assert_eq!(headers[1].text, "Section 1");
    assert_eq!(headers[1].level, 2);
}
