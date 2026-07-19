//! Pure resolution of a retrieved chunk's location within its source note's
//! full text, for the Ask workspace's Source reader (CONTEXT.md: **Ask
//! workspace**, `SourcesPanel`). Three-step resolution, most confident
//! first: an exact substring match of the retrieved chunk text (first
//! occurrence wins on a duplicate); the `ContentChunk` core's own chunker
//! computes for the note (matched by innermost heading), located by
//! substring; and, only when that chunk's text can't be found verbatim in
//! the note (e.g. it was normalized server-side), core's
//! `note::scan::heading_section_range` — content analysis over raw note
//! text belongs in core, not the TUI.

use std::ops::Range;

use kimun_core::nfs::VaultPath;
use kimun_core::note::{NoteDetails, scan};

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
    let chunk = chunks.iter().find(|c| {
        c.breadcrumb_last()
            .is_some_and(|h| h.eq_ignore_ascii_case(heading))
    })?;

    if let Some(start) = note_text.find(&chunk.text) {
        return Some(start..start + chunk.text.len());
    }

    // Last resort: the chunk's own text isn't a verbatim substring either
    // (normalization — core's chunker strips diacritics, reformats lists,
    // etc.). Fall back to the heading line itself; this is core content
    // analysis, so it lives in `note::scan`, not here.
    scan::heading_section_range(note_text, heading)
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

    #[test]
    fn section_range_prefers_the_first_occurrence_on_a_duplicate_chunk_text() {
        // `str::find` returns the first match; document that a repeated
        // chunk body resolves to its earlier occurrence, not a later one.
        let note = "# a\nshared body\n# b\nshared body\n";
        let r = section_range(note, "b", "shared body").unwrap();
        assert_eq!(r, 4..15, "first occurrence, under heading a, wins");
    }
}
