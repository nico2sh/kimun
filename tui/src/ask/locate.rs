//! Pure resolution of a retrieved chunk's location within its source note's
//! full text, for the Ask workspace's Source reader (CONTEXT.md: **Ask
//! workspace**, `SourcesPanel`). Three-step resolution, most confident
//! first: an exact substring match of the retrieved chunk text; the
//! `ContentChunk` core's own chunker computes for the note (matched by
//! innermost heading), located by substring; and, only when that chunk's
//! text can't be found verbatim in the note (e.g. it was normalized
//! server-side), the heading line itself, highlighted through to the next
//! heading or the note's end.

use std::ops::Range;

use kimun_core::nfs::VaultPath;
use kimun_core::note::NoteDetails;

/// Locates the byte range of the section identified by `heading`/
/// `chunk_text` within `note_text`. `None` when nothing resolves — the
/// reader then shows the note from the top, unhighlighted. Pure function: no
/// I/O, no vault access.
pub fn section_range(note_text: &str, heading: &str, chunk_text: &str) -> Option<Range<usize>> {
    if !chunk_text.is_empty()
        && let Some(start) = note_text.find(chunk_text)
    {
        return Some(start..start + chunk_text.len());
    }

    // Recompute the note's own chunks (core's chunker, not the server's) and
    // find the one whose innermost heading matches.
    let (chunks, _links) = NoteDetails::chunks_and_links_of(&VaultPath::root(), note_text);
    let chunk = chunks
        .iter()
        .find(|c| c.breadcrumb_last().is_some_and(|h| h.eq_ignore_ascii_case(heading)))?;

    if let Some(start) = note_text.find(&chunk.text) {
        return Some(start..start + chunk.text.len());
    }

    // Last resort: the chunk's own text isn't a verbatim substring either
    // (normalization). Fall back to the heading line, highlighted through to
    // the next heading (any level) or the note's end.
    heading_line_range(note_text, heading)
}

/// Locates the Markdown heading line whose text equals `heading`
/// case-insensitively, and returns the range from that line's start through
/// to the start of the next heading line, or the note's end.
fn heading_line_range(note_text: &str, heading: &str) -> Option<Range<usize>> {
    let mut offset = 0usize;
    let mut start = None;
    for line in note_text.split_inclusive('\n') {
        let stripped = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = stripped.trim_start();
        if start.is_none() {
            if trimmed.starts_with('#') {
                let text = trimmed.trim_start_matches('#').trim();
                if text.eq_ignore_ascii_case(heading) {
                    start = Some(offset);
                }
            }
        } else if trimmed.starts_with('#') {
            return start.map(|s| s..offset);
        }
        offset += line.len();
    }
    start.map(|s| s..note_text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_range_prefers_exact_chunk_text() {
        let note = "# a\nalpha body\n# b\nbeta body\n";
        let r = section_range(note, "b", "beta body").unwrap();
        assert_eq!(&note[r], "beta body");
    }

    #[test]
    fn section_range_falls_back_to_heading_match() {
        let note = "# intro\nreal text here\n";
        // chunk text was normalized server-side and no longer matches verbatim
        let r = section_range(note, "INTRO", "normalized text").unwrap();
        assert!(note[r].contains("real text here"));
    }

    #[test]
    fn section_range_gives_up_gracefully() {
        assert!(section_range("# x\nbody\n", "missing", "nope").is_none());
    }

    #[test]
    fn section_range_matches_via_chunk_when_exact_text_absent_but_chunk_matches() {
        // Two headings; chunk_text doesn't match verbatim anywhere, but the
        // heading resolves via core's own chunker (whose text, being derived
        // straight from the note, is itself found verbatim).
        let note = "# one\nfirst body\n# two\nsecond body\n";
        let r = section_range(note, "two", "does not appear literally").unwrap();
        assert!(note[r].contains("second body"));
    }

    // `heading_line_range` (the true last-resort fallback, reached only when
    // even the chunk's own recomputed text isn't a verbatim substring —
    // e.g. Markdown escapes core's chunker unescapes) is exercised directly
    // here rather than through `section_range`, since forcing that exact
    // scenario end-to-end requires cooking up Markdown whose extracted chunk
    // text diverges from its source syntax.
    #[test]
    fn heading_line_range_stops_at_the_next_heading() {
        let note = "# a\nfirst\n# b\nsecond\n# c\nthird\n";
        let r = heading_line_range(note, "b").unwrap();
        assert_eq!(&note[r], "# b\nsecond\n");
    }

    #[test]
    fn heading_line_range_runs_to_note_end_when_last() {
        let note = "# a\nfirst\n# b\nsecond\n";
        let r = heading_line_range(note, "b").unwrap();
        assert_eq!(&note[r], "# b\nsecond\n");
    }

    #[test]
    fn heading_line_range_none_when_heading_absent() {
        assert!(heading_line_range("# a\nfirst\n", "missing").is_none());
    }
}
