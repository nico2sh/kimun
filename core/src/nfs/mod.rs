pub mod visitor;
// Contains the structs to support the data types
use std::{
    ffi::OsStr,
    fmt::Display,
    hash::Hash,
    io::Write,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use gxhash::gxhash64;
use ignore::{WalkBuilder, WalkParallel};
use log::info;
use regex::Regex;
use serde::{de::Visitor, Deserialize, Serialize};

use super::{error::FSError, DirectoryDetails, NoteDetails};

use super::utilities::path_to_string;

pub const PATH_SEPARATOR: char = '/';
const NOTE_EXTENSION: &str = ".md";
// non valid chars
// Not allowed: \ | : * ? " < > | [ ] ^ #
const NON_VALID_PATH_CHARS_REGEX: &str = r#"[\\/:*?"<>|\[\]\^\#]"#;

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
            path: path.clone(),
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultPath {
    slices: Vec<VaultPathSlice>,
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
        let path_list = path
            .as_ref()
            .split(PATH_SEPARATOR)
            .filter(|p| !p.is_empty()) // We remove the empty ones,
            // so `//` are treated as `/`
            .map(VaultPathSlice::new)
            .collect();
        Self { slices: path_list }
    }

    fn from_string<S: AsRef<str>>(value: S) -> Result<Self, FSError> {
        let path = value.as_ref();
        if Self::is_valid(path) {
            Ok(Self::new(path))
        } else {
            Err(FSError::InvalidPath {
                path: path.to_string(),
            })
        }
    }

    pub fn is_valid<S: AsRef<str>>(path: S) -> bool {
        !path
            .as_ref()
            .split(PATH_SEPARATOR)
            .any(|s| !VaultPathSlice::is_valid(s))
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
        Self { slices: Vec::new() }
    }

    // returns a NotePath that increases a prefix when
    // conflicting the name
    pub fn get_name_on_conflict(&self) -> VaultPath {
        let mut slices = self.slices.clone();
        match slices.pop() {
            Some(slice) => {
                let name = &slice.name;
                let new_name = if let Some(name) = name.strip_suffix(NOTE_EXTENSION) {
                    format!("{}{}", Self::increment(name), NOTE_EXTENSION)
                } else {
                    Self::increment(name)
                };
                slices.push(VaultPathSlice::new(new_name));
                VaultPath { slices }
            }
            None => VaultPath::new("0"),
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
        self.slices
            .iter()
            .map(|slice| slice.name.to_owned())
            .collect()
    }

    pub(super) fn to_pathbuf<P: AsRef<Path>>(&self, workspace_path: P) -> PathBuf {
        let mut path = workspace_path.as_ref().to_path_buf();
        for p in &self.slices {
            let slice = p.name.clone();
            path = path.join(&slice);
        }
        path
    }

    pub fn get_name(&self) -> String {
        self.slices
            .last()
            .map_or_else(String::new, |s| s.name.clone())
    }

    pub fn from_path<P: AsRef<Path>, F: AsRef<Path>>(
        workspace_path: P,
        full_path: F,
    ) -> Result<Self, FSError> {
        let fp = full_path.as_ref();
        let relative = fp
            .strip_prefix(workspace_path)
            .map_err(|_e| FSError::InvalidPath {
                path: path_to_string(&full_path),
            })?;
        let path_list = relative
            .components()
            .map(|component| {
                let os_str = component.as_os_str();
                match os_str.to_str() {
                    Some(comp) => comp.to_owned(),
                    None => os_str.to_string_lossy().to_string(),
                }
            })
            .collect::<Vec<String>>()
            .join(PATH_SEPARATOR.to_string().as_str());

        Ok(VaultPath::new(path_list))
    }

    pub fn is_note(&self) -> bool {
        match self.slices.last() {
            Some(path_slice) => {
                let last_slice: &Path = Path::new(&path_slice.name);
                last_slice
                    .extension()
                    .and_then(OsStr::to_str)
                    .map_or_else(|| false, |s| s == "md")
            }
            None => false,
        }
    }

    pub fn get_parent_path(&self) -> (VaultPath, String) {
        let mut new_path = self.slices.clone();
        let current = new_path.pop().map_or_else(|| "".to_string(), |s| s.name);

        (Self { slices: new_path }, current)
    }

    pub fn append(&self, path: &VaultPath) -> VaultPath {
        let mut slices = self.slices.clone();
        let mut other_slices = path.slices.clone();
        slices.append(&mut other_slices);
        VaultPath { slices }
    }
}

impl Display for VaultPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.slices
                .iter()
                .map(|s| { s.to_string() })
                .collect::<Vec<String>>()
                .join(&PATH_SEPARATOR.to_string())
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct VaultPathSlice {
    name: String,
}

impl VaultPathSlice {
    fn new<S: AsRef<str>>(slice: S) -> Self {
        let re = regex::Regex::new(NON_VALID_PATH_CHARS_REGEX).unwrap();
        let final_slice = re.replace_all(slice.as_ref(), "_");

        Self {
            name: final_slice.to_string(),
        }
    }

    fn is_valid<S: AsRef<str>>(slice: S) -> bool {
        let re = regex::Regex::new(NON_VALID_PATH_CHARS_REGEX).unwrap();
        !re.is_match(slice.as_ref())
    }
}

impl Display for VaultPathSlice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
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
    use std::path::PathBuf;

    use crate::{error::FSError, utilities::path_to_string};

    use super::{load_note, VaultPath, VaultPathSlice};

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
        let slice_str = "Some?unvalid:chars?";
        let slice = VaultPathSlice::new(slice_str);

        assert_eq!("Some_unvalid_chars_", slice.name);
    }

    #[test]
    fn test_path_create_from_string() {
        let path = "this/is/five/level/path";
        let path = VaultPath::new(path);

        assert_eq!(5, path.slices.len());
        assert_eq!("this", path.slices[0].name);
        assert_eq!("is", path.slices[1].name);
        assert_eq!("five", path.slices[2].name);
        assert_eq!("level", path.slices[3].name);
        assert_eq!("path", path.slices[4].name);
    }

    #[test]
    fn test_path_with_unvalid_chars() {
        let path = "t*his/i+s/caca?/";
        let path = VaultPath::new(path);

        assert_eq!(3, path.slices.len());
        assert_eq!("t_his", path.slices[0].name);
        assert_eq!("i+s", path.slices[1].name);
        assert_eq!("caca_", path.slices[2].name);
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
}
