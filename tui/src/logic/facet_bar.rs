//! The facet pill bar model: which facets of a note are picked, how many
//! facets per kind still give results ([`FacetBar::menus`]), the picks
//! ([`FacetBar::picked`]) and the lists of picked and unpicked facets
//! ([`FacetBar::kind_list`], [`FacetBar::all_grouped`]). Pure state over
//! [`NoteFacets`], shared by every frontend.
//!
//! The lists and menus take a count function: how many pieces a facet gives
//! in the current view ([`kimun_core::note::in_view_count`]). The GUI builds
//! a map once per selection with [`kimun_core::note::in_view_counts`] and
//! passes a lookup into it. The tests pass `in_view_count` itself.

use std::cmp::Reverse;

use kimun_core::note::{Facet, FacetEntry, FacetKind, NoteFacets};

use super::text_match::match_ranges;

const KINDS: [FacetKind; 3] = [FacetKind::Tag, FacetKind::NoteLink, FacetKind::Heading];

/// One facet as a list row or a picked pill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pill {
    pub facet: Facet,
    pub label: String,
    /// Occurrences in the whole note (or collection).
    pub count: usize,
    /// Pieces the facet gives in the view the other kinds' picks show. Set
    /// by the count function in [`FacetBar::kind_list`]. [`FacetBar::picked`]
    /// has no count function and puts the whole count here.
    pub in_view: usize,
    pub picked: bool,
}

/// Per kind, how many unpicked facets give results in the current view.
/// Each kind is a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindCounts {
    /// Unpicked tags with results in view.
    pub tags: usize,
    /// Unpicked note links with results in view.
    pub links: usize,
    /// Unpicked headings with results in view.
    pub sections: usize,
    /// Which kinds have any unpicked facet at all, with results or not.
    pub any: KindAny,
}

/// Per kind, whether any unpicked facet exists. A kind without one has no
/// menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindAny {
    pub tags: bool,
    pub links: bool,
    pub sections: bool,
}

fn entry_pill(e: &FacetEntry, in_view: usize, picked: bool) -> Pill {
    Pill {
        facet: e.facet.clone(),
        label: e.label.clone(),
        count: e.count,
        in_view,
        picked,
    }
}

#[derive(Debug, Clone, Default)]
pub struct FacetBar {
    picks: Vec<Facet>,
}

impl FacetBar {
    pub fn picks(&self) -> &[Facet] {
        &self.picks
    }

    pub fn set_picks(&mut self, picks: Vec<Facet>) {
        self.picks = picks;
    }

    /// Pick `facet`, or unpick it when already picked.
    pub fn toggle(&mut self, facet: Facet) {
        match self.picks.iter().position(|p| *p == facet) {
            Some(at) => {
                self.picks.remove(at);
            }
            None => self.picks.push(facet),
        }
    }

    pub fn clear(&mut self) {
        self.picks.clear();
    }

    /// Drop picks the note no longer holds. Returns true if any dropped.
    pub fn prune(&mut self, facets: &NoteFacets) -> bool {
        let before = self.picks.len();
        self.picks
            .retain(|p| facets.entries.iter().any(|e| e.facet == *p));
        self.picks.len() != before
    }

    /// Per kind, how many unpicked facets have `count_in_view > 0`, and
    /// whether the kind has any unpicked facet.
    pub fn menus(
        &self,
        facets: &NoteFacets,
        count_in_view: &dyn Fn(&Facet) -> usize,
    ) -> KindCounts {
        let mut counts = KindCounts {
            tags: 0,
            links: 0,
            sections: 0,
            any: KindAny {
                tags: false,
                links: false,
                sections: false,
            },
        };
        for e in facets
            .entries
            .iter()
            .filter(|e| !self.picks.contains(&e.facet))
        {
            let (n, any) = match e.facet.kind {
                FacetKind::Tag => (&mut counts.tags, &mut counts.any.tags),
                FacetKind::NoteLink => (&mut counts.links, &mut counts.any.links),
                FacetKind::Heading => (&mut counts.sections, &mut counts.any.sections),
            };
            *any = true;
            if count_in_view(&e.facet) > 0 {
                *n += 1;
            }
        }
        counts
    }

    /// The picks the note still holds, in pick order. Any kind can appear.
    /// A picked pill's `in_view` holds its whole count (no count function).
    pub fn picked(&self, facets: &NoteFacets) -> Vec<Pill> {
        self.picks
            .iter()
            .filter_map(|p| facets.entries.iter().find(|e| e.facet == *p))
            .map(|e| entry_pill(e, e.count, true))
            .collect()
    }

    /// The facets of `kind` whose label matches `query` with the query's
    /// characters in order, ignoring case. Picked ones come first, in pick
    /// order. Then unpicked tags and links by `in_view` (high first, so
    /// those with none come last), whole count (high first), then label.
    /// Unpicked headings stay in document order.
    pub fn kind_list(
        &self,
        facets: &NoteFacets,
        kind: FacetKind,
        query: &str,
        count_in_view: &dyn Fn(&Facet) -> usize,
    ) -> Vec<Pill> {
        let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
        // Filter first: only the facets the query keeps are counted.
        let matches = |e: &&FacetEntry| is_subsequence(&query, &e.label);
        let picked: Vec<Pill> = self
            .picks
            .iter()
            .filter(|p| p.kind == kind)
            .filter_map(|p| facets.entries.iter().find(|e| e.facet == *p))
            .filter(matches)
            .map(|e| entry_pill(e, count_in_view(&e.facet), true))
            .collect();
        let mut unpicked: Vec<Pill> = facets
            .entries
            .iter()
            .filter(|e| e.facet.kind == kind && !self.picks.contains(&e.facet))
            .filter(matches)
            .map(|e| entry_pill(e, count_in_view(&e.facet), false))
            .collect();
        if kind != FacetKind::Heading {
            // One lowercase label per row, not two per comparison.
            unpicked.sort_by_cached_key(|p| {
                (
                    Reverse(p.in_view),
                    Reverse(p.count),
                    p.label.to_lowercase(),
                    p.label.clone(),
                )
            });
        }
        picked.into_iter().chain(unpicked).collect()
    }

    /// The facets that match `query`, grouped by kind in the order tags,
    /// links, headings, each group as [`Self::kind_list`] orders it. Empty
    /// groups are left out.
    pub fn all_grouped(
        &self,
        facets: &NoteFacets,
        query: &str,
        count_in_view: &dyn Fn(&Facet) -> usize,
    ) -> Vec<(FacetKind, Vec<Pill>)> {
        KINDS
            .into_iter()
            .filter_map(|kind| {
                let pills = self.kind_list(facets, kind, query, count_in_view);
                (!pills.is_empty()).then_some((kind, pills))
            })
            .collect()
    }

    /// One phrase per kind that has picks, in the order tag, link, heading.
    pub fn phrases(&self, facets: &NoteFacets) -> Vec<String> {
        KINDS
            .into_iter()
            .filter_map(|kind| {
                let labels: Vec<String> = self
                    .picks
                    .iter()
                    .filter(|p| p.kind == kind)
                    .map(|p| {
                        let label = facets
                            .entries
                            .iter()
                            .find(|e| e.facet == *p)
                            .map_or(p.key.as_str(), |e| e.label.as_str());
                        match kind {
                            FacetKind::Tag => format!("#{label}"),
                            _ => label.to_string(),
                        }
                    })
                    .collect();
                if labels.is_empty() {
                    return None;
                }
                let lead = match kind {
                    FacetKind::Tag => "tagged",
                    FacetKind::NoteLink => "linking to",
                    FacetKind::Heading => "in",
                };
                Some(format!("{lead} {}", labels.join(" or ")))
            })
            .collect()
    }

    /// "Showing text tagged #a or #b, and in D \u{b7} 12 pieces", with
    /// " in N notes" after the pieces when `notes` is given. `None` without
    /// picks.
    pub fn summary(
        &self,
        facets: &NoteFacets,
        pieces: usize,
        notes: Option<usize>,
    ) -> Option<String> {
        let picks = self.picks_text(facets)?;
        let noun = if pieces == 1 { "piece" } else { "pieces" };
        let in_notes = notes.map_or_else(String::new, |n| {
            format!(" in {n} {}", if n == 1 { "note" } else { "notes" })
        });
        Some(format!("{picks} \u{b7} {pieces} {noun}{in_notes}"))
    }

    /// The picks in words with no count: "Showing text tagged #a or #b, and
    /// in D". For a view that shows its counts on another line. `None`
    /// without picks.
    pub fn picks_text(&self, facets: &NoteFacets) -> Option<String> {
        if self.picks.is_empty() {
            return None;
        }
        Some(format!("Showing text {}", join_and(&self.phrases(facets))))
    }

    /// Why nothing shows: "No text tagged #a, and in D." `None` when there are
    /// no picks or `any_shown` (the caller drew something).
    pub fn empty_reason(&self, facets: &NoteFacets, any_shown: bool) -> Option<String> {
        if self.picks.is_empty() || any_shown {
            return None;
        }
        Some(format!("No text {}.", join_and(&self.phrases(facets))))
    }

    /// The most recent pick.
    pub fn last_pick(&self) -> Option<&Facet> {
        self.picks.last()
    }
}

/// The kinds' phrases as one: commas between them, and `, and` before the
/// last ("tagged #a or #b, and in D"). The comma keeps the AND apart from
/// the `or` inside a kind.
fn join_and(phrases: &[String]) -> String {
    match phrases {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// Whether `query`'s characters occur in `label` in order, case-insensitively.
fn is_subsequence(query: &[char], label: &str) -> bool {
    let mut rest = label;
    for c in query {
        let needle = [c.to_string()];
        match match_ranges(rest, &needle).first() {
            Some(&(_, end)) => rest = &rest[end..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::nfs::VaultPath;
    use kimun_core::note::{in_view_count, note_facets};

    fn facets(text: &str) -> NoteFacets {
        note_facets(text, &VaultPath::new("/n.md"))
    }

    fn tag(key: &str) -> Facet {
        Facet::new(FacetKind::Tag, key)
    }

    fn labels(pills: &[Pill]) -> Vec<&str> {
        pills.iter().map(|p| p.label.as_str()).collect()
    }

    #[test]
    fn toggle_picks_then_unpicks() {
        let mut bar = FacetBar::default();
        bar.toggle(tag("a"));
        assert_eq!(bar.picks(), [tag("a")]);
        bar.toggle(tag("a"));
        assert!(bar.picks().is_empty());
        bar.set_picks(vec![tag("x")]);
        bar.clear();
        assert!(bar.picks().is_empty());
    }

    #[test]
    fn prune_drops_picks_the_note_lost() {
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("a"), tag("gone")]);
        let f = facets("#a\n");
        assert!(bar.prune(&f));
        assert_eq!(bar.picks(), [tag("a")]);
        assert!(!bar.prune(&f));
    }

    fn link(key: &str) -> Facet {
        Facet::new(FacetKind::NoteLink, key)
    }

    fn heading(text: &str) -> Facet {
        Facet::new(FacetKind::Heading, text)
    }

    #[test]
    fn phrases_use_labels_and_join_a_kind_with_or() {
        let f = facets("# D1\n\n## D2\n\n#Run #garden [[x]] [[Y]]\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![
            heading("D1"),
            tag("run"),
            link("y"),
            tag("garden"),
            link("x"),
            heading("d2"),
        ]);
        assert_eq!(
            bar.phrases(&f),
            ["tagged #Run or #garden", "linking to y or x", "in D1 or D2"]
        );
        assert!(FacetBar::default().phrases(&f).is_empty());
    }

    #[test]
    fn summary_names_picks_and_counts_pieces() {
        let f = facets("# D\n\n#run #garden\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("run"), tag("garden"), heading("d")]);
        assert_eq!(
            bar.summary(&f, 12, None).as_deref(),
            Some("Showing text tagged #run or #garden, and in D \u{b7} 12 pieces")
        );
        bar.set_picks(vec![tag("run")]);
        assert_eq!(
            bar.summary(&f, 1, None).as_deref(),
            Some("Showing text tagged #run \u{b7} 1 piece")
        );
    }

    #[test]
    fn picks_text_names_the_picks_with_no_count() {
        let f = facets("# D\n\n#run #garden\n");
        let mut bar = FacetBar::default();
        assert_eq!(bar.picks_text(&f), None);
        bar.set_picks(vec![tag("run"), tag("garden"), heading("d")]);
        assert_eq!(
            bar.picks_text(&f).as_deref(),
            Some("Showing text tagged #run or #garden, and in D")
        );
    }

    #[test]
    fn summary_is_none_without_picks() {
        let f = facets("#a\n");
        assert_eq!(FacetBar::default().summary(&f, 3, None), None);
    }

    #[test]
    fn empty_reason_is_none_when_something_shows() {
        let f = facets("# D\n\n#run\n");
        let mut bar = FacetBar::default();
        assert_eq!(bar.empty_reason(&f, false), None);
        bar.set_picks(vec![tag("run")]);
        assert_eq!(bar.empty_reason(&f, true), None);
    }

    #[test]
    fn empty_reason_uses_the_any_shown_flag() {
        let f = facets("# D\n\n#run\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("run")]);
        // The flag decides, not the facets: `#run` selects blocks, yet the
        // caller says nothing is shown.
        assert_eq!(
            bar.empty_reason(&f, false).as_deref(),
            Some("No text tagged #run.")
        );
        assert_eq!(bar.empty_reason(&f, true), None);
        assert_eq!(FacetBar::default().empty_reason(&f, false), None);
    }

    #[test]
    fn summary_names_notes_when_given() {
        let f = facets("# D\n\n#run\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("run"), heading("d")]);
        assert_eq!(
            bar.summary(&f, 12, Some(5)).as_deref(),
            Some("Showing text tagged #run, and in D \u{b7} 12 pieces in 5 notes")
        );
        assert_eq!(
            bar.summary(&f, 1, Some(1)).as_deref(),
            Some("Showing text tagged #run, and in D \u{b7} 1 piece in 1 note")
        );
    }

    #[test]
    fn summary_without_notes_is_unchanged() {
        let f = facets("#run\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("run")]);
        assert_eq!(
            bar.summary(&f, 2, None).as_deref(),
            Some("Showing text tagged #run \u{b7} 2 pieces")
        );
        assert_eq!(FacetBar::default().summary(&f, 2, Some(3)), None);
    }

    #[test]
    fn empty_reason_joins_phrases_with_and() {
        let f = facets("# A\n\n#run\n\n# B\n\nother\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("run"), heading("b")]);
        assert_eq!(
            bar.empty_reason(&f, false).as_deref(),
            Some("No text tagged #run, and in B.")
        );
    }

    /// Three kinds: commas between, `, and` before the last.
    #[test]
    fn the_summary_joins_kinds_with_and() {
        let f = facets("# D\n\n#run #garden [[x]]\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("run"), tag("garden"), heading("d")]);
        assert_eq!(
            bar.summary(&f, 2, None).as_deref(),
            Some("Showing text tagged #run or #garden, and in D \u{b7} 2 pieces")
        );
        bar.set_picks(vec![tag("run"), link("x"), heading("d")]);
        assert_eq!(
            bar.summary(&f, 2, None).as_deref(),
            Some("Showing text tagged #run, linking to x, and in D \u{b7} 2 pieces")
        );
        assert_eq!(
            bar.empty_reason(&f, false).as_deref(),
            Some("No text tagged #run, linking to x, and in D.")
        );
    }

    #[test]
    fn last_pick_is_the_most_recent() {
        let mut bar = FacetBar::default();
        assert_eq!(bar.last_pick(), None);
        bar.toggle(tag("a"));
        bar.toggle(tag("b"));
        assert_eq!(bar.last_pick(), Some(&tag("b")));
        bar.toggle(tag("b"));
        assert_eq!(bar.last_pick(), Some(&tag("a")));
    }

    const MENU_NOTE: &str = "#a #b #c [[x]] [[y]]\n\n# H1\n\n## H2\n\n### H3\n\n#### H4\n";

    #[test]
    fn menus_count_only_unpicked_facets() {
        let f = facets(MENU_NOTE);
        let mut bar = FacetBar::default();
        let all = bar.menus(&f, &in_view(&f, &bar));
        assert_eq!((all.tags, all.links, all.sections), (3, 2, 4));
        bar.set_picks(vec![tag("a"), heading("h2")]);
        let counts = bar.menus(&f, &in_view(&f, &bar));
        // `#a` sits above H2, so no link shares the view.
        // Each kind counts in the view the other kinds' picks show, and
        // `#a` sits above H2: nothing gives results.
        assert_eq!((counts.tags, counts.links, counts.sections), (0, 0, 0));
        assert!(counts.any.tags && counts.any.links && counts.any.sections);
    }

    #[test]
    fn picked_is_in_pick_order_and_holds_headings() {
        let f = facets(MENU_NOTE);
        let mut bar = FacetBar::default();
        bar.set_picks(vec![heading("h3"), tag("b"), link("x")]);
        let picked = bar.picked(&f);
        assert_eq!(labels(&picked), ["H3", "b", "x"]);
        assert!(picked.iter().all(|p| p.picked));
        assert_eq!(picked[0].facet.kind, FacetKind::Heading);
    }

    #[test]
    fn picked_drops_a_pick_the_note_lost() {
        let f = facets("#a #b\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("gone"), tag("b")]);
        assert_eq!(labels(&bar.picked(&f)), ["b"]);
    }

    #[test]
    fn kind_list_ranks_tags_by_count_then_label() {
        let f = facets("#b #a #c #c #c #b\n");
        let bar = FacetBar::default();
        let list = bar.kind_list(&f, FacetKind::Tag, "", &in_view(&f, &bar));
        assert_eq!(labels(&list), ["c", "b", "a"]);
        assert!(list.iter().all(|p| !p.picked));
    }

    #[test]
    fn kind_list_keeps_headings_in_document_order() {
        let f = facets("# Zed\n\n## Alpha\n\n### Mid\n");
        let bar = FacetBar::default();
        let list = bar.kind_list(&f, FacetKind::Heading, "", &in_view(&f, &bar));
        assert_eq!(labels(&list), ["Zed", "Alpha", "Mid"]);
    }

    #[test]
    fn kind_list_fuzzy_filters_picked_and_unpicked() {
        let f = facets("#plant #zzz #pn #plan\n");
        let mut bar = FacetBar::default();
        bar.toggle(tag("plan"));
        let list = bar.kind_list(&f, FacetKind::Tag, "pln", &in_view(&f, &bar));
        assert_eq!(labels(&list), ["plan", "plant"]);
        assert_eq!(
            list.iter().map(|p| p.picked).collect::<Vec<_>>(),
            [true, false]
        );
    }

    #[test]
    fn all_grouped_orders_tag_link_heading_and_skips_empty_groups() {
        let f = facets("# Head\n\n[[l]] #a #a #b\n");
        let bar = FacetBar::default();
        let groups = bar.all_grouped(&f, "", &in_view(&f, &bar));
        let got: Vec<_> = groups.iter().map(|(k, p)| (*k, labels(p))).collect();
        assert_eq!(
            got,
            [
                (FacetKind::Tag, vec!["a", "b"]),
                (FacetKind::NoteLink, vec!["l"]),
                (FacetKind::Heading, vec!["Head"]),
            ]
        );
        let only = bar.all_grouped(&f, "hea", &in_view(&f, &bar));
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].0, FacetKind::Heading);
        let mut bar = FacetBar::default();
        bar.toggle(heading("head"));
        let groups = bar.all_grouped(&f, "", &in_view(&f, &bar));
        assert_eq!(groups.len(), 3);
        assert!(groups[2].1[0].picked);
        assert!(bar.all_grouped(&f, "zzz", &in_view(&f, &bar)).is_empty());
    }

    #[test]
    fn a_headings_only_note_has_only_a_sections_menu() {
        let f = facets("# A\n\ntext\n\n## B\n");
        let bar = FacetBar::default();
        let counts = bar.menus(&f, &in_view(&f, &bar));
        assert_eq!((counts.tags, counts.links, counts.sections), (0, 0, 2));
        assert_eq!(
            (counts.any.tags, counts.any.links, counts.any.sections),
            (false, false, true)
        );
    }

    #[test]
    fn kind_list_breaks_a_count_tie_ignoring_case() {
        let f = facets("#B #a\n");
        let bar = FacetBar::default();
        assert_eq!(
            labels(&bar.kind_list(&f, FacetKind::Tag, "", &in_view(&f, &bar))),
            ["a", "B"]
        );
    }

    #[test]
    fn kind_list_ignores_whitespace_in_the_query() {
        let f = facets("#plant #zzz\n");
        let bar = FacetBar::default();
        assert_eq!(
            labels(&bar.kind_list(&f, FacetKind::Tag, " p l ", &in_view(&f, &bar))),
            ["plant"]
        );
    }

    /// The bar's count function over one note, as a frontend passes it.
    fn in_view<'a>(f: &'a NoteFacets, bar: &'a FacetBar) -> impl Fn(&Facet) -> usize + 'a {
        move |facet| in_view_count(f, bar.picks(), facet)
    }

    fn rows(pills: &[Pill]) -> Vec<(&str, usize, bool)> {
        pills
            .iter()
            .map(|p| (p.label.as_str(), p.in_view, p.picked))
            .collect()
    }

    /// Blocks: 0 `# A`, 1 `#run #gym [[l]] [[m]]`, 2 `# B`, 3 `#run [[m]]`,
    /// 4 `#run #yoga [[m]] [[n]]`, 5 `[[o]]`, 6 `[[o]]`.
    const VIEW_NOTE: &str = "# A\n\n#run #gym [[l]] [[m]]\n\n# B\n\n#run [[m]]\n\n#run #yoga [[m]] [[n]]\n\n[[o]]\n\n[[o]]\n";

    #[test]
    fn kind_list_keeps_picked_pills_first_in_pick_order() {
        let f = facets(VIEW_NOTE);
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("yoga"), heading("b"), tag("gym")]);
        let list = bar.kind_list(&f, FacetKind::Tag, "", &in_view(&f, &bar));
        assert_eq!(
            rows(&list),
            [("yoga", 1, true), ("gym", 0, true), ("run", 2, false)]
        );
        // The filter applies to picked pills too.
        let list = bar.kind_list(&f, FacetKind::Tag, "g", &in_view(&f, &bar));
        assert_eq!(rows(&list), [("yoga", 1, true), ("gym", 0, true)]);
    }

    #[test]
    fn kind_list_ranks_by_in_view_count_and_puts_zero_last() {
        let f = facets(VIEW_NOTE);
        let mut bar = FacetBar::default();
        let list = bar.kind_list(&f, FacetKind::NoteLink, "", &in_view(&f, &bar));
        assert_eq!(
            rows(&list),
            [
                ("m", 3, false),
                ("o", 2, false),
                ("l", 1, false),
                ("n", 1, false)
            ]
        );
        bar.set_picks(vec![tag("run"), heading("b")]);
        let list = bar.kind_list(&f, FacetKind::NoteLink, "", &in_view(&f, &bar));
        // `o` has more whole-note hits than `l`, so it leads the zeros.
        assert_eq!(
            rows(&list),
            [
                ("m", 2, false),
                ("n", 1, false),
                ("o", 0, false),
                ("l", 0, false)
            ]
        );
        assert_eq!(list[2].count, 2);
    }

    #[test]
    fn menus_counts_only_facets_with_results_and_reports_any() {
        let f = facets(VIEW_NOTE);
        let mut bar = FacetBar::default();
        let all = bar.menus(&f, &in_view(&f, &bar));
        assert_eq!((all.tags, all.links, all.sections), (3, 4, 2));
        assert_eq!(
            (all.any.tags, all.any.links, all.any.sections),
            (true, true, true)
        );
        bar.set_picks(vec![tag("yoga")]);
        let some = bar.menus(&f, &in_view(&f, &bar));
        // Tags: `run` and `gym` count in the whole note (no other picks).
        // Links: only `m` and `n` share the `#yoga` piece. Sections: B.
        assert_eq!((some.tags, some.links, some.sections), (2, 2, 1));
        assert_eq!(
            (some.any.tags, some.any.links, some.any.sections),
            (true, true, true)
        );
        bar.set_picks(vec![tag("yoga"), tag("gym"), tag("run"), heading("a")]);
        let none = bar.menus(&f, &in_view(&f, &bar));
        assert_eq!((none.tags, none.links, none.sections), (0, 2, 1));
        assert_eq!(
            (none.any.tags, none.any.links, none.any.sections),
            (false, true, true)
        );
    }

    #[test]
    fn kind_list_counts_only_the_facets_the_filter_keeps() {
        let f = facets(VIEW_NOTE);
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("gym")]);
        let asked = std::cell::RefCell::new(Vec::new());
        let count = |facet: &Facet| {
            asked.borrow_mut().push(facet.key.clone());
            in_view_count(&f, bar.picks(), facet)
        };
        let list = bar.kind_list(&f, FacetKind::Tag, "yo", &count);
        assert_eq!(labels(&list), ["yoga"]);
        assert_eq!(*asked.borrow(), ["yoga"]);
    }

    #[test]
    fn headings_keep_document_order_with_in_view_counts() {
        let f = facets("# Zed\n\n#x one\n\n## Alpha\n\nplain\n\n# Mid\n\n#x two\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("x")]);
        let list = bar.kind_list(&f, FacetKind::Heading, "", &in_view(&f, &bar));
        assert_eq!(
            rows(&list),
            [("Zed", 1, false), ("Alpha", 0, false), ("Mid", 1, false)]
        );
        bar.set_picks(vec![tag("x"), heading("mid")]);
        let list = bar.kind_list(&f, FacetKind::Heading, "", &in_view(&f, &bar));
        assert_eq!(
            rows(&list),
            [("Mid", 1, true), ("Zed", 1, false), ("Alpha", 0, false)]
        );
    }

    #[test]
    fn all_grouped_keeps_picked_facets_when_nothing_else_remains() {
        let f = facets("# H\n\n#a [[x]]\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![heading("h"), tag("a"), link("x")]);
        let groups = bar.all_grouped(&f, "", &in_view(&f, &bar));
        assert_eq!(groups.len(), 3);
        assert!(
            groups
                .iter()
                .all(|(_, pills)| pills.iter().all(|p| p.picked))
        );
        let menus = bar.menus(&f, &in_view(&f, &bar));
        assert_eq!(
            (menus.any.tags, menus.any.links, menus.any.sections),
            (false, false, false)
        );
    }
}
