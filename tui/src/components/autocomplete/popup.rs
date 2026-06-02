use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use unicode_width::UnicodeWidthStr;

use crate::settings::themes::Theme;

use super::state::AutocompleteState;

/// Result of forwarding a key event to the popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupOutcome {
    Consumed(PopupAction),
    NotHandled,
}

/// What the host should do after a `Consumed` outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction {
    /// Internal navigation — no buffer change.
    None,
    /// User accepted the highlighted suggestion. Host pulls
    /// `state.selected()` and writes the replacement.
    Accept,
    /// User pressed Esc — close popup without changing the buffer.
    Dismiss,
}

/// Routes a key event through the popup's input model. Navigation keys are
/// consumed (Up/Down, PageUp/PageDown, Home/End). Tab and Enter accept;
/// Esc dismisses. Everything else returns `NotHandled` so the host can
/// process it as normal typing — the host then recomputes the trigger
/// context, which may close or refresh the popup naturally.
pub fn handle_key(state: &mut AutocompleteState, key: KeyEvent) -> PopupOutcome {
    // System-modifier combos (Ctrl, Alt, Cmd/SUPER, Win/META) are
    // never the popup's to consume — let the host route them as
    // shortcuts. Otherwise macOS Cmd+Tab / Cmd+Esc / Cmd+arrows
    // would be swallowed as Accept/Dismiss/Navigate.
    const SYSTEM_MODS: KeyModifiers = KeyModifiers::CONTROL
        .union(KeyModifiers::ALT)
        .union(KeyModifiers::SUPER)
        .union(KeyModifiers::META);
    if key.modifiers.intersects(SYSTEM_MODS) {
        return PopupOutcome::NotHandled;
    }
    match key.code {
        KeyCode::Down => {
            state.move_highlight_down();
            PopupOutcome::Consumed(PopupAction::None)
        }
        KeyCode::Up => {
            state.move_highlight_up();
            PopupOutcome::Consumed(PopupAction::None)
        }
        KeyCode::PageDown => {
            state.page_down();
            PopupOutcome::Consumed(PopupAction::None)
        }
        KeyCode::PageUp => {
            state.page_up();
            PopupOutcome::Consumed(PopupAction::None)
        }
        KeyCode::Home => {
            state.home();
            PopupOutcome::Consumed(PopupAction::None)
        }
        KeyCode::End => {
            state.end();
            PopupOutcome::Consumed(PopupAction::None)
        }
        KeyCode::Tab | KeyCode::Enter => {
            if state.selected().is_some() {
                PopupOutcome::Consumed(PopupAction::Accept)
            } else {
                // Empty list — let the host process the key as usual.
                PopupOutcome::NotHandled
            }
        }
        KeyCode::Esc => PopupOutcome::Consumed(PopupAction::Dismiss),
        _ => PopupOutcome::NotHandled,
    }
}

/// Render the popup as a floating layer over `screen`. Picks an anchor
/// position adjacent to `state.anchor`, flipping above the cursor when
/// there is no room below. Width grows to fit the longest visible row,
/// capped at a reasonable maximum and the available screen width. Height
/// is bounded by `state.max_visible_rows`; the popup never grows past it,
/// even when the screen has more room available — keeping it visually
/// subordinate to the editor.
pub fn render(frame: &mut Frame, state: &AutocompleteState, screen: Rect, theme: &Theme) {
    if state.items.is_empty() {
        return;
    }

    const MAX_WIDTH: u16 = 60;
    const BORDERS: u16 = 2;

    // Below the minimum drawable size the popup can't render a
    // border + a row of content, so just bail. ratatui can render
    // smaller, but the result is uninterpretable to the user.
    if screen.width < BORDERS + 1 || screen.height < BORDERS + 1 {
        return;
    }

    let (start, end) = state.visible_window();
    let visible_rows = (end - start) as u16;
    if visible_rows == 0 {
        return;
    }

    let content_width = visible_content_width(state, start, end);
    let desired_width = (content_width as u16)
        .saturating_add(BORDERS)
        .min(MAX_WIDTH);
    let popup_width = desired_width.min(screen.width);
    let popup_height = visible_rows.saturating_add(BORDERS).min(screen.height);

    let (anchor_col, anchor_row) = state.anchor;
    let screen_right = screen.x.saturating_add(screen.width);
    let popup_x = if anchor_col.saturating_add(popup_width) > screen_right {
        screen_right.saturating_sub(popup_width)
    } else {
        anchor_col.max(screen.x)
    };

    // Prefer below the trigger; flip above when there is no room.
    let screen_bottom = screen.y.saturating_add(screen.height);
    let space_below = screen_bottom.saturating_sub(anchor_row.saturating_add(1));
    let popup_y = if space_below >= popup_height {
        anchor_row.saturating_add(1)
    } else if anchor_row >= popup_height.saturating_add(screen.y) {
        anchor_row.saturating_sub(popup_height)
    } else {
        // Last resort: clamp inside the screen, accepting that the popup
        // may cover the trigger line. Better than rendering off-screen.
        screen_bottom.saturating_sub(popup_height).max(screen.y)
    };

    let area = Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, area);

    let title = format!(" {} ", visible_title(state));
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused.to_ratatui()))
        .style(theme.panel_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let inner_width = inner.width as usize;

    let items: Vec<ListItem> = (start..end)
        .map(|idx| build_row(state, idx, inner_width, theme))
        .collect();
    let list = List::new(items);
    frame.render_widget(list, inner);

    // Overflow indicators on the popup's top and bottom border. We render
    // them as a single-cell overlay on top of the existing border so the
    // popup stays exactly `max_visible_rows + 2` tall.
    if state.has_more_above() {
        render_overflow_marker(frame, area, '▲', true, theme, state.hidden_above());
    }
    if state.has_more_below() {
        render_overflow_marker(frame, area, '▼', false, theme, state.hidden_below());
    }
}

fn visible_title(state: &AutocompleteState) -> String {
    let sigil = match state.kind {
        super::TriggerKind::Wikilink => "[[".to_string(),
        super::TriggerKind::Hashtag => "#".to_string(),
        // `LinkFilter` fires for `<`, `>`, and `=`; render the operator the
        // user actually typed rather than a hardcoded one.
        super::TriggerKind::LinkFilter => state
            .opener
            .map(|c| c.to_string())
            .unwrap_or_else(|| ">".to_string()),
        super::TriggerKind::SavedSearch => "?".to_string(),
    };
    if state.query.is_empty() {
        sigil
    } else {
        format!("{}{}", sigil, state.query)
    }
}

fn visible_content_width(state: &AutocompleteState, start: usize, end: usize) -> usize {
    // Measure display cells (handles CJK, emoji) — `chars().count()`
    // is wrong for any text wider than ASCII.
    let mut widest = visible_title(state).width();
    for item in &state.items[start..end] {
        let primary = item.display.width();
        let secondary = item
            .secondary
            .as_deref()
            .map(|s| s.width() + 2)
            .unwrap_or(0);
        widest = widest.max(primary + secondary);
    }
    widest
}

fn build_row<'a>(
    state: &'a AutocompleteState,
    idx: usize,
    inner_width: usize,
    theme: &Theme,
) -> ListItem<'a> {
    let item = &state.items[idx];
    let is_highlighted = idx == state.highlighted;

    let row_style = if is_highlighted {
        Style::default()
            .bg(theme.bg_selected.to_ratatui())
            .fg(theme.fg_selected.to_ratatui())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg.to_ratatui())
    };
    let secondary_style = Style::default()
        .fg(theme.fg_muted.to_ratatui())
        .add_modifier(Modifier::DIM);

    let primary_len = item.display.width();
    let secondary_text = item.secondary.as_deref().unwrap_or("");
    let secondary_len = secondary_text.width();
    let separator = if secondary_text.is_empty() { 0 } else { 1 };

    let total = primary_len + separator + secondary_len;
    let pad = inner_width.saturating_sub(total);

    let mut spans = vec![Span::styled(item.display.clone(), row_style)];
    if separator > 0 {
        spans.push(Span::styled(" ".repeat(pad + separator), row_style));
        spans.push(Span::styled(secondary_text.to_string(), secondary_style));
    } else if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), row_style));
    }
    ListItem::new(Line::from(spans))
}

fn render_overflow_marker(
    frame: &mut Frame,
    area: Rect,
    glyph: char,
    on_top: bool,
    theme: &Theme,
    hidden_count: usize,
) {
    if area.width < 3 {
        return;
    }
    let y = if on_top {
        area.y
    } else {
        area.y + area.height - 1
    };
    let label = format!(" {} {} more ", glyph, hidden_count);
    let label_chars: Vec<char> = label.chars().collect();
    let label_width = label_chars.len() as u16;
    let max_label = area.width.saturating_sub(2);
    let label_width = label_width.min(max_label);
    let x = area.x + area.width - 1 - label_width;
    let marker = ratatui::widgets::Paragraph::new(
        label_chars
            .into_iter()
            .take(label_width as usize)
            .collect::<String>(),
    )
    .style(
        Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(Modifier::DIM),
    );
    let marker_area = Rect {
        x,
        y,
        width: label_width,
        height: 1,
    };
    frame.render_widget(marker, marker_area);
}

#[cfg(test)]
mod tests {
    use super::super::TriggerKind;
    use super::super::state::Suggestion;
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    fn sample_state(n: usize) -> AutocompleteState {
        let mut st = AutocompleteState::new(TriggerKind::Hashtag, (0, 0));
        st.set_items(
            (0..n)
                .map(|i| Suggestion {
                    display: format!("tag{i}"),
                    secondary: Some(format!("{i}")),
                })
                .collect(),
        );
        st
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn down_navigates_and_is_consumed() {
        let mut st = sample_state(5);
        let out = handle_key(&mut st, key(KeyCode::Down));
        assert_eq!(out, PopupOutcome::Consumed(PopupAction::None));
        assert_eq!(st.highlighted, 1);
    }

    #[test]
    fn tab_accepts_when_list_nonempty() {
        let mut st = sample_state(5);
        let out = handle_key(&mut st, key(KeyCode::Tab));
        assert_eq!(out, PopupOutcome::Consumed(PopupAction::Accept));
    }

    #[test]
    fn enter_accepts_when_list_nonempty() {
        let mut st = sample_state(5);
        let out = handle_key(&mut st, key(KeyCode::Enter));
        assert_eq!(out, PopupOutcome::Consumed(PopupAction::Accept));
    }

    #[test]
    fn esc_dismisses() {
        let mut st = sample_state(5);
        let out = handle_key(&mut st, key(KeyCode::Esc));
        assert_eq!(out, PopupOutcome::Consumed(PopupAction::Dismiss));
    }

    #[test]
    fn tab_with_empty_list_falls_through() {
        let mut st = sample_state(0);
        let out = handle_key(&mut st, key(KeyCode::Tab));
        assert_eq!(out, PopupOutcome::NotHandled);
    }

    #[test]
    fn typing_letter_is_not_handled() {
        let mut st = sample_state(5);
        let out = handle_key(&mut st, key(KeyCode::Char('x')));
        assert_eq!(out, PopupOutcome::NotHandled);
    }

    #[test]
    fn ctrl_modifier_falls_through() {
        let mut st = sample_state(5);
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL);
        assert_eq!(handle_key(&mut st, key), PopupOutcome::NotHandled);
    }

    #[test]
    fn page_down_jumps() {
        let mut st = sample_state(30);
        handle_key(&mut st, key(KeyCode::PageDown));
        assert_eq!(st.highlighted, 8);
    }

    #[test]
    fn end_jumps_to_last() {
        let mut st = sample_state(30);
        handle_key(&mut st, key(KeyCode::End));
        assert_eq!(st.highlighted, 29);
    }

    // ---- Rendering smoke tests ----

    fn draw(state: &AutocompleteState, area: Rect) -> Terminal<TestBackend> {
        let theme = Theme::gruvbox_dark();
        let backend = TestBackend::new(area.width.max(40), area.height.max(20));
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(f, state, f.area(), &theme);
            })
            .unwrap();
        terminal
    }

    #[test]
    fn render_does_not_panic_with_few_items() {
        let mut st = sample_state(3);
        st.anchor = (5, 5);
        draw(&st, Rect::new(0, 0, 80, 24));
    }

    #[test]
    fn render_caps_height_at_max_visible_rows() {
        let mut st = sample_state(30);
        st.anchor = (5, 5);
        st.max_visible_rows = 8;
        draw(&st, Rect::new(0, 0, 80, 24));
        // The popup occupies max_visible_rows + 2 (borders); not asserting
        // a specific cell layout here, just that render completes — the
        // height bound is enforced in the render function and tested
        // logically through state::visible_window.
        assert_eq!(st.visible_window(), (0, 8));
    }

    #[test]
    fn render_flips_above_when_no_room_below() {
        let mut st = sample_state(5);
        // Anchor near the bottom of a 10-row screen — popup must flip.
        st.anchor = (0, 9);
        draw(&st, Rect::new(0, 0, 40, 10));
        // No assertion on cell content; this just exercises the flip path
        // without panicking.
    }

    // ---- visible_title sigil ----

    #[test]
    fn title_uses_link_filter_opener_char() {
        // LinkFilter fires for `<`, `>`, and `=`; the title must reflect the
        // operator the user actually typed, not a hardcoded `>`.
        for opener in ['<', '>', '='] {
            let mut st = AutocompleteState::new(TriggerKind::LinkFilter, (0, 0));
            st.opener = Some(opener);
            st.query = "work".to_string();
            assert_eq!(visible_title(&st), format!("{opener}work"));

            st.query.clear();
            assert_eq!(visible_title(&st), opener.to_string());
        }
    }

    #[test]
    fn title_falls_back_when_link_filter_opener_missing() {
        let mut st = AutocompleteState::new(TriggerKind::LinkFilter, (0, 0));
        st.opener = None;
        assert_eq!(visible_title(&st), ">");
    }

    #[test]
    fn title_uses_fixed_sigils_for_hashtag_and_wikilink() {
        let mut st = AutocompleteState::new(TriggerKind::Hashtag, (0, 0));
        st.query = "tag".to_string();
        assert_eq!(visible_title(&st), "#tag");

        let mut st = AutocompleteState::new(TriggerKind::Wikilink, (0, 0));
        st.query = "note".to_string();
        assert_eq!(visible_title(&st), "[[note");
    }

    #[test]
    fn render_empty_state_is_noop() {
        let st = sample_state(0);
        let theme = Theme::gruvbox_dark();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        // Should complete without rendering anything.
        terminal
            .draw(|f| {
                render(f, &st, f.area(), &theme);
            })
            .unwrap();
    }
}
