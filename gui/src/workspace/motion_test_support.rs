//! What the motion tests share: the size of a drawn slot, and where the
//! editor panel stands. Real windows, read after a frame was drawn.
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use kimun_notes::logic::nav::DrawerView;

use super::slot_id;
use super::tests::Shell;

/// The size of the slot showing `view` as (width, height), `None` while it
/// is not drawn.
pub(super) fn slot_size(
    cx: &mut TestAppContext,
    shell: &Shell,
    view: DrawerView,
) -> Option<(f32, f32)> {
    cx.update_window(shell.handle, |_, window, _| {
        window.try_find(slot_id(view)).map(|slot| {
            let bounds = slot.bounds();
            (f32::from(bounds.size.width), f32::from(bounds.size.height))
        })
    })
    .unwrap()
}

/// The width of the slot showing `view`, `None` while it is not drawn.
pub(super) fn slot_width(cx: &mut TestAppContext, shell: &Shell, view: DrawerView) -> Option<f32> {
    slot_size(cx, shell, view).map(|(width, _)| width)
}

/// The width of the editor panel.
pub(super) fn editor_width(cx: &mut TestAppContext, shell: &Shell) -> f32 {
    cx.update_window(shell.handle, |_, window, _| {
        f32::from(window.find("editor-panel").bounds().size.width)
    })
    .unwrap()
}

/// The left edge of the editor panel.
pub(super) fn editor_left(cx: &mut TestAppContext, shell: &Shell) -> f32 {
    cx.update_window(shell.handle, |_, window, _| {
        f32::from(window.find("editor-panel").bounds().origin.x)
    })
    .unwrap()
}
