//! Flipping a task's checkbox in note text. See `CONTEXT.md`, **Facet view**,
//! and ADR-0062.

use pulldown_cmark::{Event, Options, Parser};

use super::walk;

/// `text` with the checkbox of the task on source line `line` (0-based) flipped:
/// `[ ]` becomes `[x]`, and `[x]` or `[X]` becomes `[ ]`.
///
/// `seen` is the line as the caller saw it, compared without its leading
/// whitespace. A chunk drawn from a nested item has lost that indent. The
/// result is `None`, and nothing changes, when the line is not a task line of
/// the note, or when its text is not `seen`: the note changed since the
/// caller read it. A `- [ ]` line inside a code block is not a task.
pub fn toggle_task(text: &str, line: usize, seen: &str) -> Option<String> {
    let (offset, _) = walk::body_start(text);
    let parser = Parser::new_ext(&text[offset..], Options::ENABLE_TASKLISTS).into_offset_iter();
    for (event, range) in parser {
        if !matches!(event, Event::TaskListMarker(_)) {
            continue;
        }
        let at = offset + range.start;
        if text[..at].matches('\n').count() != line {
            continue;
        }
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        // A CRLF note loads with its `\r`: it is not part of the line's text.
        let same = |a: &str, b: &str| {
            a.trim_start().trim_end_matches('\r') == b.trim_start().trim_end_matches('\r')
        };
        if !same(&text[line_start..line_end], seen) {
            return None;
        }
        // The marker's three bytes are `[`, the state, `]`.
        let state = text.get(at..at + 3)?;
        let flipped = match state {
            "[ ]" => "[x]",
            "[x]" | "[X]" => "[ ]",
            _ => return None,
        };
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..at]);
        out.push_str(flipped);
        out.push_str(&text[at + 3..]);
        return Some(out);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_task_closes_and_a_closed_one_opens() {
        let text = "intro\n- [ ] one\n- [x] two\n";
        let closed = toggle_task(text, 1, "- [ ] one").unwrap();
        assert_eq!(closed, "intro\n- [x] one\n- [x] two\n");
        let opened = toggle_task(&closed, 2, "- [x] two").unwrap();
        assert_eq!(opened, "intro\n- [x] one\n- [ ] two\n");
        let upper = toggle_task("- [X] up\n", 0, "- [X] up").unwrap();
        assert_eq!(upper, "- [ ] up\n");
    }

    #[test]
    fn a_nested_task_matches_without_its_indent() {
        let text = "- a\n  - [ ] deep\n";
        let out = toggle_task(text, 1, "- [ ] deep").unwrap();
        assert_eq!(out, "- a\n  - [x] deep\n");
    }

    #[test]
    fn a_note_that_changed_is_left_alone() {
        let text = "- [ ] one\n";
        assert_eq!(toggle_task(text, 0, "- [ ] other"), None);
        assert_eq!(toggle_task(text, 1, "- [ ] one"), None);
        assert_eq!(toggle_task("- plain\n", 0, "- plain"), None);
    }

    #[test]
    fn a_task_look_alike_in_code_is_not_a_task() {
        let text = "```\n- [ ] code\n```\n";
        assert_eq!(toggle_task(text, 1, "- [ ] code"), None);
    }

    #[test]
    fn frontmatter_shifts_no_line_number() {
        let text = "---\ntitle: x\n---\n- [ ] one\n";
        let out = toggle_task(text, 3, "- [ ] one").unwrap();
        assert_eq!(out, "---\ntitle: x\n---\n- [x] one\n");
    }

    #[test]
    fn a_crlf_note_matches_a_line_seen_without_its_return() {
        let text = "intro\r\n- [ ] one\r\n";
        let out = toggle_task(text, 1, "- [ ] one").unwrap();
        assert_eq!(out, "intro\r\n- [x] one\r\n");
    }

    #[test]
    fn a_quoted_task_flips() {
        let text = "> - [ ] one\n";
        let out = toggle_task(text, 0, "> - [ ] one").unwrap();
        assert_eq!(out, "> - [x] one\n");
    }
}
