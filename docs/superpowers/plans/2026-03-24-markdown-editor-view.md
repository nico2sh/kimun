# Markdown Editor View Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the raw `ratatui_textarea` render with a word-wrapping, markdown-rendering view layer that expands elements to raw text when the cursor is inside them.

**Architecture:** `TextEditorComponent` retains `TextArea` for input/cursor; a new `MarkdownEditorView` sits beside it owning layout and scroll state. `WordWrapLayout` (pure computation) wraps logical lines into `VisualLine`s. `MarkdownSpanner` (pure function, backed by `pulldown-cmark`) converts one visual line into styled ratatui `Span`s, expanding only the element the cursor is inside.

**Tech Stack:** Rust, ratatui 0.30, ratatui-textarea 0.8, pulldown-cmark 0.13

**Spec:** `docs/superpowers/specs/2026-03-24-markdown-editor-view-design.md`

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Add dep | `tui/Cargo.toml` | Add pulldown-cmark 0.13 |
| Rename | `src/components/text_editor.rs` → `src/components/text_editor/mod.rs` | TextEditorComponent (unchanged logic) |
| Create | `src/components/text_editor/word_wrap.rs` | `WordWrapLayout` + `VisualLine` — pure coordinate computation |
| Create | `src/components/text_editor/markdown.rs` | `MarkdownSpanner` — pulldown-cmark parsing + span emission |
| Create | `src/components/text_editor/view.rs` | `MarkdownEditorView` — scroll state, update(), render() |
| Modify | `src/components/text_editor/mod.rs` | Add `view: MarkdownEditorView` field, wire render + mouse |

---

## Chunk 1: Setup + WordWrapLayout

### Task 1: Add dependency + create module structure

**Files:**
- Modify: `tui/Cargo.toml`
- Rename: `src/components/text_editor.rs` → `src/components/text_editor/mod.rs`
- Create: `src/components/text_editor/word_wrap.rs` (empty)
- Create: `src/components/text_editor/markdown.rs` (empty)
- Create: `src/components/text_editor/view.rs` (empty)

- [ ] **Step 1: Add pulldown-cmark to Cargo.toml**

In `tui/Cargo.toml`, under `[dependencies]`, add:
```toml
pulldown-cmark = "0.13"
```

- [ ] **Step 2: Convert text_editor.rs to a module**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui
mkdir -p src/components/text_editor
cp src/components/text_editor.rs src/components/text_editor/mod.rs
rm src/components/text_editor.rs
```

- [ ] **Step 3: Create empty sibling files**

```bash
touch src/components/text_editor/word_wrap.rs
touch src/components/text_editor/markdown.rs
touch src/components/text_editor/view.rs
```

- [ ] **Step 4: Declare submodules in mod.rs**

At the top of `src/components/text_editor/mod.rs`, add:
```rust
pub mod markdown;
pub mod view;
pub mod word_wrap;
```

- [ ] **Step 5: Verify it compiles**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | head -20
```
Expected: compiles (empty modules are fine).

- [ ] **Step 6: Commit**

```bash
git add tui/Cargo.toml tui/Cargo.lock src/components/text_editor/
git commit -m "chore: convert text_editor to module, add pulldown-cmark"
```

---

### Task 2: WordWrapLayout — failing tests first

**Files:**
- Create: `src/components/text_editor/word_wrap.rs`

- [ ] **Step 1: Write the struct skeleton and tests**

Replace contents of `src/components/text_editor/word_wrap.rs` with:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct VisualLine {
    pub logical_row: usize,
    /// Character offset (Unicode scalar) where this visual line begins in the original line.
    pub start_col: usize,
    /// Character offset (exclusive) where this visual line ends.
    pub end_col: usize,
    pub content: String,
    pub is_first_visual_line: bool,
}

pub struct WordWrapLayout {
    visual_lines: Vec<VisualLine>,
}

impl WordWrapLayout {
    pub fn compute(_lines: &[String], _width: u16) -> Self {
        todo!()
    }

    pub fn total_visual_lines(&self) -> usize {
        self.visual_lines.len()
    }

    pub fn visual_lines(&self) -> &[VisualLine] {
        &self.visual_lines
    }

    /// Convert logical (row, col) to (visual_row, visual_col).
    pub fn logical_to_visual(&self, _row: usize, _col: usize) -> (usize, usize) {
        todo!()
    }

    /// Convert visual (vrow, vcol) to logical (row, col).
    pub fn visual_to_logical(&self, _vrow: usize, _vcol: usize) -> (usize, usize) {
        todo!()
    }
}

impl Default for WordWrapLayout {
    fn default() -> Self {
        Self {
            visual_lines: vec![VisualLine {
                logical_row: 0,
                start_col: 0,
                end_col: 0,
                content: String::new(),
                is_first_visual_line: true,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ls(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    #[test]
    fn empty_input_produces_one_visual_line() {
        let layout = WordWrapLayout::compute(&[], 40);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(layout.visual_lines()[0].logical_row, 0);
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn empty_string_produces_one_visual_line() {
        let layout = WordWrapLayout::compute(&[String::new()], 40);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(layout.visual_lines()[0].content, "");
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn short_line_fits_on_one_visual_line() {
        let layout = WordWrapLayout::compute(&ls("hello world"), 40);
        assert_eq!(layout.total_visual_lines(), 1);
        assert_eq!(layout.visual_lines()[0].content, "hello world");
        assert!(layout.visual_lines()[0].is_first_visual_line);
    }

    #[test]
    fn long_line_wraps_at_whitespace() {
        // "hello world foo" width=11 → "hello world" (11) fits; " foo" wraps
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].content, "hello world");
        assert_eq!(layout.visual_lines()[1].content, "foo");
        assert!(layout.visual_lines()[0].is_first_visual_line);
        assert!(!layout.visual_lines()[1].is_first_visual_line);
    }

    #[test]
    fn long_word_hard_breaks_at_width() {
        let layout = WordWrapLayout::compute(&["abcdefgh".to_string()], 4);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].content, "abcd");
        assert_eq!(layout.visual_lines()[1].content, "efgh");
    }

    #[test]
    fn two_logical_lines_have_correct_logical_rows() {
        let layout = WordWrapLayout::compute(&ls("abc\nxyz"), 10);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].logical_row, 0);
        assert_eq!(layout.visual_lines()[1].logical_row, 1);
    }

    #[test]
    fn unicode_chars_counted_not_bytes() {
        // "あいう" is 3 chars, 9 bytes. width=2 → hard break at 2 chars.
        let layout = WordWrapLayout::compute(&["あいう".to_string()], 2);
        assert_eq!(layout.total_visual_lines(), 2);
        assert_eq!(layout.visual_lines()[0].content, "あい");
        assert_eq!(layout.visual_lines()[1].content, "う");
    }

    #[test]
    fn logical_to_visual_start_of_line() {
        let layout = WordWrapLayout::compute(&ls("hello world"), 40);
        assert_eq!(layout.logical_to_visual(0, 0), (0, 0));
    }

    #[test]
    fn logical_to_visual_wrapped_cursor() {
        // "hello world foo" width=11 → vline0 ends at col 11, vline1 starts at col 12
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11);
        let (vrow, vcol) = layout.logical_to_visual(0, 12);
        assert_eq!(vrow, 1);
        assert_eq!(vcol, 0); // "foo" starts at col 12 in logical line
    }

    #[test]
    fn visual_to_logical_first_line() {
        let layout = WordWrapLayout::compute(&ls("hello"), 40);
        assert_eq!(layout.visual_to_logical(0, 3), (0, 3));
    }

    #[test]
    fn visual_to_logical_accounts_for_start_col() {
        // vline1.start_col = 12 (after "hello world ")
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11);
        let (row, col) = layout.visual_to_logical(1, 0);
        assert_eq!(row, 0);
        assert_eq!(col, 12);
    }

    #[test]
    fn coordinate_roundtrip_vrow_zero() {
        let layout = WordWrapLayout::compute(&ls("hello world foo"), 11);
        let (row, col) = layout.visual_to_logical(0, 3);
        let (vrow2, vcol2) = layout.logical_to_visual(row, col);
        assert_eq!((vrow2, vcol2), (0, 3));
    }
}
```

- [ ] **Step 2: Run tests — expect failures (todo! panics)**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo test -- word_wrap 2>&1 | tail -20
```
Expected: tests fail with `not yet implemented`.

---

### Task 3: Implement WordWrapLayout

**Files:**
- Modify: `src/components/text_editor/word_wrap.rs`

- [ ] **Step 1: Implement `compute`**

Replace `todo!()` in `compute` with:

```rust
pub fn compute(lines: &[String], width: u16) -> Self {
    let width = width as usize;
    let mut visual_lines = Vec::new();

    if lines.is_empty() {
        return Self::default();
    }

    for (row, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() || width == 0 {
            visual_lines.push(VisualLine {
                logical_row: row,
                start_col: 0,
                end_col: 0,
                content: String::new(),
                is_first_visual_line: true,
            });
            continue;
        }

        let total = chars.len();
        let mut start = 0;
        let mut is_first = true;

        while start < total {
            let remaining = total - start;
            if remaining <= width {
                visual_lines.push(VisualLine {
                    logical_row: row,
                    start_col: start,
                    end_col: total,
                    content: chars[start..total].iter().collect(),
                    is_first_visual_line: is_first,
                });
                break;
            }
            // Find break point: if char AT end is whitespace, break there;
            // otherwise scan backward for last whitespace in [start..end].
            let end = start + width;
            let (content_end, next_start) = if chars[end].is_whitespace() {
                (end, end + 1) // break before space, skip it
            } else {
                match chars[start..end]
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                {
                    Some((i, _)) => (start + i, start + i + 1), // break before space, skip it
                    None => (end, end), // hard break, no space found
                }
            };

            visual_lines.push(VisualLine {
                logical_row: row,
                start_col: start,
                end_col: content_end,
                content: chars[start..content_end].iter().collect(),
                is_first_visual_line: is_first,
            });
            start = next_start;
            is_first = false;
        }
    }

    Self { visual_lines }
}
```

- [ ] **Step 2: Implement `logical_to_visual`**

```rust
pub fn logical_to_visual(&self, row: usize, col: usize) -> (usize, usize) {
    // Find the last visual line for `row` whose start_col <= col.
    let vrow = self.visual_lines
        .iter()
        .enumerate()
        .filter(|(_, vl)| vl.logical_row == row && vl.start_col <= col)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    let vl = &self.visual_lines[vrow];
    (vrow, col.saturating_sub(vl.start_col))
}
```

- [ ] **Step 3: Implement `visual_to_logical`**

```rust
pub fn visual_to_logical(&self, vrow: usize, vcol: usize) -> (usize, usize) {
    let vrow = vrow.min(self.visual_lines.len().saturating_sub(1));
    let vl = &self.visual_lines[vrow];
    let col = (vl.start_col + vcol).min(vl.end_col);
    (vl.logical_row, col)
}
```

- [ ] **Step 4: Run tests — all must pass**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo test -- word_wrap 2>&1
```
Expected: all `word_wrap` tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/text_editor/word_wrap.rs
git commit -m "feat: add WordWrapLayout with coordinate transforms"
```

---

## Chunk 2: MarkdownSpanner

### Task 4: MarkdownSpanner — failing tests first

**Files:**
- Modify: `src/components/text_editor/markdown.rs`

- [ ] **Step 1: Write skeleton and tests**

Replace `src/components/text_editor/markdown.rs` with:

```rust
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use crate::settings::themes::Theme;

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub start_char: usize,
    pub end_char: usize,
    pub kind: ElementKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElementKind {
    Bold, Italic, InlineCode, Link,
    HeadingH1, HeadingH2, HeadingH3, Blockquote,
}

pub struct MarkdownSpanner;

impl MarkdownSpanner {
    pub fn render<'a>(
        content: &'a str,
        logical_line: &'a str,
        visual_start_col: usize,
        cursor_col: Option<usize>,
        is_first_visual_line: bool,
        force_raw: bool,
        available_width: u16,
        theme: &Theme,
    ) -> Vec<Span<'a>> { todo!() }

    pub fn parse_elements(line: &str) -> Vec<Element> { todo!() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;
    fn t() -> Theme { Theme::default() }
    fn text(spans: &[Span]) -> String { spans.iter().map(|s| s.content.as_ref()).collect() }

    #[test]
    fn parse_bold_range() {
        let e = MarkdownSpanner::parse_elements("**bold**");
        let b = e.iter().find(|x| x.kind == ElementKind::Bold).unwrap();
        assert_eq!((b.start_char, b.end_char), (0, 8));
    }
    #[test]
    fn parse_italic() {
        assert!(MarkdownSpanner::parse_elements("*hi*").iter().any(|e| e.kind == ElementKind::Italic));
    }
    #[test]
    fn parse_inline_code() {
        assert!(MarkdownSpanner::parse_elements("`x`").iter().any(|e| e.kind == ElementKind::InlineCode));
    }
    #[test]
    fn parse_link() {
        assert!(MarkdownSpanner::parse_elements("[t](u)").iter().any(|e| e.kind == ElementKind::Link));
    }
    #[test]
    fn parse_h1() {
        assert!(MarkdownSpanner::parse_elements("# T").iter().any(|e| e.kind == ElementKind::HeadingH1));
    }
    #[test]
    fn parse_h2() {
        assert!(MarkdownSpanner::parse_elements("## T").iter().any(|e| e.kind == ElementKind::HeadingH2));
    }
    #[test]
    fn parse_h3() {
        assert!(MarkdownSpanner::parse_elements("### T").iter().any(|e| e.kind == ElementKind::HeadingH3));
    }
    #[test]
    fn force_raw_no_styling() {
        let s = MarkdownSpanner::render("**x**","**x**",0,None,true,true,40,&t());
        assert_eq!(text(&s), "**x**");
        assert!(!s.iter().any(|sp| sp.style.add_modifier.contains(Modifier::BOLD)));
    }
    #[test]
    fn plain_text_passthrough() {
        let s = MarkdownSpanner::render("hi","hi",0,None,true,false,40,&t());
        assert_eq!(text(&s), "hi");
    }
    #[test]
    fn bold_without_cursor_hides_markers() {
        let s = MarkdownSpanner::render("**bold**","**bold**",0,None,true,false,40,&t());
        assert_eq!(text(&s), "bold");
        assert!(s.iter().any(|sp| sp.style.add_modifier.contains(Modifier::BOLD)));
    }
    #[test]
    fn bold_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("**bold**","**bold**",0,Some(3),true,false,40,&t());
        assert_eq!(text(&s), "**bold**");
    }
    #[test]
    fn bold_cursor_outside_stays_rendered() {
        let line = "hello **bold** world";
        let s = MarkdownSpanner::render(line,line,0,Some(1),true,false,40,&t());
        assert!(!text(&s).contains("**"));
    }
    #[test]
    fn italic_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("*hi*","*hi*",0,Some(1),true,false,40,&t());
        assert_eq!(text(&s), "*hi*");
    }
    #[test]
    fn inline_code_hides_backticks() {
        let s = MarkdownSpanner::render("`x`","`x`",0,None,true,false,40,&t());
        assert_eq!(text(&s), "x");
    }
    #[test]
    fn h1_first_line_contains_hash() {
        let s = MarkdownSpanner::render("# T","# T",0,None,true,false,40,&t());
        assert!(text(&s).contains('#'));
        assert!(text(&s).contains('T'));
    }
    #[test]
    fn continuation_line_no_hash() {
        let s = MarkdownSpanner::render("cont","# T cont",2,None,false,false,40,&t());
        assert!(!text(&s).contains('#'));
    }
}
```

- [ ] **Step 2: Run — expect failures**

```bash
cargo test -- markdown::tests 2>&1 | tail -5
```

---

### Task 5: Implement MarkdownSpanner

**Files:**
- Modify: `src/components/text_editor/markdown.rs`

- [ ] **Step 1: Implement `parse_elements`**

Replace its `todo!()`:

```rust
pub fn parse_elements(line: &str) -> Vec<Element> {
    let parser = Parser::new_ext(line, Options::ENABLE_STRIKETHROUGH);
    let mut elements = Vec::new();
    let mut stack: Vec<(usize, ElementKind)> = Vec::new();
    for (event, range) in parser.into_offset_iter() {
        let sc = line[..range.start].chars().count();
        let ec = line[..range.end].chars().count();
        match event {
            Event::Start(Tag::Strong) => stack.push((sc, ElementKind::Bold)),
            Event::End(TagEnd::Strong) => if let Some((s,k)) = stack.pop() {
                elements.push(Element { start_char: s, end_char: ec, kind: k });
            },
            Event::Start(Tag::Emphasis) => stack.push((sc, ElementKind::Italic)),
            Event::End(TagEnd::Emphasis) => if let Some((s,k)) = stack.pop() {
                elements.push(Element { start_char: s, end_char: ec, kind: k });
            },
            Event::Start(Tag::Link { .. }) => stack.push((sc, ElementKind::Link)),
            Event::End(TagEnd::Link) => if let Some((s,k)) = stack.pop() {
                elements.push(Element { start_char: s, end_char: ec, kind: k });
            },
            Event::Code(_) => elements.push(Element { start_char: sc, end_char: ec, kind: ElementKind::InlineCode }),
            Event::Start(Tag::Heading { level, .. }) => {
                let kind = match level {
                    HeadingLevel::H1 => ElementKind::HeadingH1,
                    HeadingLevel::H2 => ElementKind::HeadingH2,
                    _ => ElementKind::HeadingH3,
                };
                stack.push((sc, kind));
            }
            Event::End(TagEnd::Heading(_)) => if let Some((s,k)) = stack.pop() {
                elements.push(Element { start_char: s, end_char: ec, kind: k });
            },
            Event::Start(Tag::BlockQuote(_)) => stack.push((sc, ElementKind::Blockquote)),
            Event::End(TagEnd::BlockQuote) => if let Some((s,k)) = stack.pop() {
                elements.push(Element { start_char: s, end_char: ec, kind: k });
            },
            _ => {}
        }
    }
    elements
}
```

- [ ] **Step 2: Implement `render` and the `span_style` helper**

Add below the `impl` block:

```rust
fn span_style(kind: Option<ElementKind>, is_sigil_region: bool, theme: &Theme) -> Style {
    match kind {
        None => Style::default().fg(theme.fg.to_ratatui()),
        Some(ElementKind::Bold) =>
            Style::default().fg(theme.accent.to_ratatui()).add_modifier(Modifier::BOLD),
        Some(ElementKind::Italic) =>
            Style::default().fg(theme.fg_secondary.to_ratatui()).add_modifier(Modifier::ITALIC),
        Some(ElementKind::InlineCode) =>
            Style::default().fg(theme.fg.to_ratatui()).bg(theme.bg_selected.to_ratatui()),
        Some(ElementKind::Link) =>
            Style::default().fg(theme.accent.to_ratatui()).add_modifier(Modifier::UNDERLINED),
        Some(ElementKind::HeadingH1) => if is_sigil_region {
            Style::default().fg(theme.fg_muted.to_ratatui())
        } else {
            Style::default().fg(theme.accent.to_ratatui()).add_modifier(Modifier::BOLD)
        },
        Some(ElementKind::HeadingH2) =>
            Style::default().fg(theme.fg.to_ratatui()).add_modifier(Modifier::BOLD),
        Some(ElementKind::HeadingH3) =>
            Style::default().fg(theme.fg_secondary.to_ratatui()),
        Some(ElementKind::Blockquote) =>
            Style::default().fg(theme.fg_secondary.to_ratatui()),
    }
}
```

Replace `render`'s `todo!()`:

```rust
// HR
let trimmed = logical_line.trim();
if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
    if cursor_col.is_some() {
        return vec![Span::styled(content, Style::default().fg(theme.fg_muted.to_ratatui()))];
    }
    return vec![Span::styled(
        "─".repeat(available_width as usize),
        Style::default().fg(theme.fg_muted.to_ratatui()),
    )];
}
// Force-raw (inside fenced code block)
if force_raw {
    return vec![Span::styled(content, Style::default().fg(theme.fg_secondary.to_ratatui()))];
}

let elements = Self::parse_elements(logical_line);
let logical_chars: Vec<char> = logical_line.chars().collect();
let visual_end_col = visual_start_col + content.chars().count();

// Innermost element index at a logical char position
let elem_at = |pos: usize| -> Option<usize> {
    elements.iter().enumerate().rev()
        .find(|(_, e)| e.start_char <= pos && pos < e.end_char)
        .map(|(i, _)| i)
};
// Which element the cursor sits inside (for expand)
let expanded: Option<usize> = cursor_col.and_then(|c| elem_at(c));

// Walk visual region, grouping chars with same element into spans
let mut spans: Vec<Span<'a>> = Vec::new();
let mut seg_start = visual_start_col;
let mut seg_elem = elem_at(visual_start_col);

for pos in (visual_start_col + 1)..=visual_end_col {
    let next_elem = if pos < visual_end_col { elem_at(pos) } else { None };
    if pos == visual_end_col || next_elem != seg_elem {
        let seg: String = logical_chars[seg_start..pos].iter().collect();
        let is_exp = seg_elem.map_or(false, |i| expanded == Some(i));
        let style = if is_exp {
            Style::default().fg(theme.fg_muted.to_ratatui())
        } else {
            // Heading sigil = first chars of first visual line inside a heading element
            let is_sigil = is_first_visual_line && seg_start == visual_start_col
                && seg_elem.map_or(false, |i| matches!(elements[i].kind,
                    ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3));
            Self::span_style(seg_elem.map(|i| elements[i].kind), is_sigil, theme)
        };
        spans.push(Span::styled(seg, style));
        seg_start = pos;
        seg_elem = next_elem;
    }
}
if spans.is_empty() {
    spans.push(Span::styled(content, Style::default().fg(theme.fg.to_ratatui())));
}
spans
```

- [ ] **Step 3: Run tests — all must pass**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo test -- markdown::tests 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add src/components/text_editor/markdown.rs
git commit -m "feat: implement MarkdownSpanner with pulldown-cmark"
```

---

## Chunk 3: MarkdownEditorView

### Task 6: MarkdownEditorView — struct + update()

**Files:**
- Modify: `src/components/text_editor/view.rs`

- [ ] **Step 1: Write skeleton + scroll tests**

Replace `src/components/text_editor/view.rs` with:

```rust
use std::ops::Range;
use ratatui::Frame;
use ratatui::layout::Rect;
use crate::settings::themes::Theme;
use super::word_wrap::WordWrapLayout;

pub struct MarkdownEditorView {
    pub layout: WordWrapLayout,
    pub visual_scroll_offset: usize,
    pub lines_snapshot: Vec<String>,
    pub cursor_snapshot: (usize, usize),
    pub cursor_code_block: Option<Range<usize>>,
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        Self {
            layout: WordWrapLayout::default(),
            visual_scroll_offset: 0,
            lines_snapshot: Vec::new(),
            cursor_snapshot: (0, 0),
            cursor_code_block: None,
        }
    }

    pub fn update(&mut self, lines: &[String], cursor: (usize, usize), rect: Rect) {
        todo!()
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        todo!()
    }

    /// Convert mouse visual position (relative to rect, scroll-adjusted) to
    /// logical cursor position. Returns (u16, u16) for CursorMove::Jump.
    pub fn visual_to_logical_u16(&self, vrow: usize, vcol: usize) -> (u16, u16) {
        let (row, col) = self.layout.visual_to_logical(vrow, vcol);
        (row.min(u16::MAX as usize) as u16, col.min(u16::MAX as usize) as u16)
    }

    fn find_code_block(lines: &[String], cursor_row: usize) -> Option<Range<usize>> {
        todo!()
    }
}

impl Default for MarkdownEditorView {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn rect(h: u16) -> Rect { Rect { x: 0, y: 0, width: 40, height: h } }

    #[test]
    fn new_has_zero_scroll() {
        assert_eq!(MarkdownEditorView::new().visual_scroll_offset, 0);
    }

    #[test]
    fn zero_height_rect_does_not_panic() {
        let mut v = MarkdownEditorView::new();
        v.update(&["hello".to_string()], (0, 0), rect(0));
        // Should return early without panic
    }

    #[test]
    fn scroll_follows_cursor_down() {
        let mut v = MarkdownEditorView::new();
        // 5 single-word lines, each fits on one visual line, height=3
        let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
        v.update(&lines, (4, 0), rect(3)); // cursor on row 4
        // cursor_vrow = 4, scroll must be at least 4 - 3 + 1 = 2
        assert!(v.visual_scroll_offset >= 2);
    }

    #[test]
    fn scroll_follows_cursor_up() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..5).map(|i| format!("line{}", i)).collect();
        // First move cursor to bottom to push scroll down
        v.update(&lines, (4, 0), rect(3));
        // Now move cursor back to top
        v.update(&lines, (0, 0), rect(3));
        assert_eq!(v.visual_scroll_offset, 0);
    }

    #[test]
    fn visual_to_logical_u16_accounts_for_scroll() {
        let mut v = MarkdownEditorView::new();
        let lines: Vec<String> = (0..10).map(|i| format!("line{}", i)).collect();
        v.update(&lines, (5, 0), rect(3));
        let scroll = v.visual_scroll_offset;
        // Visual row 0 on screen = logical row `scroll`
        let (row, _col) = v.visual_to_logical_u16(scroll, 0);
        assert_eq!(row as usize, scroll);
    }

    #[test]
    fn code_block_detection_cursor_inside() {
        let lines = vec![
            "text".to_string(),
            "```rust".to_string(),
            "let x = 1;".to_string(),
            "```".to_string(),
            "more".to_string(),
        ];
        let block = MarkdownEditorView::find_code_block(&lines, 2);
        assert!(block.is_some());
        let r = block.unwrap();
        assert_eq!(r.start, 1);
        assert_eq!(r.end, 4); // exclusive end = line after closing fence
    }

    #[test]
    fn code_block_detection_cursor_outside() {
        let lines = vec![
            "text".to_string(),
            "```".to_string(),
            "code".to_string(),
            "```".to_string(),
        ];
        assert!(MarkdownEditorView::find_code_block(&lines, 0).is_none());
    }
}
```

- [ ] **Step 2: Run — expect failures**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo test -- view::tests 2>&1 | tail -5
```

- [ ] **Step 3: Implement `find_code_block` and `update`**

Replace their `todo!()`s:

```rust
fn find_code_block(lines: &[String], cursor_row: usize) -> Option<Range<usize>> {
    let mut open: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("```") {
            match open {
                None => open = Some(i),
                Some(start) => {
                    let range = start..i + 1;
                    if range.contains(&cursor_row) {
                        return Some(range);
                    }
                    open = None;
                }
            }
        }
    }
    None
}

pub fn update(&mut self, lines: &[String], cursor: (usize, usize), rect: Rect) {
    if rect.height == 0 { return; }
    self.lines_snapshot = lines.to_vec();
    self.cursor_snapshot = cursor;
    self.cursor_code_block = Self::find_code_block(lines, cursor.0);
    self.layout = WordWrapLayout::compute(lines, rect.width);

    let cursor_vrow = self.layout.logical_to_visual(cursor.0, cursor.1).0;
    let height = rect.height as usize;
    if cursor_vrow < self.visual_scroll_offset {
        self.visual_scroll_offset = cursor_vrow;
    } else if cursor_vrow >= self.visual_scroll_offset + height {
        self.visual_scroll_offset = cursor_vrow - height + 1;
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo test -- view::tests 2>&1
```
Expected: all view tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/components/text_editor/view.rs
git commit -m "feat: add MarkdownEditorView with scroll and code block tracking"
```

---

### Task 7: Implement `render` in MarkdownEditorView

**Files:**
- Modify: `src/components/text_editor/view.rs`

Add imports at the top of `view.rs`:
```rust
use ratatui::text::{Line, Text};
use ratatui::widgets::Paragraph;
use ratatui::layout::Position;
use super::markdown::MarkdownSpanner;
```

Replace `render`'s `todo!()`:

```rust
pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
    if rect.height == 0 { return; }
    let lines = &self.lines_snapshot;
    let cursor = self.cursor_snapshot;
    let scroll = self.visual_scroll_offset;
    let height = rect.height as usize;
    let vlines = self.layout.visual_lines();

    let visible: Vec<Line> = vlines
        .iter()
        .skip(scroll)
        .take(height)
        .map(|vl| {
            let cursor_col = if vl.logical_row == cursor.0 { Some(cursor.1) } else { None };
            let force_raw = self.cursor_code_block
                .as_ref()
                .map_or(false, |r| r.contains(&vl.logical_row));
            let logical_line = lines.get(vl.logical_row).map(|s| s.as_str()).unwrap_or("");
            let spans = MarkdownSpanner::render(
                &vl.content,
                logical_line,
                vl.start_col,
                cursor_col,
                vl.is_first_visual_line,
                force_raw,
                rect.width,
                theme,
            );
            Line::from(spans)
        })
        .collect();

    f.render_widget(
        Paragraph::new(Text::from(visible))
            .style(theme.base_style()),
        rect,
    );

    // Draw terminal cursor when focused
    if focused {
        let (cursor_vrow, visual_col) = self.layout.logical_to_visual(cursor.0, cursor.1);
        if cursor_vrow >= scroll && cursor_vrow < scroll + height {
            f.set_cursor_position(Position {
                x: rect.x + visual_col as u16,
                y: rect.y + (cursor_vrow - scroll) as u16,
            });
        }
    }
}
```

- [ ] **Run compile check**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo build 2>&1 | grep -E "^error"
```
Expected: no errors.

- [ ] **Commit**

```bash
git add src/components/text_editor/view.rs
git commit -m "feat: implement MarkdownEditorView::render with styled spans and cursor"
```

---

## Chunk 4: Wire into TextEditorComponent

### Task 8: Update TextEditorComponent

**Files:**
- Modify: `src/components/text_editor/mod.rs`

- [ ] **Step 1: Add `view` field**

Add import at the top of `mod.rs` (we're already inside the `text_editor` module, so use a relative path):
```rust
use super::view::MarkdownEditorView;
```

Add to `TextEditorComponent` struct:
```rust
view: MarkdownEditorView,
```

In `TextEditorComponent::new`, initialise it:
```rust
view: MarkdownEditorView::new(),
```

- [ ] **Step 2: Replace `render` body**

The current `render` in `mod.rs`:
```rust
fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
    self.rect = rect;
    self.text_area.set_cursor_style(...);
    self.text_area.set_selection_style(...);
    self.text_area.set_style(...);
    f.render_widget(&self.text_area, rect);
}
```

Replace with:
```rust
fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
    self.rect = rect;
    let lines: Vec<String> = self.text_area.lines().to_vec();
    let cursor = self.text_area.cursor();
    self.view.update(&lines, cursor, rect);
    self.view.render(f, rect, theme, focused);
}
```

The `text_area.set_cursor_style / set_style / set_selection_style` calls are removed — the view handles all rendering now.

- [ ] **Step 3: Update mouse handler**

In `handle_input`, find the `CursorMove::Jump(row, col)` call inside the `MouseEventKind::Down` branch:

```rust
MouseEventKind::Down(_) => {
    tx.send(AppEvent::FocusEditor).ok();
    let row = mouse.row - r.y;
    let col = mouse.column - r.x;
    self.text_area.move_cursor(CursorMove::Jump(row, col));
}
```

Replace with:
```rust
MouseEventKind::Down(_) => {
    tx.send(AppEvent::FocusEditor).ok();
    let vrow = (mouse.row - r.y) as usize + self.view.visual_scroll_offset;
    let vcol = (mouse.column - r.x) as usize;
    let (lrow, lcol) = self.view.visual_to_logical_u16(vrow, vcol);
    self.text_area.move_cursor(CursorMove::Jump(lrow, lcol));
}
```

- [ ] **Step 4: Build and run all tests**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo test 2>&1
```
Expected: all tests pass. Fix any compile errors.

- [ ] **Step 5: Commit**

```bash
git add src/components/text_editor/mod.rs
git commit -m "feat: wire MarkdownEditorView into TextEditorComponent"
```

---

### Task 9: Smoke test + cleanup

- [ ] **Step 1: Run the app and verify basic rendering**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo run -- 2>/dev/null
```
Open a note with markdown content. Verify:
- Lines wrap at the editor boundary
- `**bold**` renders as bold text (no `**` markers)
- `# Heading` renders with dimmed `#` sigil
- Moving cursor onto a bold word expands it to `**word**`
- Moving cursor into a fenced code block shows raw text for all lines in the block

- [ ] **Step 2: Run full test suite**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui && cargo test 2>&1
```
Expected: all tests pass.

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "feat: markdown editor view with word-wrap and element-level expand"
```

---
