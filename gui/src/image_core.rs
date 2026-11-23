//! The surface-neutral part of **Inline image**s (CONTEXT.md): finding the
//! whole-line embeds of a text, what each one draws, and the one paint step.
//! The editor (`editor_view/images.rs`) and the read-only views share it;
//! what stays with each surface is its own geometry (where the image goes,
//! how many lines it takes), its pins and its hover.
//!
//! A **Remote image** (CONTEXT.md) is an embed with a web address. Until it
//! has a saved copy it draws a remote look, which `remote_placeholder.rs`
//! paints. With a saved copy it draws as a vault image, from the cache file.
//! Showing an embed never starts a fetch: the surfaces call the store's
//! `load` and `auto_load` themselves.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{Bounds, Corners, Pixels, RenderImage, Rgba, SharedString, Window, fill};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::scan::link_target_filename;
use kimun_core::note::{
    FoundEmbed, ImageEmbed, RemoteImageEmbed, all_image_embeds, resolve_image_embed,
};

use crate::inline_image::{Failure, ImageState, ImageStore, fit};
use crate::remote_image::{RemoteImages, RemoteState};

/// The room an image is fitted to, in device pixels (the unit of
/// [`fit`]): the text column's width, the pane's height (`fit` takes 60% of
/// it) and one line's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImagePane {
    pub(crate) column_px: f32,
    pub(crate) pane_px: f32,
    pub(crate) row_px: f32,
}

/// The web address of a remote embed, as the finder gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteRef {
    /// The address as written, without angle brackets or title.
    pub url: String,
    /// The host in lower case, without the port.
    pub host: String,
    /// Whether the address uses `https`.
    pub secure: bool,
    /// The title after the address, without its quotes.
    pub title: Option<String>,
}

/// One embed line, as found with the lines: what it draws and how it is
/// named.
#[derive(Clone, Debug)]
pub(crate) struct Embed {
    pub(crate) row: usize,
    /// The attachment it resolved to, and its file. Both are `None` for a
    /// remote embed: its file is the saved copy, found when it is shown.
    pub(crate) found: Option<VaultPath>,
    pub(crate) file: Option<PathBuf>,
    /// The file name as written (the embed target's last part). For a
    /// remote embed, the last part of the address's path, or the host when
    /// the path has none.
    pub(crate) name: String,
    /// The alt text of `![alt](path)`; empty for `![[name]]`.
    pub(crate) alt: String,
    /// The web address of a remote embed; `None` for a vault embed.
    pub(crate) remote: Option<RemoteRef>,
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
            remote: None,
        }
    }

    /// A remote embed. It has no vault file.
    pub(crate) fn remote(embed: &RemoteImageEmbed) -> Self {
        let name = url::Url::parse(&embed.url)
            .ok()
            .and_then(|url| {
                url.path_segments()?
                    .rfind(|part| !part.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| embed.host.clone());
        Self {
            row: embed.row,
            found: None,
            file: None,
            name,
            alt: embed.alt.clone(),
            remote: Some(RemoteRef {
                url: embed.url.clone(),
                host: embed.host.clone(),
                secure: embed.secure,
                title: embed.title.clone(),
            }),
        }
    }

    /// The hover text: the alt text, or the file name without one. A remote
    /// embed with no alt text takes its title, then its file name and host.
    pub(crate) fn tip(&self) -> SharedString {
        if !self.alt.is_empty() {
            return self.alt.clone().into();
        }
        match &self.remote {
            None => self.name.clone().into(),
            Some(RemoteRef {
                title: Some(title), ..
            }) if !title.is_empty() => title.clone().into(),
            Some(remote) if self.name == remote.host => remote.host.clone().into(),
            Some(remote) => format!("{} ({})", self.name, remote.host).into(),
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
    /// A remote embed with no saved copy. It takes [`remote_rows`] rows.
    // The surfaces paint it from the next step of the remote images work.
    #[cfg_attr(not(test), allow(dead_code))]
    Remote(RemoteLook),
}

/// What a remote embed with no saved copy shows. `remote_placeholder.rs`
/// paints each one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteLook {
    /// Not loaded yet: the host and the actions.
    Placeholder { host: String, actions: Actions },
    /// The address uses `http`. It never loads and offers no action.
    Insecure,
    /// The fetch stopped at a redirect to `host`, which is not allowed.
    RedirectBlocked { host: String, actions: Actions },
    /// A fetch is under way or waits in the queue.
    Loading { host: String },
    /// The fetch failed in this session: the one-line reason, and whether
    /// to offer "Retry".
    Failed { note: String, retry: bool },
}

/// The actions a placeholder offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Actions {
    /// "Load": fetch this address once.
    pub load: bool,
    /// "Always load from" the host: allow the host, then load. Not offered
    /// for a host that is allowed already, because that one loads on its
    /// own.
    pub always: bool,
}

/// The rows of any remote look that is not an image: the text row and the
/// button row.
pub(crate) fn remote_rows() -> usize {
    2
}

/// The whole-line embeds of `text`, the note at `note`, each resolved
/// against `attachments`; at most `limit` of them when one is given. Vault
/// and remote embeds come in row order, and the limit counts both.
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
    all_image_embeds(text)
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .map(|found| match found {
            FoundEmbed::Vault(embed) => {
                let found = resolve_image_embed(embed, note, attachments);
                let file = found.as_ref().map(|found| vault.path_to_pathbuf(found));
                Embed::of(embed, found, file)
            }
            FoundEmbed::Remote(embed) => Embed::remote(embed),
        })
        .collect()
}

/// Where the image of `embed` comes from now: a file (or `None`, not
/// resolved), or the remote look of a remote embed with no saved copy.
enum Source<'a> {
    File(Option<&'a Path>),
    Saved(PathBuf),
    Remote(RemoteLook),
}

/// The source of `embed`. A vault embed never reads `remote`. A remote
/// embed asks the store for its state, which checks the cache folder on
/// disk once for each call: one call per embed per look. Add an index in
/// memory only if a profile shows the need. It writes nothing and starts
/// no fetch.
fn source_of<'a>(embed: &'a Embed, remote: &RemoteImages) -> Source<'a> {
    let Some(r) = &embed.remote else {
        return Source::File(embed.file.as_deref());
    };
    let actions = |host: &str| Actions {
        load: true,
        always: !remote.is_allowed(host),
    };
    Source::Remote(match remote.state(&r.url) {
        RemoteState::Saved(path) => return Source::Saved(path),
        RemoteState::Blocked { host } => RemoteLook::Placeholder {
            actions: actions(&host),
            host,
        },
        RemoteState::RedirectBlocked { host } => RemoteLook::RedirectBlocked {
            actions: actions(&host),
            host,
        },
        RemoteState::Insecure => RemoteLook::Insecure,
        RemoteState::Loading => RemoteLook::Loading {
            host: r.host.clone(),
        },
        RemoteState::Failed(failure) => RemoteLook::Failed {
            note: failure.note(),
            retry: failure.can_retry(),
        },
    })
}

/// The file the image of `embed` is drawn from now: the vault file, or
/// the saved copy of a remote embed. The file to pin and to ask a decode
/// for. `None` for an unresolved embed and a remote embed with no copy.
// The surfaces pin and decode a saved copy from the next step of the
// remote images work.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn image_file(embed: &Embed, remote: &RemoteImages) -> Option<PathBuf> {
    match source_of(embed, remote) {
        Source::File(file) => file.map(Path::to_path_buf),
        Source::Saved(file) => Some(file),
        Source::Remote(_) => None,
    }
}

/// The natural size of the embed's image, read from its header. `None` for
/// one that cannot be drawn: missing, unsupported, undecodable, too large,
/// or not resolved, and for a remote embed with no saved copy.
pub(crate) fn natural_of(
    embed: &Embed,
    store: &mut ImageStore,
    remote: &RemoteImages,
) -> Option<(u32, u32)> {
    let state = match source_of(embed, remote) {
        Source::File(file) => file.map(|file| store.size_of(file)),
        Source::Saved(file) => Some(store.size_of(&file)),
        Source::Remote(_) => None,
    };
    match state {
        Some(ImageState::Sized { width, height } | ImageState::Ready(_, width, height)) => {
            Some((width, height))
        }
        Some(ImageState::Failed(_)) | None => None,
    }
}

/// What the embed draws now, fitted to `pane`. A failed or unresolved image
/// is a note (an unresolved one reads "not found"). A remote embed with a
/// saved copy draws as a vault image; without one it has its remote look.
pub(crate) fn look_of(
    embed: &Embed,
    store: &mut ImageStore,
    remote: &RemoteImages,
    pane: ImagePane,
) -> ImageLook {
    let fitted = |natural| fit(natural, pane.column_px, pane.pane_px);
    let state = match source_of(embed, remote) {
        Source::File(file) => file.map(|file| store.size_of(file)),
        Source::Saved(file) => Some(store.size_of(&file)),
        Source::Remote(look) => return ImageLook::Remote(look),
    };
    match state {
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
            remote: None,
        }
    }

    /// A store with no remote state, for the vault tests: a vault embed
    /// never reads it.
    fn no_remote() -> RemoteImages {
        RemoteImages::default()
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
        assert_eq!(natural_of(&bad, &mut store, &no_remote()), None);
        assert_eq!(
            natural_of(&embed(None, "x.png", ""), &mut store, &no_remote()),
            None
        );
        assert_eq!(natural_of(&ok, &mut store, &no_remote()), Some((6, 3)));
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
        match look_of(&gone, &mut store, &no_remote(), PANE) {
            ImageLook::Note(text) => assert_eq!(text, Failure::Missing.note("nothing.png", "")),
            _ => panic!("a missing file is a note"),
        }
        match look_of(
            &embed(None, "nothing.png", "pic"),
            &mut store,
            &no_remote(),
            PANE,
        ) {
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
        match look_of(&ok, &mut store, &no_remote(), PANE) {
            ImageLook::Pending(size) => assert_eq!(size, fit((6, 3), 800.0, 1000.0)),
            _ => panic!("a sized, undecoded image is pending"),
        }
    }

    #[test]
    fn tip_prefers_alt_over_name() {
        assert_eq!(embed(None, "a.png", "A cat").tip().as_ref(), "A cat");
        assert_eq!(embed(None, "a.png", "").tip().as_ref(), "a.png");
    }

    // ---- Remote embeds ------------------------------------------------------

    use gpui_kit::TestAppContext;
    use kimun_core::ImageCache;
    use kimun_core::note::RemoteImageEmbed;
    use kimun_core::system::SystemPath;

    use crate::remote_image::{FetchFailure, RemoteState, fetch_count, set_cache};

    const URL: &str = "https://img.example.com/pics/a.png";
    const HOST: &str = "img.example.com";

    /// A PNG head with no image data: it sizes as a PNG file and fails to
    /// read its header.
    const BROKEN_PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];

    /// The app's remote store with a temp cache, and the cache.
    fn remote_store(cx: &mut TestAppContext) -> (RemoteImages, ImageCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::at(
            SystemPath::try_absolute(dir.path())
                .unwrap()
                .join("image-cache"),
        );
        let c = cache.clone();
        cx.update(|cx| set_cache(cx, c));
        let remote = cx.update(|cx| cx.global::<RemoteImages>().clone());
        (remote, cache, dir)
    }

    fn remote_embed(url: &str, alt: &str, title: Option<&str>) -> Embed {
        Embed::remote(&RemoteImageEmbed {
            row: 0,
            alt: alt.into(),
            title: title.map(str::to_string),
            url: url.into(),
            host: kimun_core::note::host_of(url).unwrap(),
            secure: url.starts_with("https://"),
        })
    }

    fn remote_look(embed: &Embed, remote: &RemoteImages) -> RemoteLook {
        match look_of(embed, &mut ImageStore::default(), remote, PANE) {
            ImageLook::Remote(look) => look,
            ImageLook::Ready(..) => panic!("expected a remote look, got Ready"),
            ImageLook::Pending(size) => panic!("expected a remote look, got Pending {size:?}"),
            ImageLook::Note(note) => panic!("expected a remote look, got Note {note}"),
        }
    }

    fn fetches(cx: &mut TestAppContext) -> usize {
        cx.update(|cx| fetch_count(cx))
    }

    #[test]
    fn embeds_of_returns_vault_and_remote_embeds_in_row_order_and_limits_both() {
        let text = "![[a.png]]\n\n![cat](https://img.example.com/c.png \"Cat\")\n\n![b](b.png)\n";
        let (v, attachments) = vault_with(text, &[("a.png", png(4, 4)), ("b.png", png(4, 4))]);
        let note = VaultPath::new("n.md");
        let all = embeds_of(text, &note, &attachments, &v.vault, None);
        assert_eq!(all.iter().map(|e| e.row).collect::<Vec<_>>(), [0, 2, 4]);
        assert!(all[0].remote.is_none() && all[0].file.is_some());
        assert!(all[2].remote.is_none() && all[2].file.is_some());
        let remote = all[1].remote.as_ref().expect("a remote embed");
        assert_eq!(remote.url, "https://img.example.com/c.png");
        assert_eq!(remote.host, HOST);
        assert!(remote.secure);
        assert_eq!(remote.title.as_deref(), Some("Cat"));
        assert_eq!(all[1].alt, "cat");
        assert!(all[1].found.is_none() && all[1].file.is_none());
        // The limit applies to the merged list.
        let capped = embeds_of(text, &note, &attachments, &v.vault, Some(2));
        assert_eq!(capped.iter().map(|e| e.row).collect::<Vec<_>>(), [0, 2]);
        let many = "![x](https://img.example.com/x.png)\n\n".repeat(101);
        let (v, attachments) = vault_with(&many, &[]);
        assert_eq!(
            embeds_of(&many, &note, &attachments, &v.vault, Some(100)).len(),
            100
        );
    }

    #[gpui_kit::test]
    fn a_remote_embed_on_an_unallowed_host_looks_like_a_placeholder_with_both_actions(
        cx: &mut TestAppContext,
    ) {
        let (remote, _cache, _dir) = remote_store(cx);
        assert_eq!(
            remote_look(&remote_embed(URL, "", None), &remote),
            RemoteLook::Placeholder {
                host: HOST.into(),
                actions: Actions {
                    load: true,
                    always: true
                },
            }
        );
    }

    #[gpui_kit::test]
    fn a_remote_embed_on_an_allowed_host_has_no_always_action(cx: &mut TestAppContext) {
        let (mut remote, _cache, _dir) = remote_store(cx);
        remote.set_allowed_hosts(&[HOST.to_string()]);
        assert_eq!(
            remote_look(&remote_embed(URL, "", None), &remote),
            RemoteLook::Placeholder {
                host: HOST.into(),
                actions: Actions {
                    load: true,
                    always: false
                },
            }
        );
    }

    #[gpui_kit::test]
    fn an_http_embed_looks_insecure_with_no_actions(cx: &mut TestAppContext) {
        let (mut remote, _cache, _dir) = remote_store(cx);
        remote.set_allowed_hosts(&[HOST.to_string()]);
        let embed = remote_embed("http://img.example.com/a.png", "", None);
        assert_eq!(remote_look(&embed, &remote), RemoteLook::Insecure);
    }

    #[gpui_kit::test]
    fn a_redirect_blocked_embed_names_the_new_host_and_offers_both_actions(
        cx: &mut TestAppContext,
    ) {
        let (remote, _cache, _dir) = remote_store(cx);
        remote.set_state_for_test(
            URL,
            RemoteState::RedirectBlocked {
                host: "tracker.example.net".into(),
            },
        );
        assert_eq!(
            remote_look(&remote_embed(URL, "", None), &remote),
            RemoteLook::RedirectBlocked {
                host: "tracker.example.net".into(),
                actions: Actions {
                    load: true,
                    always: true
                },
            }
        );
    }

    #[gpui_kit::test]
    fn a_failed_remote_embed_shows_the_note_and_a_retry_action(cx: &mut TestAppContext) {
        let (remote, _cache, _dir) = remote_store(cx);
        remote.set_state_for_test(URL, RemoteState::Failed(FetchFailure::Status(404)));
        assert_eq!(
            remote_look(&remote_embed(URL, "", None), &remote),
            RemoteLook::Failed {
                note: "Load failed: HTTP 404".into(),
                retry: true,
            }
        );
    }

    #[gpui_kit::test]
    fn a_loading_remote_embed_names_its_host(cx: &mut TestAppContext) {
        let (remote, _cache, _dir) = remote_store(cx);
        remote.set_state_for_test(URL, RemoteState::Loading);
        assert_eq!(
            remote_look(&remote_embed(URL, "", None), &remote),
            RemoteLook::Loading { host: HOST.into() }
        );
    }

    #[gpui_kit::test]
    fn a_saved_remote_embed_resolves_to_the_cache_file_and_looks_like_a_vault_image(
        cx: &mut TestAppContext,
    ) {
        let (remote, cache, _dir) = remote_store(cx);
        let saved = cache.save(URL, &png(6, 3)).unwrap();
        let embed = remote_embed(URL, "", None);
        let mut store = ImageStore::default();
        match look_of(&embed, &mut store, &remote, PANE) {
            ImageLook::Pending(size) => assert_eq!(size, fit((6, 3), 800.0, 1000.0)),
            _ => panic!("a sized, undecoded saved copy is pending"),
        }
        assert_eq!(image_file(&embed, &remote), Some(saved.clone()));
        let done = std::rc::Rc::new(std::cell::Cell::new(false));
        let (mut s, d) = (store.clone(), done.clone());
        cx.update(|cx| s.request(&saved, cx, move |_| d.set(true)));
        cx.run_until_parked();
        assert!(done.get());
        match look_of(&embed, &mut store, &remote, PANE) {
            ImageLook::Ready(_, size) => assert_eq!(size, fit((6, 3), 800.0, 1000.0)),
            _ => panic!("a decoded saved copy is ready"),
        }
        // A damaged copy gets the usual note, named by the address's file.
        let other = "https://img.example.com/broken.png";
        cache.save(other, BROKEN_PNG).unwrap();
        match look_of(&remote_embed(other, "pic", None), &mut store, &remote, PANE) {
            ImageLook::Note(note) => {
                assert_eq!(note, Failure::Undecodable.note("broken.png", "pic"));
            }
            _ => panic!("a damaged saved copy is a note"),
        }
        assert_eq!(fetches(cx), 0);
    }

    #[gpui_kit::test]
    fn natural_of_is_none_for_every_non_saved_remote_state_and_the_size_for_a_saved_one(
        cx: &mut TestAppContext,
    ) {
        let (remote, cache, _dir) = remote_store(cx);
        let mut store = ImageStore::default();
        let blocked = remote_embed(URL, "", None);
        assert_eq!(natural_of(&blocked, &mut store, &remote), None);
        assert_eq!(image_file(&blocked, &remote), None);
        let insecure = remote_embed("http://img.example.com/a.png", "", None);
        assert_eq!(natural_of(&insecure, &mut store, &remote), None);
        let states = [
            ("https://img.example.com/loading.png", RemoteState::Loading),
            (
                "https://img.example.com/failed.png",
                RemoteState::Failed(FetchFailure::TimedOut),
            ),
            (
                "https://img.example.com/redirect.png",
                RemoteState::RedirectBlocked {
                    host: "tracker.example.net".into(),
                },
            ),
        ];
        for (url, state) in states {
            remote.set_state_for_test(url, state.clone());
            assert_eq!(
                natural_of(&remote_embed(url, "", None), &mut store, &remote),
                None,
                "{state:?}"
            );
        }
        cache.save(URL, &png(6, 3)).unwrap();
        assert_eq!(natural_of(&blocked, &mut store, &remote), Some((6, 3)));
    }

    #[gpui_kit::test]
    fn showing_a_remote_embed_starts_no_fetch(cx: &mut TestAppContext) {
        let (mut remote, _cache, _dir) = remote_store(cx);
        let mut store = ImageStore::default();
        let blocked = remote_embed(URL, "", None);
        let allowed = remote_embed("https://cdn.example.com/a.png", "", None);
        remote.set_allowed_hosts(&["cdn.example.com".to_string()]);
        for _ in 0..3 {
            for embed in [&blocked, &allowed] {
                let _ = look_of(embed, &mut store, &remote, PANE);
                let _ = natural_of(embed, &mut store, &remote);
            }
        }
        cx.run_until_parked();
        assert_eq!(fetches(cx), 0);
        assert_eq!(
            remote.state(URL),
            RemoteState::Blocked { host: HOST.into() }
        );
    }

    #[test]
    fn the_remote_tooltip_prefers_alt_then_title_then_host_and_file() {
        assert_eq!(
            remote_embed(URL, "A cat", Some("Title")).tip().as_ref(),
            "A cat"
        );
        assert_eq!(remote_embed(URL, "", Some("Title")).tip().as_ref(), "Title");
        assert_eq!(
            remote_embed(URL, "", None).tip().as_ref(),
            "a.png (img.example.com)"
        );
        // An address with no file name: the host alone.
        assert_eq!(
            remote_embed("https://img.example.com/", "", None)
                .tip()
                .as_ref(),
            HOST
        );
    }

    #[test]
    fn remote_rows_is_two() {
        assert_eq!(remote_rows(), 2);
    }
}
