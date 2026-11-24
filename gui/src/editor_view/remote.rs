//! The editor's **Remote image**s (CONTEXT.md): the presses on a
//! placeholder's buttons, the menu of a loaded remote image, and the follow
//! of a remote embed's line.
//!
//! The paint step records each placeholder's buttons ([`RemoteHit`]). A
//! left press on a button runs its action and does not move the caret or
//! start a selection. A right press on a placeholder opens no menu. Any
//! other press on a placeholder is a press on the embed line.
//!
//! "Load" and "Retry" ask the shared store ([`RemoteImages`]) through the
//! placeholder piece's `run_action`, as the read-only views do. "Always
//! load from" asks the editor's owner, which writes the host to the
//! settings first ([`EditorEvent::AlwaysLoadFrom`]). "Open in browser", a
//! double-click and the `FollowLink` chord hand the address to the owner's
//! system handler ([`EditorEvent::OpenRemoteImage`]).
//!
//! When a fetch ends, the editor sizes its image lines again and redraws,
//! once. A fetch that ends never starts another.

use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{
    App, Bounds, ClipboardItem, Context, InteractiveElement as _, ParentElement as _, Pixels,
    Point, Role, SharedString, StatefulInteractiveElement as _, WeakEntity, Window, div,
};
use kimun_notes::logic::message::AppMessage;

use super::{EditorEvent, EditorView};
use crate::editor_element::image_tooltip;
use crate::remote_image::RemoteImages;
use crate::remote_placeholder::{RemoteAction, action_at, run_action};

/// The id of the "Clear image cache" item of the remote menu.
pub(crate) const CLEAR_CACHE_ITEM_ID: &str = "remote-image-clear-cache";
/// The tooltip of the "Clear image cache" item.
const CLEAR_CACHE_TIP: &str = "Remove the saved copy of this image";
/// The flash after "Copy image address".
const ADDRESS_COPIED: &str = "Image address copied";

/// Builds the menu a right press on a loaded remote image opens.
pub(crate) type RemoteMenu =
    Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// A placeholder as the last frame painted it: where a press acts.
#[derive(Clone, Debug)]
pub(crate) struct RemoteHit {
    /// The web address of the embed.
    pub(crate) url: String,
    /// The host the "Always load from" button names: the embed's host, or
    /// the host of a blocked redirect.
    pub(crate) host: Option<String>,
    /// The visible part of the placeholder, in window coordinates.
    pub(crate) area: Bounds<Pixels>,
    /// The buttons, from `paint_remote_look`.
    pub(crate) buttons: Vec<(Bounds<Pixels>, RemoteAction)>,
}

/// What a press on a placeholder does.
pub(super) enum PlaceholderPress {
    /// A left press on a button: run its action.
    Button(String, Option<String>, RemoteAction),
    /// A right press on the placeholder: nothing opens.
    Swallow,
}

impl EditorView {
    /// What a press at `at` does on a placeholder, when it lands on one. A
    /// left press elsewhere on a placeholder is a press on the embed line:
    /// `None`.
    pub(super) fn placeholder_press(
        &self,
        at: Point<Pixels>,
        left: bool,
    ) -> Option<PlaceholderPress> {
        let hit = self.remote_hits.iter().find(|hit| hit.area.contains(&at))?;
        if left {
            let action = action_at(&hit.buttons, at)?;
            return Some(PlaceholderPress::Button(
                hit.url.clone(),
                hit.host.clone(),
                action,
            ));
        }
        // A right press with a selection copies it, as on any line.
        let unselected = !self.has_selection();
        unselected.then_some(PlaceholderPress::Swallow)
    }

    /// Run the placeholder button `action` for the remote embed `url`.
    /// `host` is the host the "Always load from" button names.
    pub(super) fn run_remote_action(
        &mut self,
        url: String,
        host: Option<String>,
        action: RemoteAction,
        cx: &mut Context<Self>,
    ) {
        let view = cx.weak_entity();
        if let Some(host) = run_action(&url, host, action, cx, move |cx| fetch_ended(&view, cx)) {
            cx.emit(EditorEvent::AlwaysLoadFrom { host, url });
        }
        cx.notify();
    }

    /// Load the remote image `url` at the reader's request. When the fetch
    /// ends, this editor sizes its image lines again and redraws.
    pub(crate) fn load_remote(&mut self, url: &str, cx: &mut Context<Self>) {
        let mut store = cx.default_global::<RemoteImages>().clone();
        let view = cx.weak_entity();
        store.load(url, cx, move |cx| fetch_ended(&view, cx));
        cx.notify();
    }

    /// Fetch `url` again. The saved copy stays drawn until the new bytes
    /// replace it, and a failed fetch keeps it.
    fn reload_remote(&mut self, url: &str, cx: &mut Context<Self>) {
        let mut store = cx.default_global::<RemoteImages>().clone();
        let view = cx.weak_entity();
        store.reload(url, cx, move |cx| fetch_ended(&view, cx));
        self.remote_changed(cx);
    }

    /// Remove the saved copy of `url`. Its line shows the placeholder again.
    fn clear_remote(&mut self, url: &str, cx: &mut Context<Self>) {
        cx.default_global::<RemoteImages>().clone().clear_copy(url);
        self.remote_changed(cx);
    }

    /// A remote image changed: size the lines again and redraw. The images
    /// on screen are asked for again at the next frame: a Reload replaces a
    /// saved copy at the same path, so the set of files on screen does not
    /// change, and the new pixels need a decode.
    pub(crate) fn remote_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(images) = self.images.as_mut() {
            images.resize_lines();
            images.ask_again();
        }
        cx.notify();
    }

    /// The address of the remote embed on the caret's line, when that line
    /// is a remote embed. It checks no scheme: the workspace's
    /// `open_remote_image` opens only an `http` or `https` address in the
    /// browser. `None` in a read-only editor.
    pub(crate) fn remote_image_at_cursor(&self) -> Option<String> {
        if self.read_only {
            return None;
        }
        let row = self.editor.cursor().0;
        let embed = self
            .images
            .as_ref()?
            .embeds
            .iter()
            .find(|embed| embed.row == row)?;
        embed.remote.as_ref().map(|remote| remote.url.clone())
    }

    /// The menu a right press on the loaded remote image `url` opens, under
    /// the rule of [`Self::image_menu`]: nothing selected and the find bar
    /// shut. `None` in a read-only editor.
    pub(crate) fn remote_menu(&self, url: &str, cx: &Context<Self>) -> Option<RemoteMenu> {
        if !self.menu_free() {
            return None;
        }
        Some(remote_menu(cx.weak_entity(), url.to_string()))
    }
}

/// A fetch for `view` ended: it sizes its image lines again and redraws.
pub(super) fn fetch_ended(view: &WeakEntity<EditorView>, cx: &mut App) {
    let _ = view.update(cx, |view, cx| view.remote_changed(cx));
}

/// An item of the menu of a loaded remote image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemotePick {
    OpenInBrowser,
    CopyAddress,
    Reload,
    ClearCache,
}

impl EditorView {
    /// Run `pick`, chosen in the menu of the loaded remote image `url`.
    fn run_remote_pick(&mut self, pick: RemotePick, url: &str, cx: &mut Context<Self>) {
        match pick {
            RemotePick::OpenInBrowser => cx.emit(EditorEvent::OpenRemoteImage {
                url: url.to_string(),
            }),
            RemotePick::CopyAddress => {
                cx.write_to_clipboard(ClipboardItem::new_string(url.to_string()));
                self.tx
                    .send(AppMessage::FlashMessage(ADDRESS_COPIED.to_string()))
                    .ok();
            }
            RemotePick::Reload => self.reload_remote(url, cx),
            RemotePick::ClearCache => self.clear_remote(url, cx),
        }
    }
}

/// The menu of a loaded remote image: "Open in browser", "Copy image
/// address", "Reload", and "Clear image cache". A pick runs once the menu
/// has given the keyboard back.
fn remote_menu(view: WeakEntity<EditorView>, url: String) -> RemoteMenu {
    Rc::new(move |menu, _, _| {
        let on = |pick: RemotePick| {
            let (view, url) = (view.clone(), url.clone());
            move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let (view, url) = (view.clone(), url.clone());
                window.defer(cx, move |_, cx| {
                    let _ = view.update(cx, |view, cx| view.run_remote_pick(pick, &url, cx));
                });
            }
        };
        menu.item(PopupMenuItem::new("Open in browser").on_click(on(RemotePick::OpenInBrowser)))
            .item(PopupMenuItem::new("Copy image address").on_click(on(RemotePick::CopyAddress)))
            .item(PopupMenuItem::new("Reload").on_click(on(RemotePick::Reload)))
            .item(
                PopupMenuItem::element(|_, _| {
                    let label = SharedString::from("Clear image cache");
                    div()
                        .id(CLEAR_CACHE_ITEM_ID)
                        .role(Role::MenuItem)
                        .aria_label(label.clone())
                        .tooltip(image_tooltip(CLEAR_CACHE_TIP.into()))
                        .test_support()
                        .child(label)
                })
                .on_click(on(RemotePick::ClearCache)),
            )
    })
}
