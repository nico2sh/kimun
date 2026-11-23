//! The editor's **Inline image**s (CONTEXT.md): which lines embed a vault
//! image, which file each one draws, and how many lines it takes. The lines
//! go to the editor session ([`EditorSession::set_image_lines`]), whose
//! display reserves the room and hides the embed text off the cursor row.
//! After the frame is laid out, [`EditorView::image_draws`] says what each
//! image on screen draws (its pixels, an empty box while they decode, or a
//! failed image's note); the editor element paints it.
//!
//! Only an editor that edits draws images, once its owner hands it the vault
//! ([`EditorView::set_image_vault`]); a read-only editor (a facet view's
//! piece) never does. The vault's attachment list is walked off the main
//! thread, once per note shown and again each time the window becomes
//! active ([`EditorView::refresh_images`]: the app has no file watcher), and
//! kept. Each frame checks a key (the text
//! revision, the note, the pane, the images' states) and finds the embeds
//! again only when it changed. The cursor row is the display's concern.
//!
//! Pixels are decoded only for the images on screen, each time the set of
//! images on screen changes (a note opens, a scroll or a resize brings one
//! in). The images on screen are pinned in the cache, so none of them is
//! evicted to make room for another, and a decode that lands never asks for
//! another.
//!
//! [`EditorSession::set_image_lines`]: kimun_notes::logic::editor::session::EditorSession::set_image_lines

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::{
    App, AppContext as _, Context, RenderImage, SharedString, Task, WeakEntity, Window,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::scan::link_target_filename;
use kimun_core::note::{ImageEmbed, image_embeds, resolve_image_embed};
use kimun_notes::logic::editor::display::ImageLine;

use super::EditorView;
use crate::inline_image::{Failure, ImageState, ImageStore, Pins, fit, rows_for};

/// Fills a kit menu with the **Entry menu** of the attachment at a path:
/// what a right press on an inline image opens. The editor's owner builds
/// it, since the menu's picks are the owner's to run.
pub(crate) type EntryMenu =
    Rc<dyn Fn(PopupMenu, VaultPath, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// The room an image is fitted to, in device pixels (the unit of
/// [`fit`]): the text column's width, the pane's height (`fit` takes 60% of
/// it) and one line's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImagePane {
    pub(crate) column_px: f32,
    pub(crate) pane_px: f32,
    pub(crate) row_px: f32,
}

/// One embed line, as found with the lines: what it draws and how it is
/// named.
#[derive(Clone, Debug)]
struct Embed {
    row: usize,
    /// The attachment it resolved to, and its file.
    found: Option<VaultPath>,
    file: Option<PathBuf>,
    /// The file name as written (the embed target's last part).
    name: String,
    /// The alt text of `![alt](path)`; empty for `![[name]]`.
    alt: String,
}

impl Embed {
    fn of(embed: &ImageEmbed, found: Option<VaultPath>, file: Option<PathBuf>) -> Self {
        let target = embed.target.trim_start_matches('<').trim_end_matches('>');
        Self {
            row: embed.row,
            found,
            file,
            name: link_target_filename(target).to_string(),
            alt: embed.alt.clone(),
        }
    }

    /// The hover text: the alt text, or the file name without one.
    fn tip(&self) -> SharedString {
        if self.alt.is_empty() {
            self.name.clone().into()
        } else {
            self.alt.clone().into()
        }
    }
}

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
    /// menu**. `None` for a failed image's note.
    pub(crate) path: Option<VaultPath>,
}

/// How an image on screen looks.
#[derive(Clone)]
pub(crate) enum ImageLook {
    /// Its pixels, at the fitted size (device pixels).
    Ready(Arc<RenderImage>, (f32, f32)),
    /// An empty box of the fitted size: the pixels are not decoded (yet, or
    /// any more).
    Pending((f32, f32)),
    /// A one-line note in the embed line: the image cannot be drawn.
    Note(String),
}

/// What a set of image lines was found for. The same key again finds the
/// same lines, so nothing is recomputed.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    revision: NonZeroU64,
    note: Option<VaultPath>,
    pane: ImagePane,
    /// Counts the attachment listings that landed and the decodes that
    /// failed: the events that change the lines.
    epoch: u64,
}

/// The vault an editor draws images from, and what it knows of it.
pub(super) struct Images {
    vault: Arc<NoteVault>,
    /// The vault's attachments, once listed.
    attachments: Option<Arc<Vec<VaultPath>>>,
    /// The note the last listing was started for: another note lists again.
    listed_for: Option<VaultPath>,
    /// The listing in flight. Dropping it cancels it.
    _listing: Option<Task<()>>,
    key: Option<Key>,
    epoch: u64,
    /// The embeds the lines were found from, by row, and those lines.
    embeds: Vec<Embed>,
    lines: Vec<ImageLine>,
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
            attachments,
            listed_for: None,
            _listing: None,
            key: None,
            epoch: 0,
            embeds: Vec::new(),
            lines: Vec::new(),
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

    /// How many times this editor found its image lines (tests).
    #[cfg(test)]
    pub(crate) fn image_finds(&self) -> usize {
        self.images.as_ref().map_or(0, |images| images.finds)
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
        let unselected = self
            .editor
            .selection()
            .is_none_or(|(start, end)| start == end);
        let free = !self.read_only && unselected && self.editor.find_bar().is_none();
        self.entry_menu.clone().filter(|_| free)
    }

    /// The attachment the image on the caret's line draws, when that line
    /// is an image line whose file was found. A follow from there opens
    /// that file: a bare `![[name]]` takes the closest-file rule, which the
    /// link lookup does not know.
    pub(crate) fn image_at_cursor(&self) -> Option<VaultPath> {
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
    /// listed yet, and the decode of an image first seen.
    pub(crate) fn prepare_images(&mut self, pane: ImagePane, cx: &mut Context<Self>) {
        let Some(images) = self.images.as_mut() else {
            return;
        };
        let note = self.editor.session().note.clone();
        if note.is_some() && images.listed_for != note {
            images.listed_for = note.clone();
            let vault = images.vault.clone();
            let listing = cx.background_spawn(async move {
                let mut found = vault.list_attachments();
                found.sort();
                found
            });
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
        if images.key.as_ref() == Some(&key) {
            return;
        }
        images.key = Some(key);
        #[cfg(test)]
        {
            images.finds += 1;
        }
        images.embeds = match (&note, &images.attachments) {
            (Some(note), Some(attachments)) => {
                embeds(&self.editor.get_text(), note, attachments, &images.vault)
            }
            // Until the attachments are listed no embed resolves: the lines
            // stay text rather than flash "not found".
            _ => Vec::new(),
        };
        images.lines = image_lines(&images.embeds, pane, cx);
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
            && image_lines(&images.embeds, key.pane, cx) != images.lines
        {
            images.epoch += 1;
        }
        cx.notify();
    }

    /// After the frame is laid out: what each image on screen draws, sized
    /// for `pane`. When the set of images on screen changed since the last
    /// frame, the decode of each one not decoded is asked for.
    pub(crate) fn image_draws(
        &mut self,
        pane: ImagePane,
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
        let mut store = cx.default_global::<ImageStore>().clone();
        let mut visible = Vec::new();
        let mut draws = Vec::new();
        for spot in spots {
            let Some(embed) = images.embeds.iter().find(|e| e.row == spot.row) else {
                continue;
            };
            let state = embed.file.as_ref().map(|file| store.size_of(file));
            let fitted = |natural| fit(natural, pane.column_px, pane.pane_px);
            let look = match state {
                _ if spot.note => {
                    let failure = match state {
                        Some(ImageState::Failed(failure)) => failure,
                        _ => Failure::Missing,
                    };
                    ImageLook::Note(failure.note(&embed.name, &embed.alt))
                }
                Some(ImageState::Ready(image, width, height)) => {
                    ImageLook::Ready(image, fitted((width, height)))
                }
                Some(ImageState::Sized { width, height }) => {
                    ImageLook::Pending(fitted((width, height)))
                }
                // It failed since the lines were found: they are found again
                // at the next frame.
                Some(ImageState::Failed(_)) | None => continue,
            };
            if let (ImageLook::Ready(..) | ImageLook::Pending(_), Some(file)) = (&look, &embed.file)
                && !visible.contains(file)
            {
                visible.push(file.clone());
            }
            let path = match look {
                ImageLook::Note(_) => None,
                _ => embed.found.clone(),
            };
            draws.push(ImageDraw {
                row: spot.row,
                top: spot.top,
                lines: spot.lines,
                look,
                tip: embed.tip(),
                path,
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

/// The whole-line embeds of `text`, the note at `note`, each resolved
/// against `attachments`.
fn embeds(
    text: &str,
    note: &VaultPath,
    attachments: &[VaultPath],
    vault: &NoteVault,
) -> Vec<Embed> {
    // Most notes embed nothing: skip the scan.
    if !text.contains("![") {
        return Vec::new();
    }
    image_embeds(text)
        .iter()
        .map(|embed| {
            let found = resolve_image_embed(embed, note, attachments);
            let file = found.as_ref().map(|found| vault.path_to_pathbuf(found));
            Embed::of(embed, found, file)
        })
        .collect()
}

/// The image line of each embed, sized from its file's header. Nothing is
/// decoded here: [`EditorView::image_draws`] asks for the images on screen.
fn image_lines(embeds: &[Embed], pane: ImagePane, cx: &mut App) -> Vec<ImageLine> {
    let mut store = cx.default_global::<ImageStore>().clone();
    embeds
        .iter()
        .map(|embed| {
            let natural = match embed.file.as_ref().map(|file| store.size_of(file)) {
                Some(ImageState::Sized { width, height } | ImageState::Ready(_, width, height)) => {
                    Some((width, height))
                }
                // Missing, unsupported, undecodable, too large: no room,
                // the note takes the embed line.
                Some(ImageState::Failed(_)) | None => None,
            };
            let rows = natural.map_or(0, |natural| {
                let (_, height) = fit(natural, pane.column_px, pane.pane_px);
                rows_for(height, pane.row_px)
            });
            ImageLine {
                row: embed.row,
                rows,
            }
        })
        .collect()
}
