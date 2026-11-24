//! Preferences > **Remote images**: the allowed hosts and the image cache
//! (adr/0063). Like Workspaces, it acts at once, outside the draft.
//!
//! - The list shows the allowed hosts of the shared settings, in alphabetical
//!   order, in a bordered box that scrolls. One row is selected (a click, or
//!   Up and Down once the list has the keyboard), as in the Workspaces list.
//!   "Remove" (or Delete, or Backspace) acts on the selected host and asks
//!   nothing: the saved copies stay and "Always load from" allows the host
//!   again. It writes the settings through the window's one settings write
//!   (off the UI thread, in turn with the other writes) with the host
//!   forgotten, and the selection keeps its place in the list. Once it is
//!   on disk, the store of remote images takes the new list, so an open
//!   view stops loading that host on its own at once. The saved copies stay. A failed write is shown in the
//!   section, and the list and the store stay as they were.
//! - "Clear cached images" empties the image cache through core, off the UI
//!   thread. Then the store forgets the failures and the redirect
//!   permissions of the session, so each cleared address starts over from
//!   its placeholder. A damaged copy goes too.
//! - The cache size beside it is read off the UI thread when the section
//!   shows and after each clear.
//!
//! After a removal and after a clear, every window redraws, so the open
//! views (the editor, the facet view, the preview) show the change at once.
//! The follow-up steps run even when Preferences close first.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Role, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window,
    div, rems,
};

use kimun_notes::logic::attachment::human_size;

use super::super::reopen;
use super::workspaces::{Notice, button_row, notice_line};
use super::{PreferencesView, note};
use crate::remote_image::RemoteImages;
use crate::tasks::blocking;
use crate::theme;

/// The list of allowed hosts.
pub(crate) const REMOTE_HOSTS_ID: &str = "preferences-remote-hosts";
/// The line an empty list shows.
pub(crate) const REMOTE_NO_HOSTS_ID: &str = "preferences-remote-no-hosts";
/// The "Remove" button under the list.
pub(crate) const REMOTE_REMOVE_ID: &str = "preferences-remote-remove";
/// The "Clear cached images" button.
pub(crate) const REMOTE_CLEAR_ID: &str = "preferences-remote-clear";
/// The cache size beside it.
pub(crate) const REMOTE_SIZE_ID: &str = "preferences-remote-size";
/// The section's problem line.
pub(crate) const REMOTE_MESSAGE_ID: &str = "preferences-remote-message";

/// What an empty list says.
pub(crate) const NO_HOSTS: &str = "No hosts allowed";

/// The element id of the list's row for `host`.
pub(crate) fn remote_host_row_id(host: &str) -> SharedString {
    format!("preferences-remote-host-{host}").into()
}

/// The text beside "Clear cached images": the size, or that it is being
/// read.
pub(crate) fn size_text(size: Option<u64>) -> String {
    match size {
        Some(bytes) => format!("Cache size: {}", human_size(bytes)),
        None => "Cache size: reading…".to_string(),
    }
}

/// The allowed hosts as the list shows them: alphabetical.
pub(crate) fn sorted_hosts(mut hosts: Vec<String>) -> Vec<String> {
    hosts.sort();
    hosts
}

/// The section's state.
pub(crate) struct RemoteSection {
    /// The list's handle: a Tab stop with its own Up, Down and Delete keys.
    pub(crate) focus: FocusHandle,
    scroll: ScrollHandle,
    /// The selected row of the list (clamped when used).
    selected: usize,
    /// The cache size, once read. `None` while it is read.
    size: Option<u64>,
    /// A size read under way.
    reading: bool,
    /// A removal or a clear under way: the buttons wait for it.
    working: bool,
    message: Option<Notice>,
}

impl RemoteSection {
    pub(crate) fn new(focus: FocusHandle) -> Self {
        Self {
            focus,
            scroll: ScrollHandle::new(),
            selected: 0,
            size: None,
            reading: false,
            working: false,
            message: None,
        }
    }
}

impl PreferencesView {
    // ---- reading (the tests) ----------------------------------------------

    /// The cache size as last read.
    #[cfg(test)]
    pub(crate) fn remote_size(&self) -> Option<u64> {
        self.remote.size
    }

    /// Whether a removal or a clear is under way.
    pub(crate) fn remote_working(&self) -> bool {
        self.remote.working
    }

    /// The selected row, as the list shows it (clamped).
    #[cfg(test)]
    pub(crate) fn remote_selected(&self) -> Option<String> {
        let hosts = self.remote_hosts();
        hosts
            .get(self.remote.selected.min(hosts.len().saturating_sub(1)))
            .cloned()
    }

    /// The allowed hosts, from the shared settings in memory, alphabetical.
    fn remote_hosts(&self) -> Vec<String> {
        sorted_hosts(
            self.settings
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .remote_image_hosts
                .clone(),
        )
    }

    // ---- acting ------------------------------------------------------------

    /// Read the cache size off the UI thread. With no cache, it reads 0.
    pub(crate) fn read_remote_size(&mut self, cx: &mut Context<Self>) {
        if self.remote.reading {
            return;
        }
        let Some(cache) = cx.default_global::<RemoteImages>().cache() else {
            self.remote.size = Some(0);
            return;
        };
        self.remote.reading = true;
        let size = blocking(move || cache.size());
        cx.spawn(async move |this, cx| {
            let size = size.await.unwrap_or_default();
            this.update(cx, |view, cx| {
                view.remote.reading = false;
                view.remote.size = Some(size);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Select row `ix` of the list (a click), giving it the keyboard.
    pub(crate) fn remote_select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.remote.selected = ix;
        self.remote.scroll.scroll_to_item(ix);
        window.focus(&self.remote.focus, cx);
        cx.notify();
    }

    /// Up / Down in the list (clamped).
    pub(crate) fn remote_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let count = self.remote_hosts().len();
        if count == 0 {
            return;
        }
        let ix = self.remote.selected.min(count - 1);
        self.remote.selected = if down {
            (ix + 1).min(count - 1)
        } else {
            ix.saturating_sub(1)
        };
        self.remote.scroll.scroll_to_item(self.remote.selected);
        cx.notify();
    }

    /// "Remove" (Delete, Backspace): the selected host goes.
    pub(crate) fn remote_remove_selected(&mut self, cx: &mut Context<Self>) {
        let hosts = self.remote_hosts();
        let Some(host) = hosts.get(self.remote.selected.min(hosts.len().saturating_sub(1))) else {
            return;
        };
        self.remote_remove(host.clone(), cx);
    }

    /// Forget `host` in the settings and, once that is on disk, in the
    /// store of remote images ([module docs](self)).
    fn remote_remove(&mut self, host: String, cx: &mut Context<Self>) {
        if self.remote.working {
            return;
        }
        let settings = self.settings.clone();
        let write = reopen::write_settings(self.store.clone(), settings.clone(), move |s| {
            s.forget_host(&host);
        });
        self.remote.working = true;
        self.remote.message = None;
        cx.spawn(async move |this, cx| {
            let written = write.await;
            if written.is_ok() {
                cx.update(|cx| {
                    reopen::sync_remote_hosts(&settings, cx);
                    cx.refresh_windows();
                });
            }
            this.update(cx, |view, cx| {
                view.remote.working = false;
                // The selection keeps its place, or moves to the new last
                // row. The stored index follows, so a host added later does
                // not pull the highlight along.
                let count = view.remote_hosts().len();
                view.remote.selected = view.remote.selected.min(count.saturating_sub(1));
                if let Err(e) = written {
                    tracing::warn!("could not save the allowed hosts: {e}");
                    view.remote.message = Some(Notice::error(format!(
                        "Couldn't save the allowed hosts: {e}"
                    )));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// "Clear cached images": core empties the cache off the UI thread, then
    /// the store forgets the session, and every window redraws.
    pub(crate) fn remote_clear(&mut self, cx: &mut Context<Self>) {
        if self.remote.working {
            return;
        }
        let Some(cache) = cx.default_global::<RemoteImages>().cache() else {
            return;
        };
        self.remote.working = true;
        self.remote.message = None;
        let work = blocking(move || {
            let cleared = cache.clear().map_err(|e| e.to_string());
            (cleared, cache.size())
        });
        cx.spawn(async move |this, cx| {
            let (cleared, size) = match work.await {
                Ok((cleared, size)) => (cleared, Some(size)),
                Err(e) => (Err(e), None),
            };
            // Part of the cache can be gone even when the clear failed.
            cx.update(|cx| {
                cx.default_global::<RemoteImages>()
                    .clone()
                    .forget_sessions();
                cx.refresh_windows();
            });
            this.update(cx, |view, cx| {
                view.remote.working = false;
                if size.is_some() {
                    view.remote.size = size;
                }
                if let Err(e) = cleared {
                    tracing::warn!("could not clear the cached images: {e}");
                    view.remote.message = Some(Notice::error(format!(
                        "Couldn't clear the cached images: {e}"
                    )));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    // ---- drawing -----------------------------------------------------------

    fn render_remote_host(
        &self,
        ix: usize,
        host: &str,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let hover = theme::hover_background(t);
        div()
            .id(remote_host_row_id(host))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(SharedString::from(host.to_string()))
            .aria_selected(selected)
            .w_full()
            .px_2()
            .py_1()
            .rounded_md()
            .truncate()
            .text_sm()
            .text_color(theme::foreground(t))
            .when(selected, |r| {
                r.bg(theme::selected_background(t))
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!selected, |r| r.hover(move |style| style.bg(hover)))
            .child(host.to_string())
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.remote_select(ix, window, cx)
            }))
            .into_any_element()
    }

    pub(crate) fn render_remote_images(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let hosts = self.remote_hosts();
        let focused = self.remote.focus.is_focused(window);
        let selected = self.remote.selected.min(hosts.len().saturating_sub(1));
        let border = if focused {
            theme::focus_border(t)
        } else {
            theme::divider(t)
        };
        let list: AnyElement = if hosts.is_empty() {
            div()
                .id(REMOTE_NO_HOSTS_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(NO_HOSTS)
                // The list held the keyboard when its last host went: the
                // empty box keeps it, so Esc and Tab still reach the view.
                .track_focus(&self.remote.focus)
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(border)
                .text_sm()
                .text_color(theme::muted(t))
                .child(NO_HOSTS)
                .into_any_element()
        } else {
            let rows: Vec<AnyElement> = hosts
                .iter()
                .enumerate()
                .map(|(ix, host)| self.render_remote_host(ix, host, ix == selected, cx))
                .collect();
            div()
                .id(REMOTE_HOSTS_ID)
                .test_support()
                .role(Role::ListBox)
                .aria_label("Allowed hosts")
                .track_focus(&self.remote.focus)
                .track_scroll(&self.remote.scroll)
                .max_h(rems(10.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_1()
                .p_1()
                .rounded_md()
                .border_1()
                .border_color(border)
                .children(rows)
                .into_any_element()
        };
        let size = size_text(self.remote.size);
        let has_cache = cx.default_global::<RemoteImages>().cache().is_some();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(note(
                "Images from these hosts load on their own. Other remote images load only \
                 when you click Load.",
                t,
            ))
            .child(list)
            .child(
                button_row()
                    .child(
                        Button::new(REMOTE_REMOVE_ID)
                            .label("Remove")
                            .disabled(self.remote.working || hosts.is_empty())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.remote_remove_selected(cx)
                            })),
                    )
                    .child(
                        Button::new(REMOTE_CLEAR_ID)
                            .label("Clear cached images")
                            .disabled(self.remote.working || !has_cache)
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.remote_clear(cx)),
                            ),
                    )
                    .child(
                        div()
                            .id(REMOTE_SIZE_ID)
                            .test_support()
                            .role(Role::Label)
                            .aria_label(SharedString::from(size.clone()))
                            .text_sm()
                            .text_color(theme::muted(t))
                            .child(size),
                    ),
            )
            .children(
                self.remote
                    .message
                    .as_ref()
                    .map(|n| notice_line(REMOTE_MESSAGE_ID, n, t)),
            )
            .into_any_element()
    }
}
