//! The **Footer** docked under the editor Panel (M5b-2): the open note's
//! path as a breadcrumb, whether it has unsaved edits, what the vault is
//! doing (indexing), whether notes may be saved, and transient flash
//! messages. G5 completes it.
//!
//! The vim mode is not repeated here: the editor's own mode line, the row
//! right above the Footer, shows it with the pending command.
//!
//! The state is plain data owned by the
//! [`Workspace`](crate::workspace::Workspace), which also owns the flash's
//! expiry; [`render`] draws it with the kit's `StatusBar` in kimün's theme.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, Role, SharedString,
    StatefulInteractiveElement, Styled, div, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::settings::themes::Theme;

use crate::theme;

/// How long a flash stays up. Longer than the TUI's two seconds: a window
/// is read from further away than a terminal's last line, and the messages
/// here name paths.
pub(crate) const FLASH_DURATION: Duration = Duration::from_secs(4);

/// Element ids, for tests.
pub(crate) const ROW_ID: &str = "status-row";
pub(crate) const PATH_ID: &str = "status-path";
pub(crate) const SAVE_ID: &str = "status-save";
pub(crate) const FLASH_ID: &str = "status-flash";
pub(crate) const INDEXING_ID: &str = "status-indexing";
pub(crate) const READ_ONLY_ID: &str = "status-read-only";

/// The save state's two labels.
pub(crate) const UNSAVED: &str = "Unsaved";
pub(crate) const SAVED: &str = "Saved";

/// What the footer shows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Status {
    /// The note in the editor, `None` before one opened.
    pub(crate) note: Option<VaultPath>,
    /// The editor holds edits newer than the last save.
    pub(crate) dirty: bool,
    /// The vault's index is being brought up to date.
    pub(crate) indexing: bool,
    /// Notes must not be written (the vault has names that differ only by
    /// case; until M8/M9 give that a screen). Saves honour it.
    pub(crate) read_only: bool,
    /// The transient message on show, if any.
    pub(crate) flash: Option<SharedString>,
    /// The flash reports a problem (a failed save, a refused open): drawn in
    /// the error colour, and not replaced by a plain notice while it shows.
    pub(crate) flash_error: bool,
}

/// The footer for `status`.
pub(crate) fn render(status: &Status, theme: &Theme) -> impl IntoElement {
    let label = |id: &'static str, text: &'static str| {
        div()
            .id(id)
            .test_support()
            .role(Role::Status)
            .aria_label(text)
            .whitespace_nowrap()
            .child(text)
    };
    // The path, then the flash: both shrink (the flash first, it is cut
    // short with an ellipsis) rather than pushing the pinned labels out.
    let bar = StatusBar::new()
        .when_some(status.note.as_ref(), |bar, note| {
            bar.child(breadcrumb(note, theme))
        })
        .child(
            div()
                .id(FLASH_ID)
                .test_support()
                .role(Role::Status)
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(if status.flash_error {
                    theme::error(theme)
                } else {
                    theme::foreground(theme)
                })
                .when_some(status.flash.clone(), |flash, text| {
                    flash.aria_label(text.clone()).child(text)
                }),
        )
        .when(status.note.is_some(), |bar| {
            bar.right(save_state(status.dirty, theme))
        })
        .when(status.read_only, |bar| {
            bar.right(label(READ_ONLY_ID, "Read-only").text_color(theme::error(theme)))
        })
        .when(status.indexing, |bar| {
            bar.right(label(INDEXING_ID, "Indexing…"))
        })
        .bg(theme::panel_background(theme))
        .border_color(theme::divider(theme))
        .text_color(theme::muted(theme));
    div()
        .id(ROW_ID)
        .test_support()
        .role(Role::Footer)
        .aria_label("Status")
        .flex_none()
        .w_full()
        .child(bar)
}

/// The breadcrumb's parts for `slices` (`VaultPath::get_slices`): every
/// folder up to two, else the first and the last folder around a `…`, then
/// the note's own name.
pub(crate) fn crumbs(mut slices: Vec<String>) -> Vec<String> {
    const ELLIPSIS: &str = "\u{2026}";
    let Some(name) = slices.pop() else {
        return Vec::new();
    };
    if slices.len() > 2 {
        let last = slices.pop().unwrap_or_default();
        slices.truncate(1);
        slices.push(ELLIPSIS.to_string());
        slices.push(last);
    }
    slices.push(name);
    slices
}

/// The note's path, folder by folder (see [`crumbs`]), the note's own name
/// brightest; chevrons between, as the kit's `Breadcrumb` draws them, in
/// kimün's tokens. Short of room, the folders shrink with an ellipsis and
/// the name never does. Not clickable yet (the FILES view, G3).
fn breadcrumb(note: &VaultPath, theme: &Theme) -> AnyElement {
    let parts_text = crumbs(note.get_slices());
    let last = parts_text.len().saturating_sub(1);
    let mut parts: Vec<AnyElement> = Vec::with_capacity(parts_text.len() * 2);
    for (i, slice) in parts_text.into_iter().enumerate() {
        if i > 0 {
            parts.push(
                Icon::new(IconName::ChevronRight)
                    .size_3()
                    .flex_none()
                    .text_color(theme::muted(theme))
                    .into_any_element(),
            );
        }
        let part = div().whitespace_nowrap().child(slice);
        let part = if i == last {
            part.flex_none().text_color(theme::foreground(theme))
        } else {
            part.min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_color(theme::muted(theme))
        };
        parts.push(part.into_any_element());
    }
    div()
        .id(PATH_ID)
        .test_support()
        .role(Role::Group)
        .aria_label(SharedString::from(note.to_string()))
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .min_w_0()
        .overflow_hidden()
        .children(parts)
        .into_any_element()
}

/// "Unsaved" behind a dot in the unsaved colour, or a quiet "Saved". The
/// word carries the state; the dot only draws the eye to it.
fn save_state(dirty: bool, theme: &Theme) -> impl IntoElement {
    let text = if dirty { UNSAVED } else { SAVED };
    div()
        .id(SAVE_ID)
        .test_support()
        .role(Role::Status)
        .aria_label(text)
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .whitespace_nowrap()
        .when(dirty, |row| {
            row.text_color(theme::foreground(theme)).child(
                div()
                    .flex_none()
                    .size(rems(0.375))
                    .rounded_full()
                    .bg(theme::unsaved(theme)),
            )
        })
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::crumbs;

    fn c(slices: &[&str]) -> Vec<String> {
        crumbs(slices.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn short_paths_show_every_folder_and_the_name() {
        assert_eq!(c(&["todo.md"]), ["todo.md"]);
        assert_eq!(c(&["a", "b", "todo.md"]), ["a", "b", "todo.md"]);
        assert!(c(&[]).is_empty());
    }

    #[test]
    fn deep_paths_fold_the_middle_and_keep_the_name() {
        assert_eq!(
            c(&["a", "b", "c", "d", "todo.md"]),
            ["a", "\u{2026}", "d", "todo.md"]
        );
    }
}
