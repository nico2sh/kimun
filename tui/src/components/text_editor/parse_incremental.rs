#![allow(dead_code)]
//! Incremental-parse machinery: line-construct classification cache,
//! damage-diff against the previous buffer snapshot, safe-boundary
//! widening, and fence-range derivation. Pure functions only — no
//! `pulldown_cmark` calls (those live in `markdown.rs`).

use std::ops::Range;

/// Coarse classification of a buffer line for safe-boundary widening.
///
/// A line is a *safe boundary* when re-parsing a slice ending on that
/// line is equivalent to the corresponding slice of a full-buffer parse.
/// `Blank` and `Plain` are unconditional boundaries when their neighbour
/// is also `Blank`/`Plain` or end-of-buffer. Structural markers
/// (`FenceMarker`, `ListMarker`, etc.) are NEVER boundaries — widening
/// must reach the outer terminator of whatever construct they belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineConstructKind {
    Blank,
    Plain,
    FenceMarker,
    FenceContent,
    IndentedCode,
    ListMarker,
    ListContinuation,
    Blockquote(u8),
    SetextUnderline,
    HtmlBlock,
    Heading,
}

/// Result of widening a damaged range to safe construct boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidenResult {
    /// Widened range; caller passes this to `ParsedBuffer::parse_range`.
    Widened(Range<usize>),
    /// Range cannot be cheaply widened (cap trip, unbounded construct).
    /// Caller falls back to `ParsedBuffer::parse(lines)`.
    FullRebuild,
}

/// Maximum fraction of buffer the widened range may cover before we
/// abandon incremental and fall back to a full parse. Half the buffer
/// is the empirical cross-over where parse+splice overhead exceeds a
/// fresh full parse on the same input.
pub(super) const MAX_INCREMENTAL_FRACTION: f32 = 0.5;

/// Absolute cap on the widened range. Independent of buffer size; keeps
/// large-fence edits bounded even on small buffers.
pub(super) const MAX_INCREMENTAL_LINES: usize = 256;

/// Cursor-row hint scan window for `compute_damage_range`. Empirically
/// covers single-character edits, IME composition of up to 3 graphemes,
/// and one Enter at line end. Multi-line pastes intentionally fall
/// through to the LCP/LCS slow path.
pub(super) const CURSOR_HINT_WINDOW: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::text_editor::markdown::ParsedBuffer;

    fn kinds_of(lines: &[&str]) -> Vec<LineConstructKind> {
        let owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        ParsedBuffer::parse(&owned).kinds
    }

    #[test]
    fn plain_paragraph() {
        assert_eq!(kinds_of(&["hello world"]), vec![LineConstructKind::Plain]);
    }

    #[test]
    fn blank_line() {
        assert_eq!(kinds_of(&[""]), vec![LineConstructKind::Blank]);
    }

    #[test]
    fn atx_heading() {
        assert_eq!(kinds_of(&["# title"]), vec![LineConstructKind::Heading]);
    }

    #[test]
    fn setext_underline_above_is_plain() {
        let k = kinds_of(&["title", "====="]);
        assert_eq!(k, vec![LineConstructKind::Plain, LineConstructKind::SetextUnderline]);
    }

    #[test]
    fn fence_pair() {
        let k = kinds_of(&["```rust", "let x = 1;", "```"]);
        assert_eq!(
            k,
            vec![
                LineConstructKind::FenceMarker,
                LineConstructKind::FenceContent,
                LineConstructKind::FenceMarker,
            ]
        );
    }

    #[test]
    fn list_marker_and_continuation() {
        let k = kinds_of(&["- item", "  continuation"]);
        assert_eq!(
            k,
            vec![LineConstructKind::ListMarker, LineConstructKind::ListContinuation]
        );
    }

    #[test]
    fn blockquote_levels() {
        let k = kinds_of(&[">> two"]);
        assert_eq!(k, vec![LineConstructKind::Blockquote(2)]);
    }

    #[test]
    fn indented_code() {
        let k = kinds_of(&["", "    let x = 1;"]);
        assert_eq!(k[1], LineConstructKind::IndentedCode);
    }

    #[test]
    fn html_block() {
        let k = kinds_of(&["<div>", "body", "</div>"]);
        assert!(matches!(k[0], LineConstructKind::HtmlBlock));
    }
}
