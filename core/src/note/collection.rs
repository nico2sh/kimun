//! Facets and chunks of a collection: several notes seen as one facet view.
//! Pure functions over per-note facets. See `CONTEXT.md`, **Collection**.

use std::collections::HashMap;
use std::ops::Range;

use super::facet_counts::in_view_count;
use super::facet_view::{plan_chunks, ChunkPlan};
use super::facets::{select_blocks, Facet, FacetEntry, NoteFacets};

/// Entries of all notes merged by `Facet`: summed `count`, first label in list
/// order, `scopes` empty (a scope is per note). Heading entries keep (note order,
/// position) order. Other kinds keep first-seen order.
pub fn merge_entries(notes: &[&NoteFacets]) -> Vec<FacetEntry> {
    let mut merged: Vec<FacetEntry> = Vec::new();
    let mut index: HashMap<Facet, usize> = HashMap::new();
    for entry in notes.iter().flat_map(|n| &n.entries) {
        match index.get(&entry.facet) {
            Some(&at) => merged[at].count += entry.count,
            None => {
                index.insert(entry.facet.clone(), merged.len());
                merged.push(FacetEntry {
                    facet: entry.facet.clone(),
                    label: entry.label.clone(),
                    scopes: Vec::new(),
                    count: entry.count,
                });
            }
        }
    }
    merged
}

/// Per note, the block ranges that `picks` select. No picks: every note whole
/// (`[0..blocks.len()]`, and `[]` for a note with no blocks). With picks: the
/// result of `select_blocks` for that note (a note that selects nothing gets `[]`).
pub fn select_collection(notes: &[&NoteFacets], picks: &[Facet]) -> Vec<Vec<Range<usize>>> {
    // `select_blocks` with no picks already gives the whole note, and `[]` for
    // a note without blocks.
    notes.iter().map(|n| select_blocks(n, picks)).collect()
}

/// The sum of [`in_view_count`] over `notes`: how many pieces `facet`
/// gives in the collection's view under `picks`. To count every facet at
/// once, use [`in_view_counts`](super::in_view_counts).
pub fn in_view_count_collection(notes: &[&NoteFacets], picks: &[Facet], facet: &Facet) -> usize {
    notes.iter().map(|n| in_view_count(n, picks, facet)).sum()
}

/// The chunks of one note of a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotePlan {
    /// Index of the note in the list given to [`plan_collection`].
    pub note: usize,
    pub chunks: Vec<ChunkPlan>,
}

/// Chunks per note, in list order, under ONE cap `limit` across notes. Notes with no
/// chunk are left out. The second value counts the chunks left out by the cap
/// (over all notes). Each note's hidden counts and crumbs are those of
/// `plan_chunks(facets, text, shown, usize::MAX)`. `notes` and `shown` are
/// parallel: one entry each per note.
pub fn plan_collection(
    notes: &[(&NoteFacets, &str)],
    shown: &[Vec<Range<usize>>],
    limit: usize,
) -> (Vec<NotePlan>, usize) {
    debug_assert_eq!(notes.len(), shown.len(), "one `shown` per note");
    let mut plans: Vec<NotePlan> = Vec::new();
    let mut left = limit;
    let mut cut = 0;
    for (note, ((facets, text), ranges)) in notes.iter().zip(shown).enumerate() {
        let (mut chunks, _) = plan_chunks(facets, text, ranges, usize::MAX);
        if chunks.len() > left {
            cut += chunks.len() - left;
            chunks.truncate(left);
        }
        left -= chunks.len();
        if !chunks.is_empty() {
            plans.push(NotePlan { note, chunks });
        }
    }
    (plans, cut)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use crate::nfs::VaultPath;
    use crate::note::{note_facets, ChunkPlan, FacetKind};
    use std::ops::Range;

    fn nf(text: &str) -> NoteFacets {
        note_facets(text, &VaultPath::new("/n.md"))
    }

    fn tag(key: &str) -> Facet {
        Facet::new(FacetKind::Tag, key)
    }

    fn entry<'a>(es: &'a [FacetEntry], f: &Facet) -> &'a FacetEntry {
        es.iter().find(|e| &e.facet == f).expect("entry")
    }

    #[test]
    fn merge_sums_counts_and_keeps_the_first_label() {
        let (a, b) = (nf("one #Idea and #idea\n"), nf("two #IDEA\n"));
        let merged = merge_entries(&[&a, &b]);
        let e = entry(&merged, &tag("idea"));
        assert_eq!(e.count, 3);
        assert_eq!(e.label, "Idea");
        assert!(e.scopes.is_empty());
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn equal_facets_merge_across_notes_ignoring_case_and_heading_level() {
        let (a, b) = (nf("# Café\n\none\n"), nf("## cafe\n\ntwo\n"));
        let merged = merge_entries(&[&a, &b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].count, 2);
        assert_eq!(merged[0].label, "Café");
    }

    #[test]
    fn heading_entries_keep_note_then_position_order() {
        let (a, b) = (nf("# B\n\n# A\n"), nf("# C\n\n# A\n\n# D\n"));
        let merged = merge_entries(&[&a, &b]);
        let keys: Vec<&str> = merged.iter().map(|e| e.facet.key.as_str()).collect();
        assert_eq!(keys, ["b", "a", "c", "d"]);
    }

    #[test]
    fn merge_of_no_notes_is_empty() {
        assert!(merge_entries(&[]).is_empty());
    }

    #[test]
    fn no_picks_select_every_note_whole() {
        let (a, b) = (nf("x\n\ny\n"), nf("z\n"));
        let out = select_collection(&[&a, &b], &[]);
        assert_eq!(out, [vec![0..2], vec![0..1]]);
    }

    #[test]
    fn a_note_without_blocks_selects_nothing() {
        let (a, b) = (nf(""), nf("z\n"));
        let out = select_collection(&[&a, &b], &[]);
        assert_eq!(out, [vec![], vec![0..1]]);
    }

    #[test]
    fn picks_select_per_note_with_or_and_and() {
        let a = nf("# H\n\n#a one\n\n#b two\n");
        let b = nf("#a only\n\n#c other\n");
        let or = select_collection(&[&a, &b], &[tag("a"), tag("b")]);
        assert_eq!(or, [vec![1..3], vec![0..1]]);
        let h = Facet::new(FacetKind::Heading, "h");
        let and = select_collection(&[&a, &b], &[h, tag("b")]);
        assert_eq!(and, [vec![2..3], vec![]]);
    }

    #[test]
    fn a_note_that_selects_nothing_is_empty_but_keeps_its_slot() {
        let (a, b, c) = (nf("#a x\n"), nf("none\n"), nf("#a y\n"));
        let out = select_collection(&[&a, &b, &c], &[tag("a")]);
        assert_eq!(out, [vec![0..1], vec![], vec![0..1]]);
    }

    #[test]
    fn collection_counts_sum_over_notes() {
        let a = nf("# H\n\n#a one [[l]]\n\n#a two\n");
        let b = nf("#a three [[l]]\n\n[[l]] four\n");
        let c = nf("none\n");
        let notes = [&a, &b, &c];
        let l = Facet::new(FacetKind::NoteLink, "l");
        assert_eq!(in_view_count_collection(&notes, &[], &l), 3);
        assert_eq!(in_view_count_collection(&notes, &[tag("a")], &l), 2);
        assert_eq!(in_view_count_collection(&notes, &[tag("a")], &tag("a")), 3);
        assert_eq!(in_view_count_collection(&[], &[tag("a")], &l), 0);
    }

    fn paras(prefix: &str, n: usize) -> String {
        (0..n).map(|i| format!("{prefix}{i}\n\n")).collect()
    }

    fn texts(plan: &[NotePlan]) -> Vec<(usize, Vec<String>)> {
        plan.iter()
            .map(|p| (p.note, p.chunks.iter().map(|c| c.text.clone()).collect()))
            .collect()
    }

    fn alternating(n: usize) -> Vec<Range<usize>> {
        (0..n).map(|i| i * 2..i * 2 + 1).collect()
    }

    #[test]
    fn one_cap_spans_notes_in_list_order() {
        let (ta, tb) = (paras("a", 6), paras("b", 6));
        let (fa, fb) = (nf(&ta), nf(&tb));
        let shown = [alternating(2), alternating(3)];
        let (plan, cut) = plan_collection(&[(&fa, &ta), (&fb, &tb)], &shown, 3);
        assert_eq!(
            texts(&plan),
            [
                (0, vec!["a0".to_string(), "a2".to_string()]),
                (1, vec!["b0".to_string()])
            ]
        );
        assert_eq!(cut, 2);
    }

    #[test]
    fn the_cut_count_covers_all_notes() {
        let (ta, tb, tc) = (paras("a", 6), paras("b", 6), paras("c", 6));
        let (fa, fb, fc) = (nf(&ta), nf(&tb), nf(&tc));
        let shown = [alternating(1), alternating(3), alternating(2)];
        let (plan, cut) = plan_collection(&[(&fa, &ta), (&fb, &tb), (&fc, &tc)], &shown, 2);
        assert_eq!(plan.len(), 2);
        assert_eq!(cut, 4);
        let (plan, cut) = plan_collection(&[(&fa, &ta), (&fb, &tb), (&fc, &tc)], &shown, 0);
        assert!(plan.is_empty());
        assert_eq!(cut, 6);
    }

    #[test]
    fn a_note_with_no_chunks_is_left_out_of_the_plan() {
        let (ta, tb, tc) = (paras("a", 3), paras("b", 3), paras("c", 3));
        let (fa, fb, fc) = (nf(&ta), nf(&tb), nf(&tc));
        let shown = [vec![0..1], vec![], vec![1..2]];
        let (plan, cut) = plan_collection(&[(&fa, &ta), (&fb, &tb), (&fc, &tc)], &shown, 10);
        assert_eq!(plan.iter().map(|p| p.note).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(cut, 0);
    }

    #[test]
    fn hidden_counts_stay_inside_each_note() {
        let (ta, tb) = (paras("a", 5), paras("b", 5));
        let (fa, fb) = (nf(&ta), nf(&tb));
        let shown = [vec![1..2], vec![2..3]];
        let (plan, _) = plan_collection(&[(&fa, &ta), (&fb, &tb)], &shown, 10);
        let h = |c: &ChunkPlan| (c.hidden_above, c.hidden_below);
        assert_eq!(h(&plan[0].chunks[0]), (1, 3));
        assert_eq!(h(&plan[1].chunks[0]), (2, 2));
    }

    #[test]
    fn hidden_counts_are_not_recomputed_after_the_cut() {
        let ta = paras("a", 6);
        let fa = nf(&ta);
        let shown = [vec![0..1, 2..3, 4..5]];
        let (plan, cut) = plan_collection(&[(&fa, &ta)], &shown, 1);
        assert_eq!(cut, 2);
        assert_eq!(plan[0].chunks[0].hidden_below, 1);
    }

    #[test]
    fn crumbs_belong_to_each_notes_own_headings() {
        let (ta, tb) = ("# Alpha\n\npa\n", "# Beta\n\npb\n");
        let (fa, fb) = (nf(ta), nf(tb));
        let shown = [vec![1..2], vec![1..2]];
        let (plan, _) = plan_collection(&[(&fa, ta), (&fb, tb)], &shown, 10);
        assert_eq!(plan[0].chunks[0].crumbs[0].label, "Alpha");
        assert_eq!(plan[1].chunks[0].crumbs[0].label, "Beta");
    }

    #[test]
    fn parallel_notes_and_shown_plan_without_the_length_check_firing() {
        let (ta, tb) = (paras("a", 2), paras("b", 2));
        let (fa, fb) = (nf(&ta), nf(&tb));
        let shown = [vec![0..1], vec![1..2]];
        let (plan, cut) = plan_collection(&[(&fa, &ta), (&fb, &tb)], &shown, 10);
        assert_eq!(plan.len(), 2);
        assert_eq!(cut, 0);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "one `shown` per note")]
    fn a_shown_list_of_another_length_is_caught_in_debug_builds() {
        let ta = paras("a", 2);
        let fa = nf(&ta);
        plan_collection(&[(&fa, &ta)], &[], 10);
    }

    #[test]
    fn crlf_notes_plan_the_same() {
        let lf = "# A\n\none\n\ntwo\n";
        let crlf = lf.replace('\n', "\r\n");
        let (fl, fc) = (nf(lf), nf(&crlf));
        let shown = [vec![1..2]];
        let (a, ca) = plan_collection(&[(&fl, lf)], &shown, 10);
        let (b, cb) = plan_collection(&[(&fc, crlf.as_str())], &shown, 10);
        assert_eq!(ca, cb);
        assert_eq!(a[0].chunks[0].blocks, b[0].chunks[0].blocks);
        assert_eq!(a[0].chunks[0].crumbs, b[0].chunks[0].crumbs);
        assert_eq!(b[0].chunks[0].text, "one");
    }
}
