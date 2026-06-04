//! `PanelOrder` — the pure focus/order/visibility state machine for the
//! editor screen's persistent **Panels**. Keyed only on `PanelKind`, so it
//! carries no vault or heavy component state and is testable in isolation.
//! `PanelSet` (below) composes it with the concrete panels.

use ratatui::Frame;
use ratatui::crossterm::event::MouseEventKind;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::widgets::{Block, Borders};

use crate::components::Component;
use crate::components::backlinks_panel::QueryPanel;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::components::panel::PanelKind;
use crate::components::sidebar::SidebarComponent;
use crate::components::text_editor::TextEditorComponent;
use crate::settings::themes::Theme;

struct Slot {
    kind: PanelKind,
    visible: bool,
}

/// Ordered panels + which one is focused. Order is config-driven and may be
/// permuted at runtime; focus tracks a panel by kind, not by index, so it
/// survives reordering. The editor panel is always visible.
pub struct PanelOrder {
    slots: Vec<Slot>,
    focus: usize,
}

impl PanelOrder {
    /// Default layout: sidebar (visible) → editor (visible, focused) → Query
    /// panel (hidden).
    pub fn new() -> Self {
        let slots = vec![
            Slot {
                kind: PanelKind::Sidebar,
                visible: true,
            },
            Slot {
                kind: PanelKind::Editor,
                visible: true,
            },
            Slot {
                kind: PanelKind::Query,
                visible: false,
            },
        ];
        let focus = slots
            .iter()
            .position(|s| s.kind == PanelKind::Editor)
            .expect("editor slot present");
        Self { slots, focus }
    }

    /// The currently focused panel.
    pub fn focused(&self) -> PanelKind {
        self.slots[self.focus].kind
    }

    /// The panel one step left of focus in the current order, or `None` if
    /// focus is already at the left end.
    pub fn prev_kind(&self) -> Option<PanelKind> {
        self.focus.checked_sub(1).map(|i| self.slots[i].kind)
    }

    /// The panel one step right of focus in the current order, or `None` if
    /// focus is already at the right end.
    pub fn next_kind(&self) -> Option<PanelKind> {
        self.slots.get(self.focus + 1).map(|s| s.kind)
    }

    /// Move focus to `kind`. No-op if the kind is not present.
    pub fn focus(&mut self, kind: PanelKind) {
        if let Some(i) = self.slots.iter().position(|s| s.kind == kind) {
            self.focus = i;
        }
    }

    /// Whether `kind` is currently visible.
    pub fn is_visible(&self, kind: PanelKind) -> bool {
        self.slots
            .iter()
            .find(|s| s.kind == kind)
            .is_some_and(|s| s.visible)
    }

    /// Reveal `kind`.
    pub fn show(&mut self, kind: PanelKind) {
        if let Some(s) = self.slots.iter_mut().find(|s| s.kind == kind) {
            s.visible = true;
        }
    }

    /// Hide `kind`. The editor is always visible, so hiding it is a no-op.
    pub fn hide(&mut self, kind: PanelKind) {
        if kind == PanelKind::Editor {
            return;
        }
        if let Some(s) = self.slots.iter_mut().find(|s| s.kind == kind) {
            s.visible = false;
        }
        // If focus was on the panel we just hid, move it to the nearest
        // visible panel (the editor is always visible, so a target exists).
        if !self.slots[self.focus].visible {
            self.focus = self.nearest_visible(self.focus);
        }
    }

    /// The visible panels in their current left→right order. Drives the
    /// render layout: each panel contributes one column in this sequence.
    pub fn visible_in_order(&self) -> Vec<PanelKind> {
        self.slots
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.kind)
            .collect()
    }

    /// Reorder the panels to match `order` (a permutation of the panel kinds).
    /// Each panel keeps its visibility, and focus stays on the same panel by
    /// kind — so a config reorder needs no other state changes. Kinds omitted
    /// from `order` keep their relative position at the end.
    pub fn set_order(&mut self, order: &[PanelKind]) {
        let focused_kind = self.focused();
        let mut new: Vec<Slot> = Vec::with_capacity(self.slots.len());
        for &k in order {
            if let Some(pos) = self.slots.iter().position(|s| s.kind == k) {
                new.push(self.slots.remove(pos));
            }
        }
        new.append(&mut self.slots);
        self.slots = new;
        self.focus = self
            .slots
            .iter()
            .position(|s| s.kind == focused_kind)
            .unwrap_or(0);
    }

    /// Index of the nearest visible slot, searching outward from `from`.
    /// Falls back to `from` if somehow nothing is visible (cannot happen —
    /// the editor is always visible).
    fn nearest_visible(&self, from: usize) -> usize {
        let n = self.slots.len();
        (1..n)
            .flat_map(|d| [from.checked_sub(d), Some(from + d).filter(|&i| i < n)])
            .flatten()
            .find(|&i| self.slots[i].visible)
            .unwrap_or(from)
    }
}

impl Default for PanelOrder {
    fn default() -> Self {
        Self::new()
    }
}

/// Column width a panel occupies when laid out. The editor fills; the sidebar
/// and Query panel are fixed-width. (A future config could override these.)
fn panel_column(kind: PanelKind) -> Constraint {
    match kind {
        PanelKind::Sidebar => Constraint::Length(30),
        PanelKind::Editor => Constraint::Min(0),
        PanelKind::Query => Constraint::Length(40),
    }
}

/// Lay `visible` out left→right as one column per panel. Shared by `render`
/// and mouse hit-testing so clicks are always tested against the same rects
/// the panels were drawn into.
fn layout_columns(visible: &[PanelKind], area: Rect) -> Vec<(PanelKind, Rect)> {
    let constraints: Vec<Constraint> = visible.iter().map(|k| panel_column(*k)).collect();
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    visible
        .iter()
        .copied()
        .zip(columns.iter().copied())
        .collect()
}

/// The panel whose column contains the given screen cell, if any.
fn kind_at(columns: &[(PanelKind, Rect)], column: u16, row: u16) -> Option<PanelKind> {
    columns
        .iter()
        .find(|(_, rect)| rect.contains(Position::new(column, row)))
        .map(|(kind, _)| *kind)
}

/// The editor screen's persistent **Panels** — the sidebar, the editor, and
/// the **Query panel** — plus the `PanelOrder` that decides their order,
/// visibility, and which one is focused. Routes input and render to the
/// focused / visible panels; the host (`EditorScreen`) reaches a specific
/// panel through the typed accessors for panel-specific calls.
pub struct PanelSet {
    order: PanelOrder,
    sidebar: SidebarComponent,
    editor: TextEditorComponent,
    query: QueryPanel,
    /// The column each visible panel was drawn into on the last render —
    /// the single source of truth for mouse hit-testing. Empty until the
    /// first render.
    column_rects: Vec<(PanelKind, Rect)>,
}

impl PanelSet {
    pub fn from_panels(
        sidebar: SidebarComponent,
        editor: TextEditorComponent,
        query: QueryPanel,
    ) -> Self {
        Self {
            order: PanelOrder::new(),
            sidebar,
            editor,
            query,
            column_rects: Vec::new(),
        }
    }

    // ── Order / focus / visibility (delegate to PanelOrder) ─────────────────

    pub fn focused(&self) -> PanelKind {
        self.order.focused()
    }

    pub fn focused_label(&self) -> &'static str {
        self.order.focused().label()
    }

    pub fn prev_kind(&self) -> Option<PanelKind> {
        self.order.prev_kind()
    }

    pub fn next_kind(&self) -> Option<PanelKind> {
        self.order.next_kind()
    }

    pub fn is_visible(&self, kind: PanelKind) -> bool {
        self.order.is_visible(kind)
    }

    pub fn show(&mut self, kind: PanelKind) {
        self.order.show(kind);
    }

    pub fn hide(&mut self, kind: PanelKind) {
        self.order.hide(kind);
    }

    pub fn set_order(&mut self, order: &[PanelKind]) {
        self.order.set_order(order);
    }

    /// Move focus to `kind`. Any transition away from the editor closes its
    /// autocomplete popup so it doesn't linger while another panel owns input.
    pub fn focus(&mut self, kind: PanelKind) {
        if kind != PanelKind::Editor {
            self.editor.close_autocomplete();
        }
        self.order.focus(kind);
    }

    // ── Typed accessors for panel-specific calls ───────────────────────────

    pub fn sidebar(&self) -> &SidebarComponent {
        &self.sidebar
    }
    pub fn sidebar_mut(&mut self) -> &mut SidebarComponent {
        &mut self.sidebar
    }
    pub fn editor(&self) -> &TextEditorComponent {
        &self.editor
    }
    pub fn editor_mut(&mut self) -> &mut TextEditorComponent {
        &mut self.editor
    }
    pub fn query(&self) -> &QueryPanel {
        &self.query
    }
    pub fn query_mut(&mut self) -> &mut QueryPanel {
        &mut self.query
    }

    // ── Routing ────────────────────────────────────────────────────────────

    /// Footer hints for the focused panel.
    pub fn focused_hints(&self) -> Vec<(String, String)> {
        match self.order.focused() {
            PanelKind::Sidebar => self.sidebar.hint_shortcuts(),
            PanelKind::Editor => self.editor.hint_shortcuts(),
            PanelKind::Query => self.query.hint_shortcuts(),
        }
    }

    /// Route an input event to the focused panel. The Query panel speaks
    /// `handle_key`, so non-key events are not delivered to it.
    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match self.order.focused() {
            PanelKind::Sidebar => self.sidebar.handle_input(event, tx),
            PanelKind::Editor => self.editor.handle_input(event, tx),
            PanelKind::Query => {
                if let InputEvent::Key(key) = event {
                    self.query.handle_key(key, tx)
                } else {
                    EventState::NotConsumed
                }
            }
        }
    }

    /// Route a mouse event by hit-testing the panel columns from the last
    /// render. A button-down click focuses the panel under the cursor — the
    /// one consistent click-to-focus rule for every panel — and the event is
    /// then forwarded to that panel for its internal behavior (cursor
    /// placement, list selection, scrolling). Events outside every column
    /// (or before the first render) are not consumed.
    pub fn handle_mouse(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Mouse(mouse) = event else {
            return EventState::NotConsumed;
        };
        let Some(kind) = kind_at(&self.column_rects, mouse.column, mouse.row) else {
            return EventState::NotConsumed;
        };
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            self.focus(kind);
        }
        match kind {
            PanelKind::Sidebar => {
                self.sidebar.handle_input(event, tx);
            }
            PanelKind::Editor => {
                self.editor.handle_input(event, tx);
            }
            // The Query panel has no internal mouse behavior (yet).
            PanelKind::Query => {}
        }
        EventState::Consumed
    }

    /// Lay the visible panels out left→right in their current order and render
    /// each. `show_focus` is false while an overlay is open, so no panel draws
    /// its focused highlight under the overlay.
    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, show_focus: bool) {
        let visible = self.order.visible_in_order();
        if visible.is_empty() {
            return;
        }
        let columns = layout_columns(&visible, area);
        self.column_rects = columns.clone();

        let focused = self.order.focused();
        for (kind, rect) in &columns {
            let is_focused = show_focus && *kind == focused;
            let rect = *rect;
            match kind {
                PanelKind::Sidebar => self.sidebar.render(f, rect, theme, is_focused),
                PanelKind::Query => self.query.render(f, rect, theme, is_focused),
                PanelKind::Editor => {
                    // The editor's frame is drawn here (not by the component) so
                    // the dirty marker and focus border live with the layout.
                    let title = if self.editor.is_dirty() {
                        "Editor [+]"
                    } else {
                        "Editor"
                    };
                    let block = Block::default()
                        .title(title)
                        .borders(Borders::ALL)
                        .border_style(theme.border_style(is_focused))
                        .style(theme.base_style());
                    let inner = block.inner(rect);
                    f.render_widget(block, rect);
                    self.editor.render(f, inner, theme, is_focused);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use crate::test_support::{mouse_down_at, temp_vault};
    use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    use tokio::sync::mpsc::unbounded_channel;

    async fn make_panel_set() -> PanelSet {
        let vault = temp_vault("panelset").await;
        vault.validate_and_init().await.unwrap();
        let settings = AppSettings::default();
        let sidebar = SidebarComponent::new(
            settings.key_bindings.clone(),
            vault.clone(),
            settings.icons(),
            &settings,
        );
        let editor = TextEditorComponent::new(settings.key_bindings.clone(), &settings);
        let query = QueryPanel::new(vault, settings.key_bindings.clone());
        PanelSet::from_panels(sidebar, editor, query)
    }

    /// Lay the visible panels out over a fixed area, as a render would.
    fn lay_out(panels: &mut PanelSet) {
        panels.column_rects =
            layout_columns(&panels.order.visible_in_order(), Rect::new(0, 0, 120, 40));
    }

    fn scroll_at(col: u16, row: u16) -> InputEvent {
        InputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// A button-down click focuses the panel whose column it lands in — for
    /// every panel, in any prior focus state.
    #[tokio::test]
    async fn click_focuses_panel_under_cursor() {
        let mut panels = make_panel_set().await;
        panels.show(PanelKind::Query);
        lay_out(&mut panels);
        let (tx, _rx) = unbounded_channel();

        assert_eq!(panels.focused(), PanelKind::Editor);
        // Sidebar (0..30) | Editor (30..80) | Query (80..120).
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(90, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.focused(), PanelKind::Query);
        // Regression: a click on the editor must focus it even while the
        // Query panel is focused.
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(50, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.focused(), PanelKind::Editor);
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(5, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.focused(), PanelKind::Sidebar);
    }

    /// Before the first render no rects exist, and clicks outside every
    /// column must not move focus.
    #[tokio::test]
    async fn click_outside_panels_changes_nothing() {
        let mut panels = make_panel_set().await;
        let (tx, _rx) = unbounded_channel();

        // No render yet → no rects → nothing to hit.
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(10, 10), &tx),
            EventState::NotConsumed
        );
        assert_eq!(panels.focused(), PanelKind::Editor);

        lay_out(&mut panels);
        // Query is hidden, so its default column (x ≥ 80) belongs to the
        // editor; only cells outside the laid-out area miss.
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(10, 50), &tx),
            EventState::NotConsumed
        );
        assert_eq!(panels.focused(), PanelKind::Editor);
    }

    /// Scrolling routes to the panel under the cursor without stealing focus.
    #[tokio::test]
    async fn scroll_does_not_change_focus() {
        let mut panels = make_panel_set().await;
        lay_out(&mut panels);
        let (tx, _rx) = unbounded_channel();

        assert_eq!(
            panels.handle_mouse(&scroll_at(5, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.focused(), PanelKind::Editor);
    }

    #[test]
    fn default_focus_is_editor() {
        let order = PanelOrder::new();
        assert_eq!(order.focused(), PanelKind::Editor);
    }

    #[test]
    fn adjacent_kinds_follow_order_and_clamp_at_ends() {
        let order = PanelOrder::new();
        // focus = editor (middle)
        assert_eq!(order.prev_kind(), Some(PanelKind::Sidebar));
        assert_eq!(order.next_kind(), Some(PanelKind::Query));
    }

    #[test]
    fn focus_moves_and_clamps_at_ends() {
        let mut order = PanelOrder::new();
        order.focus(PanelKind::Sidebar);
        assert_eq!(order.focused(), PanelKind::Sidebar);
        assert_eq!(order.prev_kind(), None);
        assert_eq!(order.next_kind(), Some(PanelKind::Editor));

        order.focus(PanelKind::Query);
        assert_eq!(order.prev_kind(), Some(PanelKind::Editor));
        assert_eq!(order.next_kind(), None);
    }

    #[test]
    fn show_hide_toggles_visibility_except_editor() {
        let mut order = PanelOrder::new();
        assert!(order.is_visible(PanelKind::Sidebar));
        assert!(!order.is_visible(PanelKind::Query));

        order.show(PanelKind::Query);
        assert!(order.is_visible(PanelKind::Query));
        order.hide(PanelKind::Sidebar);
        assert!(!order.is_visible(PanelKind::Sidebar));

        // Editor cannot be hidden.
        order.hide(PanelKind::Editor);
        assert!(order.is_visible(PanelKind::Editor));
    }

    #[test]
    fn hiding_focused_panel_moves_focus_to_visible() {
        let mut order = PanelOrder::new();
        order.focus(PanelKind::Sidebar);
        order.hide(PanelKind::Sidebar);
        // Focus cannot stay on a hidden panel.
        assert!(order.is_visible(order.focused()));
        assert_eq!(order.focused(), PanelKind::Editor);
    }

    #[test]
    fn set_order_permutes_keeping_focus_and_visibility() {
        let mut order = PanelOrder::new();
        // Sidebar visible, Query hidden, focus on Editor.
        order.set_order(&[PanelKind::Query, PanelKind::Editor, PanelKind::Sidebar]);

        // Focus tracks the same panel by kind.
        assert_eq!(order.focused(), PanelKind::Editor);
        // Adjacency now follows the new order.
        assert_eq!(order.prev_kind(), Some(PanelKind::Query));
        assert_eq!(order.next_kind(), Some(PanelKind::Sidebar));
        // Visibility is preserved per kind across the reorder.
        assert!(order.is_visible(PanelKind::Sidebar));
        assert!(!order.is_visible(PanelKind::Query));
    }

    #[test]
    fn layout_columns_splits_area_in_panel_order() {
        let area = Rect::new(0, 0, 120, 40);
        let visible = [PanelKind::Sidebar, PanelKind::Editor, PanelKind::Query];
        let columns = layout_columns(&visible, area);

        assert_eq!(columns.len(), 3);
        // Order is preserved and widths follow panel_column: sidebar 30,
        // Query 40, editor takes the rest.
        assert_eq!(columns[0].0, PanelKind::Sidebar);
        assert_eq!(columns[0].1.width, 30);
        assert_eq!(columns[1].0, PanelKind::Editor);
        assert_eq!(columns[1].1.width, 50);
        assert_eq!(columns[2].0, PanelKind::Query);
        assert_eq!(columns[2].1.width, 40);
        // Columns tile the area left→right.
        assert_eq!(columns[0].1.x, 0);
        assert_eq!(columns[1].1.x, 30);
        assert_eq!(columns[2].1.x, 80);
    }

    #[test]
    fn kind_at_hit_tests_panel_columns() {
        let area = Rect::new(0, 0, 120, 40);
        let visible = [PanelKind::Sidebar, PanelKind::Editor, PanelKind::Query];
        let columns = layout_columns(&visible, area);

        assert_eq!(kind_at(&columns, 0, 0), Some(PanelKind::Sidebar));
        assert_eq!(kind_at(&columns, 29, 10), Some(PanelKind::Sidebar));
        assert_eq!(kind_at(&columns, 30, 10), Some(PanelKind::Editor));
        assert_eq!(kind_at(&columns, 79, 39), Some(PanelKind::Editor));
        assert_eq!(kind_at(&columns, 80, 0), Some(PanelKind::Query));
        assert_eq!(kind_at(&columns, 119, 39), Some(PanelKind::Query));
        // Outside the laid-out area.
        assert_eq!(kind_at(&columns, 120, 10), None);
        assert_eq!(kind_at(&columns, 10, 40), None);
        // No columns yet (before the first render).
        assert_eq!(kind_at(&[], 10, 10), None);
    }

    #[test]
    fn visible_in_order_skips_hidden_and_follows_order() {
        let mut order = PanelOrder::new();
        assert_eq!(
            order.visible_in_order(),
            vec![PanelKind::Sidebar, PanelKind::Editor]
        );
        order.show(PanelKind::Query);
        order.set_order(&[PanelKind::Query, PanelKind::Editor, PanelKind::Sidebar]);
        assert_eq!(
            order.visible_in_order(),
            vec![PanelKind::Query, PanelKind::Editor, PanelKind::Sidebar]
        );
    }
}
