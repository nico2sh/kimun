//! Vault images for the editor's inline embeds (CONTEXT.md, "Inline image").
//!
//! A standalone loader: it answers an image's natural size from the file
//! header (cheap, synchronous), decodes the pixels on a background task
//! through GPUI's own decoder, and keeps the decoded images under a byte
//! budget. It reads files only (`metadata`, `File::open`), which the GUI may
//! do; locations are real filesystem paths from `NoteVault::path_to_pathbuf`.
//!
//! Sizes are device pixels: one image pixel is one device pixel, as GPUI's
//! `img()` draws it. `fit` and `rows_for` work in the same unit.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::{App, AppContext as _, Global, ImageFormat, RenderImage};

/// The largest file we read: 20 MB.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// The decoded bytes (width * height * 4, all frames) we keep, about 64 MB.
const BUDGET_BYTES: usize = 64 * 1024 * 1024;
/// An image is never taller than this share of the pane.
const PANE_HEIGHT_SHARE: f32 = 0.6;

/// Why an image cannot be drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The file does not exist (or cannot be read).
    Missing,
    /// The extension is not one we draw.
    NotAnImage,
    /// The extension is ours but the bytes do not decode.
    Undecodable,
    /// The file is over the size limit.
    TooLarge,
}

impl Failure {
    /// The one-line note that stands in for the image.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the inline image painter writes it (next step)")
    )]
    pub fn note(&self, name: &str, alt: &str) -> String {
        let head = match self {
            Failure::Missing => "image not found",
            Failure::NotAnImage => "not an image",
            Failure::Undecodable => "cannot draw image",
            Failure::TooLarge => "image too large",
        };
        if alt.is_empty() {
            format!("{head}: {name}")
        } else {
            format!("{head}: {name} ({alt})")
        }
    }
}

/// What we know about an image right now.
#[derive(Clone)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the inline image painter reads the image and the failure (next step)"
    )
)]
pub enum ImageState {
    /// The natural size is known; the pixels are not decoded yet.
    Sized {
        width: u32,
        height: u32,
    },
    /// Decoded: the image (frame 0 is the one to draw) and its natural size.
    Ready(Arc<RenderImage>, u32, u32),
    Failed(Failure),
}

/// The size to draw `natural` at: capped at the column width and at 60% of
/// the pane height, aspect kept, never scaled up.
pub fn fit(natural: (u32, u32), column_px: f32, pane_px: f32) -> (f32, f32) {
    let (w, h) = (natural.0 as f32, natural.1 as f32);
    if w <= 0.0 || h <= 0.0 {
        return (0.0, 0.0);
    }
    let scale = (column_px / w)
        .min(pane_px * PANE_HEIGHT_SHARE / h)
        .clamp(0.0, 1.0);
    (w * scale, h * scale)
}

/// The whole rows an image of `height_px` takes, at least one.
pub fn rows_for(height_px: f32, row_px: f32) -> usize {
    if row_px <= 0.0 || height_px <= 0.0 {
        return 1;
    }
    ((height_px / row_px).ceil() as usize).max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Raster(ImageFormat),
    Svg,
}

impl Kind {
    fn of(path: &Path) -> Option<Kind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "png" => Kind::Raster(ImageFormat::Png),
            "jpg" | "jpeg" => Kind::Raster(ImageFormat::Jpeg),
            "gif" => Kind::Raster(ImageFormat::Gif),
            "webp" => Kind::Raster(ImageFormat::Webp),
            "svg" => Kind::Svg,
            _ => return None,
        })
    }

    fn format(self) -> ImageFormat {
        match self {
            Kind::Raster(f) => f,
            Kind::Svg => ImageFormat::Svg,
        }
    }
}

/// A file's identity for the cache: modified time plus size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileKey {
    modified: Option<SystemTime>,
    len: u64,
}

struct Entry {
    key: FileKey,
    state: ImageState,
    bytes: usize,
    used: u64,
}

/// Entries by path, with a monotonic use counter for least-recently-used
/// eviction over a byte budget.
struct Cache {
    entries: HashMap<PathBuf, Entry>,
    tick: u64,
    budget: usize,
}

impl Cache {
    fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            tick: 0,
            budget,
        }
    }

    /// The state for `path` if it was stored under the same `key`.
    fn get(&mut self, path: &Path, key: FileKey) -> Option<ImageState> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries.get_mut(path).filter(|e| e.key == key)?;
        entry.used = tick;
        Some(entry.state.clone())
    }

    /// Stores `state` (holding `bytes` decoded bytes), then drops the least
    /// recently used entries until the total fits the budget. The entry just
    /// stored stays even when it alone is over budget.
    fn insert(&mut self, path: PathBuf, key: FileKey, state: ImageState, bytes: usize) {
        self.tick += 1;
        self.entries.insert(
            path.clone(),
            Entry {
                key,
                state,
                bytes,
                used: self.tick,
            },
        );
        let mut total: usize = self.entries.values().map(|e| e.bytes).sum();
        while total > self.budget {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(p, e)| **p != path && e.bytes > 0)
                .min_by_key(|(_, e)| e.used)
                .map(|(p, _)| p.clone())
            else {
                break;
            };
            if let Some(e) = self.entries.remove(&oldest) {
                total -= e.bytes;
            }
        }
    }
}

struct Inner {
    cache: Cache,
    /// Paths with a decode in flight, so a repeated request does nothing.
    pending: HashMap<PathBuf, FileKey>,
    /// Decodes started so far (tests count them).
    #[cfg(test)]
    decodes_started: usize,
}

/// Natural sizes and decoded images for the vault's image files. A clone
/// shares the same cache. The app keeps one as a global, so every editor
/// shares one byte budget.
#[derive(Clone)]
pub struct ImageStore {
    inner: Rc<RefCell<Inner>>,
}

impl Default for ImageStore {
    fn default() -> Self {
        Self::with_budget(BUDGET_BYTES)
    }
}

impl ImageStore {
    /// A store that keeps `budget` decoded bytes (tests use a small one).
    fn with_budget(budget: usize) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                cache: Cache::new(budget),
                pending: HashMap::new(),
                #[cfg(test)]
                decodes_started: 0,
            })),
        }
    }

    /// A store over a budget of `budget` decoded bytes.
    #[cfg(test)]
    pub(crate) fn with_test_budget(budget: usize) -> Self {
        Self::with_budget(budget)
    }

    /// How many decodes this store started.
    #[cfg(test)]
    pub(crate) fn decodes_started(&self) -> usize {
        self.inner.borrow().decodes_started
    }
}

impl Global for ImageStore {}

/// Checks the file and returns its kind and key, or why it cannot be drawn.
fn inspect(path: &Path) -> Result<(Kind, FileKey), Failure> {
    let meta = std::fs::metadata(path).map_err(|_| Failure::Missing)?;
    if !meta.is_file() {
        return Err(Failure::Missing);
    }
    let kind = Kind::of(path).ok_or(Failure::NotAnImage)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(Failure::TooLarge);
    }
    Ok((
        kind,
        FileKey {
            modified: meta.modified().ok(),
            len: meta.len(),
        },
    ))
}

/// The natural size from the header: no pixel decode, no SVG render.
fn header_size(path: &Path, kind: Kind) -> Result<(u32, u32), Failure> {
    let file = File::open(path).map_err(|_| Failure::Missing)?;
    let (w, h) = match kind {
        Kind::Raster(_) => image::ImageReader::new(BufReader::new(file))
            .with_guessed_format()
            .map_err(|_| Failure::Undecodable)?
            .into_dimensions()
            .map_err(|_| Failure::Undecodable)?,
        Kind::Svg => {
            let mut bytes = Vec::new();
            file.take(MAX_FILE_BYTES)
                .read_to_end(&mut bytes)
                .map_err(|_| Failure::Missing)?;
            let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default())
                .map_err(|_| Failure::Undecodable)?;
            let size = tree.size();
            (size.width().round() as u32, size.height().round() as u32)
        }
    };
    if w == 0 || h == 0 {
        return Err(Failure::Undecodable);
    }
    Ok((w, h))
}

/// Decoded bytes of an image: every frame counts.
fn decoded_bytes(image: &RenderImage) -> usize {
    (0..image.frame_count())
        .map(|i| {
            let s = image.size(i);
            s.width.0.max(0) as usize * s.height.0.max(0) as usize * 4
        })
        .sum()
}

impl ImageStore {
    /// The image's state from what is cached, reading only the header when it
    /// is new or the file changed.
    pub fn size_of(&mut self, path: &Path) -> ImageState {
        let (kind, key) = match inspect(path) {
            Ok(found) => found,
            Err(f) => return ImageState::Failed(f),
        };
        let mut inner = self.inner.borrow_mut();
        if let Some(state) = inner.cache.get(path, key) {
            return state;
        }
        let state = match header_size(path, kind) {
            Ok((width, height)) => ImageState::Sized { width, height },
            Err(f) => ImageState::Failed(f),
        };
        inner
            .cache
            .insert(path.to_path_buf(), key, state.clone(), 0);
        state
    }

    /// Decodes the image on a background task. When it finishes (decoded or
    /// failed), the result is cached and `on_done` runs on the main thread, so
    /// the caller can ask for a redraw. Does nothing when the image is already
    /// decoded or a decode is in flight.
    pub fn request(&mut self, path: &Path, cx: &mut App, on_done: impl FnOnce(&mut App) + 'static) {
        let (kind, key, width, height) = match (inspect(path), self.size_of(path)) {
            (Ok((kind, key)), ImageState::Sized { width, height }) => (kind, key, width, height),
            _ => return,
        };
        {
            let mut inner = self.inner.borrow_mut();
            if inner.pending.get(path) == Some(&key) {
                return;
            }
            inner.pending.insert(path.to_path_buf(), key);
            #[cfg(test)]
            {
                inner.decodes_started += 1;
            }
        }
        let renderer = cx.svg_renderer();
        let owned = path.to_path_buf();
        let read_path = owned.clone();
        let inner = self.inner.clone();
        let decode = cx.background_spawn(async move {
            let bytes = std::fs::read(&read_path).map_err(|_| Failure::Missing)?;
            let image = gpui_kit::Image::from_bytes(kind.format(), bytes)
                .to_image_data(renderer)
                .map_err(|_| Failure::Undecodable)?;
            if image.frame_count() == 0 {
                return Err(Failure::Undecodable);
            }
            Ok(image)
        });
        cx.spawn(async move |cx| {
            let result = decode.await;
            {
                let mut inner = inner.borrow_mut();
                if inner.pending.get(&owned) == Some(&key) {
                    inner.pending.remove(&owned);
                }
                match result {
                    Ok(image) => {
                        let bytes = decoded_bytes(&image);
                        inner.cache.insert(
                            owned,
                            key,
                            ImageState::Ready(image, width, height),
                            bytes,
                        );
                    }
                    Err(f) => inner.cache.insert(owned, key, ImageState::Failed(f), 0),
                }
            }
            cx.update(on_done);
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use std::path::PathBuf;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 10, 10, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn gif(w: u16, h: u16) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            for shade in [10u8, 200] {
                let img = image::RgbaImage::from_pixel(
                    w as u32,
                    h as u32,
                    image::Rgba([shade, 0, 0, 255]),
                );
                enc.encode_frame(image::Frame::new(img)).unwrap();
            }
        }
        out
    }

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn fit_never_scales_up() {
        assert_eq!(fit((100, 50), 800.0, 1000.0), (100.0, 50.0));
    }

    #[test]
    fn fit_caps_to_column_width_and_keeps_aspect() {
        assert_eq!(fit((1000, 500), 400.0, 10_000.0), (400.0, 200.0));
    }

    #[test]
    fn fit_caps_to_sixty_percent_of_pane_height() {
        // Pane 500 -> max height 300; 400x600 -> 200x300.
        assert_eq!(fit((400, 600), 10_000.0, 500.0), (200.0, 300.0));
    }

    #[test]
    fn rows_for_rounds_up_and_is_at_least_one() {
        assert_eq!(rows_for(0.0, 20.0), 1);
        assert_eq!(rows_for(5.0, 20.0), 1);
        assert_eq!(rows_for(20.0, 20.0), 1);
        assert_eq!(rows_for(21.0, 20.0), 2);
        assert_eq!(rows_for(100.0, 20.0), 5);
    }

    #[test]
    fn header_size_is_read_without_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = png(7, 5);
        let n = bytes.len();
        for b in &mut bytes[n - 20..n - 12] {
            *b ^= 0xFF; // corrupt the pixel data, keep the header
        }
        let p = write(&dir, "a.png", &bytes);
        let mut store = ImageStore::default();
        assert!(matches!(
            store.size_of(&p),
            ImageState::Sized {
                width: 7,
                height: 5
            }
        ));
    }

    #[test]
    fn a_twenty_one_megabyte_file_is_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, "big.png", &vec![0u8; 21 * 1024 * 1024]);
        let mut store = ImageStore::default();
        assert!(matches!(
            store.size_of(&p),
            ImageState::Failed(Failure::TooLarge)
        ));
    }

    #[test]
    fn a_text_file_named_png_is_undecodable() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, "t.png", b"hello, not an image");
        let mut store = ImageStore::default();
        assert!(matches!(
            store.size_of(&p),
            ImageState::Failed(Failure::Undecodable)
        ));
    }

    #[test]
    fn a_missing_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ImageStore::default();
        assert!(matches!(
            store.size_of(&dir.path().join("nope.png")),
            ImageState::Failed(Failure::Missing)
        ));
    }

    #[test]
    fn an_unlisted_extension_is_not_an_image() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, "x.bmp", &png(2, 2));
        let mut store = ImageStore::default();
        assert!(matches!(
            store.size_of(&p),
            ImageState::Failed(Failure::NotAnImage)
        ));
    }

    #[test]
    fn a_zero_size_svg_is_undecodable() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            &dir,
            "z.svg",
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#,
        );
        let mut store = ImageStore::default();
        assert!(matches!(
            store.size_of(&p),
            ImageState::Failed(Failure::Undecodable)
        ));
    }

    #[test]
    fn an_svg_reports_its_natural_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            &dir,
            "s.svg",
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="12"><rect width="30" height="12"/></svg>"#,
        );
        let mut store = ImageStore::default();
        assert!(matches!(
            store.size_of(&p),
            ImageState::Sized {
                width: 30,
                height: 12
            }
        ));
    }

    #[test]
    fn failure_notes_name_the_file_and_the_alt() {
        assert_eq!(
            Failure::Missing.note("pic.png", "alt"),
            "image not found: pic.png (alt)"
        );
        assert_eq!(Failure::NotAnImage.note("a.bmp", ""), "not an image: a.bmp");
        assert_eq!(
            Failure::Undecodable.note("a.png", "x"),
            "cannot draw image: a.png (x)"
        );
        assert_eq!(
            Failure::TooLarge.note("a.png", ""),
            "image too large: a.png"
        );
    }

    #[gpui_kit::test]
    fn a_gif_decodes_its_first_frame(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, "g.gif", &gif(6, 4));
        let mut store = ImageStore::default();
        assert!(matches!(
            store.size_of(&p),
            ImageState::Sized {
                width: 6,
                height: 4
            }
        ));
        let store = std::rc::Rc::new(std::cell::RefCell::new(store));
        let done = std::rc::Rc::new(std::cell::Cell::new(false));
        let (s, d) = (store.clone(), done.clone());
        cx.update(|cx| {
            s.borrow_mut().request(&p, cx, move |_| d.set(true));
        });
        cx.run_until_parked();
        assert!(done.get());
        match store.borrow_mut().size_of(&p) {
            ImageState::Ready(img, 6, 4) => {
                let first = img.size(0);
                assert_eq!((first.width.0, first.height.0), (6, 4));
            }
            _ => panic!("expected a ready image"),
        }
    }

    #[test]
    fn cache_key_changes_with_mtime_or_size() {
        let t = std::time::SystemTime::UNIX_EPOCH;
        let later = t + std::time::Duration::from_secs(5);
        let a = FileKey {
            modified: Some(t),
            len: 10,
        };
        assert_eq!(
            a,
            FileKey {
                modified: Some(t),
                len: 10
            }
        );
        assert_ne!(
            a,
            FileKey {
                modified: Some(later),
                len: 10
            }
        );
        assert_ne!(
            a,
            FileKey {
                modified: Some(t),
                len: 11
            }
        );
        let mut cache = Cache::new(1000);
        let p = PathBuf::from("/x.png");
        cache.insert(
            p.clone(),
            a,
            ImageState::Sized {
                width: 1,
                height: 1,
            },
            0,
        );
        assert!(cache.get(&p, a).is_some());
        assert!(
            cache
                .get(
                    &p,
                    FileKey {
                        modified: Some(later),
                        len: 10
                    }
                )
                .is_none()
        );
    }

    #[test]
    fn cache_drops_the_least_recently_used_over_budget() {
        let key = FileKey {
            modified: None,
            len: 1,
        };
        let ready = || ImageState::Sized {
            width: 1,
            height: 1,
        };
        let mut cache = Cache::new(100);
        let (a, b, c) = (PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c"));
        cache.insert(a.clone(), key, ready(), 40);
        cache.insert(b.clone(), key, ready(), 40);
        assert!(cache.get(&a, key).is_some()); // a is now newer than b
        cache.insert(c.clone(), key, ready(), 40);
        assert!(cache.get(&b, key).is_none());
        assert!(cache.get(&a, key).is_some());
        assert!(cache.get(&c, key).is_some());
    }
}
