//! The image cache: saved copies of remote images.
//!
//! Host-scoped, so it lives under the app folder and never in a vault. A copy
//! is named by a hash of the normalized address plus the extension that the
//! first bytes of the image show. The extension matters because the image
//! loader reads the format from the file name.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use twox_hash::XxHash64;

use super::{app_dir, ensure_dir, read_dir, remove_file, replace_atomically};
use super::{SystemError, SystemPath};
use crate::note::normalize_image_url;

/// The most bytes the cache keeps. A save over this removes the oldest copies.
pub const IMAGE_CACHE_CAP: u64 = 200 * 1024 * 1024;

/// The folder name of the cache inside the app folder.
const CACHE_DIR_NAME: &str = "image-cache";

/// The extensions a copy can have. The same set as [`image_extension`] gives.
const EXTENSIONS: [&str; 4] = ["png", "jpg", "gif", "webp"];

/// The seed of the address hash. Fixed, so a name is stable across runs.
const HASH_SEED: u64 = 42;

/// How old a temporary file must be before eviction removes it. A younger one
/// can belong to a save that is between its write and its rename.
const STALE_TEMP_AGE: Duration = Duration::from_secs(60 * 60);

/// The extension of an image format, read from the first bytes: `png`, `jpg`,
/// `gif` or `webp`. `None` for any other data.
pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// A folder of saved copies of remote images.
#[derive(Debug, Clone)]
pub struct ImageCache {
    root: SystemPath,
}

impl ImageCache {
    /// A cache in the folder `root`. The folder is made on the first save.
    pub fn at(root: SystemPath) -> ImageCache {
        ImageCache { root }
    }

    /// The cache in the app folder, in the `image-cache` folder.
    pub fn in_app_dir() -> Result<ImageCache, SystemError> {
        Ok(ImageCache::at(app_dir()?.join(CACHE_DIR_NAME)))
    }

    /// The folder that holds the copies.
    pub fn root(&self) -> &SystemPath {
        &self.root
    }

    fn stem(url: &str) -> Option<String> {
        let key = normalize_image_url(url)?;
        Some(format!(
            "{:016x}",
            XxHash64::oneshot(HASH_SEED, key.as_bytes())
        ))
    }

    /// Saves `bytes` as the copy of `url` and returns the path of the copy.
    /// The write goes through a temporary file, so a copy is never half
    /// written. Then the cache evicts down to [`IMAGE_CACHE_CAP`]. Fails for
    /// an address that is not `http` or `https`, and for bytes that are not a
    /// known image format.
    pub fn save(&self, url: &str, bytes: &[u8]) -> Result<PathBuf, SystemError> {
        let invalid = |what: &str| SystemError::Io {
            action: "save image",
            path: url.to_string(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, what.to_string()),
        };
        let stem = Self::stem(url).ok_or_else(|| invalid("not a remote address"))?;
        let ext = image_extension(bytes).ok_or_else(|| invalid("not a known image format"))?;
        ensure_dir(&self.root)?;
        let path = self.root.join(format!("{stem}.{ext}")).into_path_buf();
        replace_atomically(&path, bytes)?;
        // The same address can hold another format now. Drop the old copies.
        for other in EXTENSIONS.iter().filter(|e| **e != ext) {
            remove_file(&self.root.join(format!("{stem}.{other}")).into_path_buf())?;
        }
        self.evict_to(IMAGE_CACHE_CAP, &path)?;
        Ok(path)
    }

    /// The saved copy of `url`, if there is one.
    pub fn find(&self, url: &str) -> Option<PathBuf> {
        let stem = Self::stem(url)?;
        EXTENSIONS
            .iter()
            .map(|ext| self.root.join(format!("{stem}.{ext}")).into_path_buf())
            .find(|path| path.is_file())
    }

    /// Removes the copy of `url`. A missing copy is not an error.
    pub fn remove(&self, url: &str) -> Result<(), SystemError> {
        let Some(stem) = Self::stem(url) else {
            return Ok(());
        };
        for ext in EXTENSIONS {
            remove_file(&self.root.join(format!("{stem}.{ext}")).into_path_buf())?;
        }
        Ok(())
    }

    /// Whether `path` names a file in the cache folder: a copy, or a
    /// temporary file of a save.
    pub fn holds(&self, path: &Path) -> bool {
        path.parent() == Some(self.root.as_path())
    }

    /// The total bytes of the saved copies.
    pub fn size(&self) -> u64 {
        self.copies().iter().map(|c| c.len).sum()
    }

    /// Removes every file in the cache folder. The folder stays.
    pub fn clear(&self) -> Result<(), SystemError> {
        if !self.root.is_dir() {
            return Ok(());
        }
        for entry in read_dir(&self.root)? {
            if entry.as_path().is_file() {
                remove_file(entry.as_path())?;
            }
        }
        Ok(())
    }

    /// Removes the oldest copies, by modification time, until the total is at
    /// most `cap` bytes. It never removes `keep`. It also removes stale
    /// temporary files.
    pub fn evict_to(&self, cap: u64, keep: &Path) -> Result<(), SystemError> {
        if !self.root.is_dir() {
            return Ok(());
        }
        for entry in read_dir(&self.root)? {
            if is_temporary(entry.as_path()) && is_stale(entry.as_path()) {
                remove_file(entry.as_path())?;
            }
        }
        let mut copies = self.copies();
        let mut total: u64 = copies.iter().map(|c| c.len).sum();
        copies.sort_by_key(|c| c.modified);
        for copy in copies {
            if total <= cap {
                break;
            }
            if copy.path == keep {
                continue;
            }
            remove_file(&copy.path)?;
            total -= copy.len;
        }
        Ok(())
    }

    /// The copies in the folder. Temporary files are not copies.
    fn copies(&self) -> Vec<Copy> {
        let Ok(entries) = read_dir(&self.root) else {
            return Vec::new();
        };
        entries
            .into_iter()
            .filter(|e| !is_temporary(e.as_path()))
            .filter_map(|e| {
                let meta = std::fs::metadata(e.as_path()).ok()?;
                meta.is_file().then(|| Copy {
                    path: e.into_path_buf(),
                    len: meta.len(),
                    modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                })
            })
            .collect()
    }
}

struct Copy {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

fn is_temporary(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "tmp")
}

/// Whether a temporary file is old enough that no save can still own it.
fn is_stale(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age >= STALE_TEMP_AGE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{app_dir_under, sys, Host};
    use std::time::Duration;

    fn png(extra: &[u8]) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(extra);
        v
    }

    fn cache() -> (tempfile::TempDir, ImageCache) {
        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::at(sys(dir.path()).join("image-cache"));
        (dir, cache)
    }

    fn names(cache: &ImageCache) -> Vec<String> {
        match std::fs::read_dir(cache.root()) {
            Ok(rd) => rd
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn age(path: &Path, secs_ago: u64) {
        let t = SystemTime::now() - Duration::from_secs(secs_ago);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn extension_follows_the_first_bytes() {
        assert_eq!(image_extension(&png(b"x")), Some("png"));
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_extension(b"GIF89a.."), Some("gif"));
        assert_eq!(image_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_extension(b"<html>"), None);
        assert_eq!(image_extension(b""), None);
    }

    #[test]
    fn save_for_unknown_bytes_or_a_bad_address_fails() {
        let (_d, c) = cache();
        assert!(c.save("https://a.com/x", b"<html>").is_err());
        assert!(c.save("ftp://a.com/x", &png(b"1")).is_err());
    }

    #[test]
    fn save_then_find_returns_the_same_bytes() {
        let (_d, c) = cache();
        let bytes = png(b"hello");
        let saved = c.save("https://a.com/x.png", &bytes).unwrap();
        let found = c.find("https://a.com/x.png").unwrap();
        assert_eq!(saved, found);
        assert_eq!(std::fs::read(found).unwrap(), bytes);
    }

    #[test]
    fn two_spellings_of_one_image_share_one_copy() {
        let (_d, c) = cache();
        let a = c.save("https://A.com/x.png#top", &png(b"1")).unwrap();
        let b = c.save("https://a.com/x.png", &png(b"1")).unwrap();
        assert_eq!(a, b);
        assert_eq!(names(&c).len(), 1);
    }

    #[test]
    fn find_is_none_for_an_unsaved_address_and_for_a_bad_address() {
        let (_d, c) = cache();
        c.save("https://a.com/x.png", &png(b"1")).unwrap();
        assert!(c.find("https://a.com/other.png").is_none());
        assert!(c.find("not a url").is_none());
        assert!(c.find("ftp://a.com/x.png").is_none());
    }

    #[test]
    fn a_save_leaves_no_temporary_file_behind() {
        let (_d, c) = cache();
        c.save("https://a.com/x.png", &png(b"1")).unwrap();
        let names = names(&c);
        assert_eq!(names.len(), 1);
        assert!(!names[0].ends_with(".tmp"));
    }

    #[test]
    fn an_interrupted_write_is_never_found() {
        let (_d, c) = cache();
        let saved = c.save("https://a.com/x.png", &png(b"1")).unwrap();
        let stem = saved.file_stem().unwrap().to_string_lossy().into_owned();
        c.remove("https://a.com/x.png").unwrap();
        let stray = c
            .root()
            .join(format!("{stem}.png.123.0.tmp"))
            .into_path_buf();
        std::fs::write(&stray, png(b"half")).unwrap();
        assert!(c.find("https://a.com/x.png").is_none());
        assert_eq!(c.size(), 0);
        age(&stray, 2 * 60 * 60);
        c.evict_to(u64::MAX, Path::new("")).unwrap();
        assert!(!stray.exists());
    }

    #[test]
    fn eviction_keeps_a_fresh_temporary_file_and_removes_an_old_one() {
        let (_d, c) = cache();
        c.save("https://a.com/x.png", &png(b"1")).unwrap();
        let fresh = c.root().join("aaaa.png.1.0.tmp").into_path_buf();
        let old = c.root().join("bbbb.png.1.1.tmp").into_path_buf();
        std::fs::write(&fresh, b"in flight").unwrap();
        std::fs::write(&old, b"abandoned").unwrap();
        age(&old, 2 * 60 * 60);
        c.evict_to(u64::MAX, Path::new("")).unwrap();
        assert!(fresh.exists());
        assert!(!old.exists());
    }

    #[test]
    fn a_new_format_for_one_address_replaces_the_old_copy() {
        let (_d, c) = cache();
        c.save("https://a.com/x", &png(b"1")).unwrap();
        let jpg = c
            .save("https://a.com/x", &[0xFF, 0xD8, 0xFF, 0xE0])
            .unwrap();
        assert_eq!(names(&c).len(), 1);
        assert_eq!(c.find("https://a.com/x").unwrap(), jpg);
    }

    #[test]
    fn remove_deletes_one_copy_and_ignores_a_missing_one() {
        let (_d, c) = cache();
        c.save("https://a.com/1.png", &png(b"1")).unwrap();
        c.save("https://a.com/2.png", &png(b"2")).unwrap();
        c.remove("https://a.com/1.png").unwrap();
        assert!(c.find("https://a.com/1.png").is_none());
        assert!(c.find("https://a.com/2.png").is_some());
        c.remove("https://a.com/1.png").unwrap();
        c.remove("https://a.com/never.png").unwrap();
    }

    #[test]
    fn holds_names_the_files_in_the_cache_folder_only() {
        let (dir, c) = cache();
        let copy = c.save("https://a.com/x.png", &png(b"1")).unwrap();
        assert!(c.holds(&copy));
        assert!(!c.holds(&dir.path().join("x.png")));
        assert!(!c.holds(&c.root().join("sub").join("x.png").into_path_buf()));
        assert!(!c.holds(c.root().as_path()));
    }

    #[test]
    fn size_and_clear() {
        let (_d, c) = cache();
        assert_eq!(c.size(), 0);
        let mut total = 0;
        for i in 0..3 {
            let bytes = png(&vec![i as u8; 10 * (i + 1)]);
            total += bytes.len() as u64;
            c.save(&format!("https://a.com/{i}.png"), &bytes).unwrap();
        }
        assert_eq!(c.size(), total);
        c.clear().unwrap();
        assert_eq!(c.size(), 0);
        assert!(names(&c).is_empty());
        assert!(c.root().is_dir());
        c.save("https://a.com/0.png", &png(b"again")).unwrap();
        assert!(c.find("https://a.com/0.png").is_some());
    }

    #[test]
    fn evict_to_removes_the_oldest_and_never_the_kept_copy() {
        let (_d, c) = cache();
        let old = c.save("https://a.com/old.png", &png(&[0; 100])).unwrap();
        let mid = c.save("https://a.com/mid.png", &png(&[0; 100])).unwrap();
        let new = c.save("https://a.com/new.png", &png(&[0; 100])).unwrap();
        age(&old, 300);
        age(&mid, 200);
        age(&new, 100);
        // Room for two copies: only the oldest goes.
        let one = new.metadata().unwrap().len();
        c.evict_to(one * 2, &new).unwrap();
        assert!(!old.exists());
        assert!(mid.exists());
        assert!(new.exists());
        c.evict_to(1, &new).unwrap();
        assert!(!mid.exists());
        assert!(new.exists());

        // The kept copy survives even when it is the oldest.
        let other = c.save("https://a.com/other.png", &png(&[0; 100])).unwrap();
        age(&new, 500);
        age(&other, 10);
        c.evict_to(1, &new).unwrap();
        assert!(new.exists());
        assert!(!other.exists());
    }

    #[test]
    fn a_save_over_the_cap_evicts_the_oldest_and_keeps_the_new_copy() {
        let (_d, c) = cache();
        let old = c.save("https://a.com/old.png", &png(&[0; 100])).unwrap();
        age(&old, 300);
        let new = c.save("https://a.com/new.png", &png(&[0; 100])).unwrap();
        let cap = new.metadata().unwrap().len() + 10;
        c.evict_to(cap, &new).unwrap();
        assert!(!old.exists());
        assert!(new.exists());
        assert!(c.size() <= cap);
    }

    #[test]
    fn the_cache_folder_is_under_the_app_dir_and_not_in_a_vault() {
        let home = tempfile::tempdir().unwrap();
        let app = app_dir_under(&sys(home.path()), Host::Unix);
        let root = app.join(CACHE_DIR_NAME);
        assert!(root.as_path().starts_with(app.as_path()));
        assert_eq!(root.as_path().file_name().unwrap(), "image-cache");
        let cache = ImageCache::at(root.clone());
        assert_eq!(cache.root(), &root);
    }
}
