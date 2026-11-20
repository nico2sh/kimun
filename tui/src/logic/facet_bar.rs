//! The facet pill bar model: which facets of a note are picked, how many
//! are left to pick per kind ([`FacetBar::menus`]), the picks
//! ([`FacetBar::picked`]) and the lists of unpicked facets
//! ([`FacetBar::kind_list`], [`FacetBar::all_grouped`]). Pure state over
//! [`NoteFacets`], shared by every frontend.

use kimun_core::note::{Facet, FacetEntry, FacetKind, NoteFacets};

use super::text_match::match_ranges;

const KINDS: [FacetKind; 3] = [FacetKind::Tag, FacetKind::NoteLink, FacetKind::Heading];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pill {
    pub facet: Facet,
    pub label: String,
    pub count: usize,
    pub picked: bool,
}

/// How many unpicked facets of each kind a note holds. Each one is a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindCounts {
    /// Unpicked tags.
    pub tags: usize,
    /// Unpicked note links.
    pub links: usize,
    /// Unpicked headings.
    pub sections: usize,
}

fn entry_pill(e: &FacetEntry, picked: bool) -> Pill {
    Pill {
        facet: e.facet.clone(),
        label: e.label.clone(),
        count: e.count,
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

    /// How many unpicked facets of each kind the note holds.
    pub fn menus(&self, facets: &NoteFacets) -> KindCounts {
        let count = |kind| self.kind_list(facets, kind, "").len();
        KindCounts {
            tags: count(FacetKind::Tag),
            links: count(FacetKind::NoteLink),
            sections: count(FacetKind::Heading),
        }
    }

    /// The picks the note still holds, in pick order. Any kind can appear.
    pub fn picked(&self, facets: &NoteFacets) -> Vec<Pill> {
        self.picks
            .iter()
            .filter_map(|p| facets.entries.iter().find(|e| e.facet == *p))
            .map(|e| entry_pill(e, true))
            .collect()
    }

    /// The unpicked facets of `kind` whose label matches `query` with the
    /// query's characters in order, ignoring case. Tags and links go by count (high
    /// first), then label. Headings stay in document order.
    pub fn kind_list(&self, facets: &NoteFacets, kind: FacetKind, query: &str) -> Vec<Pill> {
        let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
        let pills: Vec<Pill> = if kind == FacetKind::Heading {
            facets
                .entries
                .iter()
                .filter(|e| e.facet.kind == kind && !self.picks.contains(&e.facet))
                .map(|e| entry_pill(e, false))
                .collect()
        } else {
            self.unpicked_ranked(facets)
                .filter(|e| e.facet.kind == kind)
                .map(|e| entry_pill(e, false))
                .collect()
        };
        pills
            .into_iter()
            .filter(|p| is_subsequence(&query, &p.label))
            .collect()
    }

    /// The unpicked facets that match `query`, grouped by kind in the order
    /// tags, links, headings. Empty groups are left out.
    pub fn all_grouped(&self, facets: &NoteFacets, query: &str) -> Vec<(FacetKind, Vec<Pill>)> {
        KINDS
            .into_iter()
            .filter_map(|kind| {
                let pills = self.kind_list(facets, kind, query);
                (!pills.is_empty()).then_some((kind, pills))
            })
            .collect()
    }

    /// Unpicked tags and links together: count (highest first), then label
    /// (case-insensitive, then exact).
    fn unpicked_ranked<'a>(&self, facets: &'a NoteFacets) -> impl Iterator<Item = &'a FacetEntry> {
        let mut entries: Vec<&FacetEntry> = facets
            .entries
            .iter()
            .filter(|e| e.facet.kind != FacetKind::Heading && !self.picks.contains(&e.facet))
            .collect();
        entries.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
                .then_with(|| a.label.cmp(&b.label))
        });
        entries.into_iter()
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

    /// "Showing text tagged #a or #b, in D \u{b7} 12 pieces", with
    /// " in N notes" after the pieces when `notes` is given. `None` without
    /// picks.
    pub fn summary(
        &self,
        facets: &NoteFacets,
        pieces: usize,
        notes: Option<usize>,
    ) -> Option<String> {
        if self.picks.is_empty() {
            return None;
        }
        let noun = if pieces == 1 { "piece" } else { "pieces" };
        let in_notes = notes.map_or_else(String::new, |n| {
            format!(" in {n} {}", if n == 1 { "note" } else { "notes" })
        });
        Some(format!(
            "Showing text {} \u{b7} {pieces} {noun}{in_notes}",
            self.phrases(facets).join(", ")
        ))
    }

    /// Why nothing shows: "No text tagged #a and in D." `None` when there are
    /// no picks or `any_shown` (the caller drew something).
    pub fn empty_reason(&self, facets: &NoteFacets, any_shown: bool) -> Option<String> {
        if self.picks.is_empty() || any_shown {
            return None;
        }
        Some(format!("No text {}.", self.phrases(facets).join(" and ")))
    }

    /// The most recent pick.
    pub fn last_pick(&self) -> Option<&Facet> {
        self.picks.last()
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
    use kimun_core::note::note_facets;

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
            Some("Showing text tagged #run or #garden, in D \u{b7} 12 pieces")
        );
        bar.set_picks(vec![tag("run")]);
        assert_eq!(
            bar.summary(&f, 1, None).as_deref(),
            Some("Showing text tagged #run \u{b7} 1 piece")
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
            Some("Showing text tagged #run, in D \u{b7} 12 pieces in 5 notes")
        );
        assert_eq!(
            bar.summary(&f, 1, Some(1)).as_deref(),
            Some("Showing text tagged #run, in D \u{b7} 1 piece in 1 note")
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
            Some("No text tagged #run and in B.")
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
        let all = bar.menus(&f);
        assert_eq!((all.tags, all.links, all.sections), (3, 2, 4));
        bar.set_picks(vec![tag("a"), heading("h2")]);
        let counts = bar.menus(&f);
        assert_eq!((counts.tags, counts.links, counts.sections), (2, 2, 3));
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
        let list = FacetBar::default().kind_list(&f, FacetKind::Tag, "");
        assert_eq!(labels(&list), ["c", "b", "a"]);
        assert!(list.iter().all(|p| !p.picked));
    }

    #[test]
    fn kind_list_keeps_headings_in_document_order() {
        let f = facets("# Zed\n\n## Alpha\n\n### Mid\n");
        let list = FacetBar::default().kind_list(&f, FacetKind::Heading, "");
        assert_eq!(labels(&list), ["Zed", "Alpha", "Mid"]);
    }

    #[test]
    fn kind_list_fuzzy_filters_and_skips_picked() {
        let f = facets("#plant #zzz #pn #plan\n");
        let mut bar = FacetBar::default();
        bar.toggle(tag("plan"));
        let list = bar.kind_list(&f, FacetKind::Tag, "pln");
        assert_eq!(labels(&list), ["plant"]);
    }

    #[test]
    fn all_grouped_orders_tag_link_heading_and_skips_empty_groups() {
        let f = facets("# Head\n\n[[l]] #a #a #b\n");
        let groups = FacetBar::default().all_grouped(&f, "");
        let got: Vec<_> = groups.iter().map(|(k, p)| (*k, labels(p))).collect();
        assert_eq!(
            got,
            [
                (FacetKind::Tag, vec!["a", "b"]),
                (FacetKind::NoteLink, vec!["l"]),
                (FacetKind::Heading, vec!["Head"]),
            ]
        );
        let only = FacetBar::default().all_grouped(&f, "hea");
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].0, FacetKind::Heading);
        let mut bar = FacetBar::default();
        bar.toggle(heading("head"));
        assert_eq!(bar.all_grouped(&f, "").len(), 2);
    }

    #[test]
    fn a_headings_only_note_has_only_a_sections_menu() {
        let f = facets("# A\n\ntext\n\n## B\n");
        let counts = FacetBar::default().menus(&f);
        assert_eq!((counts.tags, counts.links, counts.sections), (0, 0, 2));
    }

    #[test]
    fn kind_list_breaks_a_count_tie_ignoring_case() {
        let f = facets("#B #a\n");
        assert_eq!(
            labels(&FacetBar::default().kind_list(&f, FacetKind::Tag, "")),
            ["a", "B"]
        );
    }

    #[test]
    fn kind_list_ignores_whitespace_in_the_query() {
        let f = facets("#plant #zzz\n");
        assert_eq!(
            labels(&FacetBar::default().kind_list(&f, FacetKind::Tag, " p l ")),
            ["plant"]
        );
    }

    #[test]
    fn all_grouped_is_empty_when_only_picked_facets_remain() {
        let f = facets("# H\n\n#a [[x]]\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![heading("h"), tag("a"), link("x")]);
        assert!(bar.all_grouped(&f, "").is_empty());
    }
}
