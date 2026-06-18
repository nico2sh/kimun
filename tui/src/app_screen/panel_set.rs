//! `PanelOrder` — the pure focus/order/visibility state machine for the
//! editor screen's persistent **Panels**. Keyed only on `PanelKind`, so it
//! carries no vault or heavy component state and is testable in isolation.
//! `PanelSet` (below) composes it with the concrete panels.

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};

use crate::components::Component;
use crate::components::activity_rail::{ActivityRail, RAIL_WIDTH};
use crate::components::attachment_view::AttachmentView;
use crate::components::drawer::{DrawerHost, DrawerView};
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::components::panel::{PanelKind, panel_block};
use crate::components::query_panel::QueryPanel;
use crate::components::sidebar::SidebarComponent;
use crate::components::text_editor::TextEditorComponent;
use crate::settings::themes::Theme;

/// Default drawer width in columns. Resizable at runtime by dragging the
/// drawer↔editor divider.
const DEFAULT_DRAWER_WIDTH: u16 = 34;
/// Narrowest the drawer can be dragged.
const MIN_DRAWER_WIDTH: u16 = 20;
/// Columns that must always remain for the editor when dragging the divider.
const MIN_EDITOR_WIDTH: u16 = 20;

struct Slot {
    kind: PanelKind,
    visible: bool,
}

/// Ordered panels + which one is focused. The layout is fixed left→right:
/// rail → drawer → editor. The rail and the editor are always visible; the
/// drawer toggles. Focus tracks a panel by kind and cycles over the visible
/// panels, wrapping at both ends.
pub struct PanelOrder {
    slots: Vec<Slot>,
    focus: usize,
}

impl PanelOrder {
    /// Default layout: rail → drawer (visible) → editor (focused).
    pub fn new() -> Self {
        let slots = vec![
            Slot {
                kind: PanelKind::Rail,
                visible: true,
            },
            Slot {
                kind: PanelKind::Drawer,
                visible: true,
            },
            Slot {
                kind: PanelKind::Editor,
                visible: true,
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

    /// The visible panel one step left of focus, wrapping at the left end.
    pub fn prev_kind(&self) -> Option<PanelKind> {
        self.step(|i, n| (i + n - 1) % n)
    }

    /// The visible panel one step right of focus, wrapping at the right end.
    pub fn next_kind(&self) -> Option<PanelKind> {
        self.step(|i, n| (i + 1) % n)
    }

    /// Step through the *visible* panels from the focused one.
    fn step(&self, advance: impl Fn(usize, usize) -> usize) -> Option<PanelKind> {
        let visible = self.visible_in_order();
        let n = visible.len();
        if n < 2 {
            return None;
        }
        let i = visible.iter().position(|&k| k == self.focused())?;
        Some(visible[advance(i, n)])
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

    /// Hide `kind`. The rail and the editor are always visible, so hiding
    /// them is a no-op.
    pub fn hide(&mut self, kind: PanelKind) {
        if kind == PanelKind::Editor || kind == PanelKind::Rail {
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

/// Column width a panel occupies when laid out. The rail is fixed, the
/// drawer is runtime-resizable, and the editor fills the remainder.
fn panel_column(kind: PanelKind, drawer_width: u16) -> Constraint {
    match kind {
        PanelKind::Rail => Constraint::Length(RAIL_WIDTH),
        PanelKind::Drawer => Constraint::Length(drawer_width),
        PanelKind::Editor => Constraint::Min(0),
    }
}

/// Lay `visible` out left→right as one column per panel. Shared by `render`
/// and mouse hit-testing so clicks are always tested against the same rects
/// the panels were drawn into.
fn layout_columns(visible: &[PanelKind], area: Rect, drawer_width: u16) -> Vec<(PanelKind, Rect)> {
    let constraints: Vec<Constraint> = visible
        .iter()
        .map(|k| panel_column(*k, drawer_width))
        .collect();
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

/// The editor screen's persistent **Panels** — the activity rail, the single
/// drawer, and the editor — plus the `PanelOrder` that decides visibility and
/// which one is focused. Routes input and render to the focused / visible
/// panels; the host (`EditorScreen`) reaches a specific drawer view through
/// the typed accessors for view-specific calls.
pub struct PanelSet {
    order: PanelOrder,
    rail: ActivityRail,
    drawer: DrawerHost,
    /// The note editor. Retained for the whole screen lifetime — including
    /// while an attachment is shown — so its backend (e.g. a live nvim
    /// process) and undo history survive the round trip. See ADR-0017.
    editor: TextEditorComponent,
    /// When `Some`, the editor *area* shows this read-only attachment view in
    /// place of the note editor (which stays dormant underneath). This `Option`
    /// is the editor area's sum type: Text (`None`) xor Attachment (`Some`).
    attachment: Option<AttachmentView>,
    /// Current drawer width in columns (divider-draggable).
    drawer_width: u16,
    /// Whether a divider drag is in progress.
    dragging_divider: bool,
    /// The column each visible panel was drawn into on the last render —
    /// the single source of truth for mouse hit-testing. Empty until the
    /// first render.
    column_rects: Vec<(PanelKind, Rect)>,
}

impl PanelSet {
    pub fn from_panels(
        drawer: DrawerHost,
        editor: TextEditorComponent,
        icons: crate::settings::icons::Icons,
        key_bindings: crate::keys::KeyBindings,
    ) -> Self {
        Self {
            order: PanelOrder::new(),
            rail: ActivityRail::new(key_bindings, icons),
            drawer,
            editor,
            attachment: None,
            drawer_width: DEFAULT_DRAWER_WIDTH,
            dragging_divider: false,
            column_rects: Vec::new(),
        }
    }

    // ── Order / focus / visibility (delegate to PanelOrder) ─────────────────

    pub fn focused(&self) -> PanelKind {
        self.order.focused()
    }

    /// Status-bar label for the focused panel; the drawer resolves to its
    /// active view's label.
    pub fn focused_label(&self) -> &'static str {
        match self.order.focused() {
            PanelKind::Drawer => self.drawer.active_view().label(),
            kind => kind.label(),
        }
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

    /// Move focus to `kind`. Any transition away from the editor closes its
    /// autocomplete popup so it doesn't linger while another panel owns input.
    pub fn focus(&mut self, kind: PanelKind) {
        if kind != PanelKind::Editor {
            self.editor.close_autocomplete();
        }
        self.order.focus(kind);
    }

    // ── Drawer view control ─────────────────────────────────────────────────

    pub fn active_drawer_view(&self) -> DrawerView {
        self.drawer.active_view()
    }

    /// Whether the drawer's active view is a text-input context (status-bar
    /// ⌨/≣ indicator).
    pub fn drawer_is_text_input(&self) -> bool {
        self.drawer.is_text_input()
    }

    /// Grow/shrink the drawer by `delta` columns (leader window commands),
    /// clamped to the same bounds the divider drag enforces.
    pub fn adjust_drawer_width(&mut self, delta: i16) {
        // Before the first render there are no rects to clamp against —
        // skip rather than allow an unbounded grow.
        let Some(right) = self.column_rects.iter().map(|(_, r)| r.right()).max() else {
            return;
        };
        let new = self.drawer_width.saturating_add_signed(delta);
        let max_width = right.saturating_sub(RAIL_WIDTH + MIN_EDITOR_WIDTH);
        self.drawer_width = new.clamp(MIN_DRAWER_WIDTH, max_width.max(MIN_DRAWER_WIDTH));
    }

    /// Switch the drawer to `view` and reveal it. Keeps the rail cursor in
    /// step so keyboard navigation continues from the active item.
    pub fn open_drawer_view(&mut self, view: DrawerView) {
        self.drawer.set_view(view);
        self.rail.set_cursor(view);
        self.order.show(PanelKind::Drawer);
    }

    // ── Typed accessors for panel-specific calls ───────────────────────────

    pub fn sidebar(&self) -> &SidebarComponent {
        self.drawer.sidebar()
    }
    pub fn sidebar_mut(&mut self) -> &mut SidebarComponent {
        self.drawer.sidebar_mut()
    }
    /// The note editor, or `None` while an attachment is shown in its place.
    /// Callers reaching for the editor cross a mode boundary and must handle
    /// the attachment case (see ADR-0017).
    pub fn editor(&self) -> Option<&TextEditorComponent> {
        self.attachment.is_none().then_some(&self.editor)
    }
    pub fn editor_mut(&mut self) -> Option<&mut TextEditorComponent> {
        if self.attachment.is_some() {
            return None;
        }
        Some(&mut self.editor)
    }

    /// Show `view` in the editor area, replacing any prior attachment and
    /// hiding the note editor. Closes the editor's autocomplete so it can't
    /// linger under the attachment, and focuses the editor panel.
    pub fn show_attachment(&mut self, view: AttachmentView) {
        self.editor.close_autocomplete();
        self.attachment = Some(view);
        self.order.focus(PanelKind::Editor);
    }

    /// Return the editor area to the note editor, discarding any attachment.
    /// No-op when already showing the editor.
    pub fn clear_attachment(&mut self) {
        self.attachment = None;
    }

    /// Whether the editor area is currently showing an attachment.
    pub fn is_showing_attachment(&self) -> bool {
        self.attachment.is_some()
    }

    /// The vault path of the attachment on show, if any — used to open it with
    /// the OS default program.
    pub fn attachment_path(&self) -> Option<&kimun_core::nfs::VaultPath> {
        self.attachment.as_ref().map(|v| v.path())
    }

    /// The active editor-area content as a `Component` — the attachment view
    /// when one is shown, otherwise the note editor. Lets input/hint routing
    /// dispatch once instead of branching on `self.attachment` at each site.
    /// (Render stays separate: it wraps the editor in its own dirty-title block.)
    fn editor_area(&self) -> &dyn Component {
        match &self.attachment {
            Some(view) => view,
            None => &self.editor,
        }
    }
    fn editor_area_mut(&mut self) -> &mut dyn Component {
        match &mut self.attachment {
            Some(view) => view,
            None => &mut self.editor,
        }
    }
    pub fn query(&self) -> &QueryPanel {
        self.drawer.query()
    }
    pub fn query_mut(&mut self) -> &mut QueryPanel {
        self.drawer.query_mut()
    }
    pub fn tags_mut(&mut self) -> &mut crate::components::drawer_views::TagsPanel {
        self.drawer.tags_mut()
    }
    pub fn links_mut(&mut self) -> &mut crate::components::drawer_views::LinksPanel {
        self.drawer.links_mut()
    }
    pub fn outline_mut(&mut self) -> &mut crate::components::drawer_views::OutlinePanel {
        self.drawer.outline_mut()
    }
    pub fn drawer_set_config_info(&mut self, info: crate::components::drawer::ConfigInfo) {
        self.drawer.set_config_info(info);
    }

    // ── Routing ────────────────────────────────────────────────────────────

    /// Footer hints for the focused panel.
    pub fn focused_hints(&self) -> Vec<(String, String)> {
        match self.order.focused() {
            PanelKind::Rail => self.rail.hint_shortcuts(),
            PanelKind::Drawer => self.drawer.hint_shortcuts(),
            PanelKind::Editor => self.editor_area().hint_shortcuts(),
        }
    }

    /// Route an input event to the focused panel.
    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match self.order.focused() {
            PanelKind::Rail => self.rail.handle_input(event, tx),
            PanelKind::Drawer => self.drawer.handle_input(event, tx),
            PanelKind::Editor => self.editor_area_mut().handle_input(event, tx),
        }
    }

    /// The divider hit zone: the drawer's right border column (the cell
    /// between drawer content and editor).
    fn on_divider(&self, column: u16, row: u16) -> bool {
        self.column_rects
            .iter()
            .find(|(kind, _)| *kind == PanelKind::Drawer)
            .is_some_and(|(_, rect)| {
                rect.height > 0
                    && column == rect.right().saturating_sub(1)
                    && row >= rect.y
                    && row < rect.bottom()
            })
    }

    /// Apply a divider drag: the drawer's new width follows the cursor,
    /// clamped so both the drawer and the editor keep a usable minimum.
    fn drag_divider_to(&mut self, column: u16) {
        let Some((_, drawer_rect)) = self
            .column_rects
            .iter()
            .find(|(kind, _)| *kind == PanelKind::Drawer)
        else {
            return;
        };
        let total_right = self
            .column_rects
            .iter()
            .map(|(_, r)| r.right())
            .max()
            .unwrap_or(drawer_rect.right());
        let max_width = total_right
            .saturating_sub(drawer_rect.x)
            .saturating_sub(MIN_EDITOR_WIDTH);
        let new_width = column.saturating_sub(drawer_rect.x).saturating_add(1);
        self.drawer_width = new_width.clamp(MIN_DRAWER_WIDTH, max_width.max(MIN_DRAWER_WIDTH));
    }

    /// Route a mouse event by hit-testing the panel columns from the last
    /// render. Dragging the drawer↔editor divider resizes the drawer. A
    /// button-down click focuses the panel under the cursor — the one
    /// consistent click-to-focus rule for every panel — and the event is
    /// then forwarded to that panel for its internal behavior (cursor
    /// placement, list selection, scrolling). Events outside every column
    /// (or before the first render) are not consumed.
    pub fn handle_mouse(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Mouse(mouse) = event else {
            return EventState::NotConsumed;
        };

        // Divider drag lifecycle.
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if self.on_divider(mouse.column, mouse.row) => {
                self.dragging_divider = true;
                return EventState::Consumed;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging_divider => {
                self.drag_divider_to(mouse.column);
                return EventState::Consumed;
            }
            MouseEventKind::Up(MouseButton::Left) if self.dragging_divider => {
                self.dragging_divider = false;
                return EventState::Consumed;
            }
            _ => {}
        }

        let Some(kind) = kind_at(&self.column_rects, mouse.column, mouse.row) else {
            return EventState::NotConsumed;
        };
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            self.focus(kind);
        }
        match kind {
            PanelKind::Rail => {
                self.rail.handle_input(event, tx);
            }
            PanelKind::Drawer => {
                self.drawer.handle_mouse(event, tx);
            }
            PanelKind::Editor => {
                self.editor_area_mut().handle_input(event, tx);
            }
        }
        EventState::Consumed
    }

    /// Lay the visible panels out left→right and render each. `show_focus`
    /// is false while an overlay is open, so no panel draws its focused
    /// highlight under the overlay.
    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, show_focus: bool) {
        let visible = self.order.visible_in_order();
        if visible.is_empty() {
            return;
        }
        let columns = layout_columns(&visible, area, self.drawer_width);
        self.column_rects = columns.clone();

        let focused = self.order.focused();
        let drawer_view = self
            .is_visible(PanelKind::Drawer)
            .then(|| self.drawer.active_view());
        for (kind, rect) in &columns {
            let is_focused = show_focus && *kind == focused;
            let rect = *rect;
            match kind {
                PanelKind::Rail => self.rail.render(f, rect, theme, is_focused, drawer_view),
                PanelKind::Drawer => self.drawer.render(f, rect, theme, is_focused),
                PanelKind::Editor => {
                    // The editor area's frame is drawn here (not by the
                    // component) so the dirty marker and focus border live with
                    // the layout. The frame title reflects which content the
                    // area is showing — the note editor or an attachment.
                    match &mut self.attachment {
                        Some(view) => {
                            let block = panel_block("Attachment", theme, is_focused);
                            let inner = block.inner(rect);
                            f.render_widget(block, rect);
                            view.render(f, inner, theme, is_focused);
                        }
                        None => {
                            let title = if self.editor.is_dirty() {
                                "Editor [+]"
                            } else {
                                "Editor"
                            };
                            let block = panel_block(title, theme, is_focused);
                            let inner = block.inner(rect);
                            f.render_widget(block, rect);
                            self.editor.render(f, inner, theme, is_focused);
                        }
                    }
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
        let query = QueryPanel::new(
            vault.clone(),
            settings.key_bindings.clone(),
            settings.icons(),
        );
        let tags = crate::components::drawer_views::TagsPanel::new(vault.clone(), settings.icons());
        let links =
            crate::components::drawer_views::LinksPanel::new(vault.clone(), settings.icons());
        let outline = crate::components::drawer_views::OutlinePanel::new(vault, settings.icons());
        let drawer = DrawerHost::new(sidebar, query, tags, links, outline);
        PanelSet::from_panels(drawer, editor, settings.icons(), settings.key_bindings)
    }

    /// Lay the visible panels out over a fixed area, as a render would.
    fn lay_out(panels: &mut PanelSet) {
        panels.column_rects = layout_columns(
            &panels.order.visible_in_order(),
            Rect::new(0, 0, 120, 40),
            panels.drawer_width,
        );
    }

    fn scroll_at(col: u16, row: u16) -> InputEvent {
        InputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn drag_at(col: u16, row: u16) -> InputEvent {
        InputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn up_at(col: u16, row: u16) -> InputEvent {
        InputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
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
        lay_out(&mut panels);
        let (tx, _rx) = unbounded_channel();

        assert_eq!(panels.focused(), PanelKind::Editor);
        // Rail (0..7) | Drawer (7..41) | Editor (41..120).
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(3, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.focused(), PanelKind::Rail);
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(20, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.focused(), PanelKind::Drawer);
        // Regression: a click on the editor must focus it even while another
        // panel is focused.
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(60, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.focused(), PanelKind::Editor);
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
        // Cells outside the laid-out area miss.
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
            panels.handle_mouse(&scroll_at(20, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.focused(), PanelKind::Editor);
    }

    /// Dragging the drawer↔editor divider resizes the drawer, clamped to the
    /// minimum widths on both sides.
    #[tokio::test]
    async fn divider_drag_resizes_drawer() {
        let mut panels = make_panel_set().await;
        lay_out(&mut panels);
        let (tx, _rx) = unbounded_channel();

        // Drawer occupies 7..41; the divider is its right border column (40).
        assert_eq!(
            panels.handle_mouse(&mouse_down_at(40, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(
            panels.handle_mouse(&drag_at(60, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.drawer_width, 54); // 60 - 7 + 1
        // Dragging far left clamps to the minimum drawer width.
        assert_eq!(
            panels.handle_mouse(&drag_at(0, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.drawer_width, MIN_DRAWER_WIDTH);
        // Dragging far right clamps so the editor keeps its minimum.
        assert_eq!(
            panels.handle_mouse(&drag_at(200, 5), &tx),
            EventState::Consumed
        );
        assert_eq!(panels.drawer_width, 120 - 7 - MIN_EDITOR_WIDTH);
        // Release ends the drag: further drags are plain panel events.
        assert_eq!(
            panels.handle_mouse(&up_at(80, 5), &tx),
            EventState::Consumed
        );
        let width_before = panels.drawer_width;
        panels.handle_mouse(&drag_at(50, 5), &tx);
        assert_eq!(panels.drawer_width, width_before);
    }

    #[test]
    fn default_focus_is_editor() {
        let order = PanelOrder::new();
        assert_eq!(order.focused(), PanelKind::Editor);
    }

    #[test]
    fn focus_cycle_wraps_over_visible_panels() {
        let mut order = PanelOrder::new();
        // Focus = editor (right end); next wraps to the rail.
        assert_eq!(order.next_kind(), Some(PanelKind::Rail));
        assert_eq!(order.prev_kind(), Some(PanelKind::Drawer));

        order.focus(PanelKind::Rail);
        assert_eq!(order.prev_kind(), Some(PanelKind::Editor)); // wrap left
        assert_eq!(order.next_kind(), Some(PanelKind::Drawer));
    }

    #[test]
    fn focus_cycle_skips_hidden_drawer() {
        let mut order = PanelOrder::new();
        order.hide(PanelKind::Drawer);
        assert_eq!(order.next_kind(), Some(PanelKind::Rail));
        order.focus(PanelKind::Rail);
        assert_eq!(order.next_kind(), Some(PanelKind::Editor));
    }

    #[test]
    fn show_hide_toggles_visibility_except_rail_and_editor() {
        let mut order = PanelOrder::new();
        assert!(order.is_visible(PanelKind::Drawer));

        order.hide(PanelKind::Drawer);
        assert!(!order.is_visible(PanelKind::Drawer));
        order.show(PanelKind::Drawer);
        assert!(order.is_visible(PanelKind::Drawer));

        // The rail and the editor cannot be hidden.
        order.hide(PanelKind::Editor);
        assert!(order.is_visible(PanelKind::Editor));
        order.hide(PanelKind::Rail);
        assert!(order.is_visible(PanelKind::Rail));
    }

    #[test]
    fn hiding_focused_panel_moves_focus_to_visible() {
        let mut order = PanelOrder::new();
        order.focus(PanelKind::Drawer);
        order.hide(PanelKind::Drawer);
        // Focus cannot stay on a hidden panel.
        assert!(order.is_visible(order.focused()));
    }

    #[test]
    fn layout_columns_splits_area_in_panel_order() {
        let area = Rect::new(0, 0, 120, 40);
        let visible = [PanelKind::Rail, PanelKind::Drawer, PanelKind::Editor];
        let columns = layout_columns(&visible, area, DEFAULT_DRAWER_WIDTH);

        assert_eq!(columns.len(), 3);
        // Rail fixed, drawer at its width, editor takes the rest.
        assert_eq!(columns[0].0, PanelKind::Rail);
        assert_eq!(columns[0].1.width, RAIL_WIDTH);
        assert_eq!(columns[1].0, PanelKind::Drawer);
        assert_eq!(columns[1].1.width, DEFAULT_DRAWER_WIDTH);
        assert_eq!(columns[2].0, PanelKind::Editor);
        assert_eq!(columns[2].1.width, 120 - RAIL_WIDTH - DEFAULT_DRAWER_WIDTH);
        // Columns tile the area left→right.
        assert_eq!(columns[0].1.x, 0);
        assert_eq!(columns[1].1.x, RAIL_WIDTH);
        assert_eq!(columns[2].1.x, RAIL_WIDTH + DEFAULT_DRAWER_WIDTH);
    }

    #[test]
    fn hidden_drawer_gives_width_to_editor() {
        let area = Rect::new(0, 0, 120, 40);
        let visible = [PanelKind::Rail, PanelKind::Editor];
        let columns = layout_columns(&visible, area, DEFAULT_DRAWER_WIDTH);

        assert_eq!(columns.len(), 2);
        assert_eq!(columns[1].0, PanelKind::Editor);
        assert_eq!(columns[1].1.width, 120 - RAIL_WIDTH);
    }

    #[test]
    fn kind_at_hit_tests_panel_columns() {
        let area = Rect::new(0, 0, 120, 40);
        let visible = [PanelKind::Rail, PanelKind::Drawer, PanelKind::Editor];
        let columns = layout_columns(&visible, area, DEFAULT_DRAWER_WIDTH);

        assert_eq!(kind_at(&columns, 0, 0), Some(PanelKind::Rail));
        assert_eq!(kind_at(&columns, 6, 10), Some(PanelKind::Rail));
        assert_eq!(kind_at(&columns, 7, 10), Some(PanelKind::Drawer));
        assert_eq!(kind_at(&columns, 40, 39), Some(PanelKind::Drawer));
        assert_eq!(kind_at(&columns, 41, 0), Some(PanelKind::Editor));
        assert_eq!(kind_at(&columns, 119, 39), Some(PanelKind::Editor));
        // Outside the laid-out area.
        assert_eq!(kind_at(&columns, 120, 10), None);
        assert_eq!(kind_at(&columns, 10, 40), None);
        // No columns yet (before the first render).
        assert_eq!(kind_at(&[], 10, 10), None);
    }

    #[test]
    fn open_drawer_view_reveals_and_switches() {
        let mut order = PanelOrder::new();
        order.hide(PanelKind::Drawer);
        assert!(!order.is_visible(PanelKind::Drawer));
        order.show(PanelKind::Drawer);
        assert!(order.is_visible(PanelKind::Drawer));
    }

    #[test]
    fn visible_in_order_skips_hidden() {
        let mut order = PanelOrder::new();
        assert_eq!(
            order.visible_in_order(),
            vec![PanelKind::Rail, PanelKind::Drawer, PanelKind::Editor]
        );
        order.hide(PanelKind::Drawer);
        assert_eq!(
            order.visible_in_order(),
            vec![PanelKind::Rail, PanelKind::Editor]
        );
    }
}
