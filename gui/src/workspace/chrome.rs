//! Chrome the tab pill and the group header share: the close affordance,
//! the unsaved dot, and the `Label (chord)` tooltip text.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, Role, SharedString,
    StatefulInteractiveElement, Styled, Window, div, rems,
};
use kimun_notes::logic::actions::ActionShortcuts;

use crate::{keymap, theme};

/// `label`, followed by the chord bound to `action` in parentheses, or bare
/// `label` when `action` is unbound.
pub(crate) fn tooltip_with_chord(label: &str, action: &ActionShortcuts, cx: &App) -> SharedString {
    match keymap::chord_label(cx, action) {
        Some(chord) => format!("{label} ({chord})").into(),
        None => label.to_string().into(),
    }
}

/// The close glyph: the kit's own close icon (its dialogs and toasts draw
/// it), carried by the default bundle. Lucide's `x` is not bundled.
pub(crate) const CLOSE_ICON: IconName = IconName::Close;

/// A close button: the close icon, the hover fill, a tooltip, and a click
/// that stops propagating (the host's own click does not follow) before
/// `on_close` runs. The caller sets the size, the rounding and whether it
/// shrinks: those differ between the tab pill and the group header.
pub(crate) fn close_button(
    id: ElementId,
    label: &'static str,
    tip: SharedString,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl Styled + IntoElement {
    let hover = theme::hover_background(&theme::current(cx));
    div()
        .id(id)
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(CLOSE_ICON).size_3())
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_close(event, window, cx);
        })
}

/// The small dot that marks unsaved text. The caller adds `flex_none` where
/// its row needs it.
pub(crate) fn unsaved_dot(id: ElementId, cx: &App) -> impl Styled + IntoElement {
    div()
        .id(id)
        .test_support()
        .size(rems(0.375))
        .rounded_full()
        .bg(theme::unsaved(&theme::current(cx)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gpui_kit::TestAppContext;

    use super::*;
    use crate::keymap::{Keymap, Platform};

    #[gpui_kit::test]
    fn the_tooltip_names_the_chord_only_when_bound(cx: &mut TestAppContext) {
        cx.update(|cx| keymap::init(cx, Keymap::defaults(Platform::LinuxWindows)));
        let bound = cx.read(|cx| tooltip_with_chord("New tab", &ActionShortcuts::NewTab, cx));
        let chord = cx
            .read(|cx| keymap::chord_label(cx, &ActionShortcuts::NewTab))
            .expect("NewTab is bound by default");
        assert_eq!(bound.as_ref(), format!("New tab ({chord})"));

        let unbind = BTreeMap::from([(String::from(ActionShortcuts::NewTab), Vec::new())]);
        let (keymap, errors) = Keymap::load(Platform::LinuxWindows, &unbind);
        assert!(errors.is_empty(), "{errors:?}");
        cx.update(|cx| keymap::init(cx, keymap));
        let bare = cx.read(|cx| tooltip_with_chord("New tab", &ActionShortcuts::NewTab, cx));
        assert_eq!(bare.as_ref(), "New tab");
    }
}
