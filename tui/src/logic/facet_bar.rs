//! The facet pill bar model: which facets of a note are picked, and the rows
//! of pills to show. Pure state over [`NoteFacets`], shared by every frontend.

use kimun_core::note::{Facet, FacetEntry, FacetKind, NoteFacets};

use super::text_match::match_ranges;

/// Pills shown in one row before the rest fold into "+N more".
pub const PILLS_PER_ROW: usize = 12;

const KINDS: [FacetKind; 3] = [FacetKind::Tag, FacetKind::NoteLink, FacetKind::Heading];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pill {
    pub facet: Facet,
    pub label: String,
    pub count: usize,
    pub picked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: FacetKind,
    pub pills: Vec<Pill>,
    /// Pills of this kind not shown in `pills`.
    pub more: usize,
    /// True for the heading row of a note with more than [`PILLS_PER_ROW`]
    /// headings: `pills` then holds only the picked ones and `more` counts
    /// the unpicked ones.
    pub collapsed: bool,
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

    /// One row per kind that has entries, in the order tag, note link, heading.
    pub fn rows(&self, facets: &NoteFacets) -> Vec<Row> {
        KINDS
            .into_iter()
            .filter_map(|kind| {
                let all = self.ordered(facets, kind);
                if all.is_empty() {
                    return None;
                }
                let total = all.len();
                // Picked pills come first, so they always fit unless the picks
                // alone overflow the cap; they are never hidden.
                let picked = all.iter().filter(|p| p.picked).count();
                let collapsed = kind == FacetKind::Heading && total > PILLS_PER_ROW;
                let shown = if collapsed {
                    picked
                } else {
                    PILLS_PER_ROW.max(picked)
                };
                let pills: Vec<Pill> = all.into_iter().take(shown).collect();
                let more = total - pills.len();
                Some(Row {
                    kind,
                    pills,
                    more,
                    collapsed,
                })
            })
            .collect()
    }

    /// The full list of one kind for the "+N more" popover, fuzzy-filtered by
    /// `query`. The query's characters must appear in the label in order
    /// (case-insensitive); it is matched one character at a time with
    /// [`match_ranges`].
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
    fn rows_are_per_kind_and_skip_empty_kinds() {
        let f = facets("# Head\n\nsome #a and [[plan]]\n");
        let rows = FacetBar::default().rows(&f);
        let kinds: Vec<_> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [FacetKind::Tag, FacetKind::NoteLink, FacetKind::Heading]
        );
        let f = facets("just #a\n");
        let rows = FacetBar::default().rows(&f);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, FacetKind::Tag);
        assert_eq!(rows[0].more, 0);
    }

    #[test]
    fn tags_sort_by_count_headings_keep_document_order() {
        let f = facets("# Zed\n\n#b #a\n\n#c #c #c\n\n## Alpha\n\n#b\n");
        let rows = FacetBar::default().rows(&f);
        assert_eq!(labels(&rows[0].pills), ["c", "b", "a"]);
        assert_eq!(labels(&rows[1].pills), ["Zed", "Alpha"]);
    }

    #[test]
    fn picked_pills_come_first_and_always_show() {
        let f = facets("#a #a #b\n");
        let mut bar = FacetBar::default();
        bar.toggle(tag("b"));
        let row = &bar.rows(&f)[0];
        assert_eq!(labels(&row.pills), ["b", "a"]);
        assert!(row.pills[0].picked && !row.pills[1].picked);

        let many: String = (0..20).map(|i| format!("#t{i:02} ")).collect();
        let f = facets(&many);
        let mut bar = FacetBar::default();
        bar.toggle(tag("t19"));
        let row = &bar.rows(&f)[0];
        assert_eq!(row.pills[0].label, "t19");
        assert_eq!(row.pills.len(), PILLS_PER_ROW);
    }

    #[test]
    fn a_row_caps_at_pills_per_row_and_counts_the_rest() {
        let many: String = (0..30).map(|i| format!("#t{i:02} ")).collect();
        let f = facets(&many);
        let row = &FacetBar::default().rows(&f)[0];
        assert_eq!(row.pills.len(), PILLS_PER_ROW);
        assert_eq!(row.more, 30 - PILLS_PER_ROW);
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
    fn rows_are_ordered_tag_link_heading() {
        let f = facets("# Head\n\nsome #a and [[plan]]\n");
        let kinds: Vec<_> = FacetBar::default()
            .rows(&f)
            .iter()
            .map(|r| r.kind)
            .collect();
        assert_eq!(
            kinds,
            [FacetKind::Tag, FacetKind::NoteLink, FacetKind::Heading]
        );
    }

    #[test]
    fn the_heading_row_collapses_over_the_pill_cap_and_keeps_picked_headings() {
        let f = facets(&many_headings(400));
        let mut bar = FacetBar::default();
        bar.toggle(heading("H123"));
        let rows = bar.rows(&f);
        let row = &rows[0];
        assert_eq!(row.kind, FacetKind::Heading);
        assert!(row.collapsed);
        assert_eq!(labels(&row.pills), ["H123"]);
        assert_eq!(row.more, 399);
        // The full list still opens.
        assert_eq!(bar.more_list(&f, FacetKind::Heading, "").len(), 400);
    }

    #[test]
    fn a_short_heading_row_is_not_collapsed() {
        let f = facets(&many_headings(PILLS_PER_ROW));
        let row = &FacetBar::default().rows(&f)[0];
        assert!(!row.collapsed);
        assert_eq!(row.pills.len(), PILLS_PER_ROW);
        assert_eq!(row.more, 0);
        // Only headings collapse; a long tag row is capped, not collapsed.
        let many: String = (0..30).map(|i| format!("#t{i:02} ")).collect();
        assert!(!FacetBar::default().rows(&facets(&many))[0].collapsed);
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
