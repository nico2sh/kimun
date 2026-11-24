//! Preferences > **Remote images**: the allowed hosts and the image cache
//! (adr/0063). Like Workspaces, it acts at once, outside the draft.
//!
//! - The list shows each allowed host of the shared settings with a
//!   "Remove" button. "Remove" writes the settings through the window's one
//!   settings write (off the UI thread, in turn with the other writes) with
//!   the host forgotten. Once it is on disk, the store of remote images
//!   takes the new list, so an open view stops loading that host on its
//!   own at once. The saved copies stay. A failed write is shown in the
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
use gpui_kit::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, div,
};

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

/// The element id of the "Remove" button of `host`.
pub(crate) fn remote_remove_id(host: &str) -> SharedString {
    format!("preferences-remote-remove-{host}").into()
}

/// `bytes` in a human unit, with one decimal past bytes: "0 B", "512 B",
/// "1.5 KB", "12.3 MB".
pub(crate) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The text beside "Clear cached images": the size, or that it is being
/// read.
pub(crate) fn size_text(size: Option<u64>) -> String {
    match size {
        Some(bytes) => format!("Cache size: {}", human_size(bytes)),
        None => "Cache size: reading…".to_string(),
    }
}

/// The section's state.
#[derive(Default)]
pub(crate) struct RemoteSection {
    /// The cache size, once read. `None` while it is read.
    size: Option<u64>,
    /// A size read under way.
    reading: bool,
    /// A removal or a clear under way: the buttons wait for it.
    working: bool,
    message: Option<Notice>,
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

    /// The allowed hosts, from the shared settings in memory.
    fn remote_hosts(&self) -> Vec<String> {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .remote_image_hosts
            .clone()
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

    /// "Remove" on `host`: forget it in the settings and, once that is on
    /// disk, in the store of remote images ([module docs](self)).
    pub(crate) fn remote_remove(&mut self, host: String, cx: &mut Context<Self>) {
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

    fn render_remote_host(&self, host: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let remove = host.to_string();
        div()
            .id(remote_host_row_id(host))
            .test_support()
            .role(Role::ListItem)
            .aria_label(SharedString::from(host.to_string()))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .px_2()
            .py_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme::foreground(t))
                    .child(host.to_string()),
            )
            .child(
                Button::new(remote_remove_id(host))
                    .label("Remove")
                    .disabled(self.remote.working)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.remote_remove(remove.clone(), cx)
                    })),
            )
            .into_any_element()
    }

    pub(crate) fn render_remote_images(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let hosts = self.remote_hosts();
        let list: AnyElement = if hosts.is_empty() {
            div()
                .id(REMOTE_NO_HOSTS_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(NO_HOSTS)
                .text_sm()
                .text_color(theme::muted(t))
                .child(NO_HOSTS)
                .into_any_element()
        } else {
            div()
                .id(REMOTE_HOSTS_ID)
                .test_support()
                .role(Role::List)
                .aria_label("Allowed hosts")
                .flex()
                .flex_col()
                .children(hosts.iter().map(|host| self.render_remote_host(host, cx)))
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
