//! Terminal-free cell arithmetic for the markdown editor: where a logical
//! column lands on screen once the sigils the caret has not revealed are
//! concealed, and the inverse. The renderer and the layout mask share these so
//! they cannot disagree. See ADR-0049.

use super::{ParsedLine, cluster_width_at};
use unicode_segmentation::UnicodeSegmentation;

/// Rendered screen column for `cursor_col`, treating that same column as
/// the caret — so the markdown element it lands in counts as revealed.
///
/// Correct for mapping the real caret. To map an arbitrary column (a
/// selection edge, a **replace preview** span boundary) use
/// [`rendered_col_with_reveal`] and pass the caret separately:
/// otherwise the mapper reveals whatever element the *boundary* touches,
/// counts that element's hidden sigils as drawn, and the highlight lands
/// right of the text it belongs to.
pub fn rendered_cursor_col_with(
    logical_line: &str,
    parsed: &ParsedLine,
    visual_start_col: usize,
    cursor_col: usize,
    is_first_visual_line: bool,
    force_raw: bool,
) -> usize {
    rendered_col_with_reveal(
        logical_line,
        parsed,
        visual_start_col,
        cursor_col,
        Some(cursor_col),
        is_first_visual_line,
        force_raw,
    )
}

/// Rendered screen column for `target_col`, with element reveal driven by
/// `reveal_col` — the row's real caret column, or `None` when the caret is
/// on another row.
///
/// The split matters because `render_runs` reveals only the element under
/// the *caret*. Any mapping that assumed the measured column was the caret
/// would disagree with what was actually drawn.
#[allow(clippy::too_many_arguments)]
pub fn rendered_col_with_reveal(
    logical_line: &str,
    parsed: &ParsedLine,
    visual_start_col: usize,
    cursor_col: usize,
    reveal_col: Option<usize>,
    is_first_visual_line: bool,
    force_raw: bool,
) -> usize {
    if force_raw {
        // Tab-aware: code is rendered with tabs expanded to TAB_STOP, so the
        // rendered cursor column must sum expanded widths, not char counts.
        let mut rendered = 0usize;
        let mut char_pos = 0usize;
        for cluster in logical_line.graphemes(true) {
            if char_pos >= cursor_col {
                break;
            }
            let pos = char_pos;
            char_pos += cluster.chars().count();
            if pos < visual_start_col {
                continue;
            }
            rendered += cluster_width_at(cluster, rendered);
        }
        return rendered;
    }
    let trimmed = logical_line.trim();
    if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
        return cursor_col.saturating_sub(visual_start_col);
    }

    let elements = &parsed.elements;
    let content_vis = &parsed.content_vis;
    let logical_char_count = logical_line.chars().count();

    // Reveal follows the caret, never the column being measured.
    let expanded: Option<usize> = reveal_col.and_then(|c| parsed.elem_at(c));
    // Must match `render_runs`, or the caret draws in the wrong cell on a
    // row that reveals in full.
    let reveals_whole_row = row_reveals_whole(logical_line, parsed, reveal_col, force_raw);
    // Ungated on the visual row, as in `render_runs`: a sigil region that
    // outruns the pane draws on its continuation row, so a column there
    // must measure it rather than collapsing to zero.
    let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
    let list_sigil_end: Option<usize> = parsed.list_sigil_end();
    let blockquote_sigil_end: Option<usize> = if is_first_visual_line {
        parsed.blockquote_sigil_end()
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
        // span — kept consistent with `render_runs`'s placeholder injection.
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
                rendered_col += img.placeholder_width;
            }
        }

        let is_content = pos < content_vis.len() && content_vis[pos];
        let in_heading_sigil = heading_sigil_end.is_some_and(|s_end| pos < s_end);
        let in_list_sigil = list_sigil_end.is_some_and(|s_end| pos < s_end);
        let in_blockquote_sigil = blockquote_sigil_end.is_some_and(|s_end| pos < s_end);
        let in_expanded_elem =
            expanded.is_some_and(|i| elements[i].start_char <= pos && pos < elements[i].end_char);
        let in_any_element = parsed.in_any_element(pos);
        let visible = is_content
            || in_heading_sigil
            || in_list_sigil
            || in_blockquote_sigil
            || in_expanded_elem
            || reveals_whole_row
            || !in_any_element;
        if visible {
            rendered_col += cluster_width_at(cluster, rendered_col);
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
    let mut visible = visible_positions_raw(logical_line, parsed, cursor_col, force_raw);
    if cursor_col.is_some() && draws_nothing(&visible) {
        visible.iter_mut().for_each(|v| *v = true);
    }
    visible
}

/// Whether a row's visibility mask would leave it entirely unpainted.
fn draws_nothing(visible: &[bool]) -> bool {
    !visible.is_empty() && visible.iter().all(|v| !v)
}

/// Whether the caret's own row reveals in full rather than element-wise.
///
/// **Reveal** is scoped to the element under the caret, and at end of line
/// there is no element under the caret — `elem_at` is half-open — so a row
/// that is *entirely* concealed markdown (`[](url)`, `**<br/>**`) reveals
/// nothing and draws nothing, while the caret sits on it. A row the caret
/// is on must never be invisible, so the whole row reveals instead.
///
/// A fact about the logical row, not about a visual slice of it, so both
/// the wrap mask and the renderer derive it from the same predicate and
/// cannot disagree. The empty-content fallback this replaces was keyed on
/// the *slice* — which is decided by the wrap that the mask feeds, and so
/// could never be stated as a hint.
pub(crate) fn row_reveals_whole(
    logical_line: &str,
    parsed: &ParsedLine,
    cursor_col: Option<usize>,
    force_raw: bool,
) -> bool {
    cursor_col.is_some()
        && draws_nothing(&visible_positions_raw(
            logical_line,
            parsed,
            cursor_col,
            force_raw,
        ))
}

fn visible_positions_raw(
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
    // Reveal the blockquote marker only while the cursor is on this line;
    // otherwise it stays hidden and the view draws the `│` gutter instead.
    let blockquote_sigil_end: Option<usize> = if cursor_col.is_some() {
        parsed.blockquote_sigil_end()
    } else {
        None
    };

    (0..total)
        .map(|pos| {
            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
            let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
            let in_blockquote_sigil = blockquote_sigil_end.is_some_and(|end| pos < end);
            let in_any_element = parsed.in_any_element(pos);
            let in_expanded = expanded.is_some_and(|i| {
                parsed.elements[i].start_char <= pos && pos < parsed.elements[i].end_char
            });
            is_content
                || in_heading_sigil
                || in_list_sigil
                || in_blockquote_sigil
                || in_expanded
                || !in_any_element
        })
        .collect()
}

/// Logical column for the cell at `rendered_col`, inverse of
/// [`rendered_col_with_reveal`].
///
/// `reveal_col` is the row's real caret column, or `None` when the caret is
/// on another row — the same value the render loop passes as `cursor_col`.
/// It has to be threaded here because `render_runs` reveals the element
/// under the caret, and a revealed element's sigils occupy cells: measuring
/// the row as if nothing were revealed put every column past the first
/// revealed sigil out by the width of what was wrongly skipped.
#[allow(clippy::too_many_arguments)]
pub fn rendered_col_to_logical_with(
    logical_line: &str,
    parsed: &ParsedLine,
    visual_start_col: usize,
    rendered_col: usize,
    reveal_col: Option<usize>,
    is_first_visual_line: bool,
    force_raw: bool,
) -> usize {
    if force_raw {
        // Tab-aware inverse of `rendered_cursor_col_with`'s force-raw branch:
        // walk expanded widths to find the logical char at `rendered_col`.
        let mut rendered = 0usize;
        let mut char_pos = 0usize;
        for cluster in logical_line.graphemes(true) {
            let pos = char_pos;
            if pos < visual_start_col {
                char_pos += cluster.chars().count();
                continue;
            }
            if rendered >= rendered_col {
                return pos;
            }
            rendered += cluster_width_at(cluster, rendered);
            char_pos += cluster.chars().count();
        }
        return char_pos;
    }
    let trimmed = logical_line.trim();
    if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
        return visual_start_col + rendered_col;
    }

    let content_vis = &parsed.content_vis;
    let logical_char_count = logical_line.chars().count();
    // Ungated on the visual row, as in `render_runs`: the inverse mapping
    // has to land inside a sigil region that outran the pane, not past it.
    let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
    let list_sigil_end: Option<usize> = parsed.list_sigil_end();
    // Reveal, exactly as `render_runs` applies it.
    let expanded: Option<usize> = reveal_col.and_then(|c| parsed.elem_at(c));
    let reveals_whole_row = row_reveals_whole(logical_line, parsed, reveal_col, force_raw);
    // Mirror `rendered_cursor_col_with`: on the first visual line a
    // blockquote's `> ` markers are revealed (visible) when the cursor is on
    // the row. On non-cursor rows the caller passes `visual_start_col` past
    // the markers (the gutter case), so this clause is inert there.
    let blockquote_sigil_end: Option<usize> = if is_first_visual_line {
        parsed.blockquote_sigil_end()
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

        // A click landing inside the placeholder region maps back to the
        // start of the image span (the only logical position that visually
        // corresponds to the placeholder).
        if let Some(img) = parsed
            .image_placeholders
            .iter()
            .find(|p| p.start_char == pos)
        {
            // Only when a placeholder is actually drawn. A revealed image —
            // the caret inside it, or the whole row revealing — shows its raw
            // markdown instead, and counting a placeholder that is not on
            // screen walks this mapping past every column after it.
            let drawn = !reveals_whole_row
                && !expanded.is_some_and(|i| {
                    parsed.elements[i].start_char == img.start_char
                        && parsed.elements[i].end_char == img.end_char
                });
            if drawn {
                if rendered_count + img.placeholder_width > rendered_col {
                    return pos;
                }
                rendered_count += img.placeholder_width;
            }
        }
        let is_content = pos < content_vis.len() && content_vis[pos];
        let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
        let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
        let in_blockquote_sigil = blockquote_sigil_end.is_some_and(|end| pos < end);
        let in_expanded_elem = expanded.is_some_and(|i| {
            parsed.elements[i].start_char <= pos && pos < parsed.elements[i].end_char
        });
        let in_any_element = parsed.in_any_element(pos);
        let drawn = is_content
            || in_heading_sigil
            || in_list_sigil
            || in_blockquote_sigil
            || in_expanded_elem
            || reveals_whole_row
            || !in_any_element;
        // An undrawn column belongs to the drawn column that follows it, so
        // the cell resolves past a concealed run rather than to its head.
        // `crate::ropetext::Layout::position_at_cell` steps the same way, and this
        // is what keeps `position_at_cell ∘ cell_of` monotone — the reason
        // to prefer it over "the first position at this cell boundary",
        // which lands the caret inside the markup that was concealed.
        if drawn {
            if rendered_count >= rendered_col {
                return pos;
            }
            rendered_count += cluster_width_at(cluster, rendered_count);
        }
    }
    logical_char_count
}
