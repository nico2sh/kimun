//! The surface-neutral part of **Inline image**s (CONTEXT.md): finding the
//! whole-line embeds of a text, what each one draws, and the one paint step.
//! The editor (`editor_view/images.rs`) and the read-only views share it;
//! what stays with each surface is its own geometry (where the image goes,
//! how many lines it takes), its pins and its hover.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{Bounds, Corners, Pixels, RenderImage, Rgba, SharedString, Window, fill};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::scan::link_target_filename;
use kimun_core::note::{ImageEmbed, image_embeds, resolve_image_embed};

use crate::inline_image::{Failure, ImageState, ImageStore, fit};

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
pub(crate) struct Embed {
    pub(crate) row: usize,
    /// The attachment it resolved to, and its file.
    pub(crate) found: Option<VaultPath>,
    pub(crate) file: Option<PathBuf>,
    /// The file name as written (the embed target's last part).
    pub(crate) name: String,
    /// The alt text of `![alt](path)`; empty for `![[name]]`.
    pub(crate) alt: String,
}

impl Embed {
    pub(crate) fn of(embed: &ImageEmbed, found: Option<VaultPath>, file: Option<PathBuf>) -> Self {
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
    pub(crate) fn tip(&self) -> SharedString {
        if self.alt.is_empty() {
            self.name.clone().into()
        } else {
            self.alt.clone().into()
        }
    }
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

/// The whole-line embeds of `text`, the note at `note`, each resolved
/// against `attachments`; at most `limit` of them when one is given.
pub(crate) fn embeds_of(
    text: &str,
    note: &VaultPath,
    attachments: &[VaultPath],
    vault: &NoteVault,
    limit: Option<usize>,
) -> Vec<Embed> {
    // Most notes embed nothing: skip the scan.
    if !text.contains("![") {
        return Vec::new();
    }
    image_embeds(text)
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .map(|embed| {
            let found = resolve_image_embed(embed, note, attachments);
            let file = found.as_ref().map(|found| vault.path_to_pathbuf(found));
            Embed::of(embed, found, file)
        })
        .collect()
}

/// The natural size of the embed's image, read from its header. `None` for
/// one that cannot be drawn: missing, unsupported, undecodable, too large,
/// or not resolved.
pub(crate) fn natural_of(embed: &Embed, store: &mut ImageStore) -> Option<(u32, u32)> {
    match embed.file.as_ref().map(|file| store.size_of(file)) {
        Some(ImageState::Sized { width, height } | ImageState::Ready(_, width, height)) => {
            Some((width, height))
        }
        Some(ImageState::Failed(_)) | None => None,
    }
}

/// What the embed draws now, fitted to `pane`. A failed or unresolved image
/// is a note (an unresolved one reads "not found").
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the read-only views use it from the next task")
)]
pub(crate) fn look_of(embed: &Embed, store: &mut ImageStore, pane: ImagePane) -> ImageLook {
    let fitted = |natural| fit(natural, pane.column_px, pane.pane_px);
    match embed.file.as_ref().map(|file| store.size_of(file)) {
        Some(ImageState::Ready(image, width, height)) => {
            ImageLook::Ready(image, fitted((width, height)))
        }
        Some(ImageState::Sized { width, height }) => ImageLook::Pending(fitted((width, height))),
        Some(ImageState::Failed(failure)) => ImageLook::Note(failure.note(&embed.name, &embed.alt)),
        None => ImageLook::Note(Failure::Missing.note(&embed.name, &embed.alt)),
    }
}

/// Paint one image: its pixels in `rect` clipped to `clip`, or the
/// `placeholder` fill while there are none.
pub(crate) fn paint_look(
    window: &mut Window,
    rect: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    image: Option<&Arc<RenderImage>>,
    placeholder: Rgba,
) {
    match image {
        // A frame the GPU cannot take is skipped, not fatal.
        Some(pixels) => {
            let _ = window.paint_image(clip, rect, Corners::default(), pixels.clone(), 0, false);
        }
        None => window.paint_quad(fill(rect.intersect(&clip), placeholder)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_vault::test_vault;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([200, 10, 10, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    const PANE: ImagePane = ImagePane {
        column_px: 800.0,
        pane_px: 1000.0,
        row_px: 20.0,
    };

    /// A vault whose note `n.md` holds `text` and whose attachments are
    /// `files`; its attachment list too.
    fn vault_with(
        text: &str,
        files: &[(&str, Vec<u8>)],
    ) -> (crate::test_vault::TestVault, Vec<VaultPath>) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let v = rt.block_on(async {
            let v = test_vault(&[("n.md", text)], &[]).await;
            for (path, bytes) in files {
                v.vault
                    .save_attachment(&VaultPath::new(*path), bytes)
                    .await
                    .unwrap();
            }
            v
        });
        let attachments = v.vault.list_attachments();
        (v, attachments)
    }

    fn embed(file: Option<PathBuf>, name: &str, alt: &str) -> Embed {
        Embed {
            row: 0,
            found: None,
            file,
            name: name.into(),
            alt: alt.into(),
        }
    }

    #[test]
    fn embeds_of_stops_at_the_limit_and_none_gives_all() {
        let text = "![[a.png]]\n\n".repeat(101);
        let (v, attachments) = vault_with(&text, &[("a.png", png(4, 4))]);
        let note = VaultPath::new("n.md");
        let capped = embeds_of(&text, &note, &attachments, &v.vault, Some(100));
        assert_eq!(capped.len(), 100);
        let all = embeds_of(&text, &note, &attachments, &v.vault, None);
        assert_eq!(all.len(), 101);
        assert!(all.iter().all(|e| e.found.is_some() && e.file.is_some()));
    }

    #[test]
    fn embeds_of_skips_a_note_without_bang_bracket() {
        let text = "just [[a]] and [text](b.png)\n";
        let (v, attachments) = vault_with(text, &[]);
        let note = VaultPath::new("n.md");
        assert!(embeds_of(text, &note, &attachments, &v.vault, None).is_empty());
    }

    #[test]
    fn natural_of_is_none_for_a_failed_file() {
        let (v, _) = vault_with(
            "",
            &[("bad.png", b"not a png".to_vec()), ("ok.png", png(6, 3))],
        );
        let mut store = ImageStore::default();
        let bad = embed(
            Some(v.vault.path_to_pathbuf(&VaultPath::new("bad.png"))),
            "bad.png",
            "",
        );
        let ok = embed(
            Some(v.vault.path_to_pathbuf(&VaultPath::new("ok.png"))),
            "ok.png",
            "",
        );
        assert_eq!(natural_of(&bad, &mut store), None);
        assert_eq!(natural_of(&embed(None, "x.png", ""), &mut store), None);
        assert_eq!(natural_of(&ok, &mut store), Some((6, 3)));
    }

    #[test]
    fn look_of_gives_a_note_for_a_missing_file_and_pending_for_a_sized_one() {
        let (v, _) = vault_with("", &[("ok.png", png(6, 3))]);
        let mut store = ImageStore::default();
        let gone = embed(
            Some(v.vault.path_to_pathbuf(&VaultPath::new("nothing.png"))),
            "nothing.png",
            "",
        );
        match look_of(&gone, &mut store, PANE) {
            ImageLook::Note(text) => assert_eq!(text, Failure::Missing.note("nothing.png", "")),
            _ => panic!("a missing file is a note"),
        }
        match look_of(&embed(None, "nothing.png", "pic"), &mut store, PANE) {
            ImageLook::Note(text) => {
                assert_eq!(text, Failure::Missing.note("nothing.png", "pic"));
            }
            _ => panic!("an unresolved embed is a note"),
        }
        let ok = embed(
            Some(v.vault.path_to_pathbuf(&VaultPath::new("ok.png"))),
            "ok.png",
            "",
        );
        match look_of(&ok, &mut store, PANE) {
            ImageLook::Pending(size) => assert_eq!(size, fit((6, 3), 800.0, 1000.0)),
            _ => panic!("a sized, undecoded image is pending"),
        }
    }

    #[test]
    fn tip_prefers_alt_over_name() {
        assert_eq!(embed(None, "a.png", "A cat").tip().as_ref(), "A cat");
        assert_eq!(embed(None, "a.png", "").tip().as_ref(), "a.png");
    }
}
