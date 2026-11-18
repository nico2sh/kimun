//! The **known-value suggestions** under a field: the known values (the
//! vault's property keys, tags, folders, note names) that hold what was typed,
//! prefix matches first, walked with Up/Down and taken with Enter or a click.
//! Shared by the properties form's Key field, the sort dialog's Key field and
//! the query builder's fields, so all suggest, rank and draw them the same way.
//!
//! The owner keeps the keys (loaded off the UI thread), the field, and what
//! taking a suggestion does; this holds the shown suggestions and the
//! highlighted one, and draws the list.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Bounds, ClickEvent, Context, Div, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ParentElement as _, Pixels, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};
use kimun_notes::logic::autocomplete::PopupCommand;
use kimun_notes::settings::themes::Theme;

use super::popup_layer;
use crate::theme;

/// The most suggestions shown under a field.
const SHOWN: usize = 5;

/// The suggestions shown under one key field, and the one Up/Down is on.
#[derive(Debug, Default)]
pub(crate) struct KeySuggestions {
    shown: Vec<String>,
    highlighted: Option<usize>,
    /// `shown` lists a blank field's choices (see [`Self::refresh_browsing`]).
    browsing: bool,
}

impl KeySuggestions {
    /// The `known` keys containing `typed` (case-insensitive, prefix matches
    /// first), never `typed` itself; nothing for a blank field. Lets go of
    /// any highlight.
    pub(crate) fn refresh(&mut self, known: &[String], typed: &str) {
        let typed = typed.trim().to_lowercase();
        self.highlighted = None;
        self.browsing = false;
        self.shown.clear();
        if typed.is_empty() {
            return;
        }
        let mut hits: Vec<(bool, &String)> = known
            .iter()
            .filter_map(|k| {
                let lower = k.to_lowercase();
                (lower != typed && lower.contains(&typed)).then(|| (!lower.starts_with(&typed), k))
            })
            .collect();
        hits.sort();
        self.shown = hits
            .into_iter()
            .take(SHOWN)
            .map(|(_, k)| k.clone())
            .collect();
    }

    /// [`Self::refresh`], except that a blank field offers the first `known`
    /// keys in order, so a field can be browsed before anything is typed
    /// (the query builder's tag, folder and key fields). Such a list is
    /// only offered: see [`Self::is_browsing`].
    pub(crate) fn refresh_browsing(&mut self, known: &[String], typed: &str) {
        self.refresh(known, typed);
        if typed.trim().is_empty() {
            self.shown = known.iter().take(SHOWN).cloned().collect();
            self.browsing = !self.shown.is_empty();
        }
    }

    /// Whether the list shown is an offer for a blank field, not matches for
    /// what was typed: Escape is the dialog's then, until one is highlighted.
    pub(crate) fn is_browsing(&self) -> bool {
        self.browsing && self.highlighted.is_none()
    }

    /// [`Self::refresh`], with `leading` offered first whatever `typed` is
    /// (the open note, offered even in an empty field).
    pub(crate) fn refresh_with_leading(
        &mut self,
        known: &[String],
        typed: &str,
        leading: Option<&str>,
    ) {
        self.refresh(known, typed);
        if let Some(leading) = leading {
            self.shown.insert(0, leading.to_string());
            self.shown.truncate(SHOWN);
        }
    }

    /// The suggestions shown, best first (tests).
    #[cfg(test)]
    pub(crate) fn shown(&self) -> &[String] {
        &self.shown
    }

    /// Whether nothing is suggested.
    pub(crate) fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    /// The suggestion Up/Down is on.
    pub(crate) fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    /// The suggestion at `ix`.
    pub(crate) fn get(&self, ix: usize) -> Option<&String> {
        self.shown.get(ix)
    }

    /// Up/Down: walk the suggestions (Up from the first lets go). False when
    /// there is nothing to walk, so the key goes on.
    pub(crate) fn step(&mut self, down: bool) -> bool {
        if self.shown.is_empty() {
            return false;
        }
        let last = self.shown.len() - 1;
        self.highlighted = match (self.highlighted, down) {
            (None, true) => Some(0),
            (None, false) => None,
            (Some(ix), true) => Some((ix + 1).min(last)),
            (Some(0), false) => None,
            (Some(ix), false) => Some(ix - 1),
        };
        true
    }

    /// Escape: let go of the highlight. True when there was one.
    pub(crate) fn release(&mut self) -> bool {
        self.highlighted.take().is_some()
    }

    /// The list `list_id` under the field, in the layout (`None` when empty):
    /// one option per key, its element id from `id_of(key)`; a click calls
    /// `pick` with its position on the owner view.
    pub(crate) fn render<V: 'static>(
        &self,
        list_id: &'static str,
        aria_label: &'static str,
        id_of: fn(&str) -> SharedString,
        t: &Theme,
        cx: &mut Context<V>,
        pick: fn(&mut V, usize, &mut Window, &mut Context<V>),
    ) -> Option<AnyElement> {
        if self.shown.is_empty() {
            return None;
        }
        let rows = self.rows(
            id_of,
            |key| key.to_string(),
            t,
            |row, ix| {
                row.on_click(
                    cx.listener(move |this, _: &ClickEvent, window, cx| pick(this, ix, window, cx)),
                )
            },
        );
        Some(
            div()
                .id(list_id)
                .role(Role::ListBox)
                .aria_label(aria_label)
                .flex()
                .flex_col()
                .children(rows)
                .into_any_element(),
        )
    }

    /// The list as a layer floating under `field` (the box of the field it is
    /// for), so it never changes the layout around it (`None` when empty).
    /// Each option shows as `label_of(key)` while the key stays what is
    /// taken; a press (not a click: it must act before the field loses the
    /// keyboard) calls `pick` with its position, and the wheel calls `step`
    /// with whether it turned down.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_floating<V: 'static>(
        &self,
        field: Bounds<Pixels>,
        list_id: &'static str,
        id_of: fn(&str) -> SharedString,
        label_of: fn(&str) -> String,
        t: &Theme,
        window: &Window,
        cx: &mut Context<V>,
        pick: fn(&mut V, usize, &mut Window, &mut Context<V>),
        step: fn(&mut V, bool, &mut Context<V>),
    ) -> Option<AnyElement> {
        if self.shown.is_empty() {
            return None;
        }
        let rows = self.rows(id_of, label_of, t, |row, ix| {
            row.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    pick(this, ix, window, cx);
                    cx.stop_propagation();
                }),
            )
        });
        let list = popup_layer::frame(
            div()
                .id(list_id)
                .test_support()
                .role(Role::ListBox)
                .aria_label("Suggestions"),
            popup_layer::width_of(field),
            t,
        )
        .on_scroll(popup_layer::on_wheel(
            cx,
            move |this, command, _, cx| match command {
                PopupCommand::Up => step(this, false, cx),
                PopupCommand::Down => step(this, true, cx),
                _ => {}
            },
        ))
        .children(rows);
        Some(popup_layer::floating_under(
            field,
            self.shown.len(),
            list,
            window,
        ))
    }

    /// One option per suggestion, `attach` adding what a press does.
    fn rows(
        &self,
        id_of: fn(&str) -> SharedString,
        label_of: fn(&str) -> String,
        t: &Theme,
        attach: impl Fn(Stateful<Div>, usize) -> Stateful<Div>,
    ) -> Vec<AnyElement> {
        let hover = theme::hover_background(t);
        self.shown
            .iter()
            .enumerate()
            .map(|(ix, key)| {
                let highlighted = self.highlighted == Some(ix);
                let label = label_of(key);
                let row = div()
                    .id(id_of(key))
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .text_sm()
                    // One line, so the popup's height is its rows' count (the
                    // flip decision relies on it); a long value ends in "…".
                    .truncate()
                    .text_color(theme::foreground(t))
                    .when(highlighted, |row| row.bg(theme::selected_background(t)))
                    .when(!highlighted, |row| row.hover(move |style| style.bg(hover)))
                    .child(label.clone());
                attach(row, ix)
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_label(SharedString::from(label))
                    .aria_selected(highlighted)
                    .into_any_element()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn prefix_matches_lead_and_the_typed_key_is_left_out() {
        let known = keys(&["deadline", "due", "lead", "priority", "Due"]);
        let mut s = KeySuggestions::default();
        s.refresh(&known, "d");
        assert_eq!(s.shown(), ["Due", "deadline", "due", "lead"]);
        s.refresh(&known, "due");
        assert!(s.shown().is_empty(), "the key itself is not offered");
        s.refresh(&known, "  ");
        assert!(s.shown().is_empty());
    }

    #[test]
    fn up_and_down_walk_and_up_from_the_first_lets_go() {
        let mut s = KeySuggestions::default();
        assert!(!s.step(true), "nothing to walk");
        s.refresh(&keys(&["rank", "rating"]), "ra");
        assert!(s.step(true));
        assert_eq!(s.highlighted(), Some(0));
        s.step(true);
        s.step(true);
        assert_eq!(s.highlighted(), Some(1));
        s.step(false);
        s.step(false);
        assert_eq!(s.highlighted(), None);
        s.step(true);
        assert!(s.release());
        assert!(!s.release());
    }
}
