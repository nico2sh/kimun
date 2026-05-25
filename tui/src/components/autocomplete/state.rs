use super::TriggerKind;

/// Default number of suggestion rows visible at once. The popup never grows
/// beyond this regardless of available screen space.
pub const DEFAULT_MAX_VISIBLE_ROWS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Primary text shown in the row. For wikilinks this is the note name
    /// (the wikilink target); for hashtags it is the tag label.
    pub display: String,
    /// Optional dimmed text shown right-aligned in the row. Used for tag
    /// usage counts and note paths (when disambiguating same-name notes).
    pub secondary: Option<String>,
}

/// State machine for the autocomplete popup.
///
/// Owned by the host (editor or search box) via the controller. The
/// trigger context lives in `kind` + `query` + `anchor`; the visible
/// window is `scroll_offset..(scroll_offset+max_visible_rows)`. All scroll
/// math is encapsulated here so the widget and the host stay dumb.
#[derive(Debug, Clone)]
pub struct AutocompleteState {
    pub kind: TriggerKind,
    pub query: String,
    pub items: Vec<Suggestion>,
    pub highlighted: usize,
    pub scroll_offset: usize,
    pub max_visible_rows: usize,
    /// Screen anchor where the popup is rendered (column, row in cells).
    /// Host computes from the trigger byte offset.
    pub anchor: (u16, u16),
}

impl AutocompleteState {
    pub fn new(kind: TriggerKind, anchor: (u16, u16)) -> Self {
        Self {
            kind,
            query: String::new(),
            items: Vec::new(),
            highlighted: 0,
            scroll_offset: 0,
            max_visible_rows: DEFAULT_MAX_VISIBLE_ROWS,
            anchor,
        }
    }

    /// Replaces the suggestion list, snaps highlight to the first row, and
    /// resets the scroll window. Called every time core returns a new
    /// query result.
    pub fn set_items(&mut self, items: Vec<Suggestion>) {
        self.items = items;
        self.highlighted = 0;
        self.scroll_offset = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The currently highlighted suggestion, or `None` when the list is
    /// empty.
    pub fn selected(&self) -> Option<&Suggestion> {
        self.items.get(self.highlighted)
    }

    /// Inclusive-exclusive range of item indices currently visible in the
    /// popup. Always `≤ max_visible_rows` items.
    pub fn visible_window(&self) -> (usize, usize) {
        let start = self.scroll_offset.min(self.items.len());
        let end = (start + self.max_visible_rows).min(self.items.len());
        (start, end)
    }

    pub fn has_more_above(&self) -> bool {
        self.scroll_offset > 0
    }

    pub fn has_more_below(&self) -> bool {
        let (_, end) = self.visible_window();
        end < self.items.len()
    }

    /// Count of items hidden above the visible window.
    pub fn hidden_above(&self) -> usize {
        self.scroll_offset
    }

    /// Count of items hidden below the visible window.
    pub fn hidden_below(&self) -> usize {
        let (_, end) = self.visible_window();
        self.items.len().saturating_sub(end)
    }

    pub fn move_highlight_down(&mut self) {
        if self.items.is_empty() {
            return;
        }
        if self.highlighted + 1 < self.items.len() {
            self.highlighted += 1;
            self.ensure_visible();
        }
    }

    pub fn move_highlight_up(&mut self) {
        if self.highlighted > 0 {
            self.highlighted -= 1;
            self.ensure_visible();
        }
    }

    pub fn page_down(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let step = self.max_visible_rows.max(1);
        self.highlighted = (self.highlighted + step).min(self.items.len() - 1);
        self.ensure_visible();
    }

    pub fn page_up(&mut self) {
        let step = self.max_visible_rows.max(1);
        self.highlighted = self.highlighted.saturating_sub(step);
        self.ensure_visible();
    }

    pub fn home(&mut self) {
        self.highlighted = 0;
        self.ensure_visible();
    }

    pub fn end(&mut self) {
        if !self.items.is_empty() {
            self.highlighted = self.items.len() - 1;
            self.ensure_visible();
        }
    }

    /// Slides the scroll window so the highlighted row sits inside it.
    /// Keeps the window stable when the highlight is already visible.
    fn ensure_visible(&mut self) {
        let window = self.max_visible_rows.max(1);
        if self.highlighted < self.scroll_offset {
            self.scroll_offset = self.highlighted;
        } else if self.highlighted >= self.scroll_offset + window {
            self.scroll_offset = self.highlighted + 1 - window;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: &str) -> Suggestion {
        Suggestion {
            display: n.to_string(),
            secondary: None,
        }
    }

    fn state_with(n: usize, max_rows: usize) -> AutocompleteState {
        let mut st = AutocompleteState::new(TriggerKind::Hashtag, (0, 0));
        st.max_visible_rows = max_rows;
        st.set_items((0..n).map(|i| s(&format!("item{i}"))).collect());
        st
    }

    #[test]
    fn empty_state_has_no_selection() {
        let st = state_with(0, 8);
        assert!(st.selected().is_none());
        assert!(!st.has_more_above());
        assert!(!st.has_more_below());
    }

    #[test]
    fn fits_in_window_shows_no_overflow_indicators() {
        let st = state_with(3, 8);
        assert!(!st.has_more_above());
        assert!(!st.has_more_below());
        assert_eq!(st.visible_window(), (0, 3));
    }

    #[test]
    fn overflow_indicator_only_below_at_top() {
        let st = state_with(30, 8);
        assert!(!st.has_more_above());
        assert!(st.has_more_below());
        assert_eq!(st.hidden_below(), 22);
        assert_eq!(st.visible_window(), (0, 8));
    }

    #[test]
    fn arrow_down_inside_window_does_not_scroll() {
        let mut st = state_with(30, 8);
        st.move_highlight_down();
        assert_eq!(st.highlighted, 1);
        assert_eq!(st.scroll_offset, 0);
    }

    #[test]
    fn arrow_down_past_window_scrolls() {
        let mut st = state_with(30, 8);
        for _ in 0..8 {
            st.move_highlight_down();
        }
        assert_eq!(st.highlighted, 8);
        assert_eq!(st.scroll_offset, 1);
        assert!(st.has_more_above());
        assert!(st.has_more_below());
    }

    #[test]
    fn scrolling_back_to_top_clears_top_indicator() {
        let mut st = state_with(30, 8);
        for _ in 0..10 {
            st.move_highlight_down();
        }
        assert!(st.has_more_above());
        for _ in 0..20 {
            st.move_highlight_up();
        }
        assert_eq!(st.highlighted, 0);
        assert_eq!(st.scroll_offset, 0);
        assert!(!st.has_more_above());
        assert!(st.has_more_below());
    }

    #[test]
    fn page_down_jumps_by_window_size() {
        let mut st = state_with(30, 8);
        st.page_down();
        assert_eq!(st.highlighted, 8);
        st.page_down();
        assert_eq!(st.highlighted, 16);
    }

    #[test]
    fn page_down_clamps_at_last_item() {
        let mut st = state_with(10, 8);
        st.page_down();
        st.page_down();
        st.page_down();
        assert_eq!(st.highlighted, 9);
    }

    #[test]
    fn page_up_clamps_at_zero() {
        let mut st = state_with(10, 8);
        st.page_up();
        assert_eq!(st.highlighted, 0);
        assert_eq!(st.scroll_offset, 0);
    }

    #[test]
    fn end_jumps_to_last_item_and_scrolls() {
        let mut st = state_with(30, 8);
        st.end();
        assert_eq!(st.highlighted, 29);
        assert_eq!(st.scroll_offset, 22);
        assert!(st.has_more_above());
        assert!(!st.has_more_below());
    }

    #[test]
    fn home_jumps_to_first_item_and_unscrolls() {
        let mut st = state_with(30, 8);
        st.end();
        st.home();
        assert_eq!(st.highlighted, 0);
        assert_eq!(st.scroll_offset, 0);
    }

    #[test]
    fn set_items_resets_selection_and_scroll() {
        let mut st = state_with(30, 8);
        st.end();
        st.set_items((0..3).map(|i| s(&format!("x{i}"))).collect());
        assert_eq!(st.highlighted, 0);
        assert_eq!(st.scroll_offset, 0);
    }

    #[test]
    fn highlight_stays_visible_after_navigation() {
        let mut st = state_with(30, 8);
        for _ in 0..15 {
            st.move_highlight_down();
        }
        let (start, end) = st.visible_window();
        assert!(st.highlighted >= start && st.highlighted < end);
    }
}
