//! The saved-search breadcrumb shared by the Query panel and the Ctrl+K note
//! browser: which saved search the current query came from, with a `• edited`
//! marker once the live query diverges. Sticky provenance — it survives edits
//! and sorts, and is dropped only when the query becomes blank (host-defined:
//! an empty field, or the panel's default backlinks query) or another saved
//! search is expanded.
//!
//! Both hosts embed one of these and forward two events to it —
//! [`on_query_consumed`](SavedSearchBreadcrumb::on_query_consumed) after a list
//! keystroke, and [`set`](SavedSearchBreadcrumb::set) for a programmatic apply —
//! then read [`border_title`](SavedSearchBreadcrumb::border_title) at render.
//! The state machine lives here, not in the hosts, so the rule is defined once.

/// Provenance for a query loaded from a saved search.
struct Provenance {
    name: String,
    /// The stored query with its order directive stripped, precomputed once so
    /// the per-render edited check only has to strip the *live* query.
    stored_stripped: String,
}

#[derive(Default)]
pub struct SavedSearchBreadcrumb {
    pinned: Option<Provenance>,
}

impl SavedSearchBreadcrumb {
    /// Pin provenance for a freshly expanded saved search (or clear it when
    /// `name` is `None`). Used by the programmatic apply path (the Saved
    /// Searches modal). A blank `stored_query` is treated as "nothing to
    /// pin" so the breadcrumb never shows over an empty query.
    pub fn set(&mut self, name: Option<String>, stored_query: &str) {
        self.pinned = match name {
            Some(name) if !stored_query.trim().is_empty() => Some(Provenance {
                name,
                stored_stripped: strip(stored_query),
            }),
            _ => None,
        };
    }

    /// React to a list keystroke that was consumed. `accepted` is the name of
    /// a saved search just expanded (if any); `query` is the resulting live
    /// query; `query_is_blank` is the host's notion of "no active query" (an
    /// empty field, or the Query panel's default backlinks query).
    ///
    /// - accepted + non-blank query  → pin (a fresh expansion)
    /// - accepted but blank query     → clear (expanded to nothing)
    /// - no accept, blank query       → clear (field emptied / reset to default)
    /// - no accept, non-blank query   → keep (sticky; the `• edited` marker is
    ///   derived in [`label`](Self::label))
    pub fn on_query_consumed(
        &mut self,
        accepted: Option<String>,
        query: &str,
        query_is_blank: bool,
    ) {
        match accepted {
            Some(name) => self.set(if query_is_blank { None } else { Some(name) }, query),
            None if query_is_blank => self.pinned = None,
            None => {}
        }
    }

    /// The breadcrumb label for the searchbox border: the saved-search name,
    /// plus ` • edited` once `query` diverges from the stored query. The
    /// comparison ignores the order directive (and incidentally inner
    /// whitespace), so changing only the sort is not "edited". `None` when no
    /// saved search is active.
    pub fn label(&self, query: &str) -> Option<String> {
        let p = self.pinned.as_ref()?;
        Some(if strip(query) != p.stored_stripped {
            format!("{} • edited", p.name)
        } else {
            p.name.clone()
        })
    }

    /// The searchbox border title: the chevroned breadcrumb (`‹ name ›`) when a
    /// saved search is active, else the host's `fallback` (e.g. `" Query"`).
    pub fn border_title(&self, query: &str, fallback: &str) -> String {
        match self.label(query) {
            Some(label) => format!(" ‹ {label} › "),
            None => fallback.to_string(),
        }
    }
}

/// The query with its order directive removed — the canonical form the edited
/// check compares against (so a sort change does not read as an edit).
fn strip(query: &str) -> String {
    kimun_core::strip_order_directive(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned(stored: &str) -> SavedSearchBreadcrumb {
        let mut b = SavedSearchBreadcrumb::default();
        b.set(Some("todo".into()), stored);
        b
    }

    #[test]
    fn unedited_label_is_the_name() {
        assert_eq!(pinned("#todo").label("#todo").as_deref(), Some("todo"));
    }

    #[test]
    fn diverged_query_marks_edited() {
        assert_eq!(
            pinned("#todo").label("#todox").as_deref(),
            Some("todo • edited")
        );
    }

    #[test]
    fn order_directive_change_is_not_edited() {
        assert_eq!(
            pinned("#todo").label("#todo or:title").as_deref(),
            Some("todo")
        );
    }

    #[test]
    fn set_with_blank_query_does_not_pin() {
        let mut b = SavedSearchBreadcrumb::default();
        b.set(Some("todo".into()), "   ");
        assert_eq!(b.label("   "), None);
    }

    #[test]
    fn accept_with_blank_expansion_clears_rather_than_pins() {
        let mut b = SavedSearchBreadcrumb::default();
        // A saved search whose stored query is empty must not pin a breadcrumb
        // over the now-empty field.
        b.on_query_consumed(Some("empty".into()), "", true);
        assert_eq!(b.label(""), None);
    }

    #[test]
    fn blank_query_clears_sticky_breadcrumb() {
        let mut b = pinned("#todo");
        b.on_query_consumed(None, "", true);
        assert_eq!(b.label(""), None);
    }

    #[test]
    fn non_blank_edit_keeps_sticky_breadcrumb() {
        let mut b = pinned("#todo");
        b.on_query_consumed(None, "#todox", false);
        assert_eq!(b.label("#todox").as_deref(), Some("todo • edited"));
    }

    #[test]
    fn border_title_chevrons_label_else_fallback() {
        assert_eq!(
            pinned("#todo").border_title("#todo", " Query"),
            " ‹ todo › "
        );
        assert_eq!(
            SavedSearchBreadcrumb::default().border_title("#todo", " Query"),
            " Query"
        );
    }
}
