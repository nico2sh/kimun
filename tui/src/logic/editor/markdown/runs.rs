//! Terminal-free semantic rendering of one visual line.
//!
//! [`render_runs`] walks a logical line exactly as the TUI spanner does and
//! returns [`Run`]s: owned text tagged with a semantic [`Role`] and the
//! emphasis mask. Mapping a role to colours is the painter's job (the TUI's
//! `role_style`), so this module knows nothing about any theme or toolkit.

use super::reveal::row_reveals_whole;
use super::{
    ElementKind, ParsedLine, blockquote_gutter, cluster_display_width, cluster_width_at,
    string_display_width, tab_width_at,
};
use crate::logic::editor::overlay::{OverlayKind, OverlaySet};
use unicode_segmentation::UnicodeSegmentation;

/// What a run of text *is*, as far as styling goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// No element, not a sigil region.
    Plain,
    /// Markers / hidden-syntax region.
    Sigil,
    /// An expanded element or a whole revealed row: raw source, no modifiers.
    Revealed,
    /// Horizontal rule drawn as `─` x width.
    Rule,
    /// Text inside a fenced block.
    CodeText,
    /// Trailing pad of a code box.
    CodeBoxPad,
    /// The `│` blockquote gutter.
    BlockquoteBar,
    /// Bold, Italic, Strikethrough, InlineCode, Link, Image, Blockquote,
    /// WikiLink, Label, and non-sigil headings.
    Element(ElementKind),
    /// The marker region of a heading (H1, H2 or H3).
    ElementSigil(ElementKind),
}

impl Role {
    /// The role for the spanner's `(kind, is_sigil_region)` key: the sigil
    /// flag matters only for no element and the three heading kinds.
    pub fn for_style(kind: Option<ElementKind>, sigil_region: bool) -> Role {
        match (kind, sigil_region) {
            (None, true) => Role::Sigil,
            (None, false) => Role::Plain,
            (
                Some(
                    k @ (ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3),
                ),
                true,
            ) => Role::ElementSigil(k),
            (Some(k), _) => Role::Element(k),
        }
    }
}

/// A piece of text with one role and one emphasis mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub role: Role,
    /// `MOD_BOLD | MOD_ITALIC | MOD_STRIKE`.
    pub mods: u8,
    /// Painted over the code-box background (see [`box_runs`]).
    pub code_box: bool,
    /// Overlay kinds covering this run (see [`mark_overlay`]).
    pub overlays: OverlaySet,
}

impl Run {
    pub fn new(text: impl Into<String>, role: Role, mods: u8) -> Run {
        Run {
            text: text.into(),
            role,
            mods,
            code_box: false,
            overlays: OverlaySet::default(),
        }
    }
}

/// Semantic runs for one visual line. See the module docs.
#[allow(clippy::too_many_arguments)]
pub fn render_runs(
    content: &str,
    logical_line: &str,
    parsed: &ParsedLine,
    visual_start_col: usize,
    cursor_col: Option<usize>,
    is_first_visual_line: bool,
    force_raw: bool,
    available_width: u16,
) -> Vec<Run> {
    // HR
    let trimmed = logical_line.trim();
    if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
        if cursor_col.is_some() {
            return vec![Run::new(content, Role::Sigil, 0)];
        }
        return vec![Run::new(
            "─".repeat(available_width as usize),
            Role::Rule,
            0,
        )];
    }
    // Force-raw (inside fenced code block). Expand tabs to spaces (at the
    // editor's TAB_STOP) so the rendered width is deterministic — matching
    // `raw_display_width` (used to size the code box) and the non-force-raw
    // tab handling, instead of emitting a literal tab whose width the
    // terminal decides.
    if force_raw {
        // `CodeText` paints like plain text, so fenced code matches
        // indented code, which renders through the plain-text path.
        if !content.contains('\t') {
            return vec![Run::new(content, Role::CodeText, 0)];
        }
        let mut expanded = String::with_capacity(content.len());
        let mut col = 0usize;
        for cluster in content.graphemes(true) {
            let w = cluster_width_at(cluster, col);
            if cluster == "\t" {
                for _ in 0..w {
                    expanded.push(' ');
                }
            } else {
                expanded.push_str(cluster);
            }
            col += w;
        }
        return vec![Run::new(expanded, Role::CodeText, 0)];
    }

    // Blockquote gutter: when the cursor is off this line, draw a `│` bar
    // per nesting depth (in `blockquote_bar`) in place of the hidden `>`
    // markers, on EVERY visual row. When the cursor IS on the line the
    // markers are revealed raw instead (handled by the sigil path below).
    let bq_gutter: Option<Vec<Run>> = if cursor_col.is_none() {
        parsed
            .blockquote_depth()
            .map(|d| vec![Run::new(blockquote_gutter(d), Role::BlockquoteBar, 0)])
    } else {
        None
    };

    let elements = &parsed.elements;
    let content_vis = &parsed.content_vis;
    let content_char_count = content.chars().count();

    let expanded: Option<usize> = cursor_col.and_then(|c| parsed.elem_at(c));
    // The caret's row is never left invisible: see `row_reveals_whole`.
    let reveals_whole_row = row_reveals_whole(logical_line, parsed, cursor_col, force_raw);

    // Ungated on the visual row, matching `visible_positions_with` — which
    // is the wrap mask, and so decides how many cells each row reserves. A
    // sigil region can outrun the pane (a setext underline, a heading whose
    // `#` run is the whole row), and the mask reserves those cells on the
    // continuation row. Gating here left that row with no spans at all.
    // Inert for an ordinary `# Title` / `- item`, whose `visual_start_col`
    // is already past the sigil on every row but the first.
    let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
    let list_sigil_end: Option<usize> = parsed.list_sigil_end();
    // Ungated too, and for the same reason — the reveal window is not the
    // `>` run: `blockquote_sigil_end` is the element's first content char,
    // which is the *whole row* when the quote holds no `Event::Text` (an
    // HTML block inside a quote), so it outruns the pane readily. The
    // cursor-vs-gutter distinction that does the real work here is
    // `bq_gutter.is_none()` at the emit site below, which makes this
    // predicate identical to the mask's `cursor_col.is_some()` gate.
    let blockquote_sigil_end: Option<usize> = parsed.blockquote_sigil_end();

    let mut runs: Vec<Run> = Vec::new();
    let mut seg_str: String = String::new();
    let mut seg_elem: Option<usize> = None;
    let mut seg_is_sigil = false;
    let mut seg_is_expanded = false;
    let mut seg_mods: u8 = 0;
    // Tracks the current rendered visual column for tab-stop calculation.
    let mut visual_col = 0usize;

    let flush = |seg_str: &mut String,
                 seg_elem: Option<usize>,
                 seg_is_sigil: bool,
                 seg_is_expanded: bool,
                 seg_mods: u8,
                 runs: &mut Vec<Run>| {
        if seg_str.is_empty() {
            return;
        }
        let seg = std::mem::take(seg_str);
        if seg_is_expanded {
            runs.push(Run::new(seg, Role::Revealed, 0));
        } else {
            // The emphasis mask of outer elements rides along so a Link /
            // WikiLink nested in `**…**` / `*…*` keeps the bold/italic the
            // innermost-element role alone would drop.
            let role = Role::for_style(seg_elem.map(|i| elements[i].kind), seg_is_sigil);
            runs.push(Run::new(seg, role, seg_mods));
        }
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
            let cursor_in_image = reveals_whole_row
                || expanded.is_some_and(|i| {
                    elements[i].start_char == img.start_char && elements[i].end_char == img.end_char
                });
            if !cursor_in_image {
                flush(
                    &mut seg_str,
                    seg_elem,
                    seg_is_sigil,
                    seg_is_expanded,
                    seg_mods,
                    &mut runs,
                );
                visual_col += img.placeholder_width;
                runs.push(Run::new(
                    img.placeholder.as_str(),
                    Role::Element(ElementKind::Image),
                    0,
                ));
                seg_elem = None;
                seg_is_sigil = false;
                seg_is_expanded = false;
                seg_mods = 0;
            }
        }

        let is_content = pos < content_vis.len() && content_vis[pos];
        let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
        let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
        // Only reveal the raw `> ` markers when there is no gutter, i.e.
        // when the cursor is on this line.
        let in_blockquote_sigil =
            bq_gutter.is_none() && blockquote_sigil_end.is_some_and(|end| pos < end);
        let in_expanded_elem =
            expanded.is_some_and(|i| elements[i].start_char <= pos && pos < elements[i].end_char);
        let this_elem = parsed.elem_at(pos);
        let emit = is_content
            || in_heading_sigil
            || in_list_sigil
            || in_blockquote_sigil
            || in_expanded_elem
            || reveals_whole_row
            || this_elem.is_none();
        if !emit {
            flush(
                &mut seg_str,
                seg_elem,
                seg_is_sigil,
                seg_is_expanded,
                seg_mods,
                &mut runs,
            );
            seg_elem = None;
            seg_is_sigil = false;
            seg_is_expanded = false;
            seg_mods = 0;
            continue;
        }
        // A row revealing in full is styled as an expanded element would
        // be — muted, so it reads as raw source under the caret rather than
        // as a live link the reader could follow.
        let this_is_expanded = in_expanded_elem || reveals_whole_row;
        let this_is_sigil = (in_heading_sigil || in_list_sigil || in_blockquote_sigil)
            && !is_content
            && !this_is_expanded;
        let this_mods = parsed.modifiers_at(pos);
        if this_elem != seg_elem
            || this_is_sigil != seg_is_sigil
            || this_is_expanded != seg_is_expanded
            || this_mods != seg_mods
        {
            flush(
                &mut seg_str,
                seg_elem,
                seg_is_sigil,
                seg_is_expanded,
                seg_mods,
                &mut runs,
            );
            seg_elem = this_elem;
            seg_is_sigil = this_is_sigil;
            seg_is_expanded = this_is_expanded;
            seg_mods = this_mods;
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
        seg_mods,
        &mut runs,
    );

    // Prepend the blockquote bar gutter (cursor-off-line case).
    if let Some(mut gutter) = bq_gutter {
        gutter.extend(runs);
        runs = gutter;
    }
    runs
}

/// Mark the code-box background on every run of a code-block visual line and
/// pad the line with a [`Role::CodeBoxPad`] run up to `box_width` display
/// columns, producing a solid rectangle hugging the block's widest line.
/// Content already wider than the box (the box was capped at editor width;
/// wider rows wrap) is left as-is.
///
/// Measures with the same cluster + tab-aware model as `raw_display_width`
/// (which sizes `box_width`), so the padding can never disagree with the
/// target on emoji presentation sequences or tabs.
pub fn box_runs(runs: Vec<Run>, box_width: u16) -> Vec<Run> {
    let mut width = 0usize;
    let mut out: Vec<Run> = runs
        .into_iter()
        .map(|mut r| {
            for g in r.text.graphemes(true) {
                width += cluster_width_at(g, width);
            }
            r.code_box = true;
            r
        })
        .collect();
    let target = box_width as usize;
    if width < target {
        let mut pad = Run::new(" ".repeat(target - width), Role::CodeBoxPad, 0);
        pad.code_box = true;
        out.push(pad);
    }
    out
}

/// Byte offset into `s` after consuming exactly `target_width` display
/// columns, or `s.len()` if `target_width` exceeds its width. Walks whole
/// grapheme clusters, measured with [`cluster_display_width`], so the result
/// never lands mid-cluster and agrees with wrap and cursor math.
fn byte_offset_for_display_width(s: &str, target_width: usize) -> usize {
    let mut consumed = 0usize;
    for (byte_pos, g) in s.grapheme_indices(true) {
        if consumed >= target_width {
            return byte_pos;
        }
        consumed += cluster_display_width(g);
    }
    s.len()
}

/// Split `runs` at the boundaries of a rendered-column range and add `kind` to
/// the overlapping portion. The one place column-to-byte accounting for a
/// partial overlay lives. Columns are plain cluster display widths (not
/// tab-stop aware), matching the rendered-column mapping the caller uses.
pub fn mark_overlay(runs: Vec<Run>, cols: std::ops::Range<usize>, kind: OverlayKind) -> Vec<Run> {
    if cols.is_empty() {
        return runs;
    }
    let mut result = Vec::new();
    let mut col = 0usize;

    for run in runs {
        let content: &str = &run.text;
        let run_width = string_display_width(content);
        let run_end = col + run_width;

        let overlap_start = cols.start.max(col);
        let overlap_end = cols.end.min(run_end);

        if overlap_start >= overlap_end {
            result.push(run);
        } else {
            let prefix_width = overlap_start - col;
            let marked_width = overlap_end - overlap_start;

            let prefix_byte = byte_offset_for_display_width(content, prefix_width);
            let marked_end =
                byte_offset_for_display_width(&content[prefix_byte..], marked_width) + prefix_byte;

            let piece = |text: &str| Run {
                text: text.to_string(),
                role: run.role,
                mods: run.mods,
                code_box: run.code_box,
                overlays: run.overlays,
            };
            if prefix_byte > 0 {
                result.push(piece(&content[..prefix_byte]));
            }
            let mut mid = piece(&content[prefix_byte..marked_end]);
            mid.overlays.insert(kind);
            result.push(mid);
            if marked_end < content.len() {
                result.push(piece(&content[marked_end..]));
            }
        }

        col = run_end;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::editor::markdown::{MOD_BOLD, MOD_ITALIC, MOD_STRIKE, TAB_STOP};
    use crate::logic::editor::overlay::{OverlayKind, OverlaySet};

    fn runs(line: &str, cursor: Option<usize>, force_raw: bool, width: u16) -> Vec<Run> {
        let parsed = ParsedLine::parse(line);
        render_runs(line, line, &parsed, 0, cursor, true, force_raw, width)
    }
    fn joined(rs: &[Run]) -> String {
        rs.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn revealed_segments_carry_no_modifiers() {
        let rs = runs("**bold**", Some(3), false, 40);
        assert_eq!(joined(&rs), "**bold**");
        let revealed: Vec<_> = rs.iter().filter(|r| r.role == Role::Revealed).collect();
        assert!(!revealed.is_empty());
        assert!(revealed.iter().all(|r| r.mods == 0), "{rs:?}");
    }

    #[test]
    fn hr_rule_is_as_wide_as_the_width_given() {
        let rs = runs("---", None, false, 12);
        assert_eq!(rs, vec![Run::new("─".repeat(12), Role::Rule, 0)]);
    }

    #[test]
    fn hr_row_under_the_caret_shows_source() {
        let rs = runs("---", Some(1), false, 12);
        assert_eq!(rs, vec![Run::new("---", Role::Sigil, 0)]);
    }

    #[test]
    fn force_raw_is_code_text_with_tabs_expanded() {
        let rs = runs("\tx", None, true, 40);
        assert_eq!(
            rs,
            vec![Run::new(
                format!("{}x", " ".repeat(TAB_STOP)),
                Role::CodeText,
                0
            )]
        );
        let plain = runs("a b", None, true, 40);
        assert_eq!(plain, vec![Run::new("a b", Role::CodeText, 0)]);
    }

    #[test]
    fn bold_text_is_element_bold_with_bold_mask() {
        let rs = runs("**bold**", None, false, 40);
        assert_eq!(joined(&rs), "bold");
        assert!(
            rs.iter().any(|r| r.role == Role::Element(ElementKind::Bold)
                && r.mods & MOD_BOLD != 0
                && r.text == "bold"),
            "{rs:?}"
        );
    }

    #[test]
    fn strikethrough_text_carries_strike_mask() {
        let rs = runs("~~gone~~", None, false, 40);
        assert_eq!(joined(&rs), "gone");
        assert!(rs.iter().any(|r| r.mods & MOD_STRIKE != 0));
    }

    #[test]
    fn italic_link_keeps_italic_mask_on_link_role() {
        let rs = runs("*[text](http://x)*", None, false, 40);
        assert_eq!(joined(&rs), "text");
        assert!(
            rs.iter().any(|r| r.role == Role::Element(ElementKind::Link)
                && r.mods & MOD_ITALIC != 0
                && r.text.contains("text")),
            "{rs:?}"
        );
    }

    #[test]
    fn blockquote_gutter_is_the_bar_role() {
        let rs = runs("> quoted", None, false, 40);
        assert_eq!(
            rs.first(),
            Some(&Run::new(blockquote_gutter(1), Role::BlockquoteBar, 0))
        );
    }

    #[test]
    fn image_placeholder_is_element_image_without_mods() {
        let rs = runs("a ![alt](pic.gif) b", None, false, 40);
        assert!(
            rs.iter()
                .any(|r| r.role == Role::Element(ElementKind::Image) && r.mods == 0),
            "{rs:?}"
        );
    }

    #[test]
    fn heading_marker_is_a_sigil_role() {
        let rs = runs("# Title", None, false, 40);
        assert!(
            rs.iter()
                .any(|r| r.role == Role::ElementSigil(ElementKind::HeadingH1)),
            "{rs:?}"
        );
        assert!(
            rs.iter()
                .any(|r| r.role == Role::Element(ElementKind::HeadingH1) && r.text == "Title")
        );
    }

    #[test]
    fn for_style_is_injective_on_distinct_styles() {
        use ElementKind::*;
        let all = [
            Bold,
            Italic,
            Strikethrough,
            InlineCode,
            Link,
            Image,
            HeadingH1,
            HeadingH2,
            HeadingH3,
            Blockquote,
            WikiLink,
            Label,
        ];
        let mut seen = std::collections::HashSet::new();
        assert!(seen.insert(Role::for_style(None, false)));
        assert!(seen.insert(Role::for_style(None, true)));
        for k in all {
            let plain = Role::for_style(Some(k), false);
            let sig = Role::for_style(Some(k), true);
            assert!(seen.insert(plain), "{k:?}");
            if matches!(k, HeadingH1 | HeadingH2 | HeadingH3) {
                assert_ne!(plain, sig);
                assert!(seen.insert(sig), "{k:?}");
            } else {
                assert_eq!(plain, sig, "{k:?} ignores the sigil flag");
            }
        }
        assert_eq!(seen.len(), 2 + 12 + 3);
    }

    // ---- overlay / code-box passes -------------------------------------

    fn r(text: &str) -> Run {
        Run::new(text, Role::Plain, 0)
    }

    #[test]
    fn mark_overlay_splits_runs_at_the_column_boundaries() {
        let out = mark_overlay(vec![r("hello "), r("world")], 3..8, OverlayKind::Match);
        let texts: Vec<_> = out.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, vec!["hel", "lo ", "wo", "rld"]);
        let marked: Vec<_> = out
            .iter()
            .map(|x| x.overlays.contains(OverlayKind::Match))
            .collect();
        assert_eq!(marked, vec![false, true, true, false]);
    }

    #[test]
    fn mark_overlay_counts_wide_clusters_as_two_cells() {
        let out = mark_overlay(vec![r("a😀b")], 1..3, OverlayKind::Selection);
        let marked: String = out
            .iter()
            .filter(|x| x.overlays.contains(OverlayKind::Selection))
            .map(|x| x.text.as_str())
            .collect();
        assert_eq!(marked, "😀");
        assert_eq!(joined(&out), "a😀b");
    }

    #[test]
    fn mark_overlay_respects_emoji_cluster_width() {
        // "a❤️b": ❤️ (U+2764 + VS16) is 2 columns. Marking 1..3 covers exactly
        // the heart, never splitting the cluster nor bleeding into 'b'.
        let heart = "\u{2764}\u{FE0F}";
        let out = mark_overlay(vec![r(&format!("a{heart}b"))], 1..3, OverlayKind::Selection);
        let marked: String = out
            .iter()
            .filter(|x| x.overlays.contains(OverlayKind::Selection))
            .map(|x| x.text.as_str())
            .collect();
        assert_eq!(marked, heart);
        for x in &out {
            assert!(
                !x.text.contains('\u{2764}') || x.text.contains(heart),
                "cluster split: {:?}",
                x.text
            );
        }
    }

    #[test]
    fn mark_overlay_stacks_kinds_on_one_run() {
        let a = mark_overlay(vec![r("abc")], 0..3, OverlayKind::Selection);
        let b = mark_overlay(a, 0..3, OverlayKind::TaskBox);
        assert_eq!(b.len(), 1);
        assert_eq!(
            b[0].overlays.iter().collect::<Vec<_>>(),
            vec![OverlayKind::TaskBox, OverlayKind::Selection]
        );
    }

    #[test]
    fn mark_overlay_empty_range_is_a_no_op() {
        let runs = vec![r("abc")];
        assert_eq!(mark_overlay(runs.clone(), 2..2, OverlayKind::Match), runs);
    }

    #[test]
    fn box_runs_pads_to_width_with_a_background_only_pad_run() {
        let out = box_runs(vec![r("ab")], 6);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|x| x.code_box));
        assert_eq!(out[0].text, "ab");
        assert_eq!(out[1].text, "    ");
        assert_eq!(out[1].role, Role::CodeBoxPad);
        assert_eq!(out[1].mods, 0);
        assert_eq!(out[1].overlays, OverlaySet::default());
    }

    #[test]
    fn box_runs_measures_emoji_clusters_at_full_width() {
        // "a❤️" is 3 columns; a box of 3 needs no pad run.
        let content = "a\u{2764}\u{FE0F}";
        assert_eq!(
            crate::logic::editor::markdown::raw_display_width(content),
            3
        );
        let out = box_runs(vec![r(content)], 3);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, content);
        assert!(out[0].code_box);
    }

    #[test]
    fn box_runs_is_tab_stop_aware() {
        // A literal tab at column 1 measures to the next stop, not 1 cell.
        let tab = tab_width_at(1);
        let out = box_runs(vec![r("a\t")], (1 + tab + 2) as u16);
        assert_eq!(out.last().map(|x| x.text.len()), Some(2));
    }

    #[test]
    fn box_runs_leaves_wider_content_unpadded() {
        let out = box_runs(vec![r("abcdef")], 3);
        assert_eq!(out.len(), 1);
        assert!(out[0].code_box);
    }
}
