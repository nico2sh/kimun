//! The editor's **Inline image**s (CONTEXT.md): which lines embed a vault
//! image, which file each one draws, and how many lines it takes. The lines
//! go to the editor session ([`EditorSession::set_image_lines`]), whose
//! display reserves the room and hides the embed text off the cursor row.
//! This module paints nothing.
//!
//! Only an editor that edits draws images, once its owner hands it the vault
//! ([`EditorView::set_image_vault`]); a read-only editor (a facet view's
//! piece) never does. The vault's attachment list is walked off the main
//! thread, once per note shown, and kept. Each frame checks a key (the text
//! revision, the note, the pane, the images' states) and finds the embeds
//! again only when it changed. The cursor row is the display's concern.
//!
//! [`EditorSession::set_image_lines`]: kimun_notes::logic::editor::session::EditorSession::set_image_lines

use std::collections::HashSet;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Context, Task, WeakEntity};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::{image_embeds, resolve_image_embed};
use kimun_notes::logic::editor::display::ImageLine;

use super::EditorView;
use crate::inline_image::{ImageState, ImageStore, fit, rows_for};

/// The room an image is fitted to, in device pixels (the unit of
/// [`fit`]): the text column's width, the pane's height (`fit` takes 60% of
/// it) and one line's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImagePane {
    pub(crate) column_px: f32,
    pub(crate) pane_px: f32,
    pub(crate) row_px: f32,
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
    /// The files this editor asked to decode, since the vault was handed in
    /// (a note opened). Each is asked once: an image the cache later drops
    /// to stay under its budget is not decoded again, or two images over
    /// the budget would evict each other and decode forever.
    requested: HashSet<PathBuf>,
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
            requested: HashSet::new(),
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
            let listing = cx.background_spawn(async move { vault.list_attachments() });
            images._listing = Some(cx.spawn(async move |this, cx| {
                let found = listing.await;
                let _ = this.update(cx, |view, cx| {
                    if let Some(images) = view.images.as_mut() {
                        images.attachments = Some(Arc::new(found));
                        images.epoch += 1;
                    }
                    cx.notify();
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
        let lines = match (&note, &images.attachments) {
            (Some(note), Some(attachments)) => {
                let attachments = attachments.clone();
                image_lines(
                    &self.editor.get_text(),
                    note,
                    &attachments,
                    images,
                    pane,
                    cx.weak_entity(),
                    cx,
                )
            }
            // Until the attachments are listed no embed resolves: the lines
            // stay text rather than flash "not found".
            _ => Vec::new(),
        };
        self.editor.session_mut().set_image_lines(lines);
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

/// The image lines of `text`, the note at `note`: each whole-line embed,
/// resolved against `attachments` and sized from its file's header. An
/// image not decoded yet, and never asked for by this editor
/// (`images.requested`), is asked for now. `view` redraws when its pixels
/// land.
fn image_lines(
    text: &str,
    note: &VaultPath,
    attachments: &[VaultPath],
    images: &mut Images,
    pane: ImagePane,
    view: WeakEntity<EditorView>,
    cx: &mut App,
) -> Vec<ImageLine> {
    // Most notes embed nothing: skip the scan.
    if !text.contains("![") {
        return Vec::new();
    }
    let mut store = cx.default_global::<ImageStore>().clone();
    image_embeds(text)
        .into_iter()
        .map(|embed| {
            let file = resolve_image_embed(&embed, note, attachments)
                .map(|found| images.vault.path_to_pathbuf(&found));
            let natural = match file.as_ref().map(|file| (file, store.size_of(file))) {
                Some((file, ImageState::Sized { width, height })) => {
                    if images.requested.insert(file.clone()) {
                        store.request(file, cx, on_decoded(view.clone(), file.clone()));
                    }
                    Some((width, height))
                }
                Some((_, ImageState::Ready(_, width, height))) => Some((width, height)),
                // Missing, not an image, undecodable, too large: no room,
                // the note takes the embed line.
                Some((_, ImageState::Failed(_))) | None => None,
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
