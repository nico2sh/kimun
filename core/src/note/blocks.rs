//! The display blocks of a note: a flat, document-order list that a facet
//! view shows, hides and reveals one at a time. See `CONTEXT.md`, **Facet
//! view**.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::walk;

/// What a [`NoteBlock`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// A heading and its level, 1 to 6.
    Heading(u8),
    Paragraph,
    /// A list item's own text, without its nested lists.
    ListItem,
    /// One body row of a table.
    TableRow,
    Code,
    Html,
    Rule,
}

/// One unit a facet view shows or hides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteBlock {
    /// Byte range of the block's own text in the note as given. A list
    /// item's range stops where its first nested list starts.
    pub range: Range<usize>,
    pub kind: BlockKind,
    /// List nesting depth: 0 for a top-level item and for every block that
    /// is not inside a list.
    pub depth: u8,
    /// A table row's header and delimiter rows, `None` for any other block.
    pub lead: Option<Range<usize>>,
}

/// The blocks of `text` in document order, frontmatter skipped.
pub fn note_blocks(text: &str) -> Vec<NoteBlock> {
    let (offset, _) = walk::body_start(text);
    let body = &text[offset..];
    let parser = Parser::new_ext(body, Options::ENABLE_TABLES).into_offset_iter();

    let mut blocks: Vec<NoteBlock> = Vec::new();
    // Open list items: the item's block index, and whether its own range is
    // already cut at its first nested list.
    let mut items: Vec<(usize, bool)> = Vec::new();
    let mut table: Option<Range<usize>> = None;
    let mut table_rows = 0usize;
    let mut lead: Option<Range<usize>> = None;
    let mut in_head = false;
    let mut skip_depth = 0usize;

    let push = |blocks: &mut Vec<NoteBlock>,
                range: Range<usize>,
                kind: BlockKind,
                depth: u8,
                lead: Option<Range<usize>>| {
        let range = trim_range(text, range.start + offset..range.end + offset);
        blocks.push(NoteBlock {
            range,
            kind,
            depth,
            lead: lead.map(|l| l.start + offset..l.end + offset),
        });
    };

    for (event, range) in parser {
        if skip_depth > 0 {
            match event {
                Event::Start(_) => skip_depth += 1,
                Event::End(_) => skip_depth -= 1,
                _ => {}
            }
            continue;
        }
        // Inside an item's own text (before its first nested list) only the
        // item is a block: any other child is part of that text.
        let uncut = items.last().is_some_and(|&(_, cut)| !cut);
        let child_of_text = matches!(
            event,
            Event::Start(Tag::Heading { .. })
                | Event::Start(Tag::CodeBlock(_))
                | Event::Start(Tag::HtmlBlock)
                | Event::Start(Tag::Table(_))
        );
        if uncut && child_of_text {
            skip_depth = 1;
            continue;
        }
        if uncut && matches!(event, Event::Rule) {
            continue;
        }
        // Depth of a block that sits after an item's own text was cut.
        let nested = items
            .last()
            .map_or(0, |&(idx, _)| blocks[idx].depth.saturating_add(1));
        match event {
            Event::Start(Tag::Item) => {
                let depth = items.len().min(u8::MAX as usize) as u8;
                items.push((blocks.len(), false));
                push(&mut blocks, range, BlockKind::ListItem, depth, None);
            }
            Event::End(TagEnd::Item) => {
                items.pop();
            }
            Event::Start(Tag::List(_)) => {
                if let Some((idx, cut)) = items.last_mut() {
                    if !*cut {
                        *cut = true;
                        let end = range.start + offset;
                        let block = &mut blocks[*idx];
                        block.range = trim_range(text, block.range.start..end);
                    }
                }
            }
            Event::Start(Tag::Paragraph) => {
                let inside_own_text = items.last().is_some_and(|&(_, cut)| !cut);
                if !inside_own_text {
                    push(&mut blocks, range, BlockKind::Paragraph, nested, None);
                }
                skip_depth = 1;
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let level = level as u8;
                push(&mut blocks, range, BlockKind::Heading(level), nested, None);
                skip_depth = 1;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                push(&mut blocks, range, BlockKind::Code, nested, None);
                skip_depth = 1;
            }
            Event::Start(Tag::HtmlBlock) => {
                push(&mut blocks, range, BlockKind::Html, nested, None);
                skip_depth = 1;
            }
            Event::Rule => push(&mut blocks, range, BlockKind::Rule, nested, None),
            Event::Start(Tag::Table(_)) => {
                table = Some(range);
                table_rows = 0;
                lead = None;
            }
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableRow) if !in_head => {
                let table_start = table.as_ref().map_or(range.start, |t| t.start);
                let header = lead.get_or_insert(table_start..range.start).clone();
                push(
                    &mut blocks,
                    range,
                    BlockKind::TableRow,
                    nested,
                    Some(header),
                );
                table_rows += 1;
                skip_depth = 1;
            }
            Event::End(TagEnd::Table) => {
                if table_rows == 0 {
                    if let Some(t) = table.take() {
                        push(&mut blocks, t, BlockKind::TableRow, nested, None);
                    }
                }
                table = None;
            }
            _ => {}
        }
    }
    blocks
}

/// `range` without trailing whitespace, so a CRLF note and the same note in
/// LF give blocks that read alike.
fn trim_range(text: &str, range: Range<usize>) -> Range<usize> {
    let end = range.start + text[range.clone()].trim_end().len();
    range.start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(BlockKind, u8)> {
        note_blocks(text)
            .into_iter()
            .map(|b| (b.kind, b.depth))
            .collect()
    }

    fn slice<'a>(text: &'a str, b: &NoteBlock) -> &'a str {
        &text[b.range.clone()]
    }

    #[test]
    fn paragraphs_and_headings_in_order() {
        assert_eq!(
            kinds("# A\n\nOne.\n\nTwo.\n"),
            [
                (BlockKind::Heading(1), 0),
                (BlockKind::Paragraph, 0),
                (BlockKind::Paragraph, 0)
            ]
        );
    }

    #[test]
    fn list_item_own_text_stops_at_the_nested_list() {
        let text = "- a\n  - b\n  - c\n- d\n";
        let blocks = note_blocks(text);
        let depths: Vec<u8> = blocks.iter().map(|b| b.depth).collect();
        assert_eq!(depths, [0, 1, 1, 0]);
        assert!(blocks.iter().all(|b| b.kind == BlockKind::ListItem));
        assert_eq!(slice(text, &blocks[0]), "- a");
        assert_eq!(slice(text, &blocks[1]), "- b");
        assert_eq!(slice(text, &blocks[3]), "- d");
    }

    #[test]
    fn loose_item_with_two_paragraphs_is_one_block() {
        assert_eq!(kinds("- a\n\n  more\n"), [(BlockKind::ListItem, 0)]);
    }

    #[test]
    fn table_rows_carry_the_header() {
        let text = "| h1 | h2 |\n|----|----|\n| a | b |\n| c | d |\n";
        let blocks = note_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert!(blocks.iter().all(|b| b.kind == BlockKind::TableRow));
        let lead = blocks[0].lead.clone().expect("lead");
        assert_eq!(&text[lead.clone()], "| h1 | h2 |\n|----|----|\n");
        assert_eq!(blocks[1].lead, Some(lead));
        assert_eq!(slice(text, &blocks[0]), "| a | b |");
    }

    #[test]
    fn blockquote_contributes_inner_blocks() {
        assert_eq!(
            kinds("> one\n>\n> - item\n"),
            [(BlockKind::Paragraph, 0), (BlockKind::ListItem, 0)]
        );
    }

    #[test]
    fn code_html_and_rule_are_blocks() {
        assert_eq!(
            kinds("```\ncode\n```\n\n<div>x</div>\n\n---\n"),
            [
                (BlockKind::Code, 0),
                (BlockKind::Html, 0),
                (BlockKind::Rule, 0)
            ]
        );
    }

    #[test]
    fn frontmatter_is_skipped() {
        let text = "---\nt: x\n---\nBody\n";
        let blocks = note_blocks(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(slice(text, &blocks[0]), "Body");
    }

    #[test]
    fn crlf_note_matches_lf_note() {
        let samples = [
            "# A\n\n- a\n  - b\n- c\n\n| h |\n|---|\n| x |\n\n> q\n",
            "---\nt: x\n---\nBody\n\ntext\n",
        ];
        for lf in samples {
            let crlf = lf.replace('\n', "\r\n");
            assert_eq!(kinds(lf), kinds(&crlf), "{lf:?}");
        }
    }

    #[test]
    fn empty_and_frontmatter_only_notes_give_no_blocks() {
        assert!(note_blocks("").is_empty());
        assert!(note_blocks("---\nt: x\n---\n").is_empty());
    }

    #[test]
    fn blocks_never_overlap_inside_list_items() {
        let samples = [
            "- a\n\n  ```\n  x\n  ```\n\n  b #t\n",
            "- a #t\n  ```\n  x\n  ```\n- c\n",
            "- a\n\n  ---\n\n  b\n",
            "- a\n  | h |\n  |---|\n  | r #t |\n",
            "- # H #t\n- c\n",
        ];
        for text in samples {
            let blocks = note_blocks(text);
            for pair in blocks.windows(2) {
                assert!(
                    pair[0].range.end <= pair[1].range.start,
                    "{text:?}: {:?} overlaps {:?}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[test]
    fn extreme_list_nesting_does_not_overflow() {
        let text: String = (0..300)
            .map(|i| format!("{}- x\n", "  ".repeat(i)))
            .collect();
        let blocks = note_blocks(&text);
        assert_eq!(blocks.len(), 300);
        assert_eq!(blocks.last().map(|b| b.depth), Some(u8::MAX));
    }
}
