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
    let names: Vec<String> = turn
        .sources
        .iter()
        .map(|s| s.path.get_clean_name())
        .collect();
    let linked = citations::link_sources(&turn.answer, &names);

    let mut seen = Vec::new();
    for name in &names {
        if !seen.contains(name) {
            seen.push(name.clone());
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
        Turn {
            id: 0,
            question: question.to_string(),
            answer: answer.to_string(),
            sources,
            status: TurnStatus::Done,
        }
    }

    fn source(path: &str, heading: &str) -> AskSource {
        AskSource {
            path: VaultPath::new(path),
            heading: heading.to_string(),
            score: 1.0,
            text: String::new(),
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
    fn suggested_path_nests_under_ask_and_slugs_the_question() {
        let path = suggested_path("How do I Ship v2?");
        assert_eq!(path.to_string(), "ask/how-do-i-ship-v2");
    }
}
