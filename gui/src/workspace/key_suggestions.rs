//! The **known-key suggestions** under a property-key field: the vault's
//! property keys that hold what was typed, prefix matches first, walked
//! with Up/Down and taken with Enter or a click. Shared by the properties
//! form's Key field and the sort dialog's Key field, so both suggest, rank
//! and draw keys the same way.
//!
//! The owner keeps the keys (loaded off the UI thread), the field, and what
//! taking a suggestion does; this holds the shown suggestions and the
//! highlighted one, and draws the list.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};
use kimun_notes::settings::themes::Theme;

use crate::theme;

/// The most suggestions shown under a field.
const SHOWN: usize = 5;

/// The suggestions shown under one key field, and the one Up/Down is on.
#[derive(Debug, Default)]
pub(crate) struct KeySuggestions {
    shown: Vec<String>,
    highlighted: Option<usize>,
}

impl KeySuggestions {
    /// The `known` keys containing `typed` (case-insensitive, prefix matches
    /// first), never `typed` itself; nothing for a blank field. Lets go of
    /// any highlight.
    pub(crate) fn refresh(&mut self, known: &[String], typed: &str) {
        let typed = typed.trim().to_lowercase();
        self.highlighted = None;
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

    /// The suggestions shown, best first (tests).
    #[cfg(test)]
    pub(crate) fn shown(&self) -> &[String] {
        &self.shown
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

    /// The list `list_id` under the field (`None` when empty): one option
    /// per key, its element id from `id_of(key)`; a click calls `pick` with
    /// its position on the owner view.
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
        let hover = theme::hover_background(t);
        Some(
            div()
                .id(list_id)
                .role(Role::ListBox)
                .aria_label(aria_label)
                .flex()
                .flex_col()
                .children(self.shown.iter().enumerate().map(|(ix, key)| {
                    let highlighted = self.highlighted == Some(ix);
                    div()
                        .id(id_of(key))
                        .test_support()
                        .role(Role::ListBoxOption)
                        .aria_label(SharedString::from(key.clone()))
                        .aria_selected(highlighted)
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .text_sm()
                        .text_color(theme::foreground(t))
                        .when(highlighted, |row| row.bg(theme::selected_background(t)))
                        .when(!highlighted, |row| row.hover(move |style| style.bg(hover)))
                        .child(key.clone())
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            pick(this, ix, window, cx)
                        }))
                }))
                .into_any_element(),
        )
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
