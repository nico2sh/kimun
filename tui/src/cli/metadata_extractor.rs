use crate::cli::json_output::JsonHeader;

/// The note's labels as the index records them — inline `#hashtags` plus
/// the frontmatter `tags` property, TOML or YAML.
pub fn extract_tags(content: &str) -> Vec<String> {
    kimun_core::note::note_tags(content)
}

pub fn extract_links(content: &str) -> Vec<String> {
    kimun_core::note::note_link_targets(content)
}

/// The note's headings — frontmatter and `#` lines inside code skipped.
pub fn extract_headers(content: &str) -> Vec<JsonHeader> {
    kimun_core::note::note_headings(content)
        .into_iter()
        .map(JsonHeader::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_come_from_the_note_walk() {
        let links = extract_links("see [[a|A]] and [b](b.md) and `[[code]]` <https://c.d>");
        assert_eq!(links, ["a", "b.md", "https://c.d"]);
    }

    #[test]
    fn a_wikilink_inside_an_html_block_is_not_a_link() {
        // HTML is opaque: nothing inside it is a link.
        let links =
            extract_links("<details>\n<summary>More</summary>\nSee [[hidden]]\n</details>\n");
        assert!(links.is_empty(), "{links:?}");
    }

    #[test]
    fn frontmatter_tags_in_either_format() {
        let yaml = "---\ntags:\n  - Project\n  - urgent\ntitle: Test\n---\nbody";
        assert_eq!(extract_tags(yaml), ["project", "urgent"]);
        let toml = "+++\ntags = [\"meeting\"]\n+++\nbody #notes";
        assert_eq!(extract_tags(toml), ["meeting", "notes"]);
    }

    #[test]
    fn extract_tags_matches_core_label_rules() {
        let body =
            "---\ntags: [yaml_tag]\n---\nplain #body and #tag-with-dash\n```\n#code_tag\n```";
        let tags = extract_tags(body);
        // Expected: yaml_tag (frontmatter), body (extracted), tag (dash-terminated).
        // NOT expected: code_tag (in fence), tag-with-dash (dash not in label).
        assert!(tags.contains(&"yaml_tag".to_string()));
        assert!(tags.contains(&"body".to_string()));
        assert!(tags.contains(&"tag".to_string()));
        assert!(!tags.contains(&"code_tag".to_string()), "got: {:?}", tags);
        assert!(
            !tags.contains(&"tag-with-dash".to_string()),
            "got: {:?}",
            tags
        );
    }
}
