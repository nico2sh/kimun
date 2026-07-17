//! The ONE home of citation-marker (`[n]`) logic (CONTEXT.md: **Citation**).
//! Scanning, stripping (copy, history), and wikilink conversion (saved
//! answers) all live here; no other module may parse `[n]`.

pub struct CitationSpan {
    pub range: std::ops::Range<usize>,
    pub index: usize,
}

/// Scan text for all `[digits]` citation markers, returning spans with byte ranges and 1-based indices.
pub fn scan(text: &str) -> Vec<CitationSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            // at least one digit, closed by ']', not part of '[[…' or '…]]'
            if j > i + 1 && j < bytes.len() && bytes[j] == b']' {
                let is_bracket_adjacent = (start > 0 && bytes[start - 1] == b'[')
                    || (j + 1 < bytes.len() && bytes[j + 1] == b']');
                if !is_bracket_adjacent {
                    let index: usize = text[i + 1..j].parse().unwrap_or(0);
                    if index > 0 {
                        spans.push(CitationSpan { range: start..j + 1, index });
                    }
                    i = j + 1;
                    continue;
                } else {
                    // Skip past both brackets if they form a wikilink
                    i = j + 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    spans
}

/// Remove all `[n]` citation markers from text, tidying whitespace only where markers are removed.
pub fn strip(text: &str) -> String {
    rewrite(text, |_| String::new())
}

/// Convert in-range `[n]` markers to `[[source_name]]`; keep out-of-range markers untouched.
pub fn link_sources(text: &str, source_names: &[String]) -> String {
    rewrite(text, |span| match source_names.get(span.index - 1) {
        Some(name) => format!("[[{name}]]"),
        None => text[span.range.clone()].to_string(),
    })
}

/// Shared splice loop: replace each scanned span via `f`, tidying locally only when a marker is removed.
fn rewrite(text: &str, f: impl Fn(&CitationSpan) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for span in scan(text) {
        out.push_str(&text[last..span.range.start]);
        let replacement = f(&span);
        if replacement.is_empty() {
            let next = text[span.range.end..].chars().next();
            let follows_break = matches!(next, None | Some(' ' | '.' | ',' | ';' | ':' | '!' | '?' | '\n'));
            if follows_break && out.ends_with(' ') {
                out.pop();
            }
        } else {
            out.push_str(&replacement);
        }
        last = span.range.end;
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_markers_with_ranges_and_indices() {
        let t = "Alpha [1] beta [12].";
        let spans = scan(t);
        assert_eq!(spans.len(), 2);
        assert_eq!(&t[spans[0].range.clone()], "[1]");
        assert_eq!(spans[0].index, 1);
        assert_eq!(spans[1].index, 12);
    }

    #[test]
    fn scan_ignores_non_numeric_brackets() {
        assert!(scan("a [[wikilink]] and [tag] and [1a]").is_empty());
    }

    #[test]
    fn strip_removes_markers_and_tidies_double_spaces() {
        assert_eq!(strip("Fact [1] stands. Next [2]."), "Fact stands. Next.");
    }

    #[test]
    fn link_sources_rewrites_in_range_and_keeps_out_of_range() {
        let names = vec!["alpha".to_string()];
        assert_eq!(
            link_sources("See [1] not [7].", &names),
            "See [[alpha]] not [7]."
        );
    }

    #[test]
    fn scan_ignores_numeric_wikilinks() {
        assert!(scan("see [[1]] and [[42]]").is_empty());
    }

    #[test]
    fn strip_preserves_text_without_markers() {
        let t = "code:\n    indented  twice .";
        assert_eq!(strip(t), t);
    }

    #[test]
    fn strip_tidies_only_around_removed_markers() {
        assert_eq!(strip("a [1] b"), "a b");
        assert_eq!(strip("end [2]."), "end.");
        assert_eq!(strip("tail [3]"), "tail");
    }
}
