//! The facet pill bar model: which facets of a note are picked, the one
//! filter line of pills to show ([`FacetBar::line`]), how many of them fit
//! ([`fit_count`]) and the lists behind it. Pure state over [`NoteFacets`],
//! shared by every frontend.

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

/// The one-line filter bar: the pills in display order and the heading count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarLine {
    /// Picked pills first (pick order), then unpicked tags and links by count
    /// (desc), ties by label. A picked heading is included as a picked pill;
    /// an unpicked heading never appears here.
    pub pills: Vec<Pill>,
    /// Total headings in the note (the `Sections N` chip).
    pub sections: usize,
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

    /// The filter line of `facets`: see [`BarLine`].
    pub fn line(&self, facets: &NoteFacets) -> BarLine {
        let entry = |facet: &Facet| facets.entries.iter().find(|e| e.facet == *facet);
        let pill = |e: &FacetEntry, picked: bool| Pill {
            facet: e.facet.clone(),
            label: e.label.clone(),
            count: e.count,
            picked,
        };
        let mut pills: Vec<Pill> = self
            .picks
            .iter()
            .filter_map(|p| entry(p).map(|e| pill(e, true)))
            .collect();
        pills.extend(self.unpicked_ranked(facets).map(|e| pill(e, false)));
        let sections = facets
            .entries
            .iter()
            .filter(|e| e.facet.kind == FacetKind::Heading)
            .count();
        BarLine { pills, sections }
    }

    /// For the list: the unpicked tags then the unpicked links (each in
    /// [`Self::line`] order) whose label matches `query` with the same fuzzy
    /// match as [`Self::search`]. Headings are excluded; empty groups are
    /// left out.
    pub fn grouped(&self, facets: &NoteFacets, query: &str) -> Vec<(FacetKind, Vec<Pill>)> {
        let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
        let line = self.line(facets);
        [FacetKind::Tag, FacetKind::NoteLink]
            .into_iter()
            .filter_map(|kind| {
                let pills: Vec<Pill> = line
                    .pills
                    .iter()
                    .filter(|p| !p.picked && p.facet.kind == kind)
                    .filter(|p| is_subsequence(&query, &p.label))
                    .cloned()
                    .collect();
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

    /// The full list of one kind (the `Sections N` list of headings),
    /// fuzzy-filtered by `query`. The query's characters must appear in the
    /// label in order (case-insensitive); it is matched one character at a
    /// time with [`match_ranges`].
    pub fn more_list(&self, facets: &NoteFacets, kind: FacetKind, query: &str) -> Vec<Pill> {
        let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
        self.ordered(facets, kind)
            .into_iter()
            .filter(|p| is_subsequence(&query, &p.label))
            .collect()
    }

    /// Fuzzy search over every kind at once: kinds in the order tag, link,
    /// heading, each in its [`Self::more_list`] order. An empty query returns
    /// every facet.
    pub fn search(&self, facets: &NoteFacets, query: &str) -> Vec<Pill> {
        KINDS
            .into_iter()
            .flat_map(|kind| self.more_list(facets, kind, query))
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

    /// Every pill of `kind`: picked first, then headings in document order,
    /// tags and links by count (highest first) then label.
    fn ordered(&self, facets: &NoteFacets, kind: FacetKind) -> Vec<Pill> {
        let mut entries: Vec<&FacetEntry> = facets
            .entries
            .iter()
            .filter(|e| e.facet.kind == kind)
            .collect();
        if kind != FacetKind::Heading {
            entries.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
        }
        let mut pills: Vec<Pill> = entries
            .into_iter()
            .map(|e| Pill {
                picked: self.picks.contains(&e.facet),
                facet: e.facet.clone(),
                label: e.label.clone(),
                count: e.count,
            })
            .collect();
        // Stable: keeps the order above inside each group.
        pills.sort_by_key(|p| !p.picked);
        pills
    }
}

/// The number of pills that fit one line. `widths` are the pills' widths in
/// order, `gap` the space between items, `available` the room for the pills
/// plus the `+N` chip, `more_width` the width of that chip.
///
/// Returns the largest `n` such that the first `n` pills (with a `gap`
/// between each) fit in `available`; when `n` is less than `widths.len()` the
/// `gap` and `more_width` of the `+N` chip must fit too. When all pills fit
/// there is no chip, so none is reserved. Never less than `keep` (the picked
/// pills, which are always shown), and never more than `widths.len()`.
pub fn fit_count(widths: &[f32], gap: f32, available: f32, more_width: f32, keep: usize) -> usize {
    let total = widths.len();
    let mut used = 0.0_f32;
    let mut fit = 0;
    for (i, w) in widths.iter().enumerate() {
        used += if i == 0 { *w } else { gap + *w };
        let n = i + 1;
        let fits = if n == total {
            used <= available
        } else {
            used + gap + more_width <= available
        };
        if fits {
            fit = n;
        }
    }
    fit.max(keep.min(total))
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
    fn line_puts_picked_first_in_pick_order() {
        let f = facets("#a #a #a #b #b #c [[x]]\n");
        let mut bar = FacetBar::default();
        bar.set_picks(vec![tag("c"), link("x")]);
        let line = bar.line(&f);
        assert_eq!(labels(&line.pills), ["c", "x", "a", "b"]);
        assert!(line.pills[0].picked && line.pills[1].picked);
        assert!(!line.pills[2].picked && !line.pills[3].picked);
        bar.set_picks(vec![link("x"), tag("c")]);
        assert_eq!(labels(&bar.line(&f).pills)[..2], ["x", "c"]);
    }

    #[test]
    fn line_ranks_unpicked_tags_and_links_together_by_count() {
        let f = facets("#a [[l]] [[l]] #b #b #b [[m]]\n");
        let line = FacetBar::default().line(&f);
        assert_eq!(labels(&line.pills), ["b", "l", "a", "m"]);
    }

    #[test]
    fn line_breaks_ties_by_label() {
        let f = facets("#b #a #C #c2 [[Z]]\n");
        let line = FacetBar::default().line(&f);
        assert_eq!(labels(&line.pills), ["a", "b", "C", "c2", "z"]);
    }

    #[test]
    fn an_unpicked_heading_is_never_inline_and_a_picked_one_is() {
        let f = facets("# Head\n\n#a\n\n## Sub\n");
        let mut bar = FacetBar::default();
        assert_eq!(labels(&bar.line(&f).pills), ["a"]);
        bar.toggle(heading("sub"));
        let line = bar.line(&f);
        assert_eq!(labels(&line.pills), ["Sub", "a"]);
        assert!(line.pills[0].picked);
        assert_eq!(line.pills[0].facet.kind, FacetKind::Heading);
    }

    #[test]
    fn sections_counts_every_heading() {
        let f = facets(&many_headings(400));
        let mut bar = FacetBar::default();
        bar.toggle(heading("h123"));
        let line = bar.line(&f);
        assert_eq!(line.sections, 400);
        assert_eq!(labels(&line.pills), ["H123"]);
        assert_eq!(FacetBar::default().line(&facets("#a\n")).sections, 0);
    }

    #[test]
    fn grouped_lists_tags_then_links_and_leaves_headings_out() {
        let f = facets("# Head\n\n[[l]] #a #a #b\n");
        let groups = FacetBar::default().grouped(&f, "");
        let got: Vec<_> = groups.iter().map(|(k, p)| (*k, labels(p))).collect();
        assert_eq!(
            got,
            [
                (FacetKind::Tag, vec!["a", "b"]),
                (FacetKind::NoteLink, vec!["l"])
            ]
        );
    }

    #[test]
    fn grouped_excludes_picked_pills() {
        let f = facets("#a #b [[l]]\n");
        let mut bar = FacetBar::default();
        bar.toggle(tag("a"));
        let groups = bar.grouped(&f, "");
        assert_eq!(labels(&groups[0].1), ["b"]);
    }

    #[test]
    fn grouped_filters_with_the_fuzzy_match() {
        let f = facets("#plant [[plan]] #zzz #pn\n");
        let groups = FacetBar::default().grouped(&f, "pln");
        let got: Vec<_> = groups.iter().map(|(k, p)| (*k, labels(p))).collect();
        assert_eq!(
            got,
            [
                (FacetKind::Tag, vec!["plant"]),
                (FacetKind::NoteLink, vec!["plan"])
            ]
        );
    }

    #[test]
    fn grouped_leaves_out_an_empty_group() {
        let f = facets("#a [[l]]\n");
        let groups = FacetBar::default().grouped(&f, "l");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, FacetKind::NoteLink);
        assert!(FacetBar::default().grouped(&f, "q").is_empty());
    }

    #[test]
    fn fit_count_fits_what_it_can_and_reserves_the_more_chip() {
        // Three of 40 plus gaps (40+10+40+10+40 = 140) and the chip (10+30) = 180 fit; four do not.
        let widths = [40.0, 40.0, 40.0, 40.0, 40.0];
        assert_eq!(fit_count(&widths, 10.0, 180.0, 30.0, 0), 3);
        assert_eq!(fit_count(&widths, 10.0, 179.0, 30.0, 0), 2);
    }

    #[test]
    fn fit_count_returns_all_when_everything_fits_without_the_more_chip() {
        let widths = [40.0, 40.0, 40.0];
        // 40+10+40+10+40 = 140: fits with no chip even though a chip would not fit.
        assert_eq!(fit_count(&widths, 10.0, 140.0, 30.0, 0), 3);
        assert_eq!(fit_count(&widths, 10.0, 139.0, 30.0, 0), 2);
    }

    #[test]
    fn fit_count_never_goes_below_keep() {
        let widths = [40.0, 40.0, 40.0];
        assert_eq!(fit_count(&widths, 10.0, 5.0, 30.0, 2), 2);
        assert_eq!(fit_count(&widths, 10.0, 1000.0, 30.0, 2), 3);
        // keep beyond the number of pills is capped by it.
        assert_eq!(fit_count(&widths, 10.0, 5.0, 30.0, 9), 3);
    }

    #[test]
    fn fit_count_fits_nothing_when_the_more_chip_alone_is_wider_than_the_room() {
        let widths = [40.0, 40.0];
        // One pill and the chip (40+10+50) and both pills (90) overflow 30.
        assert_eq!(fit_count(&widths, 10.0, 30.0, 50.0, 0), 0);
    }

    #[test]
    fn fit_count_of_a_single_pill_wider_than_the_room() {
        assert_eq!(fit_count(&[200.0], 10.0, 100.0, 30.0, 0), 0);
        // A picked pill is kept even when it does not fit (the line clips).
        assert_eq!(fit_count(&[200.0], 10.0, 100.0, 30.0, 1), 1);
    }

    #[test]
    fn fit_count_returns_keep_when_fewer_than_keep_fit() {
        let widths = [40.0, 40.0, 40.0, 40.0];
        // Not even one pill with the chip (40+10+30 = 80) fits in 50.
        assert_eq!(fit_count(&widths, 10.0, 50.0, 30.0, 2), 2);
        assert_eq!(fit_count(&widths, 10.0, 80.0, 30.0, 2), 2);
    }

    #[test]
    fn line_skips_a_pick_the_note_no_longer_holds() {
        let f = facets("#a #b\n");
        let mut bar = FacetBar::default();
        // Not pruned yet: the line leaves the lost pick out.
        bar.set_picks(vec![tag("gone"), tag("b")]);
        let line = bar.line(&f);
        assert_eq!(labels(&line.pills), ["b", "a"]);
        assert!(line.pills[0].picked && !line.pills[1].picked);
    }

    #[test]
    fn fit_count_of_nothing_is_zero() {
        assert_eq!(fit_count(&[], 10.0, 100.0, 30.0, 0), 0);
        assert_eq!(fit_count(&[], 10.0, 100.0, 30.0, 3), 0);
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

    #[test]
    fn more_list_is_fuzzy_filtered_and_complete() {
        let mut text: String = (0..28).map(|i| format!("#t{i:02} ")).collect();
        text.push_str("#idea #idle\n");
        let f = facets(&text);
        let bar = FacetBar::default();
        assert_eq!(bar.more_list(&f, FacetKind::Tag, "").len(), 30);
        let found = bar.more_list(&f, FacetKind::Tag, "ide");
        let mut got = labels(&found);
        got.sort();
        assert_eq!(got, ["idea", "idle"]);
        assert!(bar.more_list(&f, FacetKind::Heading, "ide").is_empty());
    }

    fn link(key: &str) -> Facet {
        Facet::new(FacetKind::NoteLink, key)
    }

    fn heading(text: &str) -> Facet {
        Facet::new(FacetKind::Heading, text)
    }

    fn many_headings(n: usize) -> String {
        (0..n).map(|i| format!("# H{i:03}\n\n")).collect()
    }

    #[test]
    fn search_matches_across_kinds_in_kind_order() {
        let f = facets("# Plan day\n\n#plant and [[plan]] and #zzz\n");
        let found = FacetBar::default().search(&f, "plan");
        let got: Vec<_> = found
            .iter()
            .map(|p| (p.facet.kind, p.label.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (FacetKind::Tag, "plant"),
                (FacetKind::NoteLink, "plan"),
                (FacetKind::Heading, "Plan day"),
            ]
        );
    }

    #[test]
    fn search_with_an_empty_query_returns_everything() {
        let f = facets("# Head\n\nsome #a and [[plan]]\n");
        let found = FacetBar::default().search(&f, "");
        assert_eq!(labels(&found), ["a", "plan", "Head"]);
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
}
