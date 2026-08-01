//! What the editor actually paints, pinned cell by cell.
//!
//! This is the contract the editing-engine swap (adr/0039) must preserve: any
//! diff here is a regression until argued otherwise, and the handful of intended
//! differences are enumerated in `plans/2026-07-30-editor-engine.md`.
//!
//! The corpus came from the abandoned tree-sitter change, which snapshotted
//! `MarkdownSpanner::render_with`'s spans. This harness renders through
//! `MarkdownEditorView` into a `TestBackend` instead, so a snapshot covers the
//! whole drawn result — wrapping, concealed sigils, the blockquote bar, the code
//! box background, the cursor's reveal — rather than one function's output. The
//! recorded outputs were captured in July 2026 against the pulldown renderer, not
//! in May against a renderer that no longer exists.
//!
//! Not covered, and deliberately: overlays that the view is *told* about rather
//! than deriving (the find bar's matches, a search needle, the replace preview,
//! the selection) and anything the drawer or the panel layout contributes. Those
//! have their own tests. A pass here is not a claim that the editor screen is
//! unchanged, only that the editor's own painting is.

use std::num::NonZeroU64;

use insta::assert_snapshot;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use kimun_notes::components::text_editor::snapshot::EditorSnapshot;
use kimun_notes::components::text_editor::view::MarkdownEditorView;
use kimun_notes::settings::themes::Theme;

/// Render `text` and dump every painted cell.
///
/// A trailing row is appended and the cursor parked on it, because the cursor's
/// row is **revealed** — drawn as raw markdown so it can be edited — and a corpus
/// meant to pin the *styled* rendering must therefore keep the cursor off it. The
/// reveal itself is pinned separately, by the cases that pass a cursor.
fn painted(text: &str, width: u16) -> String {
    let with_room = format!("{text}\n");
    let rows = with_room.split('\n').count();
    render(&with_room, width, PANE_HEIGHT, (rows - 1, 0))
}

/// Render with the cursor at `cursor`, so its row reveals.
fn painted_with_cursor(text: &str, width: u16, cursor: (usize, usize)) -> String {
    render(text, width, PANE_HEIGHT, cursor)
}

/// Tall enough that no case scrolls, since a scrolled snapshot silently loses its
/// first rows — sizing the pane by *logical* rows did exactly that to every case
/// that wraps. Trailing blank rows are trimmed from the dump, and `render`
/// refuses to record a snapshot that reached the bottom of the pane.
const PANE_HEIGHT: u16 = 40;

fn render(text: &str, width: u16, height: u16, cursor: (usize, usize)) -> String {
    let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let revision = NonZeroU64::new(1).expect("one is not zero");
    let snapshot = EditorSnapshot::borrowed(&lines, cursor, revision);

    let rect = Rect {
        x: 0,
        y: 0,
        width,
        height: height.max(1),
    };
    let theme = Theme::default();
    let mut view = MarkdownEditorView::new();
    view.update(&snapshot, rect);

    let mut terminal =
        Terminal::new(TestBackend::new(rect.width, rect.height)).expect("test backend");
    terminal
        .draw(|frame| view.render(frame, rect, &theme, false, None))
        .expect("draw");

    dump(terminal.backend().buffer())
}

/// One block per painted row: the row's text, then the style runs under it.
///
/// Trailing cells that are blank *and* unstyled are dropped, since a wide pane
/// would otherwise bury every case in padding — but a blank cell carrying a
/// background is kept, because that is how the code box shows up.
fn dump(buffer: &ratatui::buffer::Buffer) -> String {
    let area = buffer.area;
    let rows: Vec<Vec<&ratatui::buffer::Cell>> = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer.cell((x, y)).expect("inside the buffer"))
                .collect()
        })
        .collect();

    // The editor paints its base style across the whole pane, so a row past the
    // end of the text is blank but not unstyled. Trailing ones are dropped; any
    // *interior* blank row is content and stays.
    let last_content = rows.iter().rposition(|row| has_content(row));
    let Some(last_content) = last_content else {
        return String::from("(nothing painted)\n");
    };
    assert!(
        last_content + 1 < rows.len(),
        "the pane was too short: content reached its last row, so the view \
         scrolled and this snapshot would be missing its first rows"
    );

    let mut out = String::new();
    for (y, cells) in rows[..=last_content].iter().enumerate() {
        let last_painted = cells
            .iter()
            .rposition(|cell| cell.symbol() != " " || styled(cell));
        let Some(last) = last_painted else {
            out.push_str(&format!("{y:>2} ·\n"));
            continue;
        };
        let text: String = cells[..=last].iter().map(|cell| cell.symbol()).collect();
        out.push_str(&format!("{y:>2} |{text}|\n"));
        for (range, style) in runs(&cells[..=last]) {
            out.push_str(&format!("   {range} {style}\n"));
        }
    }
    out
}

/// Whether a row shows anything but blanks.
fn has_content(row: &[&ratatui::buffer::Cell]) -> bool {
    row.iter().any(|cell| cell.symbol().trim() != "")
}

fn styled(cell: &ratatui::buffer::Cell) -> bool {
    cell.fg != Color::Reset || cell.bg != Color::Reset || cell.modifier != Modifier::empty()
}

/// Runs of identical styling, as `start..end`, skipping unstyled stretches.
fn runs(cells: &[&ratatui::buffer::Cell]) -> Vec<(String, String)> {
    let key = |cell: &ratatui::buffer::Cell| (cell.fg, cell.bg, cell.modifier);
    let mut out = Vec::new();
    let mut start = 0;
    while start < cells.len() {
        let mut end = start + 1;
        while end < cells.len() && key(cells[end]) == key(cells[start]) {
            end += 1;
        }
        if styled(cells[start]) {
            let (fg, bg, modifier) = key(cells[start]);
            out.push((
                format!("{start}..{end}"),
                format!("fg={fg:?} bg={bg:?} mods={modifier:?}"),
            ));
        }
        start = end;
    }
    out
}

// -- labels and hashtags -----------------------------------------------------

#[test]
fn snap_label_in_paragraph() {
    assert_snapshot!(painted("see #rust later", 80));
}

#[test]
fn snap_label_inside_inline_code() {
    assert_snapshot!(painted("use `#foo` here", 80));
}

#[test]
fn snap_label_inside_markdown_link() {
    assert_snapshot!(painted("[see docs](#section) and #real", 80));
}

#[test]
fn snap_label_inside_link_display_text() {
    assert_snapshot!(painted("[#todo](notes/project.md)", 80));
}

#[test]
fn snap_label_after_label_char() {
    assert_snapshot!(painted("foo#bar baz", 80));
}

#[test]
fn snap_label_double_hash() {
    assert_snapshot!(painted("##draft", 80));
}

#[test]
fn snap_label_adjacent_hash_run() {
    assert_snapshot!(painted("#tag#more", 80));
}

#[test]
fn snap_label_inside_fenced_block() {
    assert_snapshot!(painted("before\n```\n#inside\n```\nafter #outside", 80));
}

#[test]
fn snap_hashtag_in_paragraph() {
    assert_snapshot!(painted("prefix #tag suffix", 80));
}

#[test]
fn snap_hashtag_in_inline_code_suppressed() {
    assert_snapshot!(painted("call `#tag` here", 80));
}

#[test]
fn snap_hashtag_in_fenced_suppressed() {
    assert_snapshot!(painted("```\n#nope\n```", 80));
}

#[test]
fn snap_hashtag_in_link_destination_suppressed() {
    assert_snapshot!(painted("See [docs](https://example.com/#fragment).", 80));
}

// -- headings ----------------------------------------------------------------

#[test]
fn snap_heading_h1() {
    assert_snapshot!(painted("# Heading One", 80));
}

#[test]
fn snap_heading_h2() {
    assert_snapshot!(painted("## Heading Two", 80));
}

#[test]
fn snap_heading_h3() {
    assert_snapshot!(painted("### Heading Three", 80));
}

#[test]
fn snap_heading_h4() {
    assert_snapshot!(painted("#### Heading Four", 80));
}

#[test]
fn snap_heading_h5() {
    assert_snapshot!(painted("##### Heading Five", 80));
}

#[test]
fn snap_heading_h6() {
    assert_snapshot!(painted("###### Heading Six", 80));
}

#[test]
fn snap_setext_heading_h1() {
    assert_snapshot!(painted("Heading One\n===", 80));
}

#[test]
fn snap_setext_heading_h2() {
    assert_snapshot!(painted("Heading Two\n---", 80));
}

// -- inline ------------------------------------------------------------------

#[test]
fn snap_paragraph_emphasis() {
    assert_snapshot!(painted("This is *emphasised* text.", 80));
}

#[test]
fn snap_paragraph_strong() {
    assert_snapshot!(painted("This is **strong** text.", 80));
}

#[test]
fn snap_paragraph_inline_code() {
    assert_snapshot!(painted("Use `cargo build` to compile.", 80));
}

#[test]
fn snap_link() {
    assert_snapshot!(painted("See [docs](https://example.com).", 80));
}

#[test]
fn snap_autolink() {
    assert_snapshot!(painted("Visit <https://example.com> now.", 80));
}

#[test]
fn snap_link_reference_definition() {
    assert_snapshot!(painted("See [docs][1].\n\n[1]: https://example.com", 80));
}

#[test]
fn snap_image() {
    assert_snapshot!(painted("![alt](pic.png)", 80));
}

#[test]
fn snap_html_block() {
    assert_snapshot!(painted("<div>raw html</div>", 80));
}

// -- blocks ------------------------------------------------------------------

#[test]
fn snap_fenced_code_no_lang() {
    assert_snapshot!(painted("```\nlet x = 1;\n```", 80));
}

#[test]
fn snap_fenced_code_with_lang() {
    assert_snapshot!(painted("```rust\nfn main() {}\n```", 80));
}

#[test]
fn snap_indented_code() {
    assert_snapshot!(painted("    let x = 1;", 80));
}

#[test]
fn snap_blockquote() {
    assert_snapshot!(painted("> A quoted line.", 80));
}

#[test]
fn snap_unordered_list() {
    assert_snapshot!(painted("- first\n- second\n- third", 80));
}

#[test]
fn snap_ordered_list() {
    assert_snapshot!(painted("1. first\n2. second\n3. third", 80));
}

#[test]
fn snap_nested_list() {
    assert_snapshot!(painted("- outer\n  - inner-a\n  - inner-b", 80));
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
    assert_snapshot!(painted(text, 80));
}

#[test]
fn snap_long_line_wraps() {
    let line = "word ".repeat(40);
    assert_snapshot!(painted(&line, 40));
}

// -- what the cell-level harness adds over the span-level one -----------------

#[test]
fn snap_nested_blockquote_bars() {
    // The `>` sigils are replaced by a bar gutter, one per depth, repeated on
    // wrapped continuation rows. Invisible to a span-level snapshot.
    let text = "> outer\n> > inner runs on long enough that it has to wrap at this width";
    assert_snapshot!(painted(text, 40));
}

#[test]
fn snap_code_box_hugs_the_block() {
    // The background is sized to the widest line of the block and capped at the
    // pane, rather than banding the full width.
    assert_snapshot!(painted(
        "before\n```\nshort\na longer line here\n```\nafter",
        40
    ));
}

#[test]
fn snap_reveal_shows_raw_markdown_under_the_cursor() {
    // The cursor's row drops its styling and shows its sigils.
    assert_snapshot!(painted_with_cursor("# Heading One\nplain text", 80, (0, 3)));
}

#[test]
fn snap_reveal_leaves_other_rows_styled() {
    assert_snapshot!(painted_with_cursor(
        "# Heading One\n## Heading Two",
        80,
        (1, 3)
    ));
}

#[test]
fn snap_wrapped_row_inside_a_list() {
    assert_snapshot!(painted(
        "- an item whose text is long enough to wrap twice over",
        24
    ));
}

/// A setext underline is an all-sigil row: `heading_sigil_end` covers every
/// char of it, so once the underline outruns the pane the wrap mask reserves
/// cells on a continuation row that `render_with` has to fill.
#[test]
fn snap_setext_underline_outruns_the_pane() {
    assert_snapshot!(painted(
        "A rather long setext title that will certainly wrap\n==================================================\n\nbody text after",
        40
    ));
}

/// Every char is a concealed sigil of a Link the cursor is not in, so the
/// styled form of this row is nothing at all.
#[test]
fn snap_empty_link_alone_on_a_row() {
    assert_snapshot!(painted("before\n[](url)\nafter", 80));
}

/// An ATX heading with no text: the sigil run *is* the row, and it wraps.
#[test]
fn snap_all_sigil_heading_wraps() {
    assert_snapshot!(painted("######", 3));
}

/// A row that is entirely concealed markdown reveals in full when the caret is
/// on it, including at end of line — where `elem_at` resolves to no element, so
/// the element-scoped reveal has nothing to expand.
#[test]
fn snap_caret_at_end_of_a_fully_concealed_row() {
    assert_snapshot!(painted_with_cursor("[](url)\nafter", 80, (0, 7)));
}

/// An image row is fully concealed — the placeholder stands in for the whole
/// span, so no character of it is content — which makes it reveal in full at end
/// of line. The placeholder must then step aside, exactly as it does when the
/// caret is inside the image: it is a stand-in for markdown that is not being
/// shown, and drawing both gives `[url]![text](url)`.
#[test]
fn snap_caret_at_end_of_an_image_row_reveals_it_without_the_placeholder() {
    assert_snapshot!(painted_with_cursor("![text](url)\nafter", 80, (0, 12)));
}

/// The same row off the caret: the placeholder alone.
#[test]
fn snap_image_row_without_the_caret_draws_its_placeholder() {
    assert_snapshot!(painted_with_cursor("![text](url)\nafter", 80, (1, 0)));
}

/// The caret inside the image already took this path; it must keep it.
#[test]
fn snap_caret_inside_an_image_reveals_it_without_the_placeholder() {
    assert_snapshot!(painted_with_cursor("![text](url)\nafter", 80, (0, 4)));
}

/// The same row with the caret elsewhere: its styled form is nothing at all.
#[test]
fn snap_fully_concealed_row_without_the_caret() {
    assert_snapshot!(painted_with_cursor("[](url)\nafter", 80, (1, 0)));
}

/// `blockquote_sigil_end` is the quote's first content char, which is the whole
/// row when it holds no text event — so the reveal window outruns the pane and
/// wraps. Every char must still paint.
#[test]
fn snap_blockquote_html_wraps_under_the_caret() {
    assert_snapshot!(painted_with_cursor(
        "> <div class=\"note\">a long html line inside a quote that wraps at forty columns</div>\n\nafter",
        40,
        (0, 85)
    ));
}

/// A click on the caret's own row lands on the character clicked.
///
/// `render_with` reveals the element under the caret, and a revealed element's
/// sigils occupy cells. The inverse mapper used to measure the row as if
/// nothing were revealed, so every column past the first revealed sigil came
/// back short by the width of what it wrongly skipped — clicking the `]` of
/// `see [docs](url) here` put the caret seven characters away.
#[test]
fn a_click_on_the_revealed_row_lands_where_it_was_clicked() {
    use kimun_notes::components::text_editor::markdown::{MarkdownSpanner, ParsedBuffer};

    let line = "see [docs](url) here";
    let buf = ParsedBuffer::parse(&ropetext::Text::from(line));
    let parsed = &buf.lines[0];
    let caret = Some(8); // inside the link, so the row reveals raw

    // With the row revealed every char draws, so the mapping is the identity.
    for col in 0..=line.chars().count() {
        let cell =
            MarkdownSpanner::rendered_col_with_reveal(line, parsed, 0, col, caret, true, false);
        let back = MarkdownSpanner::rendered_col_to_logical_with(
            line, parsed, 0, cell, caret, true, false,
        );
        assert_eq!(
            back, col,
            "logical {col} drew at cell {cell}, which mapped back to {back}"
        );
    }
}

/// The row-level reveal must not become an element-level one: the caret sitting
/// just past a link — at end of a row that has visible prose — reveals nothing,
/// because the row already draws.
#[test]
fn caret_at_end_of_a_drawn_row_reveals_nothing() {
    let out = painted_with_cursor("hello [a](b)\nafter", 40, (0, 12));
    assert!(
        out.contains(" 0 |hello a"),
        "the link must stay styled, not reveal:\n{out}"
    );
}

/// The tail of a wrapped setext underline belongs to the heading, so it draws
/// in the sigil style with no colour seam at the wrap column.
///
/// `visible_positions_with` computes `heading_sigil_end` ungated, and that
/// vector is the wrap mask — so the Layout reserves every cell of the
/// underline. `render_with` gated the same value on `is_first_visual_line`,
/// left the continuation row with no spans, and the empty-content fallback
/// papered over it by re-emitting the raw slice in body `fg`.
#[test]
fn wrapped_setext_underline_draws_in_the_sigil_style() {
    let title = "A rather long setext title that will certainly wrap";
    let out = painted(
        &format!("{title}\n{}\n\nbody text after", "=".repeat(50)),
        40,
    );
    let tail: Vec<&str> = out
        .lines()
        .skip_while(|line| !line.starts_with(" 3 |"))
        .take(2)
        .collect();
    assert!(
        tail.first()
            .is_some_and(|row| row.starts_with(" 3 |==========")),
        "the reserved 10-cell tail of the underline must be painted:\n{out}"
    );
    assert!(
        tail.get(1)
            .is_some_and(|run| run.contains("fg=Rgb(124, 111, 100)")),
        "the tail is heading sigil, so it must not draw in body fg:\n{out}"
    );
}
