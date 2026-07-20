use std::{fmt::Display, path::Path, path::PathBuf, str::FromStr, sync::LazyLock};

use log::warn;
use regex::Regex;
use serde::{de::Visitor, Deserialize, Serialize};

use super::filename;
use crate::error::FSError;
use crate::utilities::path_to_string;

/// The vault-internal path separator. Always `/`, independent of the host OS:
/// a [`VaultPath`] is logical and portable, and is only translated to native
/// OS separators when resolved to a real on-disk location.
pub const PATH_SEPARATOR: char = '/';
const NOTE_EXTENSION: &str = ".md";

/// Appends the note extension to `name` if it is not already present, without
/// sanitizing the rest of the string. Unlike [`VaultPath::note_path_from`] this
/// leaves wildcards and other non-path characters intact, so search patterns
/// (e.g. `proj*`) keep their meaning. Use it only for building match patterns,
/// never for constructing real vault paths.
pub fn with_note_extension<S: AsRef<str>>(name: S) -> String {
    let name = name.as_ref();
    if name.ends_with(NOTE_EXTENSION) {
        name.to_string()
    } else {
        format!("{name}{NOTE_EXTENSION}")
    }
}

static RX_INCREMENT_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_(?P<number>[0-9]+)$").unwrap());

/// A logical, vault-internal path to a note or directory.
///
/// `VaultPath` is the core's single currency for everything inside a vault: it
/// never refers to a location outside the workspace, and it is portable across
/// Windows, macOS, and Linux. Components are sanitized and lowercased on
/// construction (see [`VaultPath::new`]) so that only characters valid on all
/// three filesystems survive and equality is effectively case-insensitive. The
/// separator is always [`PATH_SEPARATOR`] (`/`); translation to native OS paths
/// happens only at the filesystem boundary in `nfs`.
///
/// A path may be absolute (rooted at the vault root, rendered with a leading
/// `/`) or relative, and may contain `.`/`..` components until [`flatten`]ed.
///
/// [`flatten`]: VaultPath::flatten
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VaultPath {
    absolute: bool,
    pub(super) slices: Vec<VaultPathSlice>,
}

impl FromStr for VaultPath {
    type Err = FSError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_string(s)
    }
}

impl TryFrom<String> for VaultPath {
    type Error = FSError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_string(value)
    }
}

impl From<&VaultPath> for VaultPath {
    fn from(value: &VaultPath) -> Self {
        value.to_owned()
    }
}

impl TryFrom<&str> for VaultPath {
    type Error = FSError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        VaultPath::from_string(value)
    }
}

impl TryFrom<&String> for VaultPath {
    type Error = FSError;

    fn try_from(value: &String) -> Result<Self, Self::Error> {
        VaultPath::from_string(value)
    }
}

impl Serialize for VaultPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let string = self.to_string();
        serializer.serialize_str(string.as_ref())
    }
}

struct DeserializeVaultPathVisitor;
impl Visitor<'_> for DeserializeVaultPathVisitor {
    type Value = VaultPath;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("A valid path with `/` separators, no need of starting `/`")
    }
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        let path = VaultPath::new(value);
        Ok(path)
    }
}

impl<'de> Deserialize<'de> for VaultPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(DeserializeVaultPathVisitor)
    }
}

impl VaultPath {
    /// Creates a new vault path, for every invalid character
    /// it gets replaced to an underscore `_`. If you want to validate
    /// the path first, either use the `VaultPath::From` trait or use
    /// `VaultPath::is_valid()`
    pub fn new<S: AsRef<str>>(path: S) -> Self {
        let mut slices = vec![];
        let absolute = path.as_ref().starts_with(PATH_SEPARATOR);
        path.as_ref()
            .split(PATH_SEPARATOR)
            .filter(|p| !p.is_empty()) // We remove the empty ones,
            // so `//` are treated as `/`
            .for_each(|slice| {
                slices.push(VaultPathSlice::new(slice));
            });
        Self { absolute, slices }
    }

    fn from_string<S: AsRef<str>>(value: S) -> Result<Self, FSError> {
        let path = value.as_ref();
        if Self::is_valid(path) {
            Ok(Self::new(path))
        } else {
            Err(FSError::InvalidPath {
                path: path.to_string(),
                message: "path contains invalid characters".to_string(),
            })
        }
    }

    /// Returns `true` if `path` is already a clean vault path needing no
    /// sanitization: every component is valid on all three target filesystems
    /// and there are no doubled separators. Use this to validate caller-supplied
    /// strings up front; [`VaultPath::new`] will instead silently repair them.
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// assert!(VaultPath::is_valid("/projects/notes.md"));
    /// assert!(!VaultPath::is_valid("bad?name"));
    /// ```
    pub fn is_valid<S: AsRef<str>>(path: S) -> bool {
        // path can only start with one slash `/`
        if path
            .as_ref()
            .starts_with(format!("{}{}", PATH_SEPARATOR, PATH_SEPARATOR).as_str())
        {
            return false;
        }
        !path
            .as_ref()
            .split(PATH_SEPARATOR)
            .any(|s| !VaultPathSlice::is_valid(s))
    }

    /// Builds a sanitized note path from `path`, ensuring it ends with the note
    /// extension. A trailing separator is dropped before the extension is added,
    /// so `notes/` becomes `notes.md`. Unlike [`with_note_extension`], the rest
    /// of the string is sanitized through [`VaultPath::new`], so this is the
    /// correct constructor for real note paths (not search patterns).
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// assert_eq!(VaultPath::note_path_from("projects/todo").to_string(), "projects/todo.md");
    /// assert_eq!(VaultPath::note_path_from("readme.md").to_string(), "readme.md");
    /// ```
    pub fn note_path_from<S: AsRef<str>>(path: S) -> Self {
        let path = path.as_ref();
        let path_clean = path.strip_suffix(PATH_SEPARATOR).unwrap_or(path);
        let p = if !path_clean.ends_with(NOTE_EXTENSION) {
            [path_clean, NOTE_EXTENSION].concat()
        } else {
            path_clean.to_owned()
        };
        VaultPath::new(p)
    }

    /// The vault root: an absolute path with no components, rendered as `/`.
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// assert_eq!(VaultPath::root().to_string(), "/");
    /// ```
    pub fn root() -> Self {
        Self {
            absolute: true,
            slices: vec![],
        }
    }

    /// The empty relative path: no components and not absolute, rendered as the
    /// empty string. Distinct from [`root`](VaultPath::root), which is absolute.
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// assert_eq!(VaultPath::empty().to_string(), "");
    /// ```
    pub fn empty() -> Self {
        Self {
            absolute: false,
            slices: vec![],
        }
    }

    /// Returns `true` when the path has no components, i.e. it is either the
    /// vault root or the empty path.
    pub fn is_root_or_empty(&self) -> bool {
        self.slices.is_empty()
    }

    /// Returns a variant of this path with its final component's name
    /// incremented to avoid a collision. A numeric `_N` suffix is added or bumped
    /// (e.g. `note.md` → `note_0.md`, `note_0.md` → `note_1.md`), preserving the
    /// note extension. Used to pick a fresh name when the desired one is taken.
    pub fn get_name_on_conflict(&self) -> VaultPath {
        let mut slices = self.slices.clone();
        match slices.pop() {
            Some(slice) => {
                if let VaultPathSlice::PathSlice(name) = slice {
                    let new_name = if let Some(name) = name.strip_suffix(NOTE_EXTENSION) {
                        format!("{}{}", Self::increment(name), NOTE_EXTENSION)
                    } else {
                        Self::increment(name)
                    };
                    slices.push(VaultPathSlice::new(new_name));
                    VaultPath {
                        absolute: self.absolute,
                        slices,
                    }
                } else {
                    VaultPath::new("0")
                }
            }
            None => VaultPath::new("0"),
        }
    }

    /// Returns the final component's name with the note extension stripped — the
    /// note's display title as derived from its filename. For directories (no
    /// extension) this is just the directory name. Compare [`get_name`], which
    /// keeps the extension.
    ///
    /// [`get_name`]: VaultPath::get_name
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// assert_eq!(VaultPath::new("/projects/todo.md").get_clean_name(), "todo");
    /// ```
    pub fn get_clean_name(&self) -> String {
        let name = self.get_name();
        if let Some(name) = name.strip_suffix(NOTE_EXTENSION) {
            name.to_string()
        } else {
            name
        }
    }

    /// Returns the full vault path as a string with the note extension stripped.
    /// E.g. `/projects/rust-notes.md` → `/projects/rust-notes`
    /// If the path does not end with the note extension, returns it unchanged.
    pub fn to_bare_string(&self) -> String {
        let s = self.to_string();
        s.strip_suffix(NOTE_EXTENSION)
            .map(|bare| bare.to_owned())
            .unwrap_or(s)
    }

    /// Returns the full vault path as a string, ensuring it ends with the note extension.
    /// E.g. `/projects/rust-notes` → `/projects/rust-notes.md`
    /// If the path already ends with the extension, returns it unchanged.
    pub fn to_string_with_ext(&self) -> String {
        with_note_extension(self.to_string())
    }

    fn increment<S: AsRef<str>>(name: S) -> String {
        let name = name.as_ref();
        let (n, suffix_num) = if let Some(caps) = RX_INCREMENT_SUFFIX.captures(name) {
            let suffix = &caps["number"];
            let n = name
                .strip_suffix(&format!("_{}", suffix))
                .map_or_else(|| name.to_string(), |s| s.to_string());
            (n, suffix.parse::<u64>().map_or_else(|_e| 0, |n| n + 1))
        } else {
            (name.to_string(), 0)
        };
        format!("{}_{}", n, suffix_num)
    }

    /// Returns the path's components as plain strings, after [`flatten`]ing
    /// (so no `.`/`..` entries remain). Useful for walking the path level by
    /// level.
    ///
    /// [`flatten`]: VaultPath::flatten
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// assert_eq!(VaultPath::new("/a/b/c.md").get_slices(), vec!["a", "b", "c.md"]);
    /// ```
    pub fn get_slices(&self) -> Vec<String> {
        self.flatten()
            .slices
            .iter()
            .map(|slice| slice.to_string())
            .collect()
    }

    /// Joins this path onto `workspace_path` to produce the canonical on-disk
    /// `PathBuf`, mapping `/` to native separators and [`flatten`]ing first.
    ///
    /// This is the *canonical* (lowercase) location only; it does not perform
    /// case-insensitive resolution, so an existing file stored under a different
    /// case will not be found. Use the `nfs` resolver for that.
    ///
    /// [`flatten`]: VaultPath::flatten
    pub fn to_pathbuf<P: AsRef<Path>>(&self, workspace_path: P) -> PathBuf {
        let mut path = workspace_path.as_ref().to_path_buf();
        for p in &self.flatten().slices {
            let slice = p.to_string();
            path = path.join(&slice);
        }
        path
    }

    /// Returns a full path without any relative slices
    /// If it tries to go up beyond the current path, drops a warning
    pub fn flatten(&self) -> VaultPath {
        let mut slices = vec![];
        for slice in &self.slices {
            match slice {
                VaultPathSlice::PathSlice(_name) => slices.push(slice.clone()),
                VaultPathSlice::Up => {
                    if slices.pop().is_none() {
                        warn!("Trying to move a directory up from root")
                    }
                }
                VaultPathSlice::Current => {}
            }
        }
        VaultPath {
            absolute: self.absolute,
            slices,
        }
    }

    /// Returns the last part of the path slices
    /// if it is a note, will return the note filename, if it is a directory, will return the directory name
    pub fn get_name(&self) -> String {
        self.flatten().slices.last().map_or_else(String::new, |s| {
            if let VaultPathSlice::PathSlice(name) = s {
                name.to_owned()
            } else {
                String::new()
            }
        })
    }

    /// Returns the path of `self` written relative to a note file's *directory*.
    ///
    /// Markdown engines resolve relative links against the containing folder,
    /// not the note file itself. Linking from `/notes/journal/today.md` to
    /// `/assets/img.png` therefore produces `../../assets/img.png` (two `..`s
    /// — for `journal/` and `notes/`), not three. This wraps
    /// [`Self::get_relative_to`] using the note's parent path so callers get the
    /// markdown-correct result.
    pub fn relative_link_from_note(&self, note_path: &VaultPath) -> VaultPath {
        let (parent, _) = note_path.flatten().get_parent_path();
        self.flatten().get_relative_to(&parent)
    }

    /// Resolve `self` as a link target written inside `note_path`.
    ///
    /// Inverse of [`Self::relative_link_from_note`]: markdown links resolve against
    /// the *directory* containing the note, so a `../work/anton.md` target in
    /// `/journal/today.md` resolves to `/work/anton.md` (flattened, absolute).
    /// Absolute targets are returned flattened as-is. A bare filename with no
    /// directory part (e.g. `anton.md`) is returned unchanged so callers can
    /// fall back to a vault-wide name lookup (wiki-style links).
    pub fn resolve_link_in_note(&self, note_path: &VaultPath) -> VaultPath {
        if self.is_note_file() {
            return self.clone();
        }
        let (parent, _) = note_path.flatten().get_parent_path();
        parent.append(self).flatten().absolute()
    }

    /// Expresses this path relative to `reference_path`, walking up with `..`
    /// for each component of the reference not shared with this path, then down
    /// into this path's remaining components. The result is always relative.
    ///
    /// Note `reference_path` is treated as a directory: every one of its trailing
    /// components becomes a `..`. To build a markdown link relative to a note
    /// *file*, use [`relative_link_from_note`], which accounts for the note's own
    /// filename.
    ///
    /// [`relative_link_from_note`]: VaultPath::relative_link_from_note
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// let from = VaultPath::new("/main/path/first");
    /// let target = VaultPath::new("/main/second");
    /// assert_eq!(target.get_relative_to(&from).to_string(), "../../second");
    /// ```
    pub fn get_relative_to(&self, reference_path: &VaultPath) -> VaultPath {
        let mut slices = vec![];
        let ref_slices = reference_path.slices.clone();
        let mut position = 0;
        for (pos, slice) in self.slices.iter().enumerate() {
            position = pos;
            if let Some(reference) = ref_slices.get(pos) {
                if !slice.eq(reference) {
                    break;
                }
            } else {
                break;
            }
        }
        ref_slices.iter().skip(position).for_each(|_| {
            slices.push(VaultPathSlice::Up);
        });
        self.slices.iter().skip(position).for_each(|slice| {
            slices.push(slice.to_owned());
        });

        VaultPath {
            absolute: false,
            slices,
        }
    }

    /// Converts a real on-disk path back into an absolute vault path by
    /// stripping the `workspace_path` prefix. Returns `FSError::InvalidPath` if
    /// `full_path` does not live inside the workspace. Each OS component is run
    /// through [`VaultPath::new`], so the result is sanitized and lowercased.
    pub fn from_path<P: AsRef<Path>, F: AsRef<Path>>(
        workspace_path: P,
        full_path: F,
    ) -> Result<Self, FSError> {
        let fp = full_path.as_ref();
        let relative = fp
            .strip_prefix(&workspace_path)
            .map_err(|_e| FSError::InvalidPath {
                path: path_to_string(&full_path),
                message: format!(
                    "The path provided is not a path belonging to the workspace: {}",
                    path_to_string(workspace_path)
                ),
            })?;
        let mut path_list = vec![PATH_SEPARATOR.to_string()];
        relative.components().for_each(|component| {
            let os_str = component.as_os_str();
            let slice = match os_str.to_str() {
                Some(comp) => comp.to_owned(),
                None => os_str.to_string_lossy().to_string(),
            };
            path_list.push(slice);
        });
        let pl = path_list.join(PATH_SEPARATOR.to_string().as_str());

        Ok(VaultPath::new(pl).absolute())
    }

    /// Returns `true` if this path is a *bare* note filename: a single,
    /// relative component ending in the note extension, with no directory part
    /// (e.g. `anton.md`). Such paths are the signal for a vault-wide, wiki-style
    /// name lookup rather than a directory-scoped path match.
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// assert!(VaultPath::new("anton.md").is_note_file());
    /// assert!(!VaultPath::new("/work/anton.md").is_note_file());
    /// ```
    pub fn is_note_file(&self) -> bool {
        match self.slices.last() {
            Some(path_slice) => path_slice.is_note() && self.slices.len() == 1 && !self.absolute,
            None => false,
        }
    }

    /// Returns `true` if this path points at a note, i.e. its final component
    /// ends with the note extension. Unlike [`is_note_file`], the path may have
    /// any number of directory components.
    ///
    /// [`is_note_file`]: VaultPath::is_note_file
    pub fn is_note(&self) -> bool {
        match self.slices.last() {
            Some(path_slice) => path_slice.is_note(),
            None => false,
        }
    }

    /// Returns Ok if the path looks like a note path; otherwise an `InvalidPath` error.
    pub fn ensure_note(&self) -> Result<(), FSError> {
        if self.is_note() {
            Ok(())
        } else {
            Err(FSError::InvalidPath {
                path: self.to_string(),
                message: "The path is not a note".to_string(),
            })
        }
    }

    /// Returns Ok if the path does not have a note extension; otherwise an `InvalidPath` error.
    pub fn ensure_directory(&self) -> Result<(), FSError> {
        if self.is_note() {
            Err(FSError::InvalidPath {
                path: self.to_string(),
                message: "The path is not a directory".to_string(),
            })
        } else {
            Ok(())
        }
    }

    /// Returns `true` if this path is relative (not rooted at the vault root).
    pub fn is_relative(&self) -> bool {
        !self.absolute
    }

    /// Returns `true` if this path is absolute (rooted at the vault root).
    pub fn is_absolute(&self) -> bool {
        self.absolute
    }

    /// Marks this path absolute in place.
    pub fn to_absolute(&mut self) {
        self.absolute = true;
    }

    /// Consumes the path and returns it marked absolute (builder-style sibling
    /// of [`to_absolute`](VaultPath::to_absolute)).
    pub fn absolute(mut self) -> Self {
        self.absolute = true;
        self
    }

    /// Marks this path relative in place.
    pub fn to_relative(&mut self) {
        self.absolute = false;
    }

    /// Canonical index identity of this path: [`flatten`]ed and vault-*absolute*
    /// (rooted at `/`), so a note has exactly one key whether it was reached as
    /// `note.md` or `/note.md`. Every path that is stored in or looked up from
    /// the index must go through this, so the index can never hold mixed
    /// relative/absolute forms of the same note. Absolute is the established
    /// index form — the vault walker that bulk-indexes notes produces it, and
    /// browse/search results carry it.
    ///
    /// [`flatten`]: VaultPath::flatten
    pub(crate) fn canonical(&self) -> VaultPath {
        self.flatten().absolute()
    }

    /// Splits the path into its parent path and the final component's name.
    /// The parent keeps this path's absoluteness; the name is the empty string
    /// when the path has no components.
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// let (parent, name) = VaultPath::new("/a/b/c.md").get_parent_path();
    /// assert_eq!(parent.to_string(), "/a/b");
    /// assert_eq!(name, "c.md");
    /// ```
    pub fn get_parent_path(&self) -> (VaultPath, String) {
        let mut new_path = self.slices.clone();
        let current = new_path
            .pop()
            .map_or_else(|| "".to_string(), |s| s.to_string());

        (
            Self {
                absolute: self.absolute,
                slices: new_path,
            },
            current,
        )
    }

    /// Appends `path` to this one. If `path` is absolute it wins outright and is
    /// returned as-is; otherwise its components are concatenated onto this path,
    /// keeping this path's absoluteness. The result is not flattened, so any
    /// `..` in `path` survives until [`flatten`] is called.
    ///
    /// [`flatten`]: VaultPath::flatten
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// let base = VaultPath::new("/main/path");
    /// let rel = VaultPath::new("sub/note.md");
    /// assert_eq!(base.append(&rel).to_string(), "/main/path/sub/note.md");
    /// ```
    pub fn append(&self, path: &VaultPath) -> VaultPath {
        if !path.is_relative() {
            // Absolute paths are absolute
            path.to_owned()
        } else {
            let mut slices = self.slices.clone();
            let mut other_slices = path.slices.clone();
            slices.append(&mut other_slices);
            VaultPath {
                absolute: self.absolute,
                slices,
            }
        }
    }

    /// Compares two paths by components only, ignoring whether each is absolute
    /// or relative. So `/a/b` is "like" `a/b`.
    ///
    /// ```
    /// use kimun_core::nfs::VaultPath;
    /// assert!(VaultPath::new("/a/b").is_like(&VaultPath::new("a/b")));
    /// ```
    pub fn is_like(&self, other: &VaultPath) -> bool {
        self.slices.eq(&other.slices)
    }
}

impl Display for VaultPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.absolute {
            write!(f, "{}", PATH_SEPARATOR)?;
        }
        write!(
            f,
            "{}",
            self.slices
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
                .join(&PATH_SEPARATOR.to_string())
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum VaultPathSlice {
    PathSlice(String),
    Up,
    Current,
}

impl VaultPathSlice {
    fn new<S: AsRef<str>>(slice: S) -> Self {
        // Replace runs of leading dots so "..foo" becomes "__foo".
        let slice = if filename::RX_PATH_NAME.is_match(slice.as_ref()) {
            slice.as_ref().replace(".", "_")
        } else {
            slice.as_ref().to_string()
        };
        if slice.eq("..") {
            VaultPathSlice::Up
        } else if slice.eq(".") {
            VaultPathSlice::Current
        } else {
            // Replace invalid chars, lowercase, strip leading/trailing spaces and
            // trailing dots (Windows silently strips them, causing silent collisions).
            let sanitized = filename::RX_PATH_CHARS
                .replace_all(&slice, "_")
                .to_lowercase();
            let sanitized = sanitized.trim().trim_end_matches('.').to_string();
            // Prefix Windows reserved device names so they don't map to device handles.
            let final_slice = if filename::RX_WIN_RESERVED.is_match(&sanitized) {
                format!("_{}", sanitized)
            } else {
                sanitized
            };

            VaultPathSlice::PathSlice(final_slice)
        }
    }

    fn is_valid<S: AsRef<str>>(slice: S) -> bool {
        let slice = slice.as_ref();
        if slice == "." || slice == ".." {
            return true;
        }
        !filename::RX_PATH_CHARS.is_match(slice)
            && !filename::RX_PATH_NAME.is_match(slice)
            && !filename::RX_WIN_RESERVED.is_match(slice)
            && !slice.ends_with('.')
            && !slice.starts_with(' ')
            && !slice.ends_with(' ')
    }

    fn is_note(&self) -> bool {
        match self {
            VaultPathSlice::PathSlice(name) => name.ends_with(NOTE_EXTENSION),
            _ => false,
        }
    }
}

impl Display for VaultPathSlice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultPathSlice::PathSlice(name) => write!(f, "{}", name),
            VaultPathSlice::Up => write!(f, ".."),
            VaultPathSlice::Current => write!(f, "."),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::with_note_extension;

    #[test]
    fn with_note_extension_appends_when_missing() {
        assert_eq!(with_note_extension("projects"), "projects.md");
    }

    #[test]
    fn with_note_extension_keeps_when_present() {
        assert_eq!(with_note_extension("projects.md"), "projects.md");
    }

    #[test]
    fn with_note_extension_preserves_wildcards_and_path() {
        // Unlike VaultPath, this does not sanitize `*` so search wildcards survive.
        assert_eq!(with_note_extension("work/proj*"), "work/proj*.md");
    }

    use crate::{error::FSError, utilities::path_to_string};

    use super::{VaultPath, VaultPathSlice};

    // --- cross-platform character validation tests ---

    #[test]
    fn control_chars_are_invalid() {
        // Control characters U+0001–U+001F must be rejected (Windows forbids them)
        assert!(!VaultPath::is_valid("note\x01name"));
        assert!(!VaultPath::is_valid("dir\x1fname"));
    }

    #[test]
    fn control_chars_are_sanitized_in_new() {
        let path = VaultPath::new("note\x07name");
        assert_eq!("note_name", path.to_string());
    }

    #[test]
    fn windows_reserved_names_are_invalid() {
        // Windows device names must be rejected regardless of extension or case
        for name in &["CON", "PRN", "AUX", "NUL", "COM1", "COM9", "LPT1", "LPT9"] {
            assert!(!VaultPath::is_valid(name), "{name} should be invalid");
            assert!(
                !VaultPath::is_valid(format!("{name}.md")),
                "{name}.md should be invalid"
            );
        }
        // Lower-case variants too
        assert!(!VaultPath::is_valid("con.md"));
        assert!(!VaultPath::is_valid("nul"));
    }

    #[test]
    fn windows_reserved_names_are_sanitized_in_new() {
        // VaultPath::new should prefix reserved names with '_' so they don't map to
        // Windows device handles. The name is already lowercased by this point.
        let path = VaultPath::new("con.md");
        assert_eq!("_con.md", path.to_string());

        let path = VaultPath::new("nul");
        assert_eq!("_nul", path.to_string());

        let path = VaultPath::new("COM1.md");
        assert_eq!("_com1.md", path.to_string());
    }

    #[test]
    fn trailing_dot_is_invalid() {
        // Windows silently strips trailing dots from filenames
        assert!(!VaultPath::is_valid("notes."));
        assert!(!VaultPath::is_valid("dir./sub"));
    }

    #[test]
    fn trailing_dot_is_sanitized_in_new() {
        let path = VaultPath::new("notes./sub");
        // trailing dot stripped from directory component
        assert_eq!("notes/sub", path.to_string());
    }

    #[test]
    fn leading_or_trailing_spaces_are_invalid() {
        assert!(!VaultPath::is_valid(" note"));
        assert!(!VaultPath::is_valid("note "));
        assert!(!VaultPath::is_valid(" dir /sub"));
    }

    #[test]
    fn leading_and_trailing_spaces_are_sanitized_in_new() {
        let path = VaultPath::new(" note ");
        assert_eq!("note", path.to_string());
    }

    #[test]
    fn should_print_correctly() {
        let path_with_root = "/some/path";
        let path_without_root = "another/one";

        let path1 = VaultPath::new(path_with_root);
        let path2 = VaultPath::new(path_without_root);

        assert_eq!("/some/path".to_string(), path1.to_string());
        assert_eq!("another/one".to_string(), path2.to_string());
    }

    #[test]
    fn test_valid_path() {
        let path = "/some/path.md";
        assert!(VaultPath::is_valid(path));
    }

    #[test]
    fn test_rel_path() {
        let path = VaultPath::new("../some/path.md");
        assert_eq!("../some/path.md", path.to_string());
        assert!(path.is_relative());
    }

    #[test]
    fn join_two_paths() {
        let path1 = VaultPath::new("main/path");
        let path2 = VaultPath::new("sub/path");
        let joined = path1.append(&path2);
        assert_eq!("main/path/sub/path".to_string(), joined.to_string());
    }

    #[test]
    fn join_two_paths_with_relative() {
        let path1 = VaultPath::new("/main/path");
        let path2 = VaultPath::new("../sub/path");
        let joined = path1.append(&path2).flatten();
        assert_eq!("/main/sub/path".to_string(), joined.to_string());
    }

    #[test]
    fn path_with_up_dir_end() {
        let path = VaultPath::new("/main/path/..");
        assert_eq!("/main".to_string(), path.flatten().to_string());
    }

    #[test]
    fn from_current_path() {
        let path = VaultPath::new("./path/subpath");
        assert!(!path.flatten().absolute);
        assert_eq!("path/subpath", path.flatten().to_string());
    }

    #[test]
    fn only_dots_three_or_more_not_allowed_in_path() {
        let path = "/some/.../path";
        assert!(!VaultPath::is_valid(path));

        let vault_path = VaultPath::new(path);
        assert_eq!("/some/___/path", vault_path.to_string());
    }

    #[test]
    fn get_relative_to() {
        let path1 = VaultPath::new("/main/path/first");
        let path2 = VaultPath::new("/main/second");
        let rel = path2.get_relative_to(&path1);

        assert_eq!("../../second".to_string(), rel.to_string());
    }

    #[test]
    fn get_relative_to_less_deep() {
        let path1 = VaultPath::new("/main/second");
        let path2 = VaultPath::new("/main/path/first");
        let rel = path2.get_relative_to(&path1);

        assert_eq!("../path/first".to_string(), rel.to_string());
    }

    #[test]
    fn get_relative_to_same() {
        let path1 = VaultPath::new("/main/second");
        let path2 = VaultPath::new("/main/second/sub/deep");
        let rel = path2.get_relative_to(&path1);

        assert_eq!("sub/deep".to_string(), rel.to_string());
    }

    #[test]
    fn relative_link_from_note_uses_parent_dir() {
        let note = VaultPath::new("/notes/journal/today.md");
        let asset = VaultPath::new("/assets/img.png");
        assert_eq!(
            "../../assets/img.png",
            asset.relative_link_from_note(&note).to_string()
        );
    }

    #[test]
    fn relative_link_from_root_note_to_assets() {
        let note = VaultPath::new("/note.md");
        let asset = VaultPath::new("/assets/img.png");
        assert_eq!(
            "assets/img.png",
            asset.relative_link_from_note(&note).to_string()
        );
    }

    #[test]
    fn relative_link_to_sibling_dir() {
        let note = VaultPath::new("/notes/today.md");
        let asset = VaultPath::new("/notes/assets/img.png");
        assert_eq!(
            "assets/img.png",
            asset.relative_link_from_note(&note).to_string()
        );
    }

    #[test]
    fn resolve_link_in_note_walks_up_and_lowercases() {
        let note = VaultPath::new("/journal/2026-03-01.md");
        let target = VaultPath::note_path_from("../Work/People/anton.md");
        assert_eq!(
            "/work/people/anton.md",
            target.resolve_link_in_note(&note).to_string()
        );
    }

    #[test]
    fn resolve_link_in_note_keeps_bare_name_for_name_lookup() {
        let note = VaultPath::new("/journal/2026-03-01.md");
        let target = VaultPath::note_path_from("anton.md");
        // Bare name unchanged (relative, single slice) so open_or_search does a
        // vault-wide name lookup rather than a directory-scoped path match.
        let resolved = target.resolve_link_in_note(&note);
        assert_eq!("anton.md", resolved.to_string());
        assert!(resolved.is_note_file());
    }

    #[test]
    fn resolve_link_in_note_absolute_target_unchanged() {
        let note = VaultPath::new("/journal/2026-03-01.md");
        let target = VaultPath::note_path_from("/work/people/anton.md");
        assert_eq!(
            "/work/people/anton.md",
            target.resolve_link_in_note(&note).to_string()
        );
    }

    #[test]
    fn resolve_link_in_note_sibling_subdir() {
        let note = VaultPath::new("/journal/2026-03-01.md");
        let target = VaultPath::note_path_from("attachments/notes.md");
        assert_eq!(
            "/journal/attachments/notes.md",
            target.resolve_link_in_note(&note).to_string()
        );
    }

    #[test]
    fn get_root() {
        let vault_path = VaultPath::root();
        assert_eq!("/".to_string(), vault_path.to_string());

        let root_path = VaultPath::new("/");
        assert_eq!(root_path, vault_path);
    }

    #[test]
    fn get_empty() {
        let vault_path = VaultPath::empty();
        assert_eq!("".to_string(), vault_path.to_string());

        let root_path = VaultPath::new("");
        assert_eq!(root_path, vault_path);
    }

    #[test]
    fn should_tell_if_its_note() {
        let path = "/some/../path.md";
        assert!(VaultPath::new(path).is_note());
    }

    #[test]
    fn paths_should_flatten_correctly() {
        let path = "some/path/../hola";
        assert!(VaultPath::is_valid(path));

        let vault_path = VaultPath::from_string(path).unwrap();
        let vault_path = vault_path.flatten();

        assert_eq!("some/hola".to_string(), vault_path.to_string());
    }

    #[test]
    fn test_file_should_not_look_like_url() {
        let valid = VaultPath::is_valid("http://example.com");

        assert!(!valid);
    }

    #[test]
    fn test_slice_char_replace() {
        let slice_str = "Some?unvalid:Chars?";
        let slice = VaultPathSlice::new(slice_str);

        assert_eq!("some_unvalid_chars_", slice.to_string());
        if let VaultPathSlice::PathSlice(name) = slice {
            assert_eq!("some_unvalid_chars_", name);
        }
    }

    #[test]
    fn test_path_create_from_string() {
        let path = "this/is/five/level/path";
        let path = VaultPath::new(path);

        assert_eq!(5, path.slices.len());
        assert_eq!("this", path.slices[0].to_string());
        assert_eq!("is", path.slices[1].to_string());
        assert_eq!("five", path.slices[2].to_string());
        assert_eq!("level", path.slices[3].to_string());
        assert_eq!("path", path.slices[4].to_string());
    }

    #[test]
    fn test_path_with_unvalid_chars() {
        let path = "t*his/i+s/caca?/";
        let path = VaultPath::new(path);

        assert_eq!(3, path.slices.len());
        assert_eq!("t_his", path.slices[0].to_string());
        assert_eq!("i+s", path.slices[1].to_string());
        assert_eq!("caca_", path.slices[2].to_string());
    }

    #[test]
    fn test_to_path_buf() {
        let workspace_path = PathBuf::from("workspace");
        let sep = std::path::MAIN_SEPARATOR_STR;

        let path = "/some/subpath";
        let path = VaultPath::new(path);
        let path_buf = path.to_pathbuf(&workspace_path);

        let path_string = path_to_string(path_buf);
        let expected_path_str = format!("workspace{sep}some{sep}subpath");
        assert_eq!(expected_path_str, path_string);
    }

    #[test]
    fn test_path_check_valid() {
        let path = PathBuf::from("/some/valid/path/workspace/note.md");
        let workspace = PathBuf::from("/some/valid/path");

        let entry = VaultPath::from_path(&workspace, &path).unwrap();

        assert_eq!("/workspace/note.md", entry.to_string());
    }

    #[test]
    fn test_vault_path_serialization() {
        use serde_json;

        let path = VaultPath::new("/test/path.md");
        let serialized = serde_json::to_string(&path).unwrap();
        assert_eq!(serialized, "\"/test/path.md\"");

        let deserialized: VaultPath = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, path);
    }

    #[test]
    fn test_vault_path_try_from() {
        let path_str = "/valid/path.md";
        let path_result: Result<VaultPath, FSError> = path_str.try_into();
        assert!(path_result.is_ok());

        let invalid_path_str = "/invalid:path.md";
        let invalid_result: Result<VaultPath, FSError> = invalid_path_str.try_into();
        assert!(invalid_result.is_err());
    }

    #[test]
    fn test_vault_path_from_str() {
        use std::str::FromStr;

        let path_str = "/test/path.md";
        let path = VaultPath::from_str(path_str).unwrap();
        assert_eq!(path.to_string(), path_str);

        let invalid_str = "/invalid:path.md";
        let result = VaultPath::from_str(invalid_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_vault_path_note_path_from() {
        let path_without_extension = "test/note";
        let path_with_extension = "test/note.md";
        let path_with_trailing_slash = "test/note/";

        let note_path1 = VaultPath::note_path_from(path_without_extension);
        let note_path2 = VaultPath::note_path_from(path_with_extension);
        let note_path3 = VaultPath::note_path_from(path_with_trailing_slash);

        assert_eq!(note_path1.to_string(), "test/note.md");
        assert_eq!(note_path2.to_string(), "test/note.md");
        assert_eq!(note_path3.to_string(), "test/note.md");

        assert!(note_path1.is_note());
        assert!(note_path2.is_note());
        assert!(note_path3.is_note());
    }

    #[test]
    fn test_vault_path_get_name_on_conflict() {
        let note_path = VaultPath::new("test.md");
        let conflicted = note_path.get_name_on_conflict();
        assert_eq!(conflicted.to_string(), "test_0.md");

        let numbered_path = VaultPath::new("test_5.md");
        let conflicted_numbered = numbered_path.get_name_on_conflict();
        assert_eq!(conflicted_numbered.to_string(), "test_6.md");

        let dir_path = VaultPath::new("directory");
        let conflicted_dir = dir_path.get_name_on_conflict();
        assert_eq!(conflicted_dir.to_string(), "directory_0");

        let empty_path = VaultPath::empty();
        let conflicted_empty = empty_path.get_name_on_conflict();
        assert_eq!(conflicted_empty.to_string(), "0");
    }

    #[test]
    fn test_vault_path_get_clean_name() {
        let note_path = VaultPath::new("/path/to/note.md");
        assert_eq!(note_path.get_clean_name(), "note");

        let dir_path = VaultPath::new("/path/to/directory");
        assert_eq!(dir_path.get_clean_name(), "directory");

        let root_path = VaultPath::root();
        assert_eq!(root_path.get_clean_name(), "");
    }

    #[test]
    fn test_vault_path_get_slices() {
        let path = VaultPath::new("/path/to/../file.md");
        let slices = path.get_slices();
        assert_eq!(slices, vec!["path", "file.md"]);
    }

    #[test]
    fn test_vault_path_is_like() {
        let path1 = VaultPath::new("/test/path.md");
        let path2 = VaultPath::new("test/path.md"); // relative version
        let path3 = VaultPath::new("/different/path.md");

        assert!(path1.is_like(&path2));
        assert!(!path1.is_like(&path3));
    }

    #[test]
    fn test_vault_path_slice_edge_cases() {
        // Test slice with dots
        let path_with_dots = VaultPath::new("...invalid");
        assert_eq!(path_with_dots.to_string(), "___invalid");

        // Test slice with invalid characters
        let path_with_invalid = VaultPath::new("test:file?.md");
        assert_eq!(path_with_invalid.to_string(), "test_file_.md");

        // Test current directory slice
        let path_with_current = VaultPath::new("./test");
        assert_eq!(path_with_current.flatten().to_string(), "test");

        // Test parent directory slice
        let path_with_parent = VaultPath::new("../test");
        assert_eq!(path_with_parent.to_string(), "../test");
    }

    #[test]
    fn test_vault_path_increment_function() {
        use super::VaultPath;

        // Test the increment functionality through get_name_on_conflict
        let base_name = VaultPath::new("test");
        let incremented = base_name.get_name_on_conflict();
        assert_eq!(incremented.to_string(), "test_0");

        let numbered_name = VaultPath::new("test_3");
        let incremented_numbered = numbered_name.get_name_on_conflict();
        assert_eq!(incremented_numbered.to_string(), "test_4");
    }

    #[test]
    fn vault_path_normalizes_to_lowercase() {
        // Paths are always stored lowercase regardless of input case
        let a = VaultPath::new("/Projects/Note.md");
        let b = VaultPath::new("/projects/note.md");
        assert_eq!(a, b);
        assert_eq!(a.to_string(), "/projects/note.md");
    }
}
