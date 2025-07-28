pub mod visitor;
// Contains the structs to support the data types
use std::{
    fmt::Display,
    hash::Hash,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    time::UNIX_EPOCH,
};

use gxhash::gxhash64;
use ignore::{WalkBuilder, WalkParallel};
use log::{info, warn};
use regex::Regex;
use serde::{de::Visitor, Deserialize, Serialize};

use super::{error::FSError, DirectoryDetails, NoteDetails};

use super::utilities::path_to_string;

pub const PATH_SEPARATOR: char = '/';
const NOTE_EXTENSION: &str = ".md";
// non valid chars
// Not allowed: \ | : * ? " < > | [ ] ^ #
const NON_VALID_PATH_CHARS_REGEX: &str = r#"[\\/:*?"<>|\[\]\^\#]"#;
// Not allowed files starting with two dots
const NON_VALID_PATH_NAME: &str = r#"^\.{2,}.+$"#;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultEntry {
    pub path: VaultPath,
    pub path_string: String,
    pub data: EntryData,
}

impl AsRef<str> for VaultEntry {
    fn as_ref(&self) -> &str {
        self.path_string.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntryData {
    Note(NoteEntryData),
    Directory(DirectoryEntryData),
    Attachment,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NoteEntryData {
    pub path: VaultPath,
    // File size, for fast check
    pub size: u64,
    pub modified_secs: u64,
}

impl NoteEntryData {
    pub fn load_details<P: AsRef<Path>>(
        &self,
        workspace_path: P,
        path: &VaultPath,
    ) -> Result<NoteDetails, FSError> {
        let content = load_note(workspace_path, path)?;
        Ok(NoteDetails::new(path, content))
    }

    fn from_path<P: AsRef<Path>>(
        workspace_path: P,
        path: &VaultPath,
    ) -> Result<NoteEntryData, FSError> {
        let file_path = path.to_pathbuf(&workspace_path);

        let metadata = file_path.metadata()?;
        let size = metadata.len();
        let modified_secs = metadata
            .modified()
            .map(|t| t.duration_since(UNIX_EPOCH).unwrap().as_secs())
            .unwrap_or_else(|_e| 0);
        Ok(NoteEntryData {
            path: path.flatten(),
            size,
            modified_secs,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirectoryEntryData {
    pub path: VaultPath,
}
impl DirectoryEntryData {
    pub fn get_details<P: AsRef<Path>>(&self) -> DirectoryDetails {
        DirectoryDetails {
            path: self.path.clone(),
        }
    }
}

fn _get_dir_content_size<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<u64, FSError> {
    let os_path = path.to_pathbuf(&workspace_path);
    let walker = ignore::WalkBuilder::new(&os_path)
        .max_depth(Some(1))
        .filter_entry(filter_files)
        .build();
    let mut content_size = 0;
    for entry in walker.flatten() {
        let entry_path = entry.path();
        if entry_path.is_file() && entry_path.extension().is_some_and(|ext| ext == "md") {
            let metadata = std::fs::metadata(&os_path)?;
            let file_size = metadata.len();
            content_size += file_size;
        }
    }
    Ok(content_size)
}

impl VaultEntry {
    pub fn new<P: AsRef<Path>>(workspace_path: P, path: VaultPath) -> Result<Self, FSError> {
        let os_path = path.to_pathbuf(&workspace_path);
        if !os_path.exists() {
            return Err(FSError::NoFileOrDirectoryFound {
                path: path_to_string(os_path),
            });
        }

        let kind = if os_path.is_dir() {
            EntryData::Directory(DirectoryEntryData { path: path.clone() })
        } else if path.is_note() {
            let note_entry_data = NoteEntryData::from_path(workspace_path, &path)?;
            EntryData::Note(note_entry_data)
        } else {
            EntryData::Attachment
        };
        let path_string = path.to_string();

        Ok(VaultEntry {
            path,
            path_string,
            data: kind,
        })
    }

    pub fn from_path<P: AsRef<Path>, F: AsRef<Path>>(
        workspace_path: P,
        full_path: F,
    ) -> Result<Self, FSError> {
        let note_path = VaultPath::from_path(&workspace_path, &full_path)?;
        Self::new(&workspace_path, note_path)
    }
}

impl Display for VaultEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.data {
            EntryData::Note(_details) => write!(f, "[NOT] {}", self.path),
            EntryData::Directory(_details) => write!(f, "[DIR] {}", self.path),
            EntryData::Attachment => write!(f, "[ATT]"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum VaultEntryDetails {
    // Hash
    Note(NoteDetails),
    Directory(DirectoryDetails),
    None,
}

impl VaultEntryDetails {
    pub fn get_title(&mut self) -> String {
        match self {
            VaultEntryDetails::Note(note_details) => note_details.get_title(),
            VaultEntryDetails::Directory(_directory_details) => String::new(),
            VaultEntryDetails::None => String::new(),
        }
    }
}

pub(crate) fn hash_text<S: AsRef<str>>(text: S) -> u64 {
    gxhash64(text.as_ref().as_bytes(), 0)
}

/// Loads a note from disk, if the file doesn't exist, returns a FSError::NotePathNotFound
/// Returns the note's text. If you want the details, use NoteDetails::from_content
pub(crate) fn load_note<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<String, FSError> {
    let os_path = path.to_pathbuf(&workspace_path);
    match std::fs::read(&os_path) {
        Ok(file) => {
            let text = String::from_utf8(file)?;
            Ok(text)
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => Err(FSError::VaultPathNotFound {
                path: path.to_owned(),
            }),
            _ => Err(FSError::ReadFileError(e)),
        },
    }
}

pub fn save_note<P: AsRef<Path>, S: AsRef<str>>(
    workspace_path: P,
    path: &VaultPath,
    text: S,
) -> Result<NoteEntryData, FSError> {
    if !path.is_note() {
        return Err(FSError::InvalidPath {
            path: path.to_string(),
            message: "Path provided is not a note".to_string(),
        });
    }
    let (parent, note) = path.get_parent_path();
    let base_path = parent.to_pathbuf(&workspace_path);
    let full_path = base_path.join(note);
    std::fs::create_dir_all(base_path)?;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(full_path)?;
    file.write_all(text.as_ref().as_bytes())?;

    let entry = NoteEntryData::from_path(workspace_path, path)?;
    Ok(entry)
}

pub fn move_note<P: AsRef<Path>>(
    workspace_path: P,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<(), FSError> {
    if !from.is_note() {
        return Err(FSError::InvalidPath {
            path: from.to_string(),
            message: "Path is not a note".to_string(),
        });
    }
    if !to.is_note() {
        return Err(FSError::InvalidPath {
            path: to.to_string(),
            message: "Path is not a note".to_string(),
        });
    }

    let full_from_path = from.to_pathbuf(&workspace_path);
    let full_to_path = to.to_pathbuf(&workspace_path);
    // We create the destination directory if doesn't exist
    if let Some(parent) = full_to_path.parent() {
        if !std::fs::exists(parent)? || !parent.is_dir() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::rename(full_from_path, full_to_path)?;
    Ok(())
}

pub fn move_directory<P: AsRef<Path>>(
    workspace_path: P,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<(), FSError> {
    if from.is_note() {
        return Err(FSError::InvalidPath {
            path: from.to_string(),
            message: "Path is not a directory".to_string(),
        });
    }
    if to.is_note() {
        return Err(FSError::InvalidPath {
            path: to.to_string(),
            message: "Path is not a Directory".to_string(),
        });
    }

    let full_from_path = from.to_pathbuf(&workspace_path);
    let full_to_path = to.to_pathbuf(&workspace_path);
    // We create the destination directory if doesn't exist
    if !std::fs::exists(&full_to_path)? || !full_to_path.is_dir() {
        std::fs::create_dir_all(&full_to_path)?;
    }
    std::fs::rename(full_from_path, full_to_path)?;
    Ok(())
}
pub fn delete_note<P: AsRef<Path>>(workspace_path: P, path: &VaultPath) -> Result<(), FSError> {
    let full_path = path.to_pathbuf(workspace_path);
    std::fs::remove_file(full_path)?;
    Ok(())
}

pub fn delete_directory<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<(), FSError> {
    let full_path = path.to_pathbuf(workspace_path);
    std::fs::remove_dir_all(full_path)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultPath {
    absolute: bool,
    slices: Vec<VaultPathSlice>,
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
        // let mut slices = path.as_ref().split(PATH_SEPARATOR).peekable();
        // let mut valid = true;
        // while let Some(slice) = slices.next() {
        //     valid = if slices.peek().is_none() {
        //         // Last element
        //         matches!(VaultPathSlice::new(slice), VaultPathSlice::PathSlice(_name))
        //     } else {
        //         VaultPathSlice::is_valid(slice)
        //     };
        //     if !valid {
        //         break;
        //     }
        // }
        // valid
    }

    // Creates a note file path, if the path ends with a separator
    // it removes it before adding the extension
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

    pub fn root() -> Self {
        Self {
            absolute: true,
            slices: vec![],
        }
    }

    pub fn empty() -> Self {
        Self {
            absolute: false,
            slices: vec![],
        }
    }

    pub fn is_root_or_empty(&self) -> bool {
        self.slices.is_empty()
    }

    // returns a NotePath that increases a prefix when
    // conflicting the name
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

    pub fn get_clean_name(&self) -> String {
        let (_, name) = self.get_parent_path();
        if let Some(name) = name.strip_suffix(NOTE_EXTENSION) {
            name.to_string()
        } else {
            name
        }
    }

    fn increment<S: AsRef<str>>(name: S) -> String {
        let name = name.as_ref();
        let re = Regex::new(r"_(?P<number>[0-9]+)$").unwrap();
        let (n, suffix_num) = if let Some(caps) = re.captures(name) {
            let suffix = &caps["number"];
            info!("Suffix: {}", suffix);
            let n = name
                .strip_suffix(&format!("_{}", suffix))
                .map_or_else(|| name.to_string(), |s| s.to_string());
            (n, suffix.parse::<u64>().map_or_else(|_e| 0, |n| n + 1))
        } else {
            info!("Suffix not found, new one: {}", 0);
            (name.to_string(), 0)
        };
        format!("{}_{}", n, suffix_num)
    }

    pub fn get_slices(&self) -> Vec<String> {
        self.flatten()
            .slices
            .iter()
            .map(|slice| slice.to_string())
            .collect()
    }

    pub(super) fn to_pathbuf<P: AsRef<Path>>(&self, workspace_path: P) -> PathBuf {
        let mut path = workspace_path.as_ref().to_path_buf();
        for p in &self.flatten().slices {
            let slice = p.to_string();
            path = path.join(&slice);
        }
        path
    }

    /// Returns a full path without any relative slices
    /// It will always return an absolute path, as it assumes the path is relative to the root
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
            absolute: true,
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

        Ok(VaultPath::new(pl))
    }

    // returns true if it's just a note file, no path relative to the vault
    pub fn is_note_file(&self) -> bool {
        match self.slices.last() {
            Some(path_slice) => path_slice.is_note() && self.slices.len() == 1 && !self.absolute,
            None => false,
        }
    }

    pub fn is_note(&self) -> bool {
        match self.slices.last() {
            Some(path_slice) => path_slice.is_note(),
            None => false,
        }
    }

    pub fn is_relative(&self) -> bool {
        !self.absolute
    }

    pub fn to_absolute(&mut self) {
        self.absolute = true;
    }

    pub fn absolute(mut self) -> Self {
        self.absolute = true;
        self
    }

    pub fn to_relative(&mut self) {
        self.absolute = false;
    }

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

    pub fn append(&self, path: &VaultPath) -> VaultPath {
        let mut slices = self.slices.clone();
        let mut other_slices = path.slices.clone();
        slices.append(&mut other_slices);
        VaultPath {
            absolute: self.absolute,
            slices,
        }
    }

    // Compares two paths, ignoring if they are absolute or not
    pub fn is_like(&self, other: &VaultPath) -> bool {
        self.slices.eq(&other.slices)
    }
}

impl Display for VaultPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{}",
            if self.absolute { "/" } else { "" },
            self.slices
                .iter()
                .map(|s| { s.to_string() })
                .collect::<Vec<String>>()
                .join(&PATH_SEPARATOR.to_string())
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum SliceKind {
    Note,
    // This can be either an attachment or a dir path
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum VaultPathSlice {
    PathSlice(String),
    Up,
    Current,
}

impl VaultPathSlice {
    fn new<S: AsRef<str>>(slice: S) -> Self {
        // We don't want filenames or directories starting with two dots or more
        let rx_dot = regex::Regex::new(NON_VALID_PATH_NAME).unwrap();
        let slice = if rx_dot.is_match(slice.as_ref()) {
            slice.as_ref().to_string().replace(".", "_")
        } else {
            slice.as_ref().to_string()
        };
        if slice.eq("..") {
            VaultPathSlice::Up
        } else if slice.eq(".") {
            VaultPathSlice::Current
        } else {
            let rx_chars = regex::Regex::new(NON_VALID_PATH_CHARS_REGEX).unwrap();
            let final_slice = rx_chars.replace_all(slice.as_ref(), "_").to_lowercase();

            VaultPathSlice::PathSlice(final_slice)
        }
    }

    fn is_valid<S: AsRef<str>>(slice: S) -> bool {
        let rx_chars = regex::Regex::new(NON_VALID_PATH_CHARS_REGEX).unwrap();
        let rx_dot = regex::Regex::new(NON_VALID_PATH_NAME).unwrap();
        let slice = slice.as_ref();
        !rx_chars.is_match(slice) && !rx_dot.is_match(slice)
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

fn filter_files(dir: &ignore::DirEntry) -> bool {
    !dir.path().starts_with(".")
}

pub fn get_file_walker<P: AsRef<Path>>(
    base_path: P,
    path: &VaultPath,
    recurse: bool,
) -> WalkParallel {
    let w = WalkBuilder::new(path.to_pathbuf(base_path))
        .max_depth(if recurse { None } else { Some(1) })
        .filter_entry(filter_files)
        // .threads(0)
        .build_parallel();

    w
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::{
        error::FSError,
        nfs::{delete_directory, delete_note, move_directory, move_note, save_note},
        utilities::path_to_string,
    };

    use super::{load_note, VaultPath, VaultPathSlice};

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
    fn test_file_not_exists() {
        let path = VaultPath::new("don't exist");
        let res = load_note(std::env::current_dir().unwrap(), &path);

        let result = if let Err(e) = res {
            matches!(e, FSError::VaultPathNotFound { path: _ })
        } else {
            false
        };

        assert!(result);
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
        let workspace_path = PathBuf::from("/usr/john/notes");
        let path = "/some/subpath";
        let path = VaultPath::new(path);
        let path_buf = path.to_pathbuf(&workspace_path);

        let path_string = path_to_string(path_buf);
        assert_eq!("/usr/john/notes/some/subpath", path_string);
    }

    #[test]
    fn test_path_check_valid() {
        let path = PathBuf::from("/some/valid/path/workspace/note.md");
        let workspace = PathBuf::from("/some/valid/path");

        let entry = VaultPath::from_path(&workspace, &path).unwrap();

        assert_eq!("workspace/note.md", entry.to_string());
    }

    #[test]
    fn create_a_note() -> Result<(), FSError> {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("note.md");
        let note_text = "this is an empty note".to_string();

        save_note(workspace_path, &note_path, &note_text)?;
        let note = load_note(workspace_path, &note_path)?;
        assert_eq!(note, note_text);

        delete_note(workspace_path, &note_path)?;
        assert!(load_note(workspace_path, &note_path).is_err());

        Ok(())
    }

    #[test]
    fn move_a_note() -> Result<(), FSError> {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("note.md");
        let dest_note_path = VaultPath::new("directory/moved_note.md");
        let note_text = "this is an empty note".to_string();

        save_note(workspace_path, &note_path, &note_text)?;
        let note = load_note(workspace_path, &note_path)?;
        assert_eq!(note, note_text);

        move_note(workspace_path, &note_path, &dest_note_path)?;
        let moved_note = load_note(workspace_path, &dest_note_path)?;
        assert_eq!(note, moved_note);
        assert!(load_note(workspace_path, &note_path).is_err());

        delete_note(workspace_path, &dest_note_path)?;
        assert!(load_note(workspace_path, &dest_note_path).is_err());

        delete_directory(workspace_path, &dest_note_path.get_parent_path().0)?;

        Ok(())
    }

    #[test]
    fn move_a_directory() -> Result<(), FSError> {
        let workspace_path = Path::new("testdata");
        let from_note_dir = VaultPath::new("old_dir");
        let from_note_path = from_note_dir.append(&VaultPath::new("note.md"));
        let dest_note_dir = VaultPath::new("new_dir/two_levels");
        let dest_note_path = dest_note_dir.append(&VaultPath::new("note.md"));
        let note_text = "this is an empty note".to_string();

        save_note(workspace_path, &from_note_path, &note_text)?;
        let note = load_note(workspace_path, &from_note_path)?;
        assert_eq!(note, note_text);

        move_directory(workspace_path, &from_note_dir, &dest_note_dir)?;
        let moved_note = load_note(workspace_path, &dest_note_path)?;
        assert_eq!(note, moved_note);
        assert!(load_note(workspace_path, &from_note_dir).is_err());

        delete_note(workspace_path, &dest_note_path)?;
        assert!(load_note(workspace_path, &dest_note_path).is_err());

        delete_directory(workspace_path, &dest_note_path.get_parent_path().0)?;

        Ok(())
    }
}
