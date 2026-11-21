//! Context-aware facet counts: how many pieces a facet gives in the view
//! that the other kinds' picks show. See `CONTEXT.md`, **Facet**.

use std::collections::HashMap;
use std::ops::Range;

use super::facets::{select_blocks, Facet, FacetEntry, FacetKind, NoteFacets};

/// How many pieces `facet` gives in the view of `facets` under `picks`.
///
/// The view leaves out every pick of `facet.kind`. So the number is the
/// count of the facet's scopes that touch a block of
/// [`select_blocks`]`(facets, picks without facet.kind)`. Picks of its own
/// kind never change it. Two occurrences in one scope count once. A facet
/// the note lacks counts 0.
pub fn in_view_count(facets: &NoteFacets, picks: &[Facet], facet: &Facet) -> usize {
    let Some(entry) = facets.entries.iter().find(|e| e.facet == *facet) else {
        return 0;
    };
    touching(
        entry,
        &select_blocks(facets, &without_kind(picks, facet.kind)),
    )
}

/// [`in_view_count`] for every facet of `notes`, summed over the notes.
///
/// Each note is selected at most once per kind, and each entry is counted
/// once. Every facet of every note has a key, with 0 for none in view.
pub fn in_view_counts(notes: &[&NoteFacets], picks: &[Facet]) -> HashMap<Facet, usize> {
    let kinds = [FacetKind::Heading, FacetKind::Tag, FacetKind::NoteLink];
    // Per kind, the picks without that kind. A kind with no pick uses all
    // the picks, so those kinds share one selection.
    let picks_for: Vec<(FacetKind, Vec<Facet>)> = kinds
        .into_iter()
        .map(|kind| (kind, without_kind(picks, kind)))
        .collect();
    let mut counts: HashMap<Facet, usize> = HashMap::new();
    for note in notes {
        let mut shown: Vec<(&[Facet], Vec<Range<usize>>)> = Vec::new();
        for entry in &note.entries {
            let own = &picks_for
                .iter()
                .find(|(kind, _)| *kind == entry.facet.kind)
                .expect("every kind")
                .1;
            let at = match shown.iter().position(|(p, _)| *p == own.as_slice()) {
                Some(at) => at,
                None => {
                    shown.push((own, select_blocks(note, own)));
                    shown.len() - 1
                }
            };
            *counts.entry(entry.facet.clone()).or_insert(0) += touching(entry, &shown[at].1);
        }
    }
    counts
}

/// `picks` without the picks of `kind`.
fn without_kind(picks: &[Facet], kind: FacetKind) -> Vec<Facet> {
    picks.iter().filter(|p| p.kind != kind).cloned().collect()
}

/// How many of `entry`'s scopes touch a range of `shown`.
fn touching(entry: &FacetEntry, shown: &[Range<usize>]) -> usize {
    entry
        .scopes
        .iter()
        .filter(|scope| touches(scope, shown))
        .count()
}

/// Whether `scope` shares a block with any range of `shown` (ascending,
/// non-overlapping).
fn touches(scope: &Range<usize>, shown: &[Range<usize>]) -> bool {
    let after = shown.partition_point(|r| r.end <= scope.start);
    shown
        .get(after)
        .is_some_and(|r| r.start < scope.end && scope.start < r.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nfs::VaultPath;
    use crate::note::{note_facets, FacetKind};

    fn facets(text: &str) -> NoteFacets {
        note_facets(text, &VaultPath::new("/n.md"))
    }

    fn tag(key: &str) -> Facet {
        Facet::new(FacetKind::Tag, key)
    }

    fn link(key: &str) -> Facet {
        Facet::new(FacetKind::NoteLink, key)
    }

    fn heading(text: &str) -> Facet {
        Facet::new(FacetKind::Heading, text)
    }

    /// Blocks: 0 `# A`, 1 `#run #gym x [[l]]`, 2 `# B`, 3 `#run y`,
    /// 4 `#run z [[l]]`.
    const NOTE: &str = "# A\n\n#run #gym x [[l]]\n\n# B\n\n#run y\n\n#run z [[l]]\n";

    #[test]
    fn a_kind_without_picks_counts_only_occurrences_inside_the_shown_pieces() {
        let f = facets(NOTE);
        let in_b = [heading("b")];
        assert_eq!(in_view_count(&f, &in_b, &tag("run")), 2);
        assert_eq!(in_view_count(&f, &in_b, &tag("gym")), 0);
        assert_eq!(in_view_count(&f, &in_b, &link("l")), 1);
        let run = [tag("run")];
        assert_eq!(in_view_count(&f, &run, &link("l")), 2);
        assert_eq!(in_view_count(&f, &run, &heading("a")), 1);
    }

    #[test]
    fn a_kind_with_picks_counts_under_the_other_kinds_picks() {
        let f = facets(NOTE);
        let picks = [tag("run"), heading("b")];
        // Tags count inside section B: the tag picks are left out.
        assert_eq!(in_view_count(&f, &picks, &tag("run")), 2);
        // `#gym` sits outside section B, so it adds nothing.
        assert_eq!(in_view_count(&f, &picks, &tag("gym")), 0);
        // Headings count inside the `#run` pieces.
        assert_eq!(in_view_count(&f, &picks, &heading("a")), 1);
        assert_eq!(in_view_count(&f, &picks, &heading("b")), 1);
    }

    #[test]
    fn a_rows_number_ignores_its_own_kinds_picks() {
        let f = facets(NOTE);
        let base = in_view_count(&f, &[heading("b")], &tag("run"));
        for picks in [
            vec![heading("b"), tag("gym")],
            vec![heading("b"), tag("run")],
            vec![tag("gym"), heading("b"), tag("run")],
        ] {
            assert_eq!(in_view_count(&f, &picks, &tag("run")), base, "{picks:?}");
        }
        let base = in_view_count(&f, &[tag("run")], &heading("a"));
        let picks = [tag("run"), heading("a"), heading("b")];
        assert_eq!(in_view_count(&f, &picks, &heading("a")), base);
    }

    #[test]
    fn merged_scopes_count_once() {
        let line = facets("#c #c #c\n");
        let c = line.entries.iter().find(|e| e.facet == tag("c")).unwrap();
        assert_eq!(c.count, 3);
        assert_eq!(in_view_count(&line, &[], &tag("c")), 1);
        // The child item lies inside the parent item's scope.
        let list = facets("- parent #c\n  - child #c\n");
        let c = list.entries.iter().find(|e| e.facet == tag("c")).unwrap();
        assert_eq!(c.count, 2);
        assert_eq!(in_view_count(&list, &[], &tag("c")), 1);
        assert_eq!(in_view_count(&list, &[tag("c")], &tag("c")), 1);
    }

    #[test]
    fn the_counts_map_matches_one_count_per_facet() {
        let a = facets(NOTE);
        let b = facets("# B\n\n#gym [[l]]\n\n#run [[m]]\n");
        let notes = [&a, &b];
        for picks in [
            vec![],
            vec![tag("run")],
            vec![heading("b")],
            vec![tag("run"), heading("b"), link("l")],
            vec![tag("nope")],
        ] {
            let map = in_view_counts(&notes, &picks);
            let all = a.entries.iter().chain(&b.entries).map(|e| &e.facet);
            for facet in all {
                let one: usize = notes.iter().map(|n| in_view_count(n, &picks, facet)).sum();
                assert_eq!(map.get(facet).copied(), Some(one), "{facet:?} {picks:?}");
            }
        }
        assert_eq!(in_view_counts(&notes, &[]).get(&tag("run")), Some(&4));
        assert!(in_view_counts(&[], &[tag("run")]).is_empty());
    }

    #[test]
    fn a_facet_the_note_lacks_counts_zero() {
        let f = facets(NOTE);
        assert_eq!(in_view_count(&f, &[], &tag("nope")), 0);
        assert_eq!(in_view_count(&f, &[tag("run")], &tag("nope")), 0);
        assert_eq!(in_view_count(&f, &[heading("b")], &tag("nope")), 0);
    }

    #[test]
    fn a_facet_never_picked_with_no_picks_counts_its_occurrences() {
        let f = facets(NOTE);
        assert_eq!(in_view_count(&f, &[], &tag("run")), 3);
        assert_eq!(in_view_count(&f, &[], &link("l")), 2);
        assert_eq!(in_view_count(&f, &[], &heading("b")), 1);
    }
}
