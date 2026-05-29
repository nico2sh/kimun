//! `ParsedBuffer`: the full-buffer parsed representation produced by
//! `parse()`. Owns the per-row classification (`kinds`), the
//! lazy-depth tracking that gates reset-boundary detection, and the
//! splice/parse-range machinery used by the incremental editor view.
//!
//! See openspec changes `parse-reset-boundaries` and
//! `parse-reset-boundaries-v2` for the design.

use super::super::parse_incremental::LineConstructKind;
use super::detect::{detect_image_placeholders, detect_wikilinks};
use super::{
    Element, ElementKind, PARSER_OPTIONS, ParsedLine, leading_ws_byte_len, list_marker_len,
    tag_to_kind,
};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone)]
pub struct ParsedBuffer {
    pub lines: Vec<ParsedLine>,
    pub kinds: Vec<LineConstructKind>,
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
    /// Per-row lazy-construct depth AFTER processing the row's open
    /// events (i.e. the prefix sum is applied row-by-row INCLUDING
    /// the current row's deltas before being stored). Counts ONLY
    /// constructs that CommonMark allows to extend across blank rows
    /// (List, BlockQuote, IndentedCode, HtmlBlock — see
    /// [`is_lazy_continuable_tag`]). Fenced code, Paragraph, Heading
    /// are NOT counted because their parse state is reset at blank
    /// rows or single-row terminators.
    ///
    /// `lazy_depth[r] == 0` is a necessary precondition for treating
    /// row `r` as a reset boundary: at depth 0 there is no open
    /// lazy-extendable construct that a fresh parser starting at `r`
    /// could miss.
    pub lazy_depth: Vec<u32>,
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
        // +1 at the row a lazy-continuable construct opens, -1 at the
        // row past where it closes. Length is `lines.len() + 1` so
        // end-of-buffer drops have a sink (read by the prefix sum
        // only up to `lines.len() - 1`). See [`is_lazy_continuable_tag`].
        let mut lazy_delta: Vec<i32> = vec![0; lines.len() + 1];
        // CodeBlock kind tracker. `Some(true)` = an Indented (lazy)
        // CodeBlock is open; `Some(false)` = a Fenced (non-lazy)
        // CodeBlock is open; `None` = no CodeBlock open. CommonMark
        // does not nest code blocks, so a single Option captures the
        // kind across the matching Start/End pair — and reading None
        // on End signals an unmatched-End invariant violation.
        let mut indented_codeblock_open: Option<bool> = None;

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

            // V2 lazy_delta tracking — only constructs that
            // lazy-extend across blank rows per CommonMark §4.4 / §4.6
            // / §5.1 / §5.2. See [`is_lazy_continuable_tag`].
            //
            // End events use an unclamped row for the drop position:
            // pulldown's `range.end` for an end-of-buffer block lands
            // past the last content byte, which `byte_to_row_col`
            // would clamp to `lines.len() - 1`. Dropping AT the last
            // content row would zero `lazy_depth` there even though
            // the construct still covers it.
            //
            // The CodeBlock arm is placed first so it absorbs both
            // variants before the generic `is_lazy_continuable_tag`
            // arm sees them — Indented would otherwise double-count.
            match &event {
                Event::Start(Tag::CodeBlock(kind)) => {
                    let is_indented = matches!(kind, CodeBlockKind::Indented);
                    indented_codeblock_open = Some(is_indented);
                    if is_indented && sr < lazy_delta.len() {
                        lazy_delta[sr] += 1;
                    }
                }
                Event::Start(tag) if is_lazy_continuable_tag(tag) && sr < lazy_delta.len() => {
                    lazy_delta[sr] += 1;
                }
                Event::End(TagEnd::CodeBlock) => {
                    let was_indented = indented_codeblock_open.take();
                    debug_assert!(
                        was_indented.is_some(),
                        "Event::End(CodeBlock) without matching Start at byte {}",
                        range.start,
                    );
                    if was_indented == Some(true) {
                        let (er_lazy, _) =
                            byte_to_row_col_unclamped(range.end, lines, &line_starts);
                        let drop_at = er_lazy.min(lines.len());
                        if drop_at < lazy_delta.len() {
                            lazy_delta[drop_at] -= 1;
                        }
                    }
                }
                Event::End(tag_end) if is_lazy_continuable_tag_end(tag_end) => {
                    let (er_lazy, _) = byte_to_row_col_unclamped(range.end, lines, &line_starts);
                    let drop_at = er_lazy.min(lines.len());
                    if drop_at < lazy_delta.len() {
                        lazy_delta[drop_at] -= 1;
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
                    if sr < kinds.len() {
                        kinds[sr] = LineConstructKind::FenceMarker;
                    }
                    // Rows between opening and closing fences are content.
                    // Pulldown can emit `er == sr` for a single-row degenerate
                    // fence (no body, no closing fence on a separate row) or
                    // `er == lines.len()` for an unclosed fence at EOF; both
                    // make the `[(sr+1)..er]` slice invalid.
                    let content_end = er.min(kinds.len());
                    if content_end > sr + 1 {
                        kinds[(sr + 1)..content_end].fill(LineConstructKind::FenceContent);
                    }
                    // Closing fence marker row (er is the row of the closing
                    // ```). When `er == sr` (degenerate single-row fence) or
                    // `er >= lines.len()` (unclosed fence at EOF) there is
                    // no separate closing row to mark.
                    if er > sr && er < kinds.len() {
                        kinds[er] = LineConstructKind::FenceMarker;
                    }
                }
                Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                    if code_block_depth == 0 {
                        code_block_start = Some(range.start);
                    }
                    code_block_depth += 1;
                    let hi = (er + 1).min(kinds.len());
                    if hi > sr {
                        kinds[sr..hi].fill(LineConstructKind::IndentedCode);
                    }
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
                    let hi = (er + 1).min(kinds.len());
                    if hi > sr {
                        kinds[sr..hi].fill(LineConstructKind::HtmlBlock);
                    }
                }
                Event::Html(_) => {
                    // Block-level HTML body. Already classified by the
                    // enclosing `Tag::HtmlBlock` arm; this branch is a
                    // safety-net for any row pulldown emits between the
                    // tag boundaries but defers to fence/code kinds when
                    // the rows happen to overlap (rare, but possible
                    // with malformed input).
                    let hi = (er + 1).min(kinds.len());
                    if hi > sr {
                        for kind in &mut kinds[sr..hi] {
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

        // Prefix-sum lazy_delta → per-row lazy_depth, record reset
        // boundaries at rows where lazy_depth == 0 AND kinds[r] is
        // Blank. The lazy_depth==0 condition rules out blank rows
        // inside a lazy-continuable construct (IndentedCode multi-
        // chunk, etc.) where splicing across the blank would diverge
        // from a fresh parse.
        let mut lazy_depth_acc: i32 = 0;
        let mut lazy_depth: Vec<u32> = Vec::with_capacity(lines.len());
        for r in 0..lines.len() {
            lazy_depth_acc += lazy_delta[r];
            debug_assert!(
                lazy_depth_acc >= 0,
                "lazy_depth went negative at row {r}: delta history is unbalanced"
            );
            // `as u32` is correct under the assert (always non-negative
            // here); in a hypothetical release with imbalanced deltas
            // it would wrap to a very large value and surface as a
            // panic in the boundary check below — preferable to a
            // silent `.max(0)` clamp that would mask the bug.
            lazy_depth.push(lazy_depth_acc as u32);
            if lazy_depth_acc == 0 && kinds[r] == LineConstructKind::Blank {
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
            lazy_depth,
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
        let lazy_depth = vec![0u32; lines.len()];
        ParsedBuffer {
            lines: out,
            kinds,
            reset_boundaries,
            lazy_depth,
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
        debug_assert!(
            other.lazy_depth.len() == range.len(),
            "splice: other.lazy_depth.len() ({}) != range.len() ({})",
            other.lazy_depth.len(),
            range.len(),
        );
        self.lines.splice(range.clone(), other.lines);
        self.kinds.splice(range.clone(), other.kinds);
        self.lazy_depth.splice(range.clone(), other.lazy_depth);

        // Rebuild `reset_boundaries`. The incremental splice path never
        // changes line count (gated upstream in try_incremental_parse).
        // Three runs, already sorted and positionally ordered:
        //   - low:  self boundaries STRICTLY before the replaced region
        //           (`b < range.start`). These rows are untouched.
        //   - mid:  the replaced region's boundaries, RECOMPUTED from the
        //           now-merged `kinds`/`lazy_depth` using the same rule as
        //           `parse` (interior row is a boundary iff Blank with
        //           lazy_depth 0). O(range.len()) ≤ the widen cap — no
        //           pulldown reparse.
        //   - high: self boundaries at/after `range.end` (untouched rows).
        //
        // Recomputing `mid` is the correctness fix: the edited rows'
        // boundary status cannot be inherited. Inheriting `self`'s
        // boundary at `range.start` (old `b <= range.start`) kept stale
        // boundaries when the edit removed a Blank row; promoting the
        // slice's sentinels invented boundaries the heuristic widener's
        // non-reset edges never had. The post-splice `kinds`/`lazy_depth`
        // are authoritative (the reset-boundary widening contract
        // guarantees lazy_depth 0 at `range.start`, so the slice's
        // isolated parse agrees with the parent context there).
        let lines_len = self.lines.len();
        let mut merged: Vec<usize> = Vec::with_capacity(self.reset_boundaries.len() + 1);
        merged.extend(
            self.reset_boundaries
                .iter()
                .copied()
                .filter(|&b| b < range.start),
        );
        for r in range.clone() {
            if r != 0
                && r != lines_len
                && self.lazy_depth[r] == 0
                && self.kinds[r] == LineConstructKind::Blank
            {
                merged.push(r);
            }
        }
        merged.extend(
            self.reset_boundaries
                .iter()
                .copied()
                .filter(|&b| b >= range.end),
        );
        // Sentinel `0` survives in `low` whenever `range.start > 0`; when
        // the edit starts at row 0 it is not interior, so add it back.
        // `lines_len` always survives in `high` (self held it, and
        // `range.end <= lines_len`).
        if merged.first() != Some(&0) {
            merged.insert(0, 0);
        }
        merged.dedup();
        debug_assert!(
            merged.windows(2).all(|w| w[0] < w[1]),
            "splice: merged boundaries must be strictly ascending: {merged:?}"
        );
        debug_assert!(
            merged.first() == Some(&0) && merged.last() == Some(&lines_len),
            "splice: merged boundaries must start with 0 and end with lines.len() ({lines_len})"
        );
        // Structural invariant: every interior reset boundary sits on a
        // Blank row — `parse` only records non-sentinel boundaries at
        // `kinds[r] == Blank` rows (0 and lines.len() are unconditional
        // sentinels). A non-Blank interior boundary means the merge
        // promoted a spurious one — the failure mode where the heuristic
        // widener's range edges leak in as boundaries. Cheap (no full
        // reparse), runs in every debug/test build so CI catches a merge
        // regression that would otherwise only surface under
        // `KIMUN_VIEW_VERIFY_INCREMENTAL`.
        debug_assert!(
            merged
                .iter()
                .all(|&b| b == 0 || b == lines_len || self.kinds[b] == LineConstructKind::Blank),
            "splice: interior reset boundary on a non-Blank row — merge \
             promoted a spurious boundary: {merged:?}"
        );
        self.reset_boundaries = merged;
    }
}

/// Whether `tag` opens a lazy-continuable construct — one whose
/// parse state can extend across blank rows per CommonMark §4.4
/// (IndentedCode), §4.6 (HtmlBlock types 1/2/6/7), §5.1
/// (BlockQuote paragraph continuation), §5.2 (loose list
/// continuation). Used to populate `ParsedBuffer::lazy_depth`,
/// which gates reset-boundary detection.
///
/// Excluded: `Paragraph` (blank terminates it), `Heading` (single
/// row), `CodeBlock(Fenced)` (explicit closing fence required, not
/// lazy across blanks), `Item` (counted via parent `List`).
///
/// Conservative on HtmlBlock: pulldown's `Tag::HtmlBlock` does not
/// distinguish CommonMark's 7 HTML-block types in its public API.
/// Treating all HtmlBlocks as lazy-continuable over-triggers full
/// rebuilds on types 3/4/5 edits but never silently miscompiles.
fn is_lazy_continuable_tag(tag: &Tag) -> bool {
    matches!(
        tag,
        Tag::List(_)
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(CodeBlockKind::Indented)
            | Tag::HtmlBlock
    )
}

/// `is_lazy_continuable_tag`'s `TagEnd` counterpart for the tags
/// that can be unambiguously identified from `TagEnd` alone.
///
/// `TagEnd::CodeBlock` is INTENTIONALLY excluded: pulldown's
/// `TagEnd` variant for CodeBlock does not carry the
/// `CodeBlockKind::{Indented, Fenced(_)}` discriminant. Callers
/// disambiguate via a parallel stack populated on
/// `Tag::CodeBlock(_)` Start events.
fn is_lazy_continuable_tag_end(tag_end: &TagEnd) -> bool {
    matches!(
        tag_end,
        TagEnd::List(_) | TagEnd::BlockQuote(_) | TagEnd::HtmlBlock
    )
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

/// Like [`byte_to_row_col`] but returns `(lines.len(), 0)` when
/// `byte_offset` is at or past the joined buffer's last content
/// byte. Used to compute the drop-row for `lazy_delta` decrements
/// on `Event::End` of end-of-buffer blocks: a block that ends
/// past-EOF must drop lazy_depth at `lines.len()` (past-array),
/// not at `lines.len() - 1` — otherwise the decrement lands ON
/// the last content row and lazy_depth there becomes 0 even
/// though the construct still semantically covers that row.
///
/// The clamped variant is correct for Start events (which always
/// land on a real content row) but wrong for End events when the
/// block reaches EOF.
fn byte_to_row_col_unclamped(
    byte_offset: usize,
    lines: &[String],
    line_starts: &[usize],
) -> (usize, usize) {
    // Discriminate Ok vs Err from binary_search:
    //
    // - `Ok(r)`: byte_offset matches `line_starts[r]` exactly. If
    //   `r < lines.len()`, it lands at the START of row r — return
    //   (r, 0). If `r == lines.len()`, it matches the past-EOF
    //   sentinel; return `(lines.len(), 0)`.
    // - `Err(r)`: byte_offset is strictly between
    //   `line_starts[r - 1]` and `line_starts[r]`, i.e. inside row
    //   `r - 1`. Compute within-row offset; if the offset is past
    //   the row's last content byte AND it's the last row, treat
    //   as past-EOF (the `'\n'` separator slot for non-last rows is
    //   handled by `Ok` exact match on the next row's start).
    //
    // The previous single check `row + 1 == lines.len() && within >=
    // line.len()` also fired for `Ok(lines.len() - 1)` with within=0
    // and a 0-length last row — incorrectly bumping a block End that
    // landed at the start of a trailing blank row out to the
    // past-EOF slot, leaving `lazy_depth` elevated on the blank that
    // actually closes the construct. Seed: `["> a", ""]` →
    // lazy_depth was `[1, 1]`; correct is `[1, 0]`.
    match line_starts.binary_search(&byte_offset) {
        Ok(r) => {
            if r < lines.len() {
                (r, 0)
            } else {
                (lines.len(), 0)
            }
        }
        Err(r) => {
            let row = r.saturating_sub(1);
            if row >= lines.len() {
                return (lines.len(), 0);
            }
            let within = byte_offset - line_starts[row];
            let line = &lines[row];
            if row + 1 == lines.len() && within >= line.len() {
                return (lines.len(), 0);
            }
            let byte_in_line = within.min(line.len());
            let char_col = line[..byte_in_line].chars().count();
            (row, char_col)
        }
    }
}
