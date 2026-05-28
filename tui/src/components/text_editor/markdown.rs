use super::parse_incremental::LineConstructKind;
use crate::settings::themes::Theme;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Shared parser options used by all pulldown-cmark call sites in this module.
const PARSER_OPTIONS: Options = Options::ENABLE_STRIKETHROUGH;

/// Visual columns per tab stop. Must match the `tabstop` setting in the nvim backend.
const TAB_STOP: usize = 4;

/// Compute the display width of a tab character at the given visual column.
fn tab_width_at(col: usize) -> usize {
    TAB_STOP - (col % TAB_STOP)
}

/// Sum of grapheme-cluster display widths across a string. Used to size
/// synthetic spans (e.g. image-link placeholders) injected during render.
fn string_display_width(s: &str) -> usize {
    s.graphemes(true).map(cluster_display_width).sum()
}

/// Display width of a grapheme cluster.
///
/// For multi-codepoint clusters (ZWJ sequences like 👨‍👩‍👧‍👦, variation selectors,
/// skin-tone modifiers) the width is determined by the first codepoint. The
/// combining codepoints that follow contribute 0 additional columns, which
/// matches the rendering behaviour of modern terminal emulators.
fn cluster_display_width(cluster: &str) -> usize {
    cluster
        .chars()
        .next()
        .and_then(unicode_width::UnicodeWidthChar::width)
        .unwrap_or(1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub start_char: usize,
    pub end_char: usize,
    pub kind: ElementKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElementKind {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link,
    HeadingH1,
    HeadingH2,
    HeadingH3,
    Blockquote,
    WikiLink,
    Image,
    Label,
}

/// A single image-link span on a parsed line, replaced visually with a
/// placeholder when rendering. `start_char`..`end_char` covers the full
/// `![alt](url)` source range. `placeholder_width` is precomputed so the
/// per-render hot path does not re-walk the placeholder graphemes.
#[derive(Debug, Clone)]
pub struct ImagePlaceholder {
    pub start_char: usize,
    pub end_char: usize,
    pub placeholder: String,
    pub placeholder_width: usize,
}

/// Pre-parsed result for a single logical line.
/// Build once per frame via `ParsedLine::parse`, then reuse across render, cursor,
/// wrap-width, and click-mapping calls to avoid redundant pulldown-cmark invocations.
#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub elements: Vec<Element>,
    /// Per-char visibility: `true` = this char is rendered content (not a markdown sigil).
    pub content_vis: Vec<bool>,
    /// Per-char: `true` = this char falls within any element's char range.
    /// Enables O(1) `in_any_element` without iterating `elements`.
    elem_vis: Vec<bool>,
    /// Per-char element index, 1-based (0 = no element). Enables O(1) `elem_at`.
    /// Stored as `u16`; supports up to 65535 elements per line.
    elem_index: Vec<u16>,
    /// Char offset where the list-item sigil (indent + marker + space) ends on
    /// this line, or `None` if this line is not the first line of a list item.
    list_sigil_end: Option<usize>,
    /// Image-link spans on this line, sorted by `start_char`. Their underlying
    /// chars are hidden (`content_vis = false`) and replaced visually by
    /// `placeholder` when rendering.
    pub image_placeholders: Vec<ImagePlaceholder>,
}

impl ParsedLine {
    /// Parse a single line in isolation. Internally delegates to
    /// `ParsedBuffer::parse`; kept for test convenience.
    ///
    /// When the line looks like an indented list item (e.g. `    - foo` or
    /// `\t- foo`), pulldown-cmark treats it as an indented code block rather
    /// than a list item on its own. To preserve the real-editor behaviour
    /// (where context from surrounding lines resolves it as a nested list
    /// item), prepend a synthetic parent list marker before handing the input
    /// to `ParsedBuffer::parse` and return the result for the original line.
    pub fn parse(line: &str) -> Self {
        let owned = line.to_string();
        if needs_synthetic_list_parent(line) {
            // "- " opens a list at column 0; the indented `line` that follows
            // becomes a nested list item with full context.
            ParsedBuffer::parse(&["- ".to_string(), owned])
                .lines
                .pop()
                .expect("ParsedBuffer::parse returns one row per input line")
        } else {
            ParsedBuffer::parse(std::slice::from_ref(&owned))
                .lines
                .pop()
                .expect("ParsedBuffer::parse always returns at least one ParsedLine")
        }
    }

    /// Element index at `pos`, or `None`. O(1) via precomputed `elem_index`.
    pub fn elem_at(&self, pos: usize) -> Option<usize> {
        self.elem_index.get(pos).and_then(|&tag| {
            if tag == 0 {
                None
            } else {
                Some((tag as usize) - 1)
            }
        })
    }

    /// Whether `pos` falls inside any tracked element. O(1) via precomputed `elem_vis`.
    pub fn in_any_element(&self, pos: usize) -> bool {
        self.elem_vis.get(pos).copied().unwrap_or(false)
    }

    /// Returns the char offset of the first *content* char inside a heading element
    /// (i.e. the end of the "# " / "## " / "### " sigil region), or `None` if this
    /// line has no heading element.
    ///
    /// Defaults to `e.end_char` so that a heading with no content text (e.g. `"#"`) is
    /// fully treated as sigil — fixes the F-02 bug where `e.start_char` was used.
    pub fn heading_sigil_end(&self) -> Option<usize> {
        self.elements
            .iter()
            .find(|e| {
                matches!(
                    e.kind,
                    ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3
                )
            })
            .map(|e| {
                let mut first_content = e.end_char; // default: all chars are sigil
                for i in e.start_char..e.end_char {
                    if i < self.content_vis.len() && self.content_vis[i] {
                        first_content = i;
                        break;
                    }
                }
                first_content
            })
    }

    /// Char offset where the list-item sigil ends on this line, or `None` if this
    /// line is not the first line of a list item.
    pub fn list_sigil_end(&self) -> Option<usize> {
        self.list_sigil_end
    }

    /// Diagnostic helper: compare every field for byte-identity. Used by
    /// the view's debug-only correctness assertion. Returns Ok(()) when
    /// all fields match, Err with a human-readable message describing the
    /// first divergence.
    #[cfg(debug_assertions)]
    pub(super) fn debug_assert_eq_to(&self, other: &Self, row: usize) {
        assert_eq!(
            self.content_vis, other.content_vis,
            "row {row} content_vis diverge"
        );
        assert_eq!(self.elem_vis, other.elem_vis, "row {row} elem_vis diverge");
        assert_eq!(
            self.elem_index, other.elem_index,
            "row {row} elem_index diverge"
        );
        assert_eq!(
            self.list_sigil_end, other.list_sigil_end,
            "row {row} list_sigil_end diverge"
        );
        assert_eq!(
            self.elements.len(),
            other.elements.len(),
            "row {row} elements.len() diverge"
        );
    }
}

#[derive(Clone)]
pub struct ParsedBuffer {
    pub lines: Vec<ParsedLine>,
    pub kinds: Vec<super::parse_incremental::LineConstructKind>,
    /// Sorted, deduped row indices `b` where pulldown-cmark's parser
    /// state is provably reset — i.e. parsing `&lines[b..j]` in
    /// isolation produces the same `ParsedLine` and
    /// `LineConstructKind` for row `b` as parsing the full buffer
    /// would, for every later boundary `j`. Used by
    /// `parse_incremental::expand_to_reset_boundary` so the
    /// incremental-parse widening is provably-equivalent to a fresh
    /// parse over the spliced range — no post-slice verification
    /// needed in release.
    ///
    /// Always contains `0` and `lines.len()` as sentinel boundaries.
    /// Conservative starting set: only Blank-prefixed rows after an
    /// `Event::End` of a top-level block. Long buffers without blank
    /// separators degrade to full-rebuild on every edit (acceptable
    /// — same behaviour as today's `widen_to_safe` + cap-trip path
    /// in that regime).
    pub reset_boundaries: Vec<usize>,
}

impl ParsedBuffer {
    /// Parse the entire editor buffer in a single pulldown-cmark pass.
    ///
    /// Returns a [`ParsedBuffer`] whose `lines` contains one `ParsedLine`
    /// per input row (multi-row markdown elements split per row) and whose
    /// `kinds` contains the per-row [`LineConstructKind`] classification
    /// that drives safe-boundary widening in `parse_incremental`.
    ///
    /// The pulldown-cmark event walk classifies the major constructs
    /// inline; three short O(n) post-passes (list-continuation,
    /// blockquote-depth, setext-underline) refine the result. No second
    /// invocation of the pulldown parser.
    pub fn parse(lines: &[String]) -> ParsedBuffer {
        // Build joined buffer and per-line byte-offset table.
        let total_bytes: usize =
            lines.iter().map(|l| l.len()).sum::<usize>() + lines.len().saturating_sub(1);
        let mut joined = String::with_capacity(total_bytes);
        let mut line_starts: Vec<usize> = Vec::with_capacity(lines.len() + 1);
        for (i, line) in lines.iter().enumerate() {
            line_starts.push(joined.len());
            joined.push_str(line);
            if i + 1 < lines.len() {
                joined.push('\n');
            }
        }
        // Sentinel past-end entry so binary_search on a byte offset that falls
        // exactly on the last line's content still returns a valid `Err(row)`
        // without landing on an `Ok` match at the real end. The `+ 1` ensures
        // the sentinel is strictly greater than any real byte offset, including
        // the trailing '\n' bytes between lines.
        line_starts.push(joined.len() + 1);

        // Pre-allocate per-line state.
        let mut content_vis: Vec<Vec<bool>> = lines
            .iter()
            .map(|l| vec![false; l.chars().count()])
            .collect();
        let mut elements: Vec<Vec<Element>> = vec![Vec::new(); lines.len()];
        let mut list_sigil_end: Vec<Option<usize>> = vec![None; lines.len()];

        // Element stack: (start_row, start_col_char, kind).
        // Spans are emitted on End events, split across rows they cross.
        let mut stack: Vec<(usize, usize, ElementKind)> = Vec::new();

        // `list_sigil_end[row]` is filled directly when we see `Start(Item)` on
        // that row — we walk the line from the Item's start column past the
        // marker (`- `, `* `, `+ `, or `N. `). This handles empty items (`- `)
        // that have no Text event inside.

        // Helper closure for pushing a multi-row span to `elements`.
        let emit_span = |row_s: usize,
                         col_s: usize,
                         row_e: usize,
                         col_e: usize,
                         kind: ElementKind,
                         elements: &mut Vec<Vec<Element>>,
                         lines: &[String]| {
            if row_s == row_e {
                if col_e > col_s && row_s < elements.len() {
                    elements[row_s].push(Element {
                        start_char: col_s,
                        end_char: col_e,
                        kind,
                    });
                }
                return;
            }
            // Multi-row: first row extends to end-of-line, middle rows cover whole line,
            // last row covers 0..col_e.
            if row_s < elements.len() {
                let end_first = lines[row_s].chars().count();
                if end_first > col_s {
                    elements[row_s].push(Element {
                        start_char: col_s,
                        end_char: end_first,
                        kind,
                    });
                }
            }
            for r in (row_s + 1)..row_e {
                if r < elements.len() {
                    let line_len = lines[r].chars().count();
                    if line_len > 0 {
                        elements[r].push(Element {
                            start_char: 0,
                            end_char: line_len,
                            kind,
                        });
                    }
                }
            }
            if row_e < elements.len() && col_e > 0 {
                elements[row_e].push(Element {
                    start_char: 0,
                    end_char: col_e,
                    kind,
                });
            }
        };

        // Per-line construct classification: initially Blank vs Plain based on
        // whitespace, then updated during the event loop and post-passes below.
        let mut kinds: Vec<LineConstructKind> = lines
            .iter()
            .map(|l| {
                if l.trim().is_empty() {
                    LineConstructKind::Blank
                } else {
                    LineConstructKind::Plain
                }
            })
            .collect();

        // Reset-boundary detection via depth prefix-sum. During the
        // event walk we record per-row depth deltas (+1 at the start
        // row of every top-level block, -1 at the row AFTER its end).
        // After the walk, a prefix sum gives the depth at the start
        // of each row — depth==0 means pulldown's parser is in the
        // "between blocks" state at that row, with no open
        // construct that could lazy-continue. A row at depth 0 whose
        // own `kinds` is Blank (or EOF) is a true reset point.
        //
        // Depth deltas (not an inline counter) handle pulldown's
        // overlapping nesting: a Paragraph inside an Item inside a
        // List emits Start events at overlapping rows; an inline
        // depth counter that decrements on the innermost End would
        // see depth==0 prematurely while the outer List is still
        // open. The delta+prefix-sum approach correctly sums all
        // open constructs.
        let mut reset_boundaries: Vec<usize> = Vec::new();
        // +1 at the open row, -1 at the row past the close. Length
        // is lines.len() + 1 so end-of-buffer deltas have a slot.
        let mut depth_delta: Vec<i32> = vec![0; lines.len() + 1];

        // Track fenced/indented code block byte ranges for F4 (label suppression).
        // Populated during the main parser pass below and converted to per-line
        // flags before the per-line label scan.
        let mut code_block_byte_ranges: Vec<(usize, usize)> = Vec::new();
        let mut code_block_depth = 0u32;
        let mut code_block_start: Option<usize> = None;

        let parser = Parser::new_ext(&joined, PARSER_OPTIONS);
        for (event, range) in parser.into_offset_iter() {
            let (sr, sc) = byte_to_row_col(range.start, lines, &line_starts);
            let (er, ec) = byte_to_row_col(range.end, lines, &line_starts);
            // Per-row depth deltas: +1 when a top-level block opens
            // at `sr`, -1 at the row past where it closes (`er + 1`,
            // clamped to `lines.len()`). Resolved into per-row depth
            // by a prefix sum after the walk.
            match &event {
                Event::Start(tag) if is_top_level_block_tag(tag) && sr < depth_delta.len() => {
                    depth_delta[sr] += 1;
                }
                Event::End(tag_end) if is_top_level_block_tag_end(tag_end) => {
                    // pulldown's `range.end` for a block falls in the
                    // row IMMEDIATELY AFTER the block's last content
                    // row (typically the start of the trailing blank
                    // or the next block). `byte_to_row_col` thus
                    // returns `er` already pointing at the
                    // "between-blocks" row — that's where depth
                    // drops, no `+1` needed.
                    let drop_at = er.min(lines.len());
                    if drop_at < depth_delta.len() {
                        depth_delta[drop_at] -= 1;
                    }
                }
                _ => {}
            }

            match event {
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                    if code_block_depth == 0 {
                        code_block_start = Some(range.start);
                    }
                    code_block_depth += 1;
                    // Opening fence marker row.
                    kinds[sr] = LineConstructKind::FenceMarker;
                    // Rows between opening and closing fences are content.
                    kinds[(sr + 1)..er].fill(LineConstructKind::FenceContent);
                    // Closing fence marker row (er is the row of the closing ```).
                    kinds[er] = LineConstructKind::FenceMarker;
                }
                Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                    if code_block_depth == 0 {
                        code_block_start = Some(range.start);
                    }
                    code_block_depth += 1;
                    kinds[sr..=er].fill(LineConstructKind::IndentedCode);
                }
                Event::End(TagEnd::CodeBlock) => {
                    code_block_depth = code_block_depth.saturating_sub(1);
                    if code_block_depth == 0
                        && let Some(start) = code_block_start.take()
                    {
                        code_block_byte_ranges.push((start, range.end));
                    }
                }
                Event::Start(ref tag) if let Some(kind) = tag_to_kind(tag) => {
                    if matches!(
                        kind,
                        ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3
                    ) {
                        kinds[sr] = LineConstructKind::Heading;
                    }
                    stack.push((sr, sc, kind));
                }
                Event::End(
                    TagEnd::Strong
                    | TagEnd::Emphasis
                    | TagEnd::Strikethrough
                    | TagEnd::Link
                    | TagEnd::Heading(_)
                    | TagEnd::BlockQuote(_),
                ) => {
                    if let Some((s_r, s_c, k)) = stack.pop() {
                        emit_span(s_r, s_c, er, ec, k, &mut elements, lines);
                    }
                }
                Event::Start(Tag::Item)
                    // Pulldown-cmark's Item range does not always start at the
                    // marker character — for nested items it starts at the
                    // indentation-beyond-the-parent boundary, which can be
                    // several chars before the marker. Scan the line from col
                    // 0 to find leading whitespace + marker instead of relying
                    // on `sc`.
                    if sr < lines.len() && list_sigil_end[sr].is_none() =>
                {
                    kinds[sr] = LineConstructKind::ListMarker;
                    let line = lines[sr].as_str();
                    let ws_end = leading_ws_byte_len(line);
                    if let Some(len) = list_marker_len(&line[ws_end..]) {
                        // ws_end is byte length but ASCII whitespace makes it
                        // equal to the char count.
                        list_sigil_end[sr] = Some(ws_end + len);
                    }
                }
                Event::Start(Tag::HtmlBlock) => {
                    kinds[sr..=er].fill(LineConstructKind::HtmlBlock);
                }
                Event::Html(_) => {
                    // Block-level HTML body. Already classified by the
                    // enclosing `Tag::HtmlBlock` arm; this branch is a
                    // safety-net for any row pulldown emits between the
                    // tag boundaries but defers to fence/code kinds when
                    // the rows happen to overlap (rare, but possible
                    // with malformed input).
                    for kind in &mut kinds[sr..=er] {
                        if !matches!(
                            *kind,
                            LineConstructKind::FenceContent
                                | LineConstructKind::IndentedCode
                                | LineConstructKind::FenceMarker
                        ) {
                            *kind = LineConstructKind::HtmlBlock;
                        }
                    }
                }
                // Inline HTML (`<span>`, `<br/>`, etc.) lives inside a
                // paragraph; it must NOT promote the row to HtmlBlock,
                // because `HtmlBlock` is a non-safe widening boundary
                // (see `parse_incremental::is_safe_boundary`). Painting
                // the paragraph row as HtmlBlock would force widening
                // to walk past it on every nearby edit. Leave kind as-is.
                Event::InlineHtml(_) => {}
                Event::End(TagEnd::Item) => {}
                Event::Code(ref code_text) if sr == er && sr < lines.len() => {
                    // Inline code — always single-line in practice.
                    let code_len = code_text.chars().count();
                    let range_char_len = ec.saturating_sub(sc);
                    let sigil_each = range_char_len.saturating_sub(code_len) / 2;
                    let cs = sc + sigil_each;
                    for vis in content_vis[sr].iter_mut().skip(cs).take(code_len) {
                        *vis = true;
                    }
                    elements[sr].push(Element {
                        start_char: sc,
                        end_char: ec,
                        kind: ElementKind::InlineCode,
                    });
                }
                Event::Text(_) | Event::SoftBreak | Event::HardBreak => {
                    // Mark content_vis for each row the event touches.
                    if sr == er {
                        if sr < content_vis.len() {
                            for vis in content_vis[sr]
                                .iter_mut()
                                .skip(sc)
                                .take(ec.saturating_sub(sc))
                            {
                                *vis = true;
                            }
                        }
                    } else {
                        // First row: from sc to end-of-line.
                        if sr < content_vis.len() {
                            let line_chars = content_vis[sr].len();
                            for vis in content_vis[sr]
                                .iter_mut()
                                .skip(sc)
                                .take(line_chars.saturating_sub(sc))
                            {
                                *vis = true;
                            }
                        }
                        // Middle rows: whole line.
                        for r in (sr + 1)..er {
                            if r < content_vis.len() {
                                for vis in content_vis[r].iter_mut() {
                                    *vis = true;
                                }
                            }
                        }
                        // Last row: 0..ec.
                        if er < content_vis.len() {
                            for vis in content_vis[er].iter_mut().take(ec) {
                                *vis = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Build a per-line flag: `true` = this line is inside a fenced/indented code block.
        // Lines whose byte range overlaps any code-block range are suppressed from label scan.
        let line_in_code_block: Vec<bool> = {
            let mut flags = vec![false; lines.len()];
            for (cb_start, cb_end) in &code_block_byte_ranges {
                // Find all lines that overlap [cb_start, cb_end).
                for (row, &ls) in line_starts[..lines.len()].iter().enumerate() {
                    let le = ls + lines[row].len();
                    // Overlap when line_start < cb_end and line_end > cb_start.
                    // We skip the fence delimiter lines (which contain the ``` markers)
                    // by checking if the line's content byte range overlaps the code
                    // block's content range. The CodeBlock event range in pulldown-cmark
                    // covers the opening fence through the closing fence, so this
                    // conservative check marks all lines within the span as in-block.
                    if ls < *cb_end && le > *cb_start {
                        flags[row] = true;
                    }
                }
            }
            flags
        };

        // Per-line post-processing: heading trailing whitespace, wikilinks, bitmasks.
        let mut out: Vec<ParsedLine> = Vec::with_capacity(lines.len());
        for (row, line) in lines.iter().enumerate() {
            let mut cv = std::mem::take(&mut content_vis[row]);
            let mut els = std::mem::take(&mut elements[row]);

            // Heading trailing-whitespace fix.
            for e in &els {
                if matches!(
                    e.kind,
                    ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3
                ) {
                    for i in (e.start_char..e.end_char).rev() {
                        match line.chars().nth(i) {
                            Some(' ' | '\t') => {
                                if i < cv.len() {
                                    cv[i] = true;
                                }
                            }
                            _ => break,
                        }
                    }
                }
            }

            detect_wikilinks(line, &mut cv, &mut els);
            let image_placeholders = detect_image_placeholders(line, &mut cv, &mut els);

            // Scan for #hashtag spans and emit Label elements.
            // Guards (all must pass):
            //   F4: skip if this line is inside a fenced code block
            //   F2: word-boundary guard (handled inside label_matches)
            //   F3: skip if the span overlaps InlineCode, Link, WikiLink, or Image
            if !line_in_code_block[row] {
                let line_str = line.as_str();
                for lm in kimun_core::note::label_matches(line_str) {
                    // Convert byte offsets to char offsets for Element storage.
                    let start_char = line_str[..lm.byte_start].chars().count();
                    let end_char =
                        start_char + line_str[lm.byte_start..lm.byte_end].chars().count();
                    // F3: overlap guard (InlineCode + Link + WikiLink + Image)
                    let overlaps_existing = els.iter().any(|e| {
                        matches!(
                            e.kind,
                            ElementKind::InlineCode
                                | ElementKind::Link
                                | ElementKind::WikiLink
                                | ElementKind::Image
                        ) && !(end_char <= e.start_char || start_char >= e.end_char)
                    });
                    if !overlaps_existing {
                        els.push(Element {
                            start_char,
                            end_char,
                            kind: ElementKind::Label,
                        });
                    }
                }
            }
            // Re-sort so elem_vis / elem_index precomputation sees elements in line order.
            els.sort_by_key(|e| e.start_char);

            // F6: use u16 for elem_index (supports up to 65535 elements per line)
            debug_assert!(
                els.len() < u16::MAX as usize,
                "Too many elements on a single line ({})",
                els.len()
            );
            let total = line.chars().count();
            let mut elem_vis = vec![false; total];
            let mut elem_index = vec![0u16; total];
            for (i, e) in els.iter().enumerate() {
                let tag = (i + 1) as u16;
                for pos in e.start_char..e.end_char {
                    if pos < total {
                        elem_vis[pos] = true;
                        elem_index[pos] = tag;
                    }
                }
            }

            out.push(ParsedLine {
                elements: els,
                content_vis: cv,
                elem_vis,
                elem_index,
                list_sigil_end: list_sigil_end[row],
                image_placeholders,
            });
        }

        // Post-pass: blockquote depth (N = number of leading `>` characters).
        // Done before the setext post-pass so that a blockquoted heading line
        // is not mis-treated as setext text.
        for row in 0..kinds.len() {
            if !matches!(
                kinds[row],
                LineConstructKind::Plain | LineConstructKind::Blank
            ) {
                continue;
            }
            let line = &lines[row];
            let mut depth: u8 = 0;
            for ch in line.chars() {
                match ch {
                    '>' => depth = depth.saturating_add(1),
                    ' ' | '\t' => continue,
                    _ => break,
                }
            }
            if depth > 0 {
                kinds[row] = LineConstructKind::Blockquote(depth);
            }
        }

        // Post-pass: setext underline classification.
        // Pulldown reports setext headings as Heading events spanning the text
        // line AND the underline line. We want to: (a) classify the underline
        // row as SetextUnderline, (b) reset the heading text row to Plain so
        // widening treats it as a safe boundary above.
        for row in 0..lines.len().saturating_sub(1) {
            if kinds[row] == LineConstructKind::Heading {
                let next = &lines[row + 1];
                let trimmed = next.trim();
                if !trimmed.is_empty()
                    && (trimmed.chars().all(|c| c == '=') || trimmed.chars().all(|c| c == '-'))
                {
                    kinds[row + 1] = LineConstructKind::SetextUnderline;
                    kinds[row] = LineConstructKind::Plain;
                }
            }
        }

        // Post-pass: list-item continuation rows.
        // Any Plain row immediately following a ListMarker or ListContinuation
        // row is itself a continuation (lazy continuation, indented body, etc.).
        for row in 1..kinds.len() {
            if matches!(
                kinds[row],
                LineConstructKind::Plain | LineConstructKind::IndentedCode
            ) && matches!(
                kinds[row - 1],
                LineConstructKind::ListMarker | LineConstructKind::ListContinuation
            ) {
                kinds[row] = LineConstructKind::ListContinuation;
            }
        }

        // Resolve depth deltas → per-row depth via prefix sum, then
        // record boundaries at rows where depth==0 AND the row is
        // Blank (or end-of-buffer). Blank-at-depth-0 means pulldown
        // has no open construct that could lazy-continue into the
        // row, so a fresh parser starting at that row produces the
        // same output as the full parse from that point.
        let mut depth: i32 = 0;
        for r in 0..lines.len() {
            depth += depth_delta[r];
            if depth == 0 && kinds[r] == LineConstructKind::Blank {
                reset_boundaries.push(r);
            }
        }
        // Sentinels: 0 (start of buffer), lines.len() (past-end).
        // Both make `expand_to_reset_boundary`'s unwrap_or fallbacks
        // unreachable in a well-formed set.
        reset_boundaries.push(0);
        reset_boundaries.push(lines.len());
        reset_boundaries.sort_unstable();
        reset_boundaries.dedup();

        ParsedBuffer {
            lines: out,
            kinds,
            reset_boundaries,
        }
    }

    /// O(N) placeholder buffer matching `lines`'s row count, every row
    /// classified `Plain`, every char marked content-visible, no
    /// elements. Used by `view.update`'s async-fallback path (perf #9
    /// in the holistic review) to install a structurally-correct
    /// `ParsedBuffer` cheaply while the real `ParsedBuffer::parse`
    /// runs on a background tokio task. Render produces unstyled
    /// markdown for one frame until the async result is installed
    /// via `install_full_parse`.
    pub fn placeholder(lines: &[String]) -> ParsedBuffer {
        let mut out = Vec::with_capacity(lines.len());
        for line in lines {
            let total = line.chars().count();
            out.push(ParsedLine {
                elements: Vec::new(),
                content_vis: vec![true; total],
                elem_vis: vec![false; total],
                elem_index: vec![0; total],
                list_sigil_end: None,
                image_placeholders: Vec::new(),
            });
        }
        let kinds = vec![LineConstructKind::Plain; lines.len()];
        let reset_boundaries = if lines.is_empty() {
            vec![0]
        } else {
            vec![0, lines.len()]
        };
        ParsedBuffer {
            lines: out,
            kinds,
            reset_boundaries,
        }
    }

    /// Parse a contiguous slice of `lines` as if it were a standalone document.
    ///
    /// **Boundary contract:** the caller must pass a `range` whose `start` and
    /// `end` land on safe construct boundaries (verified by
    /// `parse_incremental::widen_to_safe` or `expand_to_reset_boundary`).
    /// This function does not validate the contract — passing a mid-fence
    /// range will produce a `ParsedBuffer` that mis-classifies the
    /// boundary lines.
    ///
    /// Returns a `ParsedBuffer` whose `lines.len() == kinds.len() == range.len()`.
    /// The returned `reset_boundaries` are in slice-local index space
    /// (`0..range.len()`); `splice` shifts them by `range.start` when
    /// merging into the parent buffer's boundary set.
    pub fn parse_range(lines: &[String], range: Range<usize>) -> ParsedBuffer {
        Self::parse(&lines[range])
    }

    /// Replace `self.lines[range]` and `self.kinds[range]` with the contents
    /// of `other`. Both `other` vectors must have `range.len()` entries.
    pub fn splice(&mut self, range: Range<usize>, other: ParsedBuffer) {
        debug_assert!(
            other.lines.len() == other.kinds.len(),
            "splice: other has mismatched internal lengths (lines={} kinds={})",
            other.lines.len(),
            other.kinds.len(),
        );
        debug_assert!(
            other.lines.len() == range.len(),
            "splice: other.lines.len() ({}) != range.len() ({})",
            other.lines.len(),
            range.len(),
        );
        debug_assert!(
            other.kinds.len() == range.len(),
            "splice: other.kinds.len() ({}) != range.len() ({})",
            other.kinds.len(),
            range.len(),
        );
        self.lines.splice(range.clone(), other.lines);
        self.kinds.splice(range.clone(), other.kinds);

        // Merge `reset_boundaries`. The incremental splice path never
        // changes line count (gated upstream in try_incremental_parse),
        // so boundaries outside `range` keep their indices. Boundaries
        // strictly inside `range` are dropped (the splice replaces
        // them). The slice's own boundaries are shifted by
        // `range.start` and added.
        let lines_len = self.lines.len();
        let mut merged: Vec<usize> = self
            .reset_boundaries
            .iter()
            .copied()
            .filter(|&b| b <= range.start || b >= range.end)
            .collect();
        for b in other.reset_boundaries {
            merged.push(range.start + b);
        }
        // The merge can drop `lines_len` if the prior buffer's tail
        // boundary fell inside `range`. Re-add 0 and `lines_len` to
        // preserve the sentinel invariant.
        merged.push(0);
        merged.push(lines_len);
        merged.sort_unstable();
        merged.dedup();
        debug_assert!(
            merged.first() == Some(&0) && merged.last() == Some(&lines_len),
            "splice: merged boundaries must start with 0 and end with lines.len() ({lines_len})"
        );
        self.reset_boundaries = merged;
    }
}

pub struct MarkdownSpanner;

impl MarkdownSpanner {
    #[cfg(test)]
    pub fn parse_elements(line: &str) -> Vec<Element> {
        let parser = Parser::new_ext(line, PARSER_OPTIONS);
        let mut elements = Vec::new();
        let mut stack: Vec<(usize, ElementKind)> = Vec::new();
        for (event, range) in parser.into_offset_iter() {
            let sc = line[..range.start].chars().count();
            let ec = line[..range.end].chars().count();
            match event {
                Event::Start(ref tag) if let Some(kind) = tag_to_kind(tag) => {
                    stack.push((sc, kind));
                }
                Event::End(
                    TagEnd::Strong
                    | TagEnd::Emphasis
                    | TagEnd::Strikethrough
                    | TagEnd::Link
                    | TagEnd::Heading(_)
                    | TagEnd::BlockQuote(_),
                ) => {
                    if let Some((s, k)) = stack.pop() {
                        elements.push(Element {
                            start_char: s,
                            end_char: ec,
                            kind: k,
                        });
                    }
                }
                Event::Code(_) => elements.push(Element {
                    start_char: sc,
                    end_char: ec,
                    kind: ElementKind::InlineCode,
                }),
                _ => {}
            }
        }
        let mut dummy_vis = vec![true; line.chars().count()];
        detect_wikilinks(line, &mut dummy_vis, &mut elements);
        elements
    }

    // ── Public API (parse-on-the-fly wrappers, used in tests only) ───────────

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        content: &str,
        logical_line: &str,
        visual_start_col: usize,
        cursor_col: Option<usize>,
        is_first_visual_line: bool,
        force_raw: bool,
        available_width: u16,
        theme: &Theme,
    ) -> Vec<Span<'static>> {
        let parsed = ParsedLine::parse(logical_line);
        Self::render_with(
            content,
            logical_line,
            &parsed,
            visual_start_col,
            cursor_col,
            is_first_visual_line,
            force_raw,
            available_width,
            theme,
        )
        .into_iter()
        .map(|s| Span::styled(s.content.into_owned(), s.style))
        .collect()
    }

    #[cfg(test)]
    pub fn rendered_cursor_col(
        logical_line: &str,
        visual_start_col: usize,
        cursor_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        let parsed = ParsedLine::parse(logical_line);
        Self::rendered_cursor_col_with(
            logical_line,
            &parsed,
            visual_start_col,
            cursor_col,
            is_first_visual_line,
            force_raw,
        )
    }

    #[cfg(test)]
    pub fn visible_positions(
        logical_line: &str,
        cursor_col: Option<usize>,
        force_raw: bool,
    ) -> Vec<bool> {
        let parsed = ParsedLine::parse(logical_line);
        Self::visible_positions_with(logical_line, &parsed, cursor_col, force_raw)
    }

    #[cfg(test)]
    pub fn rendered_col_to_logical(
        logical_line: &str,
        visual_start_col: usize,
        rendered_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        let parsed = ParsedLine::parse(logical_line);
        Self::rendered_col_to_logical_with(
            logical_line,
            &parsed,
            visual_start_col,
            rendered_col,
            is_first_visual_line,
            force_raw,
        )
    }

    // ── `_with` variants: accept pre-parsed `&ParsedLine` ────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn render_with<'a>(
        content: &'a str,
        logical_line: &'a str,
        parsed: &'a ParsedLine,
        visual_start_col: usize,
        cursor_col: Option<usize>,
        is_first_visual_line: bool,
        force_raw: bool,
        available_width: u16,
        theme: &Theme,
    ) -> Vec<Span<'a>> {
        // HR
        let trimmed = logical_line.trim();
        if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
            if cursor_col.is_some() {
                return vec![Span::styled(
                    content,
                    Style::default().fg(theme.fg_muted.to_ratatui()),
                )];
            }
            return vec![Span::styled(
                "─".repeat(available_width as usize),
                Style::default().fg(theme.fg_muted.to_ratatui()),
            )];
        }
        // Force-raw (inside fenced code block)
        if force_raw {
            return vec![Span::styled(
                content,
                Style::default().fg(theme.fg_secondary.to_ratatui()),
            )];
        }

        let elements = &parsed.elements;
        let content_vis = &parsed.content_vis;
        let content_char_count = content.chars().count();

        let expanded: Option<usize> = cursor_col.and_then(|c| parsed.elem_at(c));

        let heading_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.heading_sigil_end()
        } else {
            None
        };
        let list_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.list_sigil_end()
        } else {
            None
        };

        let mut spans: Vec<Span<'a>> = Vec::new();
        let mut seg_str: String = String::new();
        let mut seg_elem: Option<usize> = None;
        let mut seg_is_sigil = false;
        let mut seg_is_expanded = false;
        // Tracks the current rendered visual column for tab-stop calculation.
        let mut visual_col = 0usize;

        let flush = |seg_str: &mut String,
                     seg_elem: Option<usize>,
                     seg_is_sigil: bool,
                     seg_is_expanded: bool,
                     spans: &mut Vec<Span<'a>>| {
            if seg_str.is_empty() {
                return;
            }
            let seg = std::mem::take(seg_str);
            let style = if seg_is_expanded {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                span_style(seg_elem.map(|i| elements[i].kind), seg_is_sigil, theme)
            };
            spans.push(Span::styled(seg, style));
        };

        // Iterate the visual-line slice rather than walking the whole logical
        // line and skipping clusters before `visual_start_col`. For a paragraph
        // wrapped across N visual rows this used to scan the full logical line
        // N times per frame; now each row's iteration is bounded to its own
        // slice. `char_pos` is seeded with `visual_start_col` so positions
        // continue to index into `content_vis`, `elements`, and the image
        // placeholders, which are all addressed in logical-line coordinates.
        let mut char_pos = visual_start_col;
        let visual_end_col = visual_start_col + content_char_count;
        for cluster in content.graphemes(true) {
            let pos = char_pos;
            char_pos += cluster.chars().count();
            if pos >= visual_end_col {
                break;
            }

            // Image placeholder: at the start of an `![..](..)` range, emit a
            // single styled placeholder span and let the existing emit logic
            // skip the underlying chars (they have content_vis=false). When the
            // cursor sits inside the image element we fall through and render
            // the raw markdown instead, matching the "expanded element" UX.
            if let Some(img) = parsed
                .image_placeholders
                .iter()
                .find(|p| p.start_char == pos)
            {
                let cursor_in_image = expanded.is_some_and(|i| {
                    elements[i].start_char == img.start_char && elements[i].end_char == img.end_char
                });
                if !cursor_in_image {
                    flush(
                        &mut seg_str,
                        seg_elem,
                        seg_is_sigil,
                        seg_is_expanded,
                        &mut spans,
                    );
                    let style = span_style(Some(ElementKind::Image), false, theme);
                    visual_col += img.placeholder_width;
                    spans.push(Span::styled(img.placeholder.as_str(), style));
                    seg_elem = None;
                    seg_is_sigil = false;
                    seg_is_expanded = false;
                }
            }

            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
            let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
            let in_expanded_elem = expanded
                .is_some_and(|i| elements[i].start_char <= pos && pos < elements[i].end_char);
            let this_elem = parsed.elem_at(pos);
            let emit = is_content
                || in_heading_sigil
                || in_list_sigil
                || in_expanded_elem
                || this_elem.is_none();
            if !emit {
                flush(
                    &mut seg_str,
                    seg_elem,
                    seg_is_sigil,
                    seg_is_expanded,
                    &mut spans,
                );
                seg_elem = None;
                seg_is_sigil = false;
                seg_is_expanded = false;
                continue;
            }
            let this_is_expanded = in_expanded_elem;
            let this_is_sigil =
                (in_heading_sigil || in_list_sigil) && !is_content && !in_expanded_elem;
            if this_elem != seg_elem
                || this_is_sigil != seg_is_sigil
                || this_is_expanded != seg_is_expanded
            {
                flush(
                    &mut seg_str,
                    seg_elem,
                    seg_is_sigil,
                    seg_is_expanded,
                    &mut spans,
                );
                seg_elem = this_elem;
                seg_is_sigil = this_is_sigil;
                seg_is_expanded = this_is_expanded;
            }
            if cluster == "\t" {
                let tw = tab_width_at(visual_col);
                for _ in 0..tw {
                    seg_str.push(' ');
                }
                visual_col += tw;
            } else {
                seg_str.push_str(cluster);
                visual_col += cluster_display_width(cluster);
            }
        }
        flush(
            &mut seg_str,
            seg_elem,
            seg_is_sigil,
            seg_is_expanded,
            &mut spans,
        );

        if spans.is_empty() {
            spans.push(Span::styled(
                content,
                Style::default().fg(theme.fg.to_ratatui()),
            ));
        }
        spans
    }

    pub fn rendered_cursor_col_with(
        logical_line: &str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        cursor_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        if force_raw {
            return cursor_col.saturating_sub(visual_start_col);
        }
        let trimmed = logical_line.trim();
        if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
            return cursor_col.saturating_sub(visual_start_col);
        }

        let elements = &parsed.elements;
        let content_vis = &parsed.content_vis;
        let logical_char_count = logical_line.chars().count();

        let expanded: Option<usize> = parsed.elem_at(cursor_col);
        let heading_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.heading_sigil_end()
        } else {
            None
        };
        let list_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.list_sigil_end()
        } else {
            None
        };

        let end = cursor_col.min(logical_char_count);
        let mut rendered_col = 0usize;
        let mut char_pos = 0usize;
        for cluster in logical_line.graphemes(true) {
            if char_pos >= end {
                break;
            }
            let pos = char_pos;
            char_pos += cluster.chars().count();
            if pos < visual_start_col {
                continue;
            }

            // Account for placeholder width when crossing the start of an image
            // span — kept consistent with `render_with`'s placeholder injection.
            if let Some(img) = parsed
                .image_placeholders
                .iter()
                .find(|p| p.start_char == pos)
            {
                let cursor_in_image = expanded.is_some_and(|i| {
                    elements[i].start_char == img.start_char && elements[i].end_char == img.end_char
                });
                if !cursor_in_image {
                    rendered_col += img.placeholder_width;
                }
            }

            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.is_some_and(|s_end| pos < s_end);
            let in_list_sigil = list_sigil_end.is_some_and(|s_end| pos < s_end);
            let in_expanded_elem = expanded
                .is_some_and(|i| elements[i].start_char <= pos && pos < elements[i].end_char);
            let in_any_element = parsed.in_any_element(pos);
            let visible = is_content
                || in_heading_sigil
                || in_list_sigil
                || in_expanded_elem
                || !in_any_element;
            if visible {
                rendered_col += if cluster == "\t" {
                    tab_width_at(rendered_col)
                } else {
                    cluster_display_width(cluster)
                };
            }
        }
        rendered_col
    }

    pub fn visible_positions_with(
        logical_line: &str,
        parsed: &ParsedLine,
        cursor_col: Option<usize>,
        force_raw: bool,
    ) -> Vec<bool> {
        let total = logical_line.chars().count();
        if total == 0 {
            return vec![];
        }
        if force_raw {
            return vec![true; total];
        }
        let trimmed = logical_line.trim();
        if matches!(trimmed, "---" | "***" | "___") {
            return vec![true; total];
        }

        let content_vis = &parsed.content_vis;
        let expanded: Option<usize> = cursor_col.and_then(|c| parsed.elem_at(c));
        let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
        let list_sigil_end = parsed.list_sigil_end();

        (0..total)
            .map(|pos| {
                let is_content = pos < content_vis.len() && content_vis[pos];
                let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
                let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
                let in_any_element = parsed.in_any_element(pos);
                let in_expanded = expanded.is_some_and(|i| {
                    parsed.elements[i].start_char <= pos && pos < parsed.elements[i].end_char
                });
                is_content || in_heading_sigil || in_list_sigil || in_expanded || !in_any_element
            })
            .collect()
    }

    pub fn rendered_col_to_logical_with(
        logical_line: &str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        rendered_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        if force_raw {
            return visual_start_col + rendered_col;
        }
        let trimmed = logical_line.trim();
        if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
            return visual_start_col + rendered_col;
        }

        let content_vis = &parsed.content_vis;
        let logical_char_count = logical_line.chars().count();
        let heading_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.heading_sigil_end()
        } else {
            None
        };
        let list_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.list_sigil_end()
        } else {
            None
        };

        let mut rendered_count = 0;
        let mut char_pos = 0usize;
        for cluster in logical_line.graphemes(true) {
            let pos = char_pos;
            char_pos += cluster.chars().count();
            if pos < visual_start_col {
                continue;
            }

            if rendered_count >= rendered_col {
                return pos;
            }
            // A click landing inside the placeholder region maps back to the
            // start of the image span (the only logical position that visually
            // corresponds to the placeholder).
            if let Some(img) = parsed
                .image_placeholders
                .iter()
                .find(|p| p.start_char == pos)
            {
                if rendered_count + img.placeholder_width > rendered_col {
                    return pos;
                }
                rendered_count += img.placeholder_width;
            }
            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
            let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
            let in_any_element = parsed.in_any_element(pos);
            if is_content || in_heading_sigil || in_list_sigil || !in_any_element {
                rendered_count += if cluster == "\t" {
                    tab_width_at(rendered_count)
                } else {
                    cluster_display_width(cluster)
                };
            }
        }
        logical_char_count
    }
}

/// Appends `WikiLink` elements for every `[[...]]` span in `line` and unsets
/// `content_vis` for the `[[` and `]]` bracket sigils.
fn detect_wikilinks(line: &str, content_vis: &mut [bool], elements: &mut Vec<Element>) {
    for span in kimun_core::note::wikilink_char_spans(line) {
        // Skip wikilinks that fall entirely inside an already-parsed element
        // (e.g. `[[icon]]` inside a markdown link's display text).
        let overlaps = elements
            .iter()
            .any(|e| span.start >= e.start_char && span.end <= e.end_char);
        if overlaps {
            continue;
        }
        // The inner text was marked as content by pulldown-cmark's Text event;
        // unmark the `[[` and `]]` bracket sigils.
        let close = span.end - 2;
        for pos in [span.start, span.start + 1, close, close + 1] {
            if pos < content_vis.len() {
                content_vis[pos] = false;
            }
        }
        elements.push(Element {
            start_char: span.start,
            end_char: span.end,
            kind: ElementKind::WikiLink,
        });
    }
}

/// Detects `![alt](url)` image-link spans on `line`, hides their underlying
/// chars (`content_vis = false`) so the renderer skips them, registers an
/// `Image` element for styling, and returns one [`ImagePlaceholder`] per span
/// containing the rendered placeholder text (`[filename]`).
fn detect_image_placeholders(
    line: &str,
    content_vis: &mut [bool],
    elements: &mut Vec<Element>,
) -> Vec<ImagePlaceholder> {
    use kimun_core::note::{LinkSpanKind, link_char_spans, link_target_filename};

    let mut out = Vec::new();
    for span in link_char_spans(line) {
        if span.kind != LinkSpanKind::Image {
            continue;
        }
        // Hide every char of the image syntax — including the alt text that
        // pulldown-cmark would otherwise mark as content.
        for vis in content_vis.iter_mut().take(span.end).skip(span.start) {
            *vis = false;
        }
        elements.push(Element {
            start_char: span.start,
            end_char: span.end,
            kind: ElementKind::Image,
        });
        let name = link_target_filename(&span.target);
        let placeholder = if name.is_empty() {
            "[image]".to_string()
        } else {
            format!("[{name}]")
        };
        let placeholder_width = string_display_width(&placeholder);
        out.push(ImagePlaceholder {
            start_char: span.start,
            end_char: span.end,
            placeholder,
            placeholder_width,
        });
    }
    out.sort_by_key(|p| p.start_char);
    out
}

/// Detects whether a line is an indented list item (leading spaces or tab,
/// followed by `-`/`*`/`+`/digit-dot + space). Used by `ParsedLine::parse`
/// to decide whether to feed pulldown-cmark a synthetic parent-list context
/// for single-line degenerate inputs.
fn needs_synthetic_list_parent(line: &str) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    if trimmed.len() == line.len() {
        return false; // no leading whitespace → nothing to compensate for
    }
    list_marker_len(trimmed).is_some()
}

/// If the string begins with an unordered list marker (`- `, `* `, `+ `) or an
/// ordered list marker (digits followed by `. `), returns the marker's length
/// in bytes (including the trailing space). Otherwise `None`.
///
/// Digits are ASCII only, so byte length == char length here.
/// Byte length of the leading run of ASCII space/tab characters in `line`.
/// Equal to the char count for that run (whitespace is ASCII).
pub(super) fn leading_ws_byte_len(line: &str) -> usize {
    line.bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count()
}

/// Maps a pulldown-cmark start `Tag` to its corresponding `ElementKind`, for
/// the tags whose end events emit a stacked element via the standard
/// push-on-start / pop-on-end pattern. Tags handled specially (e.g. `Item`,
/// `Code`) return `None`.
/// Whether `tag` opens a top-level block in pulldown's sense — one
/// whose `Event::End` closes a syntactic unit that could otherwise
/// lazy-continue past blank lines. Used to track parser nesting
/// depth for reset-boundary detection in `ParsedBuffer::parse`.
///
/// Includes Heading because its end implies the parser is between
/// blocks. Excludes `Tag::Item` because items live inside a `List`
/// — tracking the inner item nesting would double-count.
fn is_top_level_block_tag(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::List(_)
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::Heading { .. }
    )
}

fn is_top_level_block_tag_end(tag_end: &TagEnd) -> bool {
    matches!(
        tag_end,
        TagEnd::Paragraph
            | TagEnd::List(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::Heading(_)
    )
}

fn tag_to_kind(tag: &Tag) -> Option<ElementKind> {
    Some(match tag {
        Tag::Strong => ElementKind::Bold,
        Tag::Emphasis => ElementKind::Italic,
        Tag::Strikethrough => ElementKind::Strikethrough,
        Tag::Link { .. } => ElementKind::Link,
        Tag::BlockQuote(_) => ElementKind::Blockquote,
        Tag::Heading { level, .. } => match level {
            HeadingLevel::H1 => ElementKind::HeadingH1,
            HeadingLevel::H2 => ElementKind::HeadingH2,
            _ => ElementKind::HeadingH3,
        },
        _ => return None,
    })
}

pub(super) fn list_marker_len(s: &str) -> Option<usize> {
    if s.starts_with("- ") || s.starts_with("* ") || s.starts_with("+ ") {
        return Some(2);
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
        Some(i + 2)
    } else {
        None
    }
}

/// Convert a byte offset in the joined buffer to `(row, char_col)` within
/// `lines`. Assumes the joined buffer uses `'\n'` separators (one byte each)
/// between consecutive lines.
fn byte_to_row_col(byte_offset: usize, lines: &[String], line_starts: &[usize]) -> (usize, usize) {
    // Binary-search the row whose start byte is <= byte_offset.
    let row = match line_starts.binary_search(&byte_offset) {
        Ok(r) => r,
        Err(r) => r.saturating_sub(1),
    };
    let row = row.min(lines.len().saturating_sub(1));
    let within = byte_offset - line_starts[row];
    let line = &lines[row];
    // Clamp: if `byte_offset` is the trailing '\n', treat as end-of-line.
    let byte_in_line = within.min(line.len());
    let char_col = line[..byte_in_line].chars().count();
    (row, char_col)
}

fn span_style(kind: Option<ElementKind>, is_sigil_region: bool, theme: &Theme) -> Style {
    match kind {
        None => {
            if is_sigil_region {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                Style::default().fg(theme.fg.to_ratatui())
            }
        }
        Some(ElementKind::Bold) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::BOLD),
        Some(ElementKind::Italic) => Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(Modifier::ITALIC),
        Some(ElementKind::Strikethrough) => Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(Modifier::CROSSED_OUT),
        Some(ElementKind::InlineCode) => Style::default()
            .fg(theme.fg.to_ratatui())
            .bg(theme.bg_selected.to_ratatui()),
        Some(ElementKind::Link) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::UNDERLINED),
        Some(ElementKind::Image) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::ITALIC),
        Some(ElementKind::HeadingH1) => {
            if is_sigil_region {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                Style::default()
                    .fg(theme.accent.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            }
        }
        Some(ElementKind::HeadingH2) => {
            if is_sigil_region {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                Style::default()
                    .fg(theme.fg.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            }
        }
        Some(ElementKind::HeadingH3) => {
            if is_sigil_region {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                Style::default().fg(theme.fg_secondary.to_ratatui())
            }
        }
        Some(ElementKind::Blockquote) => Style::default().fg(theme.fg_secondary.to_ratatui()),
        Some(ElementKind::WikiLink) => Style::default()
            .fg(theme.color_directory.to_ratatui())
            .add_modifier(Modifier::UNDERLINED),
        Some(ElementKind::Label) => Style::default()
            .fg(theme.color_tag.to_ratatui())
            .add_modifier(Modifier::BOLD),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;
    fn t() -> Theme {
        Theme::default()
    }
    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn parse_bold_range() {
        let e = MarkdownSpanner::parse_elements("**bold**");
        let b = e.iter().find(|x| x.kind == ElementKind::Bold).unwrap();
        assert_eq!((b.start_char, b.end_char), (0, 8));
    }
    #[test]
    fn parse_italic() {
        assert!(
            MarkdownSpanner::parse_elements("*hi*")
                .iter()
                .any(|e| e.kind == ElementKind::Italic)
        );
    }
    #[test]
    fn parse_strikethrough() {
        let e = MarkdownSpanner::parse_elements("~~gone~~");
        let s = e
            .iter()
            .find(|x| x.kind == ElementKind::Strikethrough)
            .unwrap();
        assert_eq!((s.start_char, s.end_char), (0, 8));
    }
    #[test]
    fn strikethrough_renders_with_crossed_out_modifier() {
        let s = MarkdownSpanner::render("~~gone~~", "~~gone~~", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "gone");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::CROSSED_OUT))
        );
    }
    #[test]
    fn parse_inline_code() {
        assert!(
            MarkdownSpanner::parse_elements("`x`")
                .iter()
                .any(|e| e.kind == ElementKind::InlineCode)
        );
    }
    #[test]
    fn parse_link() {
        assert!(
            MarkdownSpanner::parse_elements("[t](u)")
                .iter()
                .any(|e| e.kind == ElementKind::Link)
        );
    }

    #[test]
    fn parse_image_emits_image_element_and_placeholder() {
        let line = "see ![alt](../assets/img.png) here";
        let parsed = ParsedLine::parse(line);
        let img = parsed
            .elements
            .iter()
            .find(|e| e.kind == ElementKind::Image)
            .expect("image element");
        assert_eq!(line.chars().nth(img.start_char), Some('!'));
        assert_eq!(line.chars().nth(img.end_char - 1), Some(')'));
        let ph = parsed
            .image_placeholders
            .iter()
            .find(|p| p.start_char == img.start_char)
            .expect("placeholder for image");
        assert_eq!(ph.placeholder, "[img.png]");
        for pos in img.start_char..img.end_char {
            assert!(
                !parsed.content_vis[pos],
                "char {pos} should be hidden inside image span"
            );
        }
    }

    #[test]
    fn render_image_substitutes_placeholder_text() {
        let line = "before ![alt](pic.gif) after";
        let parsed = ParsedLine::parse(line);
        let spans =
            MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 80, &t());
        let rendered: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("[pic.gif]"),
            "rendered text {rendered:?} should include placeholder"
        );
        assert!(
            !rendered.contains("![alt]"),
            "raw image syntax should not appear in rendered output: {rendered:?}"
        );
    }

    #[test]
    fn render_image_with_empty_alt_uses_filename() {
        let line = "![](image.png)";
        let parsed = ParsedLine::parse(line);
        let spans =
            MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 40, &t());
        let rendered: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(rendered, "[image.png]");
    }

    #[test]
    fn rendered_cursor_col_accounts_for_placeholder_width() {
        // "![](x.png)" → placeholder "[x.png]" (7 chars) replaces 10 source chars.
        let line = "a ![](x.png) b";
        let parsed = ParsedLine::parse(line);
        let after_placeholder = MarkdownSpanner::rendered_cursor_col_with(
            line,
            &parsed,
            0,
            "a ![](x.png) b".chars().count(), // cursor at end
            true,
            false,
        );
        // "a " (2) + "[x.png]" (7) + " b" (2) = 11.
        assert_eq!(after_placeholder, 11);
    }
    #[test]
    fn parse_h1() {
        assert!(
            MarkdownSpanner::parse_elements("# T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH1)
        );
    }
    #[test]
    fn parse_h2() {
        assert!(
            MarkdownSpanner::parse_elements("## T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH2)
        );
    }
    #[test]
    fn parse_h3() {
        assert!(
            MarkdownSpanner::parse_elements("### T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH3)
        );
    }
    #[test]
    fn force_raw_no_styling() {
        let s = MarkdownSpanner::render("**x**", "**x**", 0, None, true, true, 40, &t());
        assert_eq!(text(&s), "**x**");
        assert!(
            !s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD))
        );
    }
    #[test]
    fn plain_text_passthrough() {
        let s = MarkdownSpanner::render("hi", "hi", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "hi");
    }
    #[test]
    fn bold_without_cursor_hides_markers() {
        let s = MarkdownSpanner::render("**bold**", "**bold**", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "bold");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD))
        );
    }
    #[test]
    fn bold_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("**bold**", "**bold**", 0, Some(3), true, false, 40, &t());
        assert_eq!(text(&s), "**bold**");
    }
    #[test]
    fn bold_cursor_outside_stays_rendered() {
        let line = "hello **bold** world";
        let s = MarkdownSpanner::render(line, line, 0, Some(1), true, false, 40, &t());
        assert!(!text(&s).contains("**"));
    }
    #[test]
    fn italic_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("*hi*", "*hi*", 0, Some(1), true, false, 40, &t());
        assert_eq!(text(&s), "*hi*");
    }
    #[test]
    fn inline_code_hides_backticks() {
        let s = MarkdownSpanner::render("`x`", "`x`", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "x");
    }
    #[test]
    fn h1_first_line_contains_hash() {
        let s = MarkdownSpanner::render("# T", "# T", 0, None, true, false, 40, &t());
        assert!(text(&s).contains('#'));
        assert!(text(&s).contains('T'));
    }
    #[test]
    fn continuation_line_no_hash() {
        let s = MarkdownSpanner::render("cont", "# T cont", 2, None, false, false, 40, &t());
        assert!(!text(&s).contains('#'));
    }
    #[test]
    fn unordered_list_shows_marker() {
        let s = MarkdownSpanner::render("- item", "- item", 0, None, true, false, 40, &t());
        assert!(
            text(&s).starts_with("- "),
            "expected '- item', got '{}'",
            text(&s)
        );
        assert!(text(&s).contains("item"));
    }
    #[test]
    fn ordered_list_shows_marker() {
        let s = MarkdownSpanner::render("1. item", "1. item", 0, None, true, false, 40, &t());
        assert!(
            text(&s).starts_with("1. "),
            "expected '1. item', got '{}'",
            text(&s)
        );
    }
    #[test]
    fn nested_list_4space_link_rendered() {
        // 4-space indent + list marker + markdown link.
        let line = "    - [my link](url)";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        // Link styling must appear (UNDERLINED modifier) and the raw "](url)" sigils
        // must be hidden.
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED)),
            "link text should be underlined on a 4-space-indented nested list item"
        );
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            rendered.contains("my link"),
            "link display text should be visible; got {:?}",
            rendered
        );
        assert!(
            !rendered.contains("](url)"),
            "link URL sigil should be hidden; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_tab_bold_rendered() {
        let line = "\t- **bold nested**";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD)),
            "bold text should be styled on a tab-indented nested list item"
        );
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            !rendered.contains("**"),
            "bold markers should be hidden; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_4space_wikilink_rendered() {
        let line = "    - [[Target Note]]";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            !rendered.contains("[["),
            "wikilink brackets should be hidden; got {:?}",
            rendered
        );
        assert!(
            rendered.contains("Target Note"),
            "wikilink target text should render; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_2space_still_renders_link() {
        // Existing 2-space case — must not regress.
        let line = "  - [link](url)";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn empty_heading_shows_hash_sigil() {
        let line = "# ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert!(
            text(&s).contains('#'),
            "hash sigil should render in empty heading"
        );
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 1, true, false);
        assert_eq!(col, 1, "cursor after '#' should be at rendered col 1");
    }
    #[test]
    fn empty_heading_hash_only_shows() {
        let line = "#";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert!(text(&s).contains('#'));
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 1, true, false);
        assert_eq!(col, 1);
    }
    #[test]
    fn heading_trailing_spaces_are_rendered() {
        let line = "# Hello   ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(
            text(&s),
            "# Hello   ",
            "trailing spaces in heading should render"
        );
    }
    #[test]
    fn heading_trailing_spaces_cursor_col_correct() {
        let line = "# Hello   ";
        // cursor at logical pos 9 (last trailing space): positions 0..9 all emit → rendered col 9
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 9, true, false);
        assert_eq!(
            col, 9,
            "cursor in trailing space of heading should map to rendered col 9"
        );
    }
    #[test]
    fn trailing_spaces_are_rendered() {
        let line = "hello   ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "hello   ");
    }
    #[test]
    fn trailing_spaces_cursor_col_correct() {
        let line = "hello   ";
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 7, true, false);
        assert_eq!(col, 7);
    }
    #[test]
    fn list_marker_on_continuation_line_hidden() {
        let s = MarkdownSpanner::render("cont", "- cont", 2, None, false, false, 40, &t());
        assert!(!text(&s).starts_with("- "));
    }
    #[test]
    fn parsed_line_heading_sigil_end_empty_heading() {
        // "#" alone: no content chars, sigil_end should equal e.end_char (1)
        let p = ParsedLine::parse("#");
        assert_eq!(p.heading_sigil_end(), Some(1));
    }
    #[test]
    fn parsed_line_heading_sigil_end_with_content() {
        // "# T": sigil is "# " (2 chars), first content at pos 2
        let p = ParsedLine::parse("# T");
        assert_eq!(p.heading_sigil_end(), Some(2));
    }
    #[test]
    fn parsed_line_reuse_matches_individual() {
        let line = "**hello** world";
        let parsed = ParsedLine::parse(line);
        let s1 = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        let s2 = MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 40, &t());
        assert_eq!(
            s1.iter().map(|s| s.content.as_ref()).collect::<String>(),
            s2.iter().map(|s| s.content.as_ref()).collect::<String>(),
        );
    }

    // ── WikiLink tests ────────────────────────────────────────────────────────

    #[test]
    fn parse_wikilink() {
        let e = MarkdownSpanner::parse_elements("[[My Note]]");
        let wl = e.iter().find(|x| x.kind == ElementKind::WikiLink).unwrap();
        assert_eq!((wl.start_char, wl.end_char), (0, 11));
    }

    #[test]
    fn wikilink_without_cursor_hides_brackets() {
        let line = "[[My Note]]";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "My Note");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn wikilink_cursor_inside_shows_brackets() {
        let line = "[[My Note]]";
        // cursor at pos 4 (inside "My Note")
        let s = MarkdownSpanner::render(line, line, 0, Some(4), true, false, 40, &t());
        assert_eq!(text(&s), "[[My Note]]");
    }

    #[test]
    fn wikilink_cursor_outside_hides_brackets() {
        let line = "hello [[My Note]] world";
        let s = MarkdownSpanner::render(line, line, 0, Some(1), true, false, 40, &t());
        assert!(!text(&s).contains("[["));
        assert!(!text(&s).contains("]]"));
    }

    #[test]
    fn wikilink_mid_sentence() {
        let line = "See [[Topic]] for details";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "See Topic for details");
    }

    #[test]
    fn wikilink_cursor_col_accounts_for_brackets() {
        // "[[Hi]]" — cursor at pos 2 ('H') is inside the element, so it expands.
        // Rendered col counts pos 0 ('['), pos 1 ('[') as visible (expanded sigils) → col = 2.
        let col = MarkdownSpanner::rendered_cursor_col("[[Hi]]", 0, 2, true, false);
        assert_eq!(col, 2);

        // Cursor outside the wikilink (pos 0 on a plain-text line before it):
        // "See [[Hi]] x" with cursor at pos 0 — wikilink not expanded, brackets hidden.
        // pos 0 ('S') is plain text, rendered col = 0.
        let col2 = MarkdownSpanner::rendered_cursor_col("See [[Hi]] x", 0, 0, true, false);
        assert_eq!(col2, 0);
    }

    #[test]
    fn buffer_parse_nested_list_under_parent() {
        // Canonical nested-list pattern: parent at col 0, child indented 4.
        let lines = vec![
            "- parent".to_string(),
            "    - [child link](url)".to_string(),
        ];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert_eq!(parsed.len(), 2);

        // Parent line: list sigil at col 2.
        assert_eq!(parsed[0].list_sigil_end(), Some(2));

        // Child line: pulldown-cmark reports the item marker at col 4.
        assert_eq!(
            parsed[1].list_sigil_end(),
            Some(6),
            "child's sigil_end should be after '    - ' (6 chars)"
        );

        // Child line has a Link element.
        assert!(
            parsed[1]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Link),
            "nested list item should contain a Link element"
        );
    }

    #[test]
    fn buffer_parse_standalone_2space_list_still_works() {
        // Regression: 2-space indent works on its own too.
        let lines = vec!["  - [link](url)".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert!(
            parsed[0]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Link)
        );
        assert_eq!(parsed[0].list_sigil_end(), Some(4));
    }

    #[test]
    fn buffer_parse_top_level_unchanged() {
        // Ensure nothing about top-level rendering changed.
        let lines = vec!["- [link](url)".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert!(
            parsed[0]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Link)
        );
        assert_eq!(parsed[0].list_sigil_end(), Some(2));
    }

    #[test]
    fn buffer_parse_empty_lines_preserved() {
        let lines = vec![
            "# Title".to_string(),
            String::new(),
            "paragraph".to_string(),
        ];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[1].elements.len(), 0);
        assert_eq!(parsed[1].content_vis.len(), 0);
    }

    #[test]
    fn buffer_parse_ordered_nested_list() {
        let lines = vec!["1. first".to_string(), "    1. nested".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert_eq!(parsed[0].list_sigil_end(), Some(3));
        assert_eq!(parsed[1].list_sigil_end(), Some(7));
    }

    #[test]
    fn buffer_parse_setext_h1_spans_two_rows() {
        // Setext H1: the `=====` line is part of the heading span.
        // Under the old per-line parser, row 1 rendered as plain text; under the
        // whole-buffer parser, pulldown emits one HeadingH1 covering both rows and
        // row 1 has no Text events, so the underline renders in the sigil color.
        // Pin this behavior — a regression would silently un-style setext headings.
        let lines = vec!["My Heading".to_string(), "==========".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert!(
            parsed[0]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH1),
            "setext underline must tag row 0 as HeadingH1"
        );
        assert!(
            parsed[1]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH1),
            "setext underline must tag row 1 as HeadingH1"
        );
        // Row 1 has no Text events — content_vis is all false.
        assert!(
            parsed[1].content_vis.iter().all(|v| !v),
            "setext underline row has no content"
        );
    }

    #[test]
    fn buffer_parse_multiline_blockquote() {
        // Two blockquote lines in a row — pulldown folds them into one blockquote.
        // Both rows must carry a Blockquote element so rendering is consistent.
        let lines = vec!["> first line".to_string(), "> second line".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert!(
            parsed[0]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Blockquote),
            "row 0 must tag as Blockquote"
        );
        assert!(
            parsed[1]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Blockquote),
            "row 1 must tag as Blockquote"
        );
    }

    #[test]
    fn parse_line_emits_label_for_hashtag() {
        let line = "see #rust later";
        let parsed = ParsedLine::parse(line);
        let label = parsed
            .elements
            .iter()
            .find(|e| matches!(e.kind, ElementKind::Label));
        assert!(
            label.is_some(),
            "expected Label element: {:?}",
            parsed.elements
        );
        let l = label.unwrap();
        let span: String = line
            .chars()
            .skip(l.start_char)
            .take(l.end_char - l.start_char)
            .collect();
        assert_eq!(span, "#rust");
    }

    #[test]
    fn parse_line_skips_label_inside_inline_code() {
        let parsed = ParsedLine::parse("use `#foo` here");
        let has_label = parsed
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Label));
        assert!(!has_label, "should not emit Label inside inline code");
    }

    // ── New label-parity tests (F2, F3, F4) ──────────────────────────────────

    #[test]
    fn parse_line_skips_label_inside_markdown_link() {
        let parsed = ParsedLine::parse("[see docs](#section) and #real");
        let labels: Vec<_> = parsed
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Label))
            .collect();
        assert_eq!(
            labels.len(),
            1,
            "only #real should be a label, not #section in the link"
        );
        let l = labels[0];
        let span: String = "[see docs](#section) and #real"
            .chars()
            .skip(l.start_char)
            .take(l.end_char - l.start_char)
            .collect();
        assert_eq!(span, "#real");
    }

    #[test]
    fn parse_line_skips_label_inside_link_display_text() {
        let parsed = ParsedLine::parse("[#todo](notes/project.md)");
        let has_label = parsed
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Label));
        assert!(
            !has_label,
            "hashtag inside link display text should not become Label"
        );
    }

    #[test]
    fn parse_line_skips_label_after_label_char() {
        let parsed = ParsedLine::parse("foo#bar baz");
        let has_label = parsed
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Label));
        assert!(
            !has_label,
            "word#tag should not emit Label without word boundary"
        );
    }

    #[test]
    fn parse_line_skips_label_for_double_hash() {
        // `##draft` is Markdown header territory, not a label — pin the
        // highlighter to the same rule the indexer enforces so a future
        // core relaxation cannot silently re-color this span.
        let parsed = ParsedLine::parse("##draft");
        let has_label = parsed
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Label));
        assert!(!has_label, "##draft should not emit Label");
    }

    #[test]
    fn parse_line_skips_label_for_adjacent_hash_run() {
        // `#tag#more` — adjacent `#` invalidates both halves at the index
        // level; the highlighter must agree to avoid suggesting tags that
        // will never appear in the labels table.
        let parsed = ParsedLine::parse("#tag#more");
        let labels: Vec<_> = parsed
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Label))
            .collect();
        assert!(
            labels.is_empty(),
            "#tag#more should not emit Label, got {:?}",
            labels
        );
    }

    #[test]
    fn parse_buffer_skips_label_inside_fenced_block() {
        let buffer = vec![
            "before".to_string(),
            "```".to_string(),
            "#inside".to_string(),
            "```".to_string(),
            "after #outside".to_string(),
        ];
        let lines = ParsedBuffer::parse(&buffer).lines;
        let inside_labels: Vec<_> = lines[2]
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Label))
            .collect();
        assert!(
            inside_labels.is_empty(),
            "no Label emitted for hashtags in fenced blocks"
        );

        let outside_labels: Vec<_> = lines[4]
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Label))
            .collect();
        assert_eq!(outside_labels.len(), 1, "#outside still extracted");
    }

    #[test]
    fn parse_range_full_equals_parse() {
        let lines: Vec<String> = vec!["hello".into(), "world".into(), "".into(), "**bold**".into()];
        let full = ParsedBuffer::parse(&lines);
        let range_full = ParsedBuffer::parse_range(&lines, 0..lines.len());
        assert_eq!(full.lines.len(), range_full.lines.len());
        assert_eq!(full.kinds, range_full.kinds);
        for (a, b) in full.lines.iter().zip(range_full.lines.iter()) {
            assert_eq!(a.content_vis, b.content_vis);
            assert_eq!(a.elements.len(), b.elements.len());
        }
    }

    #[test]
    fn parse_range_paragraph_only_slice() {
        let lines: Vec<String> = vec![
            "intro paragraph".into(),
            "".into(),
            "middle line".into(),
            "".into(),
            "outro".into(),
        ];
        let slice = ParsedBuffer::parse_range(&lines, 2..3);
        assert_eq!(slice.lines.len(), 1);
        assert_eq!(slice.kinds, vec![LineConstructKind::Plain]);
    }

    #[test]
    fn splice_replaces_range() {
        let mut pb = ParsedBuffer::parse(&["alpha".into(), "beta".into(), "gamma".into()]);
        let replacement = ParsedBuffer::parse(&["BETA-NEW".into()]);
        let replacement_kind = replacement.kinds[0];
        pb.splice(1..2, replacement);
        assert_eq!(pb.lines.len(), 3);
        assert_eq!(pb.kinds.len(), 3);
        assert_eq!(
            pb.kinds[1], replacement_kind,
            "replacement landed at the wrong index"
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "splice")]
    fn splice_panics_on_length_mismatch_in_debug() {
        let mut pb = ParsedBuffer::parse(&["a".into(), "b".into()]);
        let too_short = ParsedBuffer::parse(&["X".into()]);
        pb.splice(0..2, too_short);
    }
}
