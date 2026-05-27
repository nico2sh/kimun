//! Render-output snapshot tests captured against the pre-change pulldown-cmark
//! implementation. These are the byte-identical contract that the tree-sitter
//! swap (treesitter-editor-rendering, step 6 onward) must preserve.
//!
//! Inputs:
//!  - Every input string from the deleted `parse_line_*` / `parse_buffer_*`
//!    structural tests in `markdown.rs`.
//!  - A multi-block corpus covering each node kind enumerated in spec.md
//!    Requirement "Render Output Byte-Identical to Pre-Change Pulldown Output".

use insta::assert_snapshot;

use super::markdown::{MarkdownSpanner, ParsedBuffer};
use super::word_wrap::WordWrapLayout;
use crate::settings::themes::Theme;

/// Serialise the editor render pipeline for a buffer at a fixed width into a
/// stable text representation: one block per visual row, blank line between
/// rows, each span on its own line as `<style-debug> | "<text>"`.
fn serialize_render(text: &str, width: u16) -> String {
    let theme = Theme::default();
    let lines: Vec<String> = if text.is_empty() {
        vec![String::new()]
    } else {
        text.split('\n').map(|s| s.to_string()).collect()
    };
    let parsed = ParsedBuffer::parse(&lines);

    // Per-line rendered-position mask, mirroring view.rs Gate 2's full rebuild
    // path (no cursor, no force_raw). Force-raw is applied per fenced-code-block
    // line below using the same `compute_fence_ranges` logic as the live view.
    let fence_rows = fence_rows_for(&lines);
    let rendered: Vec<Vec<bool>> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let force_raw = fence_rows.contains(&i);
            MarkdownSpanner::visible_positions_with(l, &parsed[i], None, force_raw)
        })
        .collect();

    let layout = WordWrapLayout::compute(&lines, width, &rendered);
    let vlines = layout.visual_lines();

    let mut out = String::new();
    for (idx, vl) in vlines.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let logical_line = lines.get(vl.logical_row).map(|s| s.as_str()).unwrap_or("");
        let force_raw = fence_rows.contains(&vl.logical_row);
        let content = vl.content(logical_line);
        let spans = MarkdownSpanner::render_with(
            content,
            logical_line,
            &parsed[vl.logical_row],
            vl.start_col,
            None,
            vl.is_first_visual_line,
            force_raw,
            width,
            &theme,
        );
        out.push_str(&format!("[row {}]\n", vl.logical_row));
        for s in spans {
            out.push_str(&format!("{:?} | {:?}\n", s.style, s.content));
        }
    }
    out
}

/// Mirror of `view.rs:compute_fence_ranges`, factored out so the snapshot
/// harness sees the same force-raw treatment as the live editor. Returns the
/// set of logical row indices that fall inside a fenced code block.
fn fence_rows_for(lines: &[String]) -> std::collections::HashSet<usize> {
    let mut rows = std::collections::HashSet::new();
    let mut in_fence = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if !in_fence {
                in_fence = true;
                rows.insert(i);
            } else {
                rows.insert(i);
                in_fence = false;
            }
            continue;
        }
        if in_fence {
            rows.insert(i);
        }
    }
    rows
}

// ── Inputs from the pre-existing parse_line_* / parse_buffer_* tests ─────────

#[test]
fn snap_label_in_paragraph() {
    assert_snapshot!(serialize_render("see #rust later", 80));
}

#[test]
fn snap_label_inside_inline_code() {
    assert_snapshot!(serialize_render("use `#foo` here", 80));
}

#[test]
fn snap_label_inside_markdown_link() {
    assert_snapshot!(serialize_render(
        "[see docs](#section) and #real",
        80
    ));
}

#[test]
fn snap_label_inside_link_display_text() {
    assert_snapshot!(serialize_render("[#todo](notes/project.md)", 80));
}

#[test]
fn snap_label_after_label_char() {
    assert_snapshot!(serialize_render("foo#bar baz", 80));
}

#[test]
fn snap_label_double_hash() {
    assert_snapshot!(serialize_render("##draft", 80));
}

#[test]
fn snap_label_adjacent_hash_run() {
    assert_snapshot!(serialize_render("#tag#more", 80));
}

#[test]
fn snap_label_inside_fenced_block() {
    let buffer = "before\n```\n#inside\n```\nafter #outside";
    assert_snapshot!(serialize_render(buffer, 80));
}

// ── Multi-block corpus (spec.md Requirement) ────────────────────────────────

#[test]
fn snap_heading_h1() {
    assert_snapshot!(serialize_render("# Heading One", 80));
}

#[test]
fn snap_heading_h2() {
    assert_snapshot!(serialize_render("## Heading Two", 80));
}

#[test]
fn snap_heading_h3() {
    assert_snapshot!(serialize_render("### Heading Three", 80));
}

#[test]
fn snap_heading_h4() {
    assert_snapshot!(serialize_render("#### Heading Four", 80));
}

#[test]
fn snap_heading_h5() {
    assert_snapshot!(serialize_render("##### Heading Five", 80));
}

#[test]
fn snap_heading_h6() {
    assert_snapshot!(serialize_render("###### Heading Six", 80));
}

#[test]
fn snap_setext_heading_h1() {
    assert_snapshot!(serialize_render("Heading One\n===", 80));
}

#[test]
fn snap_setext_heading_h2() {
    assert_snapshot!(serialize_render("Heading Two\n---", 80));
}

#[test]
fn snap_paragraph_emphasis() {
    assert_snapshot!(serialize_render("This is *emphasised* text.", 80));
}

#[test]
fn snap_paragraph_strong() {
    assert_snapshot!(serialize_render("This is **strong** text.", 80));
}

#[test]
fn snap_paragraph_inline_code() {
    assert_snapshot!(serialize_render("Use `cargo build` to compile.", 80));
}

#[test]
fn snap_fenced_code_no_lang() {
    assert_snapshot!(serialize_render("```\nlet x = 1;\n```", 80));
}

#[test]
fn snap_fenced_code_with_lang() {
    assert_snapshot!(serialize_render("```rust\nfn main() {}\n```", 80));
}

#[test]
fn snap_indented_code() {
    assert_snapshot!(serialize_render("    let x = 1;", 80));
}

#[test]
fn snap_blockquote() {
    assert_snapshot!(serialize_render("> A quoted line.", 80));
}

#[test]
fn snap_unordered_list() {
    assert_snapshot!(serialize_render("- first\n- second\n- third", 80));
}

#[test]
fn snap_ordered_list() {
    assert_snapshot!(serialize_render("1. first\n2. second\n3. third", 80));
}

#[test]
fn snap_nested_list() {
    assert_snapshot!(serialize_render("- outer\n  - inner-a\n  - inner-b", 80));
}

#[test]
fn snap_link() {
    assert_snapshot!(serialize_render("See [docs](https://example.com).", 80));
}

#[test]
fn snap_autolink() {
    assert_snapshot!(serialize_render("Visit <https://example.com> now.", 80));
}

#[test]
fn snap_link_reference_definition() {
    assert_snapshot!(serialize_render(
        "See [docs][1].\n\n[1]: https://example.com",
        80
    ));
}

#[test]
fn snap_image() {
    assert_snapshot!(serialize_render("![alt](pic.png)", 80));
}

#[test]
fn snap_html_block() {
    assert_snapshot!(serialize_render("<div>raw html</div>", 80));
}

#[test]
fn snap_hashtag_in_paragraph() {
    assert_snapshot!(serialize_render("prefix #tag suffix", 80));
}

#[test]
fn snap_hashtag_in_inline_code_suppressed() {
    assert_snapshot!(serialize_render("call `#tag` here", 80));
}

#[test]
fn snap_hashtag_in_fenced_suppressed() {
    assert_snapshot!(serialize_render("```\n#nope\n```", 80));
}

#[test]
fn snap_hashtag_in_link_destination_suppressed() {
    assert_snapshot!(serialize_render(
        "See [docs](https://example.com/#fragment).",
        80
    ));
}

#[test]
fn snap_multi_block_combined() {
    let text = "# Title\n\
                \n\
                Intro with *em* and **bold** and `code` and #tag.\n\
                \n\
                ## Section\n\
                \n\
                > Quoted line.\n\
                \n\
                - first\n\
                - second\n\
                  - nested\n\
                \n\
                ```rust\n\
                fn main() {}\n\
                ```\n\
                \n\
                Trailer with [link](https://ex.com).";
    assert_snapshot!(serialize_render(text, 80));
}

#[test]
fn snap_long_line_wraps() {
    let line: String = "word ".repeat(40);
    assert_snapshot!(serialize_render(&line, 40));
}
