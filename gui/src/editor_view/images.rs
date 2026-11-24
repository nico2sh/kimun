//! The editor's **Inline image**s (CONTEXT.md): which lines embed a vault
//! image, which file each one draws, and how many lines it takes. The lines
//! go to the editor session ([`EditorSession::set_image_lines`]), whose
//! display reserves the room and hides the embed text off the cursor row.
//! After the frame is laid out, [`EditorView::image_draws`] says what each
//! image on screen draws (its pixels, an empty box while they decode, or a
//! failed image's note); the editor element paints it.
//!
//! An editor that edits draws images once its owner hands it the vault
//! ([`EditorView::set_image_vault`]). A read-only editor (a facet view's
//! chunk) ignores that, and draws them only from a read-only source
//! ([`EditorView::set_image_source`]): the note that resolves its bare
//! names, which need not be the session's, and the height of the pane it
//! scrolls in. Its display reveals no caret row, so every embed line shows
//! only its image or its note, and a read-only editor opens no image menu
//! and follows no image.
//!
//! An editor that edits walks the vault's attachment list off the main
//! thread, once per note shown and again each time the window becomes
//! active ([`EditorView::refresh_images`]: the app has no file watcher), and
//! keeps it. A read-only source takes the vault's shared list
//! ([`AttachmentIndex`]), so fifty chunks walk the vault once. Each frame
//! checks a key (the text revision, the note, the pane, the images' states)
//! and finds the embeds again only when it changed. The cursor row is the
//! display's concern.
//!
//! Pixels are decoded only for the images on screen, each time the set of
//! images on screen changes (a note opens, a scroll or a resize brings one
//! in). The images on screen are pinned in the cache, so none of them is
//! evicted to make room for another, and a decode that lands never asks for
//! another.
//!
//! Both kinds of editor also draw **Remote image**s (CONTEXT.md). Until an
//! address has a saved copy, its embed takes the two lines of the
//! placeholder ([`remote_rows`]). A saved copy is a file like a vault image:
//! its header gives its rows, and it is pinned and decoded as one. Only the
//! embeds on screen on an allowed host load on their own. In an editor that
//! edits, the embed on the caret row waits until the caret leaves it. When
//! a fetch ends, the lines are sized again once. Remote embeds count in the
//! first 100 embeds of a read-only editor. A read-only editor shows a
//! loaded remote image with its tooltip only: no menu and no follow.
//!
//! [`EditorSession::set_image_lines`]: kimun_notes::logic::editor::session::EditorSession::set_image_lines

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::{App, AppContext as _, Context, SharedString, Task, WeakEntity, Window};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor::display::ImageLine;

use super::EditorView;
use crate::attachment_index::{AttachmentIndex, list_sorted};
use crate::image_core::{
    Embed, ImageLook, ImagePane, RemoteLook, Source, embeds_of, image_file, look_from,
    natural_of_file, remote_rows, source_of,
};
use crate::inline_image::{ImageState, ImageStore, Pins};
use crate::remote_image::RemoteImages;

/// Fills a kit menu with the **Entry menu** of the attachment at a path:
/// what a right press on an inline image opens. The editor's owner builds
/// it, since the menu's picks are the owner's to run.
pub(crate) type EntryMenu =
    Rc<dyn Fn(PopupMenu, VaultPath, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// What one image on screen draws, in the lines the display gave it.
#[derive(Clone)]
pub(crate) struct ImageDraw {
    /// The embed's logical row.
    pub(crate) row: usize,
    /// The first line it takes, from the pane's top line (negative above
    /// it), and how many.
    pub(crate) top: isize,
    pub(crate) lines: usize,
    pub(crate) look: ImageLook,
    /// What the pointer over it shows: the alt text, else the file name.
    pub(crate) tip: SharedString,
    /// The attachment it draws: a right-click on it opens its **Entry
    /// menu**. `None` for a failed image's note and a remote image.
    pub(crate) path: Option<VaultPath>,
    /// The web address of a remote embed: a right-click on its loaded image
    /// opens the remote menu, and its placeholder's buttons act on it.
    pub(crate) url: Option<String>,
}

/// What a set of image lines was found for. The same key again finds the
/// same lines, so nothing is recomputed.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    revision: NonZeroU64,
    note: Option<VaultPath>,
    pane: ImagePane,
    /// Counts the attachment listings that landed, the decodes that
    /// failed, and the remote fetches that ended or copies removed: the
    /// events that change the lines.
    epoch: u64,
}

/// At most this many embeds of a read-only editor are drawn; later ones
/// stay text. Each one reads its file's header.
const READ_ONLY_EMBEDS: usize = 100;

/// Where a read-only editor's images come from
/// ([`EditorView::set_image_source`]).
#[derive(Clone, Debug, PartialEq)]
struct ReadOnlySource {
    /// The note that resolves bare names, in place of the session's.
    note: Option<VaultPath>,
    /// The height of the pane the editor scrolls in, in device pixels: an
    /// image takes at most 60% of it.
    viewport_px: f32,
}

/// The vault an editor draws images from, and what it knows of it.
pub(super) struct Images {
    vault: Arc<NoteVault>,
    /// Set for a read-only editor: its note and pane, and the attachments
    /// come from the shared list.
    source: Option<ReadOnlySource>,
    /// The vault's attachments, once listed.
    attachments: Option<Arc<Vec<VaultPath>>>,
    /// The note the last listing was started for: another note lists again.
    listed_for: Option<VaultPath>,
    /// The listing in flight. Dropping it cancels it.
    _listing: Option<Task<()>>,
    key: Option<Key>,
    epoch: u64,
    /// The embeds the lines were found from, by row, and those lines.
    pub(super) embeds: Vec<Embed>,
    lines: Vec<ImageLine>,
    /// For each remote embed, by row: whether it had a saved copy when the
    /// lines were sized. Another view can load or clear a copy, and this
    /// editor then sizes its lines again.
    saved: Vec<(usize, bool)>,
    /// The image files on screen at the last frame. Their decodes are asked
    /// for when this set changes, never when a decode lands. They are pinned
    /// in the store, so the cache never drops an image on screen: two
    /// images over the budget are both drawn, and cannot evict each other.
    visible: Vec<PathBuf>,
    /// The pins of `visible`, made at the first frame with an image.
    pins: Option<Pins>,
    /// How many times the embeds were found (tests).
    #[cfg(test)]
    finds: usize,
}

impl Images {
    fn new(vault: Arc<NoteVault>, attachments: Option<Arc<Vec<VaultPath>>>) -> Self {
        Self {
            vault,
            source: None,
            attachments,
            listed_for: None,
            _listing: None,
            key: None,
            epoch: 0,
            embeds: Vec::new(),
            lines: Vec::new(),
            saved: Vec::new(),
            visible: Vec::new(),
            pins: None,
            #[cfg(test)]
            finds: 0,
        }
    }

    /// Find the lines again at the next frame: the session that held them
    /// was replaced (another editor engine).
    pub(super) fn forget_lines(&mut self) {
        self.key = None;
    }

    /// Size the lines again at the next frame: a remote fetch ended, or a
    /// saved copy was removed, so an embed's rows may have changed.
    pub(super) fn resize_lines(&mut self) {
        self.epoch += 1;
    }

    /// Ask for the decodes of the images on screen again at the next frame:
    /// a file there was replaced at the same path.
    pub(super) fn ask_again(&mut self) {
        self.visible.clear();
    }
}

impl EditorView {
    /// Draw inline images from `vault` from now on, listing its attachments
    /// afresh before the next frame (an opened or renamed note). `None`
    /// turns every image line back into text. A read-only editor ignores it.
    pub(crate) fn set_image_vault(
        &mut self,
        vault: Option<Arc<NoteVault>>,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let old = self.images.take();
        match vault {
            None => {
                if old.is_some() {
                    self.editor.session_mut().set_image_lines(Vec::new());
                }
            }
            Some(vault) => {
                // The same vault keeps its old list until the new one lands,
                // so the images do not blink out and back.
                let attachments = old
                    .filter(|images| Arc::ptr_eq(&images.vault, &vault))
                    .and_then(|images| images.attachments);
                self.images = Some(Images::new(vault, attachments));
            }
        }
        cx.notify();
    }

    /// Draw inline images in a read-only editor from `vault`, resolving bare
    /// names from `note` and fitting each image to 60% of `viewport_px`
    /// (the height, in device pixels, of the pane the editor scrolls in).
    /// `None` turns every image line back into text. The attachments come
    /// from the vault's shared list. An editor that edits ignores it: its
    /// door is [`Self::set_image_vault`].
    pub(crate) fn set_image_source(
        &mut self,
        note: Option<VaultPath>,
        vault: Option<Arc<NoteVault>>,
        viewport_px: f32,
        cx: &mut Context<Self>,
    ) {
        if !self.read_only {
            return;
        }
        let source = ReadOnlySource { note, viewport_px };
        let Some(vault) = vault else {
            if self.images.take().is_some() {
                self.editor.session_mut().set_image_lines(Vec::new());
                cx.notify();
            }
            return;
        };
        if let Some(images) = self.images.as_mut()
            && Arc::ptr_eq(&images.vault, &vault)
        {
            if images.source.as_ref() == Some(&source) {
                return;
            }
            // Another note or pane: the lines are found again (the key
            // holds both). The list is the vault's, so it stays.
            images.source = Some(source);
            cx.notify();
            return;
        }
        let attachments = cx.default_global::<AttachmentIndex>().get(&vault);
        let mut images = Images::new(vault, attachments);
        images.source = Some(source);
        self.images = Some(images);
        cx.notify();
    }

    /// The room a read-only source fits its images to, for text laid out
    /// at `cols` cells `cell_px` wide in lines `line_px` tall (logical
    /// pixels), at `scale` device pixels per logical one. `None` when the
    /// editor draws no images from one.
    pub(crate) fn read_only_pane(
        &self,
        cols: u16,
        cell_px: f32,
        line_px: f32,
        scale: f32,
    ) -> Option<ImagePane> {
        let source = self.images.as_ref()?.source.as_ref()?;
        Some(ImagePane {
            column_px: f32::from(cols) * cell_px * scale,
            pane_px: source.viewport_px,
            row_px: line_px * scale,
        })
    }

    /// Whether screen row `row` draws an image or a failed image's note.
    /// The embed line there hides its link: a press in a read-only editor
    /// reaches no target on it.
    pub(crate) fn on_drawn_image(&self, row: u16) -> bool {
        if self
            .images
            .as_ref()
            .is_none_or(|images| images.embeds.is_empty())
        {
            return false;
        }
        let row = row as isize;
        self.editor
            .session()
            .display
            .image_spots()
            .iter()
            .any(|spot| (spot.top..spot.top + spot.lines as isize).contains(&row))
    }

    /// How many times this editor found its image lines (tests).
    #[cfg(test)]
    pub(crate) fn image_finds(&self) -> usize {
        self.images.as_ref().map_or(0, |images| images.finds)
    }

    /// The image files on screen at the last frame, which this editor pins
    /// (tests).
    #[cfg(test)]
    pub(crate) fn shown_image_files(&self) -> Vec<PathBuf> {
        self.images
            .as_ref()
            .map_or_else(Vec::new, |images| images.visible.clone())
    }

    /// Open `menu` on a right press on a drawn image from now on; `None`
    /// leaves that press to the text (the note menu).
    pub(crate) fn set_entry_menu(&mut self, menu: Option<EntryMenu>) {
        self.entry_menu = menu;
    }

    /// The menu a right press on an image opens now, as a right press on a
    /// link asks for the Link menu: only with nothing selected (a right
    /// press copies a selection) and the find bar shut (it owns the
    /// pointer while open). `None` in a read-only editor.
    pub(crate) fn image_menu(&self) -> Option<EntryMenu> {
        self.entry_menu.clone().filter(|_| self.menu_free())
    }

    /// Whether a right press on an image opens its menu now: nothing
    /// selected, the find bar shut, and an editor that edits.
    pub(super) fn menu_free(&self) -> bool {
        let unselected = self
            .editor
            .selection()
            .is_none_or(|(start, end)| start == end);
        !self.read_only && unselected && self.editor.find_bar().is_none()
    }

    /// The attachment the image on the caret's line draws, when that line
    /// is an image line whose file was found. A follow from there opens
    /// that file: a bare `![[name]]` takes the closest-file rule, which the
    /// link lookup does not know. `None` in a read-only editor: an image
    /// there follows nothing.
    pub(crate) fn image_at_cursor(&self) -> Option<VaultPath> {
        if self.read_only {
            return None;
        }
        let row = self.editor.cursor().0;
        self.images
            .as_ref()?
            .embeds
            .iter()
            .find(|embed| embed.row == row)?
            .found
            .clone()
    }

    /// Before a frame: hand the session the image lines of the text as it is
    /// now, sized for `pane`. Finds them again only when something they
    /// depend on changed. Starts the attachment listing for a note not
    /// listed yet (a read-only source asks for the shared list once), and
    /// the decode of an image first seen.
    pub(crate) fn prepare_images(&mut self, pane: ImagePane, cx: &mut Context<Self>) {
        let Some(images) = self.images.as_mut() else {
            return;
        };
        let note = match &images.source {
            Some(source) => source.note.clone(),
            None => self.editor.session().note.clone(),
        };
        if images.source.is_some() {
            if images.listed_for.is_none() && note.is_some() {
                images.listed_for = note.clone();
                ask_shared_list(&images.vault, cx);
            }
        } else if note.is_some() && images.listed_for != note {
            images.listed_for = note.clone();
            let vault = images.vault.clone();
            let listing = cx.background_spawn(async move { list_sorted(&vault) });
            images._listing = Some(cx.spawn(async move |this, cx| {
                let found = listing.await;
                let _ = this.update(cx, |view, cx| {
                    // The same list again (a note reopened, an activation
                    // with nothing new) finds nothing new.
                    if let Some(images) = view.images.as_mut()
                        && images.attachments.as_deref() != Some(&found)
                    {
                        images.attachments = Some(Arc::new(found));
                        images.epoch += 1;
                        cx.notify();
                    }
                });
            }));
        }
        let key = Key {
            revision: self.editor.content_revision(),
            note: note.clone(),
            pane,
            epoch: images.epoch,
        };
        // A text that embeds nothing embeds nothing in another pane either:
        // a resize costs it nothing.
        let pane_only = images.key.as_ref().is_some_and(|old| {
            old.revision == key.revision && old.note == key.note && old.epoch == key.epoch
        });
        if images.key.as_ref() == Some(&key) || (pane_only && images.embeds.is_empty()) {
            images.key = Some(key);
            return;
        }
        images.key = Some(key);
        #[cfg(test)]
        {
            images.finds += 1;
        }
        let limit = images.source.as_ref().map(|_| READ_ONLY_EMBEDS);
        images.embeds = match (&note, &images.attachments) {
            (Some(note), Some(attachments)) => embeds_of(
                &self.editor.get_text(),
                note,
                attachments,
                &images.vault,
                limit,
            ),
            // Until the attachments are listed no embed resolves: the lines
            // stay text rather than flash "not found".
            _ => Vec::new(),
        };
        (images.lines, images.saved) = image_lines(&images.embeds, pane, cx);
        self.editor
            .session_mut()
            .set_image_lines(images.lines.clone());
    }

    /// Read again what may have changed on disk since the images were found
    /// (the window was inactive: a file saved, rewritten or deleted from
    /// outside the app). The attachments are listed again before the next
    /// frame, the lines found again only when a file's size changed, and the
    /// images on screen asked for again: one whose file changed is decoded
    /// again, one still cached is not.
    pub(crate) fn refresh_images(&mut self, cx: &mut Context<Self>) {
        let Some(images) = self.images.as_mut() else {
            return;
        };
        images.listed_for = None;
        images.visible.clear();
        if let Some(key) = &images.key
            && image_lines(&images.embeds, key.pane, cx).0 != images.lines
        {
            images.epoch += 1;
        }
        cx.notify();
    }

    /// After the frame is laid out: what each image on screen draws, sized
    /// for `pane`. When the set of images on screen changed since the last
    /// frame, the decode of each one not decoded is asked for. `on_screen`
    /// is the range of lines, from the pane's top line, that the window
    /// shows (a read-only chunk is taller than the screen); `None` when
    /// every line of the pane is on screen. Only a placeholder that crosses
    /// it starts an auto load.
    pub(crate) fn image_draws(
        &mut self,
        pane: ImagePane,
        on_screen: Option<std::ops::Range<isize>>,
        cx: &mut Context<Self>,
    ) -> Vec<ImageDraw> {
        let Some(images) = self.images.as_mut() else {
            return Vec::new();
        };
        // Most notes embed nothing: no work per frame.
        if images.embeds.is_empty() {
            if !images.visible.is_empty() {
                images.visible.clear();
                if let Some(pins) = &images.pins {
                    pins.set(&[]);
                }
            }
            return Vec::new();
        }
        let spots = self.editor.session().display.image_spots();
        // The embed on the caret row of an editor that edits loads only once
        // the caret leaves it: while the reader types its address, each
        // keystroke makes a new address.
        let caret_row = (!self.read_only).then(|| self.editor.cursor().0);
        let mut store = cx.default_global::<ImageStore>().clone();
        let remote = cx.default_global::<RemoteImages>().clone();
        let mut visible = Vec::new();
        let mut draws = Vec::new();
        // The remote embeds on screen whose host is allowed and that wait
        // for their first load, but not the one on the caret row.
        let mut to_load = Vec::new();
        // A remote embed whose copy came or went since the lines were sized.
        let mut stale = false;
        for spot in spots {
            let Some(embed) = images.embeds.iter().find(|e| e.row == spot.row) else {
                continue;
            };
            let url = embed.remote.as_ref().map(|r| r.url.clone());
            // The vault file, or the saved copy of a remote embed.
            let source = source_of(embed, &remote);
            let file = match &source {
                Source::File(file) => file.clone(),
                Source::Remote(_) => None,
            };
            stale |= images
                .saved
                .iter()
                .any(|&(row, saved)| row == embed.row && saved != file.is_some());
            let look = look_from(embed, source, &mut store, pane, spot.note);
            let look = match look {
                ImageLook::Remote(look) => {
                    let shown = on_screen.as_ref().is_none_or(|lines| {
                        spot.top < lines.end && spot.top + spot.lines as isize > lines.start
                    });
                    if let (RemoteLook::Placeholder { host, actions }, Some(url)) = (&look, &url)
                        && !actions.always
                        && shown
                        && caret_row != Some(embed.row)
                    {
                        to_load.push((url.clone(), host.clone()));
                    }
                    draws.push(ImageDraw {
                        row: spot.row,
                        top: spot.top,
                        lines: spot.lines,
                        look: ImageLook::Remote(look),
                        tip: embed.tip(),
                        path: None,
                        url,
                    });
                    continue;
                }
                // It failed since the lines were found: they are found again
                // at the next frame.
                ImageLook::Note(_) if !spot.note => continue,
                look => look,
            };
            if let (ImageLook::Ready(..) | ImageLook::Pending(_), Some(file)) = (&look, &file)
                && !visible.contains(file)
            {
                visible.push(file.clone());
            }
            let (path, url) = match look {
                ImageLook::Note(_) => (None, None),
                _ => (embed.found.clone(), url),
            };
            draws.push(ImageDraw {
                row: spot.row,
                top: spot.top,
                lines: spot.lines,
                look,
                tip: embed.tip(),
                path,
                url,
            });
        }
        if stale {
            images.resize_lines();
            cx.notify();
        }
        if !to_load.is_empty() {
            // The store skips an address that is saved, loading, or failed in
            // this session, so a dead address is asked once.
            let view = cx.weak_entity();
            let mut remote = remote.clone();
            remote.auto_load(&to_load, cx, move |cx| {
                super::remote::fetch_ended(&view, cx)
            });
        }
        if visible != images.visible {
            // Pinned first: a decode that lands never evicts an image on
            // screen.
            images
                .pins
                .get_or_insert_with(|| store.pins())
                .set(&visible);
            let view = cx.weak_entity();
            for file in &visible {
                if matches!(store.size_of(file), ImageState::Sized { .. }) {
                    store.request(file, cx, on_decoded(view.clone(), file.clone()));
                }
            }
            images.visible = visible;
        }
        draws
    }
}

/// Ask for the vault's shared attachment list. When it is there, the
/// editor takes it and finds its lines again if it is new to it.
fn ask_shared_list(vault: &Arc<NoteVault>, cx: &mut Context<EditorView>) {
    let mut index = cx.default_global::<AttachmentIndex>().clone();
    let view = cx.weak_entity();
    let shared = index.clone();
    let weak = Arc::downgrade(vault);
    index.request(vault, cx, move |cx| {
        let Some(vault) = weak.upgrade() else {
            return;
        };
        let list = shared.get(&vault);
        let _ = view.update(cx, |view, cx| {
            if let Some(images) = view.images.as_mut()
                && Arc::ptr_eq(&images.vault, &vault)
                && !same_list(images.attachments.as_ref(), list.as_ref())
            {
                images.attachments = list;
                images.epoch += 1;
                cx.notify();
            }
        });
    });
}

/// Whether two lists are the same shared list (every chunk holds the one
/// the index gave out, so no element is compared).
fn same_list(a: Option<&Arc<Vec<VaultPath>>>, b: Option<&Arc<Vec<VaultPath>>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// When a decode of `file` lands: redraw `view`. A decode that failed also
/// changes the lines (the image takes no room any more), so they are found
/// again. A decoded image takes the room its header gave, so nothing else
/// is recomputed.
fn on_decoded(view: WeakEntity<EditorView>, file: PathBuf) -> impl FnOnce(&mut App) + 'static {
    move |cx: &mut App| {
        let mut store = cx.default_global::<ImageStore>().clone();
        let failed = matches!(store.size_of(&file), ImageState::Failed(_));
        let _ = view.update(cx, |view, cx| {
            if failed && let Some(images) = view.images.as_mut() {
                images.epoch += 1;
            }
            cx.notify();
        });
    }
}

/// The image line of each embed, sized from its file's header, and for
/// each remote embed its row and whether it has a saved copy now. Nothing
/// is decoded here: [`EditorView::image_draws`] asks for the images on
/// screen. A remote embed with no saved copy takes the placeholder's rows.
/// A saved copy takes the rows of its height, from the saved file's header.
/// Each embed's file is looked up once: for a remote embed that is a look
/// in the cache folder on disk.
fn image_lines(
    embeds: &[Embed],
    pane: ImagePane,
    cx: &mut App,
) -> (Vec<ImageLine>, Vec<(usize, bool)>) {
    let mut store = cx.default_global::<ImageStore>().clone();
    let remote = cx.default_global::<RemoteImages>().clone();
    let mut saved = Vec::new();
    let lines = embeds
        .iter()
        .map(|embed| {
            let file = image_file(embed, &remote);
            if embed.remote.is_some() {
                saved.push((embed.row, file.is_some()));
            }
            let natural = file
                .as_deref()
                .and_then(|file| natural_of_file(file, &mut store));
            let rows = match natural {
                Some(natural) => pane.rows(natural),
                // A remote embed with no saved copy shows its placeholder.
                None if embed.remote.is_some() && file.is_none() => remote_rows(),
                None => 0,
            };
            ImageLine {
                row: embed.row,
                rows,
            }
        })
        .collect();
    (lines, saved)
}
