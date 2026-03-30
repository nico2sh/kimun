use crate::cli::json_output::JsonHeader;
use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

// Compile regexes once using OnceLock for better performance
fn hashtag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"#([a-zA-Z0-9_-]+)").unwrap())
}

fn link_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"!?\[([^\]]*)\]\(([^)]+)\)").unwrap())
}

fn wikilink_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap())
}

fn header_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.+)$").unwrap())
}

pub fn extract_tags(content: &str) -> Vec<String> {
    let mut tags: HashSet<String> = HashSet::new();

    // Extract from YAML frontmatter
    if let Some(frontmatter) = extract_frontmatter(content) {
        if let Some(yaml_tags) = extract_frontmatter_tags(&frontmatter) {
            for tag in yaml_tags {
                tags.insert(tag);
            }
        }
    }

    // Extract hashtags from content
    for capture in hashtag_regex().captures_iter(content) {
        if let Some(tag) = capture.get(1) {
            tags.insert(tag.as_str().to_string());
        }
    }

    let mut result: Vec<String> = tags.into_iter().collect();
    result.sort();
    result
}

pub fn extract_links(content: &str) -> Vec<String> {
    let mut matches: Vec<(usize, String)> = Vec::new();

    // Markdown links [text](url) - including image links ![text](url)
    for capture in link_regex().captures_iter(content) {
        let pos = capture.get(0).map(|m| m.start()).unwrap_or(0);
        if let Some(url) = capture.get(2) {
            matches.push((pos, url.as_str().to_string()));
        }
    }

    // Wikilinks [[page]]
    for capture in wikilink_regex().captures_iter(content) {
        let pos = capture.get(0).map(|m| m.start()).unwrap_or(0);
        if let Some(page) = capture.get(1) {
            matches.push((pos, page.as_str().to_string()));
        }
    }

    matches.sort_by_key(|(pos, _)| *pos);
    matches.into_iter().map(|(_, link)| link).collect()
}

pub fn extract_headers(content: &str) -> Vec<JsonHeader> {
    let mut headers: Vec<JsonHeader> = Vec::new();

    for line in content.lines() {
        if let Some(capture) = header_regex().captures(line) {
            if let (Some(level_match), Some(text_match)) = (capture.get(1), capture.get(2)) {
                let level = level_match.as_str().len() as u32;
                let text = text_match.as_str().trim().to_string();
                headers.push(JsonHeader { text, level });
            }
        }
    }

    headers
}

fn extract_frontmatter(content: &str) -> Option<String> {
    if !content.starts_with("---") {
        return None;
    }

    let lines: Vec<&str> = content.lines().collect();
    if lines.len() < 3 {
        return None;
    }

    let mut end_index = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            end_index = Some(i);
            break;
        }
    }

    if let Some(end) = end_index {
        let frontmatter_lines = &lines[1..end];
        Some(frontmatter_lines.join("\n"))
    } else {
        None
    }
}

fn extract_frontmatter_tags(frontmatter: &str) -> Option<Vec<String>> {
    let mut tags: Vec<String> = Vec::new();
    let mut in_tags_block = false;

    for line in frontmatter.lines() {
        let line = line.trim();

        // Handle "tags: [tag1, tag2]" format (inline array)
        if let Some(tags_str) = line.strip_prefix("tags:") {
            let trimmed = tags_str.trim();

            // Check if this is an inline array format
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let cleaned = trimmed.strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .unwrap_or(trimmed);

                for tag in cleaned.split(',') {
                    let clean_tag = tag.trim()
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .or_else(|| tag.trim().strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                        .unwrap_or(tag.trim());

                    if !clean_tag.is_empty() {
                        tags.push(clean_tag.to_string());
                    }
                }
            }
            // Check if this is the start of a block sequence (no content after colon, or just empty)
            else if trimmed.is_empty() {
                in_tags_block = true;
            }
            // Single tag on same line as "tags:"
            else {
                let clean_tag = trimmed
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(trimmed);
                if !clean_tag.is_empty() {
                    tags.push(clean_tag.to_string());
                }
            }
        }
        // Handle YAML block sequence format (tags: \n  - tag1 \n  - tag2)
        else if in_tags_block && line.starts_with('-') {
            if let Some(tag_str) = line.strip_prefix('-') {
                let clean_tag = tag_str.trim()
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .or_else(|| tag_str.trim().strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                    .unwrap_or(tag_str.trim());

                if !clean_tag.is_empty() {
                    tags.push(clean_tag.to_string());
                }
            }
        }
        // Handle "tag: value" format (single tag)
        else if let Some(tag_str) = line.strip_prefix("tag:") {
            let clean_tag = tag_str.trim()
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(tag_str.trim());

            if !clean_tag.is_empty() {
                tags.push(clean_tag.to_string());
            }
        }
        // Exit tags block if we encounter a new YAML key or empty line
        else if in_tags_block && (line.contains(':') || line.is_empty()) {
            in_tags_block = false;
        }
    }

    if tags.is_empty() { None } else { Some(tags) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_tags_array_format() {
        let frontmatter = r#"tags: ["project", "urgent"]
title: "Test Note""#;

        let tags = extract_frontmatter_tags(frontmatter).unwrap();
        assert_eq!(tags, vec!["project", "urgent"]);
    }

    #[test]
    fn frontmatter_single_tag_format() {
        let frontmatter = r#"tag: meeting
title: "Test Note""#;

        let tags = extract_frontmatter_tags(frontmatter).unwrap();
        assert_eq!(tags, vec!["meeting"]);
    }
}
