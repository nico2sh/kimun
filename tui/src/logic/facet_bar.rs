//! The facet pill bar model: which facets of a note are picked, and the rows
//! of pills to show. Pure state over [`NoteFacets`], shared by every frontend.

use kimun_core::note::{Facet, FacetEntry, FacetKind, NoteFacets};

use super::text_match::match_ranges;

/// Pills shown in one row before the rest fold into "+N more".
pub const PILLS_PER_ROW: usize = 12;

const KINDS: [FacetKind; 3] = [FacetKind::Heading, FacetKind::Tag, FacetKind::NoteLink];

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

    /// One row per kind that has entries, in the order heading, tag, note link.
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
                let shown = PILLS_PER_ROW.max(all.iter().filter(|p| p.picked).count());
                let pills: Vec<Pill> = all.into_iter().take(shown).collect();
                let more = total - pills.len();
                Some(Row { kind, pills, more })
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
            [FacetKind::Heading, FacetKind::Tag, FacetKind::NoteLink]
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
        assert_eq!(labels(&rows[0].pills), ["Zed", "Alpha"]);
        assert_eq!(labels(&rows[1].pills), ["c", "b", "a"]);
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
}
