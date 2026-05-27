//! Incremental tree-sitter parser for the markdown editor.
//!
//! `EditorTree` owns one `MarkdownParser` + persisted `MarkdownTree`. The
//! `MarkdownParser` convenience wrapper from `tree-sitter-md` handles both the
//! block grammar and the per-block inline grammar, and applies a single
//! `InputEdit` to all internal trees in lock-step.
//!
//! `source: Vec<u8>` mirrors `lines.join("\n")` byte-for-byte and is patched
//! in place on each `apply_edit`. `line_offsets[i]` is the byte index of the
//! start of logical line `i`; recomputed (or shift-patched) per edit.

use tree_sitter::{InputEdit, Node};
use tree_sitter_md::{MarkdownParser, MarkdownTree};

pub struct EditorTree {
    parser: MarkdownParser,
    tree: Option<MarkdownTree>,
    source: Vec<u8>,
    line_offsets: Vec<usize>,
}

impl EditorTree {
    pub fn new() -> Self {
        Self {
            parser: MarkdownParser::default(),
            tree: None,
            source: Vec::new(),
            line_offsets: vec![0],
        }
    }

    /// Reparse from scratch, discarding any prior tree.
    pub fn parse_full(&mut self, lines: &[String]) {
        rebuild_source(&mut self.source, &mut self.line_offsets, lines);
        self.tree = self.parser.parse(&self.source, None);
    }

    /// Apply a single `InputEdit` to the persisted tree, patch `source` and
    /// `line_offsets` in place, then incrementally reparse against the prior
    /// tree.
    pub fn apply_edit(&mut self, edit: InputEdit, lines: &[String]) {
        // Mirror the edit on the source bytes. The edit's byte range describes
        // the OLD-tree slice that gets replaced by the corresponding NEW
        // slice; recompute the new slice from `lines` using the edit's
        // `new_end_position`.
        let new_slice = slice_for_new_end(lines, edit);
        self.source
            .splice(edit.start_byte..edit.old_end_byte, new_slice.iter().copied());

        // Patch line_offsets: rebuild from the edit's start row onward.
        // Simpler and provably correct vs. delta-shifting; bounded by the
        // damaged tail of the buffer.
        recompute_line_offsets_from(&mut self.line_offsets, lines, edit.start_position.row);

        if let Some(old_tree) = self.tree.as_mut() {
            old_tree.edit(&edit);
        }
        self.tree = self.parser.parse(&self.source, self.tree.as_ref());

        if verify_incremental_enabled() {
            self.verify_against_fresh_parse(lines);
        }
    }

    /// When `KIMUN_EDITOR_VERIFY_INCREMENTAL=1`, parse `lines` from scratch on
    /// a fresh `MarkdownParser` and compare the resulting block tree against
    /// the post-`apply_edit` tree node-by-node. Panics on mismatch.
    fn verify_against_fresh_parse(&self, lines: &[String]) {
        let mut fresh = EditorTree::new();
        fresh.parse_full(lines);

        let inc_kinds: Vec<(String, usize, usize)> = self
            .walk_blocks()
            .iter()
            .map(|n| (n.kind().to_string(), n.start_byte(), n.end_byte()))
            .collect();
        let fresh_kinds: Vec<(String, usize, usize)> = fresh
            .walk_blocks()
            .iter()
            .map(|n| (n.kind().to_string(), n.start_byte(), n.end_byte()))
            .collect();
        assert_eq!(
            inc_kinds, fresh_kinds,
            "KIMUN_EDITOR_VERIFY_INCREMENTAL: incremental tree diverged from fresh parse",
        );
    }

    /// Smallest block-tree node containing `byte`, descending into the inline
    /// tree if the leaf block-tree node is an `inline` node with finer
    /// resolution available.
    pub fn node_at_byte(&self, byte: usize) -> Option<Node<'_>> {
        let tree = self.tree.as_ref()?;
        let block_root = tree.block_tree().root_node();
        let block_node = block_root.descendant_for_byte_range(byte, byte)?;
        if block_node.kind() == "inline" {
            if let Some(inline_tree) = tree.inline_tree(&block_node) {
                if let Some(n) = inline_tree
                    .root_node()
                    .descendant_for_byte_range(byte, byte)
                {
                    return Some(n);
                }
            }
        }
        Some(block_node)
    }

    /// Iterator over `node` and its ancestors up to the root.
    pub fn ancestors<'a>(&'a self, node: Node<'a>) -> impl Iterator<Item = Node<'a>> {
        std::iter::successors(Some(node), |n| n.parent())
    }

    /// DFS over every block-tree node.
    pub fn walk_blocks(&self) -> Vec<Node<'_>> {
        let mut out = Vec::new();
        if let Some(tree) = self.tree.as_ref() {
            let mut cursor = tree.block_tree().walk();
            let mut visited_children = false;
            loop {
                if !visited_children {
                    out.push(cursor.node());
                    if cursor.goto_first_child() {
                        continue;
                    }
                    visited_children = true;
                }
                if cursor.goto_next_sibling() {
                    visited_children = false;
                    continue;
                }
                if !cursor.goto_parent() {
                    break;
                }
            }
        }
        out
    }

    pub fn source(&self) -> &[u8] {
        &self.source
    }

    pub fn line_offsets(&self) -> &[usize] {
        &self.line_offsets
    }

    pub fn markdown_tree(&self) -> Option<&MarkdownTree> {
        self.tree.as_ref()
    }

    /// Compute the byte offset of `(row, col_chars)` in the current source,
    /// where `col_chars` is a Unicode-scalar index into the row's text.
    /// Returns `None` if the row is out of range.
    pub fn byte_offset(&self, row: usize, col_chars: usize, lines: &[String]) -> Option<usize> {
        let line_start = *self.line_offsets.get(row)?;
        let line = lines.get(row)?;
        let col_byte: usize = line
            .char_indices()
            .nth(col_chars)
            .map(|(b, _)| b)
            .unwrap_or_else(|| line.len());
        Some(line_start + col_byte)
    }
}

impl Default for EditorTree {
    fn default() -> Self {
        Self::new()
    }
}

fn verify_incremental_enabled() -> bool {
    std::env::var("KIMUN_EDITOR_VERIFY_INCREMENTAL")
        .ok()
        .as_deref()
        == Some("1")
}

fn rebuild_source(source: &mut Vec<u8>, line_offsets: &mut Vec<usize>, lines: &[String]) {
    source.clear();
    line_offsets.clear();
    if lines.is_empty() {
        line_offsets.push(0);
        return;
    }
    for (i, line) in lines.iter().enumerate() {
        line_offsets.push(source.len());
        source.extend_from_slice(line.as_bytes());
        if i + 1 < lines.len() {
            source.push(b'\n');
        }
    }
}

fn recompute_line_offsets_from(
    line_offsets: &mut Vec<usize>,
    lines: &[String],
    start_row: usize,
) {
    line_offsets.truncate(start_row);
    if start_row > lines.len() {
        return;
    }
    let mut cursor = if start_row == 0 {
        0
    } else {
        // Re-derive the start offset of `start_row` from the previous line's
        // offset + its byte length + 1 for the trailing '\n'.
        let prev_start = *line_offsets.last().unwrap();
        let prev_line_len = lines[start_row - 1].len();
        prev_start + prev_line_len + 1
    };
    if start_row >= lines.len() {
        return;
    }
    for line in &lines[start_row..] {
        line_offsets.push(cursor);
        cursor += line.len() + 1; // +1 for '\n' separator
    }
}

/// Extract the byte slice that the edit's new content occupies in `lines`
/// (i.e. the source bytes from `edit.start_position` to `edit.new_end_position`).
fn slice_for_new_end(lines: &[String], edit: InputEdit) -> Vec<u8> {
    // Reconstruct the new content by walking the new lines from
    // start_position to new_end_position.
    let mut out = Vec::with_capacity(edit.new_end_byte.saturating_sub(edit.start_byte));
    let s_row = edit.start_position.row;
    let s_col = edit.start_position.column;
    let e_row = edit.new_end_position.row;
    let e_col = edit.new_end_position.column;
    if s_row >= lines.len() {
        return out;
    }
    if s_row == e_row {
        let line = lines[s_row].as_bytes();
        let end = e_col.min(line.len());
        let start = s_col.min(end);
        out.extend_from_slice(&line[start..end]);
        return out;
    }
    let first_line = lines[s_row].as_bytes();
    let start = s_col.min(first_line.len());
    out.extend_from_slice(&first_line[start..]);
    out.push(b'\n');
    for row in (s_row + 1)..e_row {
        if row < lines.len() {
            out.extend_from_slice(lines[row].as_bytes());
        }
        out.push(b'\n');
    }
    if e_row < lines.len() {
        let last_line = lines[e_row].as_bytes();
        let end = e_col.min(last_line.len());
        out.extend_from_slice(&last_line[..end]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Point;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn vec_lines(s: &str) -> Vec<String> {
        if s.is_empty() {
            vec![String::new()]
        } else {
            s.split('\n').map(|l| l.to_string()).collect()
        }
    }

    fn assert_invariants(et: &EditorTree, lines: &[String]) {
        // source mirrors lines.join("\n")
        let expected = lines.join("\n");
        assert_eq!(et.source(), expected.as_bytes(), "source must mirror lines.join(\\n)");
        // line_offsets length equals lines.len()
        assert_eq!(et.line_offsets().len(), lines.len(), "one offset per line");
        // line_offsets monotone increasing
        for w in et.line_offsets().windows(2) {
            assert!(w[0] < w[1], "line_offsets must be monotone");
        }
        // Each offset equals the cumulative position of that line's start.
        let mut expected_off = 0usize;
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(et.line_offsets()[i], expected_off, "offset[{i}] mismatch");
            expected_off += line.len() + 1;
        }
    }

    // ── parse_full ───────────────────────────────────────────────────────────

    #[test]
    fn parse_full_empty_buffer() {
        let lines = vec_lines("");
        let mut et = EditorTree::new();
        et.parse_full(&lines);
        assert!(et.markdown_tree().is_some(), "empty buffer still produces a tree");
        assert_invariants(&et, &lines);
    }

    #[test]
    fn parse_full_single_line_paragraph() {
        let lines = vec_lines("hello world");
        let mut et = EditorTree::new();
        et.parse_full(&lines);
        assert_invariants(&et, &lines);
        let tree = et.markdown_tree().unwrap();
        let root = tree.block_tree().root_node();
        assert_eq!(root.kind(), "document");
        // Section → paragraph → inline
        let section = root.child(0).unwrap();
        assert_eq!(section.kind(), "section");
        let paragraph = section.child(0).unwrap();
        assert_eq!(paragraph.kind(), "paragraph");
    }

    #[test]
    fn parse_full_multi_block() {
        let lines = vec_lines("# Title\n\nA paragraph.\n\n```\ncode\n```");
        let mut et = EditorTree::new();
        et.parse_full(&lines);
        assert_invariants(&et, &lines);
        let kinds: Vec<&str> = et.walk_blocks().iter().map(|n| n.kind()).collect();
        assert!(kinds.contains(&"atx_heading"), "kinds missing atx_heading: {kinds:?}");
        assert!(kinds.contains(&"paragraph"), "kinds missing paragraph: {kinds:?}");
        assert!(kinds.contains(&"fenced_code_block"), "kinds missing fenced_code_block: {kinds:?}");
    }

    #[test]
    fn parse_full_multibyte_utf8() {
        let lines = vec_lines("héllo wörld 🦀");
        let mut et = EditorTree::new();
        et.parse_full(&lines);
        assert_invariants(&et, &lines);
        let root = et.markdown_tree().unwrap().block_tree().root_node();
        assert_eq!(root.kind(), "document");
    }

    // ── apply_edit ───────────────────────────────────────────────────────────

    #[test]
    fn apply_edit_intra_line_insert() {
        let mut lines = vec_lines("abc");
        let mut et = EditorTree::new();
        et.parse_full(&lines);

        // Insert 'X' at byte 1 (between 'a' and 'b').
        lines[0].insert(1, 'X');
        let edit = InputEdit {
            start_byte: 1,
            old_end_byte: 1,
            new_end_byte: 2,
            start_position: Point::new(0, 1),
            old_end_position: Point::new(0, 1),
            new_end_position: Point::new(0, 2),
        };
        et.apply_edit(edit, &lines);
        assert_invariants(&et, &lines);
        assert_eq!(et.source(), b"aXbc");
    }

    #[test]
    fn apply_edit_intra_line_delete() {
        let mut lines = vec_lines("aXbc");
        let mut et = EditorTree::new();
        et.parse_full(&lines);

        // Delete 'X' at byte 1.
        lines[0].remove(1);
        let edit = InputEdit {
            start_byte: 1,
            old_end_byte: 2,
            new_end_byte: 1,
            start_position: Point::new(0, 1),
            old_end_position: Point::new(0, 2),
            new_end_position: Point::new(0, 1),
        };
        et.apply_edit(edit, &lines);
        assert_invariants(&et, &lines);
        assert_eq!(et.source(), b"abc");
    }

    #[test]
    fn apply_edit_newline_insert_mid_line() {
        let mut lines = vec_lines("abc");
        let mut et = EditorTree::new();
        et.parse_full(&lines);

        // Split "abc" -> "a", "bc" by inserting '\n' at col 1.
        lines = vec_lines("a\nbc");
        let edit = InputEdit {
            start_byte: 1,
            old_end_byte: 1,
            new_end_byte: 2,
            start_position: Point::new(0, 1),
            old_end_position: Point::new(0, 1),
            new_end_position: Point::new(1, 0),
        };
        et.apply_edit(edit, &lines);
        assert_invariants(&et, &lines);
        assert_eq!(et.source(), b"a\nbc");
    }

    #[test]
    fn apply_edit_line_merge_backspace_at_col_zero() {
        let mut lines = vec_lines("a\nbc");
        let mut et = EditorTree::new();
        et.parse_full(&lines);

        // Backspace at row 1, col 0 — merges "bc" onto "a" to yield "abc".
        lines = vec_lines("abc");
        let edit = InputEdit {
            start_byte: 1,
            old_end_byte: 2,
            new_end_byte: 1,
            start_position: Point::new(0, 1),
            old_end_position: Point::new(1, 0),
            new_end_position: Point::new(0, 1),
        };
        et.apply_edit(edit, &lines);
        assert_invariants(&et, &lines);
        assert_eq!(et.source(), b"abc");
    }

    #[test]
    fn parse_full_after_apply_edit_matches_reparse_from_scratch() {
        let lines_old = vec_lines("hello\nworld");
        let mut et = EditorTree::new();
        et.parse_full(&lines_old);

        // Insert "X" mid-line on row 0.
        let mut lines_new = lines_old.clone();
        lines_new[0].insert(2, 'X');
        let edit = InputEdit {
            start_byte: 2,
            old_end_byte: 2,
            new_end_byte: 3,
            start_position: Point::new(0, 2),
            old_end_position: Point::new(0, 2),
            new_end_position: Point::new(0, 3),
        };
        et.apply_edit(edit, &lines_new);

        // Reparse from scratch and compare structural kinds.
        let mut from_scratch = EditorTree::new();
        from_scratch.parse_full(&lines_new);

        let inc_kinds: Vec<(String, usize, usize)> = et
            .walk_blocks()
            .iter()
            .map(|n| (n.kind().to_string(), n.start_byte(), n.end_byte()))
            .collect();
        let fresh_kinds: Vec<(String, usize, usize)> = from_scratch
            .walk_blocks()
            .iter()
            .map(|n| (n.kind().to_string(), n.start_byte(), n.end_byte()))
            .collect();
        assert_eq!(inc_kinds, fresh_kinds, "incremental and full parses must match");
    }

    // ── node_at_byte ─────────────────────────────────────────────────────────

    #[test]
    fn node_at_byte_inside_inline_code() {
        let lines = vec_lines("use `code` here");
        let mut et = EditorTree::new();
        et.parse_full(&lines);
        // Cursor at byte 6 — middle of 'c' in `code`.
        let node = et.node_at_byte(6).expect("node at byte 6");
        // Walk ancestors; one should be a code_span.
        let in_code = et
            .ancestors(node)
            .any(|n| n.kind() == "code_span");
        assert!(in_code, "byte 6 should be inside code_span; got kinds {:?}",
            et.ancestors(node).map(|n| n.kind()).collect::<Vec<_>>());
    }

    #[test]
    fn node_at_byte_in_plain_paragraph() {
        let lines = vec_lines("plain text here");
        let mut et = EditorTree::new();
        et.parse_full(&lines);
        let node = et.node_at_byte(5).expect("node at byte 5");
        let in_code = et.ancestors(node).any(|n| n.kind() == "code_span");
        assert!(!in_code, "plain text byte should not be inside code_span");
    }

    // ── byte_offset helper ───────────────────────────────────────────────────

    // ── KIMUN_EDITOR_VERIFY_INCREMENTAL flag ─────────────────────────────────

    /// Single combined test (toggle on, expect panic; toggle off, expect no
    /// panic) so the process-wide env var is touched from exactly one
    /// thread. Splitting the two cases into separate `#[test]` functions
    /// race when the harness runs tests in parallel.
    #[test]
    fn verify_flag_behaviour() {
        let prev = std::env::var("KIMUN_EDITOR_VERIFY_INCREMENTAL").ok();

        let bad_edit = |start: usize| InputEdit {
            start_byte: start,
            old_end_byte: start + 1,
            new_end_byte: start + 1,
            start_position: Point::new(0, start),
            old_end_position: Point::new(0, start + 1),
            new_end_position: Point::new(0, start + 1),
        };

        // ON — must panic.
        unsafe { std::env::set_var("KIMUN_EDITOR_VERIFY_INCREMENTAL", "1") };
        let panicked_on = {
            let mut lines = vec_lines("abc");
            let mut et = EditorTree::new();
            et.parse_full(&lines);
            lines[0].insert(1, 'X');
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                et.apply_edit(bad_edit(1), &lines);
            }))
            .is_err()
        };

        // OFF — must not panic.
        unsafe { std::env::remove_var("KIMUN_EDITOR_VERIFY_INCREMENTAL") };
        let panicked_off = {
            let mut lines = vec_lines("abc");
            let mut et = EditorTree::new();
            et.parse_full(&lines);
            lines[0].insert(1, 'X');
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                et.apply_edit(bad_edit(1), &lines);
            }))
            .is_err()
        };

        // Restore.
        match prev {
            Some(v) => unsafe { std::env::set_var("KIMUN_EDITOR_VERIFY_INCREMENTAL", v) },
            None => unsafe { std::env::remove_var("KIMUN_EDITOR_VERIFY_INCREMENTAL") },
        }

        assert!(panicked_on, "verify flag ON must catch bad InputEdit");
        assert!(!panicked_off, "verify flag OFF must not panic");
    }

    #[test]
    fn byte_offset_handles_multi_byte_chars() {
        let lines = vec_lines("héllo\nwörld");
        let mut et = EditorTree::new();
        et.parse_full(&lines);
        // Row 1 col 1 in chars = byte 1 within "wörld" (after 'w', before 'ö').
        // line_offsets[1] = 7 (5 bytes "héllo" + 1 '\n' + ...). Wait —
        // "héllo" is 6 bytes (h, é=2 bytes, l, l, o). So offset[1] = 7.
        let off = et.byte_offset(1, 1, &lines).unwrap();
        assert_eq!(off, 8);
    }
}
