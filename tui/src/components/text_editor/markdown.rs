use crate::settings::themes::Theme;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

/// Shared parser options used by all pulldown-cmark call sites in this module.
const PARSER_OPTIONS: Options = Options::ENABLE_STRIKETHROUGH;

/// Visual columns per tab stop. Must match the `tabstop` setting in the nvim backend.
const TAB_STOP: usize = 4;

/// Compute the display width of a tab character at the given visual column.
fn tab_width_at(col: usize) -> usize {
    TAB_STOP - (col % TAB_STOP)
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
    InlineCode,
    Link,
    HeadingH1,
    HeadingH2,
    HeadingH3,
    Blockquote,
    WikiLink,
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
    /// Stored as `u8`; supports up to 255 elements per line (far more than any real line).
    elem_index: Vec<u8>,
    /// Char offset where the list-item sigil (indent + marker + space) ends on
    /// this line, or `None` if this line is not the first line of a list item.
    list_sigil_end: Option<usize>,
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
                .into_iter()
                .last()
                .expect("ParsedBuffer::parse returns one row per input line")
        } else {
            ParsedBuffer::parse(std::slice::from_ref(&owned))
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
}

pub struct ParsedBuffer;

impl ParsedBuffer {
    /// Parse the entire editor buffer in a single pulldown-cmark pass and return
    /// one `ParsedLine` per input row. Multi-row elements are split so each row
    /// gets its own `Element` entry covering only that row's portion.
    pub fn parse(lines: &[String]) -> Vec<ParsedLine> {
        // Build joined buffer and per-line byte-offset table.
        let total_bytes: usize = lines.iter().map(|l| l.len()).sum::<usize>() + lines.len().saturating_sub(1);
        let mut joined = String::with_capacity(total_bytes);
        let mut line_starts: Vec<usize> = Vec::with_capacity(lines.len() + 1);
        for (i, line) in lines.iter().enumerate() {
            line_starts.push(joined.len());
            joined.push_str(line);
            if i + 1 < lines.len() {
                joined.push('\n');
            }
        }
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

        let parser = Parser::new_ext(&joined, PARSER_OPTIONS);
        for (event, range) in parser.into_offset_iter() {
            let (sr, sc) = byte_to_row_col(range.start, lines, &line_starts);
            let (er, ec) = byte_to_row_col(range.end, lines, &line_starts);
            match event {
                Event::Start(Tag::Strong) => stack.push((sr, sc, ElementKind::Bold)),
                Event::End(TagEnd::Strong) => {
                    if let Some((s_r, s_c, k)) = stack.pop() {
                        emit_span(s_r, s_c, er, ec, k, &mut elements, lines);
                    }
                }
                Event::Start(Tag::Emphasis) => stack.push((sr, sc, ElementKind::Italic)),
                Event::End(TagEnd::Emphasis) => {
                    if let Some((s_r, s_c, k)) = stack.pop() {
                        emit_span(s_r, s_c, er, ec, k, &mut elements, lines);
                    }
                }
                Event::Start(Tag::Link { .. }) => stack.push((sr, sc, ElementKind::Link)),
                Event::End(TagEnd::Link) => {
                    if let Some((s_r, s_c, k)) = stack.pop() {
                        emit_span(s_r, s_c, er, ec, k, &mut elements, lines);
                    }
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    let kind = match level {
                        HeadingLevel::H1 => ElementKind::HeadingH1,
                        HeadingLevel::H2 => ElementKind::HeadingH2,
                        _ => ElementKind::HeadingH3,
                    };
                    stack.push((sr, sc, kind));
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some((s_r, s_c, k)) = stack.pop() {
                        emit_span(s_r, s_c, er, ec, k, &mut elements, lines);
                    }
                }
                Event::Start(Tag::BlockQuote(_)) => stack.push((sr, sc, ElementKind::Blockquote)),
                Event::End(TagEnd::BlockQuote(_)) => {
                    if let Some((s_r, s_c, k)) = stack.pop() {
                        emit_span(s_r, s_c, er, ec, k, &mut elements, lines);
                    }
                }
                Event::Start(Tag::Item) => {
                    // Compute sigil_end directly from the line at the Item's start
                    // col. Pulldown-cmark reports Item range starting at the
                    // marker char (after any leading indent). We walk past the
                    // marker chars to find where the item's content begins.
                    if sr < lines.len() && list_sigil_end[sr].is_none() {
                        let chars_after: String = lines[sr].chars().skip(sc).collect();
                        let marker_len = if chars_after.starts_with("- ")
                            || chars_after.starts_with("* ")
                            || chars_after.starts_with("+ ")
                        {
                            Some(2usize)
                        } else {
                            // Ordered marker: digits followed by ". "
                            let bytes = chars_after.as_bytes();
                            let mut i = 0;
                            while i < bytes.len() && bytes[i].is_ascii_digit() {
                                i += 1;
                            }
                            if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
                                Some(i + 2)
                            } else {
                                None
                            }
                        };
                        if let Some(len) = marker_len {
                            list_sigil_end[sr] = Some(sc + len);
                        }
                    }
                }
                Event::End(TagEnd::Item) => {}
                Event::Code(ref code_text) => {
                    // Inline code — always single-line in practice.
                    if sr == er && sr < lines.len() {
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
                }
                Event::Text(_) | Event::SoftBreak | Event::HardBreak => {
                    // Mark content_vis for each row the event touches.
                    if sr == er {
                        if sr < content_vis.len() {
                            for vis in content_vis[sr].iter_mut().skip(sc).take(ec.saturating_sub(sc))
                            {
                                *vis = true;
                            }
                        }
                    } else {
                        // First row: from sc to end-of-line.
                        if sr < content_vis.len() {
                            let line_chars = content_vis[sr].len();
                            for vis in content_vis[sr].iter_mut().skip(sc).take(line_chars.saturating_sub(sc)) {
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

            debug_assert!(
                els.len() < 255,
                "Too many elements on a single line ({})",
                els.len()
            );
            let total = line.chars().count();
            let mut elem_vis = vec![false; total];
            let mut elem_index = vec![0u8; total];
            for (i, e) in els.iter().enumerate() {
                let tag = (i + 1).min(255) as u8;
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
            });
        }

        out
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
                Event::Start(Tag::Strong) => stack.push((sc, ElementKind::Bold)),
                Event::End(TagEnd::Strong) => {
                    if let Some((s, k)) = stack.pop() {
                        elements.push(Element {
                            start_char: s,
                            end_char: ec,
                            kind: k,
                        });
                    }
                }
                Event::Start(Tag::Emphasis) => stack.push((sc, ElementKind::Italic)),
                Event::End(TagEnd::Emphasis) => {
                    if let Some((s, k)) = stack.pop() {
                        elements.push(Element {
                            start_char: s,
                            end_char: ec,
                            kind: k,
                        });
                    }
                }
                Event::Start(Tag::Link { .. }) => stack.push((sc, ElementKind::Link)),
                Event::End(TagEnd::Link) => {
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
                Event::Start(Tag::Heading { level, .. }) => {
                    let kind = match level {
                        HeadingLevel::H1 => ElementKind::HeadingH1,
                        HeadingLevel::H2 => ElementKind::HeadingH2,
                        _ => ElementKind::HeadingH3,
                    };
                    stack.push((sc, kind));
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some((s, k)) = stack.pop() {
                        elements.push(Element {
                            start_char: s,
                            end_char: ec,
                            kind: k,
                        });
                    }
                }
                Event::Start(Tag::BlockQuote(_)) => stack.push((sc, ElementKind::Blockquote)),
                Event::End(TagEnd::BlockQuote(_)) => {
                    if let Some((s, k)) = stack.pop() {
                        elements.push(Element {
                            start_char: s,
                            end_char: ec,
                            kind: k,
                        });
                    }
                }
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
    pub fn render<'a>(
        content: &'a str,
        logical_line: &'a str,
        visual_start_col: usize,
        cursor_col: Option<usize>,
        is_first_visual_line: bool,
        force_raw: bool,
        available_width: u16,
        theme: &Theme,
    ) -> Vec<Span<'a>> {
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
        parsed: &ParsedLine,
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

        let mut char_pos = 0usize;
        for cluster in logical_line.graphemes(true) {
            let pos = char_pos;
            char_pos += cluster.chars().count();
            if pos < visual_start_col {
                continue;
            }
            if pos >= visual_start_col + content_char_count {
                break;
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

/// Detects whether a line is an indented list item (leading spaces or tab,
/// followed by `-`/`*`/`+`/digit-dot + space). Used by `ParsedLine::parse`
/// to decide whether to feed pulldown-cmark a synthetic parent-list context
/// for single-line degenerate inputs.
fn needs_synthetic_list_parent(line: &str) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    if trimmed.len() == line.len() {
        return false; // no leading whitespace → nothing to compensate for
    }
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' '
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
        Some(ElementKind::InlineCode) => Style::default()
            .fg(theme.fg.to_ratatui())
            .bg(theme.bg_selected.to_ratatui()),
        Some(ElementKind::Link) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::UNDERLINED),
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
}
