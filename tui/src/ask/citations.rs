//! The ONE home of citation-marker (`[n]`) logic (CONTEXT.md: **Citation**).
//! Scanning, stripping (copy, history), and wikilink conversion (saved
//! answers) all live here; no other module may parse `[n]`.

pub struct CitationSpan {
    pub range: std::ops::Range<usize>,
    pub index: usize,
}

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
            // at least one digit, closed by ']', not part of '[[…'
            if j > i + 1 && j < bytes.len() && bytes[j] == b']' {
                let index: usize = text[i + 1..j].parse().unwrap_or(0);
                if index > 0 {
                    spans.push(CitationSpan { range: start..j + 1, index });
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    spans
}

pub fn strip(text: &str) -> String {
    rewrite(text, |_| String::new())
}

pub fn link_sources(text: &str, source_names: &[String]) -> String {
    rewrite(text, |span| match source_names.get(span.index - 1) {
        Some(name) => format!("[[{name}]]"),
        None => text[span.range.clone()].to_string(),
    })
}

/// Shared splice loop: replace each scanned span via `f`, then collapse the
/// " ." / "  " droppings a removed marker leaves behind.
fn rewrite(text: &str, f: impl Fn(&CitationSpan) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for span in scan(text) {
        out.push_str(&text[last..span.range.start]);
        out.push_str(&f(&span));
        last = span.range.end;
    }
    out.push_str(&text[last..]);
    out.replace("  ", " ").replace(" .", ".").replace(" ,", ",")
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
}
