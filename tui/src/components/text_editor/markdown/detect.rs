//! Per-line detectors for non-pulldown markdown extensions: wikilinks
//! (`[[name]]`) and image placeholders (`![alt](url)`).
//!
//! Each detector mutates the per-line `content_vis` mask (hiding sigil
//! chars from the renderer) and pushes into the row's `elements`
//! vector for styling. Image detection also returns a list of
//! [`ImagePlaceholder`]s with pre-computed render widths.

use super::{Element, ElementKind, ImagePlaceholder, string_display_width};

/// Detects `[[name]]` wikilink spans on `line`, marks the `[[` / `]]`
/// sigils as non-content, and pushes a `WikiLink` element for each
/// span. Skips spans that fall entirely inside an existing element
/// (e.g. `[[icon]]` inside a markdown link's display text).
pub(super) fn detect_wikilinks(line: &str, content_vis: &mut [bool], elements: &mut Vec<Element>) {
    for span in kimun_core::note::wikilink_char_spans(line) {
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
pub(super) fn detect_image_placeholders(
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
