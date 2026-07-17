//! Turning an ask [`Turn`](super::Turn) into a saved vault note
//! (CONTEXT.md: **Saved answer**). The question becomes the note title, its
//! citation markers become wikilinks so the note joins the vault link graph.

use kimun_core::nfs::filename::note_name_from_title;
use kimun_core::nfs::VaultPath;

use super::{citations, Turn};

/// The default path offered when saving `question` as a note: `ask/<slug>`,
/// with no extension (the create-note flow applies it, same as any other new
/// note).
pub fn suggested_path(question: &str) -> VaultPath {
    VaultPath::new("ask").append(&VaultPath::new(note_name_from_title(question)))
}

/// The clean source names addressed by **citation number**: `names[n - 1]` is
/// the clean name of the source whose ordinal is `n`, so `link_sources` can
/// rewrite `[n]` → `[[name]]` by the same explicit pairing the rest of Ask
/// uses. The vec is sized to the largest ordinal; a citation number with no
/// backing source (a gap) gets an empty-string sentinel `link_sources` treats
/// as out-of-range, leaving that `[n]` untouched. This is the ONE place the
/// ordinal→name mapping is built for saving — never `sources[n - 1]`.
pub fn citation_names(turn: &Turn) -> Vec<String> {
    let max = turn.sources.iter().map(|s| s.ordinal).max().unwrap_or(0);
    let mut names = vec![String::new(); max];
    for s in &turn.sources {
        if (1..=max).contains(&s.ordinal) {
            names[s.ordinal - 1] = s.path.get_clean_name();
        }
    }
    names
}

/// Renders `turn` as note content: the question as an `# ` title, the answer
/// with citation markers converted to `[[source]]` wikilinks, and a
/// `## Sources` footer.
///
/// The footer lists every one of `turn.sources` (deduped by clean name, in
/// source/rank order), not just the ones the answer actually cited: a source
/// the model retrieved but under-cited was still part of the turn's evidence,
/// and provenance must not be silently dropped (CONTEXT.md: **Saved
/// answer** — "backlinks from the sources find it").
pub fn note_content(turn: &Turn) -> String {
    // Citations resolve by ordinal (the pairing contract); the footer lists the
    // sources in vec/rank order.
    let linked = citations::link_sources(&turn.answer, &citation_names(turn));

    let mut seen = Vec::new();
    for s in &turn.sources {
        let name = s.path.get_clean_name();
        if !seen.contains(&name) {
            seen.push(name);
        }
    }

    let mut out = format!("# {}\n\n{}\n\n## Sources\n", turn.question, linked);
    for name in &seen {
        out.push_str(&format!("- [[{name}]]\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::{AskSource, TurnStatus};

    fn turn_with(question: &str, answer: &str, sources: Vec<AskSource>) -> Turn {
        // Mirror `AskSource::from_chunk`'s fallback: a fixture source left at
        // ordinal 0 gets its 1-based vec position, so the common case reads as
        // the old position convention while ordinal-explicit tests stay exact.
        let sources = sources
            .into_iter()
            .enumerate()
            .map(|(i, mut s)| {
                if s.ordinal == 0 {
                    s.ordinal = i + 1;
                }
                s
            })
            .collect();
        Turn {
            id: 0,
            question: question.to_string(),
            answer: answer.to_string(),
            sources,
            status: TurnStatus::Done,
        }
    }

    fn source(path: &str, heading: &str) -> AskSource {
        source_ord(path, heading, 0)
    }

    /// Build a source pinned to an explicit citation `ordinal` — for the pairing
    /// tests that need ordinals out of vec order or with gaps.
    fn source_ord(path: &str, heading: &str, ordinal: usize) -> AskSource {
        AskSource {
            path: VaultPath::new(path),
            heading: heading.to_string(),
            score: 1.0,
            text: String::new(),
            ordinal,
        }
    }

    #[test]
    fn note_content_links_citations_and_lists_sources() {
        let turn = turn_with(
            "Why kimün?",
            "Because notes [1]. And general knowledge.",
            vec![source("projects/kimun.md", "intro")],
        );
        let body = note_content(&turn);
        assert!(body.starts_with("# Why kimün?\n"));
        assert!(body.contains("Because notes [[kimun]]."));
        assert!(body.contains("## Sources"));
        assert!(body.contains("- [[kimun]]"));
    }

    #[test]
    fn note_content_lists_distinct_sources_in_first_seen_order() {
        let turn = turn_with(
            "q",
            "a [1] b [2] c [1]",
            vec![
                source("projects/alpha.md", "h1"),
                source("projects/beta.md", "h2"),
            ],
        );
        let body = note_content(&turn);
        let sources_section = body.split("## Sources").nth(1).unwrap();
        let alpha_pos = sources_section.find("[[alpha]]").unwrap();
        let beta_pos = sources_section.find("[[beta]]").unwrap();
        assert!(alpha_pos < beta_pos);
        assert_eq!(sources_section.matches("[[alpha]]").count(), 1);
    }

    #[test]
    fn note_content_lists_uncited_sources_too() {
        let turn = turn_with(
            "q",
            "only cites the first source [1]",
            vec![
                source("projects/alpha.md", "h1"),
                source("projects/beta.md", "h2"),
            ],
        );
        let body = note_content(&turn);
        let sources_section = body.split("## Sources").nth(1).unwrap();
        assert!(sources_section.contains("[[alpha]]"));
        assert!(
            sources_section.contains("[[beta]]"),
            "an uncited source must still appear in the footer: {sources_section:?}"
        );
    }

    #[test]
    fn note_content_dedupes_sources_sharing_a_clean_name() {
        let turn = turn_with(
            "q",
            "cites nothing in particular",
            vec![source("a/note.md", "h1"), source("b/note.md", "h2")],
        );
        let body = note_content(&turn);
        let sources_section = body.split("## Sources").nth(1).unwrap();
        assert_eq!(sources_section.matches("[[note]]").count(), 1);
    }

    #[test]
    fn citations_link_by_ordinal_even_when_sources_are_shuffled() {
        // Sources in vec order [c, a, b] but ordinals [3, 1, 2]: `[1]` must
        // link the ordinal-1 source (alpha), NOT the first vec element (charlie).
        let turn = turn_with(
            "q",
            "first [1] second [2] third [3]",
            vec![
                source_ord("projects/charlie.md", "h", 3),
                source_ord("projects/alpha.md", "h", 1),
                source_ord("projects/beta.md", "h", 2),
            ],
        );
        let body = note_content(&turn);
        assert!(body.contains("first [[alpha]]"), "`[1]` → ordinal-1 source: {body}");
        assert!(body.contains("second [[beta]]"), "`[2]` → ordinal-2 source: {body}");
        assert!(body.contains("third [[charlie]]"), "`[3]` → ordinal-3 source: {body}");
    }

    #[test]
    fn a_gap_leaves_the_citation_marker_untouched() {
        // Ordinal 2 was dropped: `[2]` has no backing source and must stay `[2]`,
        // while `[1]` and `[3]` still link.
        let turn = turn_with(
            "q",
            "a [1] b [2] c [3]",
            vec![
                source_ord("projects/alpha.md", "h", 1),
                source_ord("projects/charlie.md", "h", 3),
            ],
        );
        let body = note_content(&turn);
        assert!(body.contains("a [[alpha]]"), "{body}");
        assert!(body.contains("b [2] c"), "gap `[2]` stays a literal marker: {body}");
        assert!(body.contains("[[charlie]]"), "{body}");
    }

    #[test]
    fn suggested_path_nests_under_ask_and_slugs_the_question() {
        let path = suggested_path("How do I Ship v2?");
        assert_eq!(path.to_string(), "ask/how-do-i-ship-v2");
    }
}
