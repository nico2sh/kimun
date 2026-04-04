pub mod visitor;
// Contains the structs to support the data types
use std::{
    fmt::Display,
    hash::Hash,
    path::{Path, PathBuf},
    str::FromStr,
    time::UNIX_EPOCH,
};

// use gxhash::gxhash64;
use ignore::{WalkBuilder, WalkParallel};
use log::{info, warn};
use regex::Regex;
use serde::{de::Visitor, Deserialize, Serialize};
use twox_hash::XxHash64;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct NoteEntryData {
    pub path: VaultPath,
    // File size, for fast check
    pub size: u64,
    pub modified_secs: u64,
}

impl NoteEntryData {
    pub async fn load_details<P: AsRef<Path>>(
        &self,
        workspace_path: P,
        path: &VaultPath,
    ) -> Result<NoteDetails, FSError> {
        let content = load_note(workspace_path, path).await?;
        Ok(NoteDetails::new(path, content))
    }

    async fn from_path<P: AsRef<Path>>(
        workspace_path: P,
        path: &VaultPath,
    ) -> Result<NoteEntryData, FSError> {
        let file_path = resolve_path_on_disk(&workspace_path, path).await;
        Self::from_os_path(path, &file_path).await
    }

    async fn from_os_path(path: &VaultPath, file_path: &Path) -> Result<NoteEntryData, FSError> {
        let metadata = tokio::fs::metadata(file_path).await?;
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

async fn _get_dir_content_size<P: AsRef<Path>>(
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
            let metadata = tokio::fs::metadata(&os_path).await?;
            let file_size = metadata.len();
            content_size += file_size;
        }
    }
    Ok(content_size)
}

impl VaultEntry {
    pub async fn new<P: AsRef<Path>>(workspace_path: P, path: VaultPath) -> Result<Self, FSError> {
        let os_path = resolve_path_on_disk(&workspace_path, &path).await;
        Self::from_vault_and_os_path(path, &os_path).await
    }

    /// Creates a `VaultEntry` when the real on-disk path is already known,
    /// skipping the case-insensitive resolve step.
    async fn from_vault_and_os_path(path: VaultPath, os_path: &Path) -> Result<Self, FSError> {
        let metadata = tokio::fs::metadata(os_path)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => FSError::NoFileOrDirectoryFound {
                    path: path_to_string(os_path),
                },
                _ => FSError::ReadFileError(e),
            })?;

        let kind = if metadata.is_dir() {
            EntryData::Directory(DirectoryEntryData { path: path.clone() })
        } else if path.is_note() {
            let note_entry_data = NoteEntryData::from_os_path(&path, os_path).await?;
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

    pub async fn from_path<P: AsRef<Path>, F: AsRef<Path>>(
        workspace_path: P,
        full_path: F,
    ) -> Result<Self, FSError> {
        let note_path = VaultPath::from_path(&workspace_path, &full_path)?;
        // full_path is the real disk path already provided by the OS walker — no re-resolve needed.
        Self::from_vault_and_os_path(note_path, full_path.as_ref()).await
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
    // XxHash3_64::oneshot(text.as_ref().as_bytes())
    XxHash64::oneshot(42, text.as_ref().as_bytes())
    // gxhash64(text.as_ref().as_bytes(), 0)
}

/// Resolves a VaultPath to the real PathBuf on disk by matching each component
/// case-insensitively. When a component doesn't exist on disk yet, the stored
/// (lowercase) name is used for the remainder of the path.
pub(crate) async fn resolve_path_on_disk<P: AsRef<Path>>(
    workspace_path: P,
    vault_path: &VaultPath,
) -> PathBuf {
    let mut current = workspace_path.as_ref().to_path_buf();
    for slice in &vault_path.flatten().slices {
        let name = slice.to_string();
        let real_name = async {
            let mut entries = tokio::fs::read_dir(&current).await.ok()?;
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_name().to_string_lossy().to_lowercase() == name {
                    return Some(entry.file_name().to_string_lossy().into_owned());
                }
            }
            None
        }
        .await
        .unwrap_or(name);
        current = current.join(real_name);
    }
    current
}

/// Sync variant of `resolve_path_on_disk` for use in non-async contexts.
pub(crate) fn resolve_path_on_disk_sync<P: AsRef<Path>>(
    workspace_path: P,
    vault_path: &VaultPath,
) -> PathBuf {
    let mut current = workspace_path.as_ref().to_path_buf();
    for slice in &vault_path.flatten().slices {
        let name = slice.to_string();
        let real_name = std::fs::read_dir(&current)
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .find(|e| e.file_name().to_string_lossy().to_lowercase() == name)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
            })
            .unwrap_or(name);
        current = current.join(real_name);
    }
    current
}

/// Loads a note from disk, if the file doesn't exist, returns a FSError::NotePathNotFound
/// Returns the note's text. If you want the details, use NoteDetails::from_content
pub(crate) async fn load_note<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<String, FSError> {
    let os_path = resolve_path_on_disk(&workspace_path, path).await;
    match tokio::fs::read(&os_path).await {
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

pub async fn create_directory<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<DirectoryEntryData, FSError> {
    if path.is_note() {
        return Err(FSError::InvalidPath {
            path: path.to_string(),
            message: "Path provided is a note".to_string(),
        });
    }

    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    tokio::fs::create_dir_all(full_path).await?;
    Ok(DirectoryEntryData {
        path: path.to_owned(),
    })
}

pub async fn save_note<P: AsRef<Path>, S: AsRef<str>>(
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
    // Resolve the full path case-insensitively so an existing `MyNote.md` is
    // written in place rather than creating a new lowercase `mynote.md` alongside it.
    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    if let Some(base_path) = full_path.parent() {
        tokio::fs::create_dir_all(base_path).await?;
    }
    tokio::fs::write(&full_path, text.as_ref().as_bytes()).await?;

    let entry = NoteEntryData::from_os_path(path, &full_path).await?;
    Ok(entry)
}

pub async fn rename_note<P: AsRef<Path>>(
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

    let full_from_path = resolve_path_on_disk(&workspace_path, from).await;
    let (to_parent, to_name) = to.get_parent_path();
    let to_base = resolve_path_on_disk(&workspace_path, &to_parent).await;
    let full_to_path = to_base.join(&to_name);
    // We create the destination directory if doesn't exist
    match tokio::fs::metadata(&to_base).await {
        Ok(m) if m.is_dir() => {}
        _ => {
            tokio::fs::create_dir_all(&to_base).await?;
        }
    }
    tokio::fs::rename(full_from_path, full_to_path).await?;
    Ok(())
}

pub async fn rename_directory<P: AsRef<Path>>(
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

    let full_from_path = resolve_path_on_disk(&workspace_path, from).await;
    let (to_parent, to_name) = to.get_parent_path();
    let to_base = resolve_path_on_disk(&workspace_path, &to_parent).await;
    let full_to_path = to_base.join(&to_name);
    // We create the destination directory if doesn't exist
    match tokio::fs::metadata(&to_base).await {
        Ok(m) if m.is_dir() => {}
        _ => {
            tokio::fs::create_dir_all(&to_base).await?;
        }
    }
    tokio::fs::rename(full_from_path, full_to_path).await?;
    Ok(())
}
pub async fn delete_note<P: AsRef<Path>>(workspace_path: P, path: &VaultPath) -> Result<(), FSError> {
    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    tokio::fs::remove_file(full_path).await?;
    Ok(())
}

pub async fn delete_directory<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<(), FSError> {
    let full_path = resolve_path_on_disk(&workspace_path, path).await;
    tokio::fs::remove_dir_all(full_path).await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
        let s = self.to_string();
        if s.ends_with(NOTE_EXTENSION) {
            s
        } else {
            format!("{}{}", s, NOTE_EXTENSION)
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

        Ok(VaultPath::new(pl).absolute())
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

    pub fn is_absolute(&self) -> bool {
        self.absolute
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

// #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
// enum SliceKind {
//     Note,
//     // This can be either an attachment or a dir path
//     Other,
// }
//
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

pub fn list_directories<P: AsRef<Path>>(
    base_path: P,
    path: &VaultPath,
    recursive: bool,
) -> Result<Vec<super::DirectoryDetails>, FSError> {
    let base_path = base_path.as_ref();
    let os_path = resolve_path_on_disk_sync(base_path, path);
    let walker = WalkBuilder::new(&os_path)
        .max_depth(if recursive { None } else { Some(1) })
        .filter_entry(filter_files)
        .build();

    let mut dirs = Vec::new();
    for entry in walker.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() && entry_path != os_path {
            let vault_path = VaultPath::from_path(base_path, entry_path)?;
            dirs.push(super::DirectoryDetails { path: vault_path });
        }
    }
    Ok(dirs)
}

pub fn get_file_walker<P: AsRef<Path>>(
    base_path: P,
    path: &VaultPath,
    recurse: bool,
) -> WalkParallel {
    let w = WalkBuilder::new(resolve_path_on_disk_sync(base_path, path))
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
        nfs::{
            create_directory, delete_directory, delete_note, rename_directory, rename_note,
            save_note, DirectoryEntryData, EntryData, VaultEntry, VaultEntryDetails,
        },
        utilities::path_to_string,
        DirectoryDetails, NoteDetails,
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

    #[tokio::test]
    async fn test_file_not_exists() {
        let path = VaultPath::new("don't exist");
        let res = load_note(std::env::current_dir().unwrap(), &path).await;

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

    #[tokio::test]
    async fn create_a_note() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let note_path = VaultPath::new("note.md");
        let note_text = "this is an empty note".to_string();

        let res = save_note(workspace_path, &note_path, &note_text).await;
        if let Err(e) = &res {
            panic!("Error saving note: {e}")
        }

        let note = load_note(workspace_path, &note_path).await;
        if let Err(e) = &note {
            panic!("Error loading note: {e}")
        }
        assert_eq!(note.unwrap(), note_text);

        let del_res = delete_note(workspace_path, &note_path).await;
        if let Err(e) = &del_res {
            panic!("Error deleting note: {e}")
        }
        assert!(load_note(workspace_path, &note_path).await.is_err());
    }

    #[tokio::test]
    async fn move_a_note() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let note_path = VaultPath::new("note.md");
        let dest_note_path = VaultPath::new("directory/moved_note.md");
        let note_text = "this is an empty note".to_string();

        let res = save_note(workspace_path, &note_path, &note_text).await;
        if let Err(e) = &res {
            panic!("Error saving note: {e}")
        }
        let note = load_note(workspace_path, &note_path).await;
        if let Err(e) = &note {
            panic!("Error loading note: {e}")
        }
        assert_eq!(note.as_ref().unwrap().to_owned(), note_text);

        let ren_res = rename_note(workspace_path, &note_path, &dest_note_path).await;
        if let Err(e) = &ren_res {
            panic!("Error renaming note: {e}")
        }
        let moved_note = load_note(workspace_path, &dest_note_path).await;
        if let Err(e) = &moved_note {
            panic!("Error loading note: {e}")
        }
        assert_eq!(note.unwrap(), moved_note.unwrap());
        assert!(load_note(workspace_path, &note_path).await.is_err());

        let del_res = delete_note(workspace_path, &dest_note_path).await;
        if let Err(e) = &del_res {
            panic!("Error deleting note: {e}")
        }
        assert!(load_note(workspace_path, &dest_note_path).await.is_err());

        let del_res = delete_directory(workspace_path, &dest_note_path.get_parent_path().0).await;
        if let Err(e) = &del_res {
            panic!("Error deleting directory: {e}")
        }
    }

    #[tokio::test]
    async fn move_a_directory() -> Result<(), FSError> {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let from_note_dir = VaultPath::new("old_dir");
        let from_note_path = from_note_dir.append(&VaultPath::new("note.md"));
        let dest_note_dir = VaultPath::new("new_dir/two_levels");
        let dest_note_path = dest_note_dir.append(&VaultPath::new("note.md"));
        let note_text = "this is an empty note".to_string();

        save_note(workspace_path, &from_note_path, &note_text).await?;
        let note = load_note(workspace_path, &from_note_path).await?;
        assert_eq!(note, note_text);

        rename_directory(workspace_path, &from_note_dir, &dest_note_dir).await?;
        let moved_note = load_note(workspace_path, &dest_note_path).await?;
        assert_eq!(note, moved_note);
        assert!(load_note(workspace_path, &from_note_dir).await.is_err());

        delete_note(workspace_path, &dest_note_path).await?;
        assert!(load_note(workspace_path, &dest_note_path).await.is_err());

        let first_level = dest_note_path.get_parent_path().0;
        let second_level = first_level.get_parent_path().0;
        delete_directory(workspace_path, &first_level).await?;
        delete_directory(workspace_path, &second_level).await?;

        Ok(())
    }

    // Additional comprehensive tests for NFS module

    #[tokio::test]
    async fn test_vault_entry_new_with_directory() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let dir_path = VaultPath::new("test_directory");

        // Create directory first
        tokio::fs::create_dir_all(workspace_path.join("test_directory"))
            .await
            .ok();

        let result = VaultEntry::new(workspace_path, dir_path.clone()).await;
        assert!(result.is_ok());

        let entry = result.unwrap();
        assert_eq!(entry.path, dir_path);
        assert_eq!(entry.path_string, dir_path.to_string());

        match entry.data {
            EntryData::Directory(dir_data) => {
                assert_eq!(dir_data.path, dir_path);
            }
            _ => panic!("Expected Directory entry data"),
        }

        // Cleanup
        tokio::fs::remove_dir_all(workspace_path.join("test_directory"))
            .await
            .ok();
    }

    #[tokio::test]
    async fn test_vault_entry_new_with_note() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("test_note.md");
        let note_content = "# Test Note\n\nThis is a test.";

        // Create note first
        save_note(workspace_path, &note_path, note_content).await.unwrap();

        let result = VaultEntry::new(workspace_path, note_path.clone()).await;
        assert!(result.is_ok());

        let entry = result.unwrap();
        assert_eq!(entry.path, note_path);

        match entry.data {
            EntryData::Note(note_data) => {
                assert_eq!(note_data.path, note_path);
                assert!(note_data.size > 0);
                assert!(note_data.modified_secs > 0);
            }
            _ => panic!("Expected Note entry data"),
        }

        // Cleanup
        delete_note(workspace_path, &note_path).await.ok();
    }

    #[tokio::test]
    async fn test_vault_entry_new_with_attachment() {
        let workspace_path = Path::new("testdata");
        let attachment_path = VaultPath::new("test.txt");

        // Create a text file (attachment)
        tokio::fs::create_dir_all(workspace_path).await.ok();
        tokio::fs::write(workspace_path.join("test.txt"), "test content")
            .await
            .unwrap();

        let result = VaultEntry::new(workspace_path, attachment_path.clone()).await;
        assert!(result.is_ok());

        let entry = result.unwrap();
        match entry.data {
            EntryData::Attachment => (),
            _ => panic!("Expected Attachment entry data"),
        }

        // Cleanup
        tokio::fs::remove_file(workspace_path.join("test.txt")).await.ok();
    }

    #[tokio::test]
    async fn test_vault_entry_new_with_nonexistent_path() {
        let workspace_path = Path::new("testdata");
        let nonexistent_path = VaultPath::new("does_not_exist.md");

        let result = VaultEntry::new(workspace_path, nonexistent_path).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            FSError::NoFileOrDirectoryFound { .. } => (),
            _ => panic!("Expected NoFileOrDirectoryFound error"),
        }
    }

    #[tokio::test]
    async fn test_vault_entry_from_path() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("from_path_test.md");
        let note_content = "Test content";

        // Create note
        save_note(workspace_path, &note_path, note_content).await.unwrap();

        let full_path = workspace_path.join("from_path_test.md");
        let result = VaultEntry::from_path(workspace_path, &full_path).await;
        assert!(result.is_ok());

        let entry = result.unwrap();
        assert_eq!(entry.path, note_path.clone().absolute());

        // Cleanup
        delete_note(workspace_path, &note_path).await.ok();
    }

    #[tokio::test]
    async fn test_vault_entry_display() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("display_test.md");
        let dir_path = VaultPath::new("display_dir");
        let attachment_path = VaultPath::new("display.txt");

        // Test note display
        save_note(workspace_path, &note_path, "content").await.unwrap();
        let note_entry = VaultEntry::new(workspace_path, note_path.clone()).await.unwrap();
        let note_display = format!("{}", note_entry);
        assert!(note_display.contains("[NOT]"));
        assert!(note_display.contains(&note_path.to_string()));

        // Test directory display
        tokio::fs::create_dir_all(workspace_path.join("display_dir")).await.ok();
        let dir_entry = VaultEntry::new(workspace_path, dir_path.clone()).await.unwrap();
        let dir_display = format!("{}", dir_entry);
        assert!(dir_display.contains("[DIR]"));
        assert!(dir_display.contains(&dir_path.to_string()));

        // Test attachment display
        tokio::fs::write(workspace_path.join("display.txt"), "content").await.ok();
        let attachment_entry = VaultEntry::new(workspace_path, attachment_path.clone()).await.unwrap();
        let attachment_display = format!("{}", attachment_entry);
        assert!(attachment_display.contains("[ATT]"));

        // Cleanup
        delete_note(workspace_path, &note_path).await.ok();
        tokio::fs::remove_dir_all(workspace_path.join("display_dir")).await.ok();
        tokio::fs::remove_file(workspace_path.join("display.txt")).await.ok();
    }

    #[tokio::test]
    async fn test_note_entry_data_load_details() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("details_test.md");
        let note_content = "# Test\n\nContent here";

        save_note(workspace_path, &note_path, note_content).await.unwrap();
        let entry = VaultEntry::new(workspace_path, note_path.clone()).await.unwrap();

        if let EntryData::Note(note_data) = entry.data {
            let details_result = note_data.load_details(workspace_path, &note_path).await;
            assert!(details_result.is_ok());

            let details = details_result.unwrap();
            assert_eq!(details.path, note_path);
            assert_eq!(details.raw_text, note_content);
        } else {
            panic!("Expected Note entry data");
        }

        // Cleanup
        delete_note(workspace_path, &note_path).await.ok();
    }

    #[test]
    fn test_directory_entry_data_get_details() {
        let dir_path = VaultPath::new("test_dir");
        let dir_data = DirectoryEntryData {
            path: dir_path.clone(),
        };

        let details = dir_data.get_details::<PathBuf>();
        assert_eq!(details.path, dir_path);
    }

    #[test]
    fn test_vault_entry_details_get_title() {
        let note_path = VaultPath::new("test.md");
        let note_content = "# My Title\n\nContent";
        let note_details = NoteDetails::new(&note_path, note_content);

        let mut note_entry_details = VaultEntryDetails::Note(note_details);
        let title = note_entry_details.get_title();
        assert_eq!(title, "My Title");

        let dir_path = VaultPath::new("test_dir");
        let dir_details = DirectoryDetails { path: dir_path };
        let mut dir_entry_details = VaultEntryDetails::Directory(dir_details);
        let dir_title = dir_entry_details.get_title();
        assert_eq!(dir_title, "");

        let mut none_details = VaultEntryDetails::None;
        let none_title = none_details.get_title();
        assert_eq!(none_title, "");
    }

    #[test]
    fn test_hash_text() {
        use super::hash_text;

        let text1 = "Hello, world!";
        let text2 = "Hello, world!";
        let text3 = "Different text";

        let hash1 = hash_text(text1);
        let hash2 = hash_text(text2);
        let hash3 = hash_text(text3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert!(hash1 > 0);
    }

    #[tokio::test]
    async fn test_create_directory_with_note_path() {
        let workspace_path = Path::new("testdata");
        let note_path = VaultPath::new("invalid.md");

        let result = create_directory(workspace_path, &note_path).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            FSError::InvalidPath { message, .. } => {
                assert_eq!(message, "Path provided is a note");
            }
            _ => panic!("Expected InvalidPath error"),
        }
    }

    #[tokio::test]
    async fn test_save_note_with_directory_path() {
        let workspace_path = Path::new("testdata");
        let dir_path = VaultPath::new("directory");
        let content = "test content";

        let result = save_note(workspace_path, &dir_path, content).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            FSError::InvalidPath { message, .. } => {
                assert_eq!(message, "Path provided is not a note");
            }
            _ => panic!("Expected InvalidPath error"),
        }
    }

    #[tokio::test]
    async fn test_rename_note_with_invalid_paths() {
        let workspace_path = Path::new("testdata");
        let dir_path = VaultPath::new("directory");
        let note_path = VaultPath::new("note.md");

        // Test renaming from directory (should fail)
        let result = rename_note(workspace_path, &dir_path, &note_path).await;
        assert!(result.is_err());

        // Test renaming to directory (should fail)
        let result = rename_note(workspace_path, &note_path, &dir_path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rename_directory_with_invalid_paths() {
        let workspace_path = Path::new("testdata");
        let dir_path = VaultPath::new("directory");
        let note_path = VaultPath::new("note.md");

        // Test renaming from note (should fail)
        let result = rename_directory(workspace_path, &note_path, &dir_path).await;
        assert!(result.is_err());

        // Test renaming to note (should fail)
        let result = rename_directory(workspace_path, &dir_path, &note_path).await;
        assert!(result.is_err());
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

    // ── Case-insensitive disk resolution tests ────────────────────────────────

    #[tokio::test]
    async fn resolve_finds_uppercase_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Journal")).await.unwrap();

        let result = super::resolve_path_on_disk(tmp.path(), &VaultPath::new("/journal")).await;
        assert_eq!(result, tmp.path().join("Journal"));
    }

    #[tokio::test]
    async fn resolve_finds_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects")).await.unwrap();
        tokio::fs::write(tmp.path().join("Projects").join("MyNote.md"), "hi").await.unwrap();

        let result = super::resolve_path_on_disk(tmp.path(), &VaultPath::new("/projects/mynote.md")).await;
        assert_eq!(result, tmp.path().join("Projects").join("MyNote.md"));
    }

    #[tokio::test]
    async fn resolve_uses_lowercase_for_nonexistent_path() {
        let tmp = tempfile::TempDir::new().unwrap();

        let result = super::resolve_path_on_disk(tmp.path(), &VaultPath::new("/newdir/note.md")).await;
        assert_eq!(result, tmp.path().join("newdir").join("note.md"));
    }

    #[test]
    fn resolve_sync_finds_uppercase_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("Archive")).unwrap();

        let result = super::resolve_path_on_disk_sync(tmp.path(), &VaultPath::new("/archive"));
        assert_eq!(result, tmp.path().join("Archive"));
    }

    #[tokio::test]
    async fn load_note_finds_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Journal")).await.unwrap();
        tokio::fs::write(tmp.path().join("Journal").join("MyNote.md"), "# Hello").await.unwrap();

        let text = super::load_note(tmp.path(), &VaultPath::new("/journal/mynote.md")).await.unwrap();
        assert_eq!(text, "# Hello");
    }

    #[tokio::test]
    async fn save_note_writes_to_existing_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Journal")).await.unwrap();
        tokio::fs::write(tmp.path().join("Journal").join("MyNote.md"), "original").await.unwrap();

        save_note(tmp.path(), &VaultPath::new("/journal/mynote.md"), "updated").await.unwrap();

        // The uppercase file should be updated
        let content = tokio::fs::read_to_string(tmp.path().join("Journal").join("MyNote.md")).await.unwrap();
        assert_eq!(content, "updated");

        // No duplicate lowercase file should have been created
        assert!(!tmp.path().join("Journal").join("mynote.md").exists());
        // No duplicate lowercase directory should have been created
        assert!(!tmp.path().join("journal").exists());
    }

    #[tokio::test]
    async fn save_note_in_uppercase_parent_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects")).await.unwrap();

        save_note(tmp.path(), &VaultPath::new("/projects/new.md"), "content").await.unwrap();

        // File should land inside the existing uppercase directory
        assert!(tmp.path().join("Projects").join("new.md").exists());
        // No duplicate lowercase directory should have been created
        assert!(!tmp.path().join("projects").exists());
    }

    #[tokio::test]
    async fn delete_note_removes_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Journal")).await.unwrap();
        let file = tmp.path().join("Journal").join("MyNote.md");
        tokio::fs::write(&file, "bye").await.unwrap();

        delete_note(tmp.path(), &VaultPath::new("/journal/mynote.md")).await.unwrap();

        assert!(!file.exists());
    }

    #[tokio::test]
    async fn delete_directory_removes_uppercase_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Archive")).await.unwrap();
        tokio::fs::write(tmp.path().join("Archive").join("note.md"), "x").await.unwrap();

        delete_directory(tmp.path(), &VaultPath::new("/archive")).await.unwrap();

        assert!(!tmp.path().join("Archive").exists());
    }

    #[tokio::test]
    async fn rename_note_finds_uppercase_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects")).await.unwrap();
        tokio::fs::write(tmp.path().join("Projects").join("MyNote.md"), "data").await.unwrap();

        rename_note(
            tmp.path(),
            &VaultPath::new("/projects/mynote.md"),
            &VaultPath::new("/projects/renamed.md"),
        ).await.unwrap();

        assert!(tmp.path().join("Projects").join("renamed.md").exists());
        assert!(!tmp.path().join("Projects").join("MyNote.md").exists());
    }

    #[tokio::test]
    async fn rename_note_into_uppercase_parent() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Inbox")).await.unwrap();
        tokio::fs::write(tmp.path().join("Inbox").join("note.md"), "data").await.unwrap();
        tokio::fs::create_dir(tmp.path().join("Archive")).await.unwrap();

        rename_note(
            tmp.path(),
            &VaultPath::new("/inbox/note.md"),
            &VaultPath::new("/archive/note.md"),
        ).await.unwrap();

        assert!(tmp.path().join("Archive").join("note.md").exists());
        // No duplicate lowercase destination directory should have been created
        assert!(!tmp.path().join("archive").exists());
    }

    #[tokio::test]
    async fn rename_directory_finds_uppercase_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("OldName")).await.unwrap();

        rename_directory(
            tmp.path(),
            &VaultPath::new("/oldname"),
            &VaultPath::new("/newname"),
        ).await.unwrap();

        assert!(tmp.path().join("newname").exists());
        assert!(!tmp.path().join("OldName").exists());
    }

    #[tokio::test]
    async fn vault_entry_from_path_uses_lowercase_vault_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects")).await.unwrap();
        tokio::fs::write(tmp.path().join("Projects").join("MyNote.md"), "# Title").await.unwrap();

        let entry = VaultEntry::from_path(tmp.path(), tmp.path().join("Projects").join("MyNote.md"))
            .await.unwrap();

        // VaultPath is always lowercase even though the disk file has uppercase
        assert_eq!(entry.path.to_string(), "/projects/mynote.md");
        assert!(matches!(entry.data, EntryData::Note(_)));
    }

    #[tokio::test]
    async fn vault_entry_new_finds_uppercase_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir(tmp.path().join("Projects")).await.unwrap();
        tokio::fs::write(tmp.path().join("Projects").join("MyNote.md"), "# Title").await.unwrap();

        let entry = VaultEntry::new(tmp.path(), VaultPath::new("/projects/mynote.md"))
            .await.unwrap();

        assert_eq!(entry.path.to_string(), "/projects/mynote.md");
        assert!(matches!(entry.data, EntryData::Note(_)));
    }
}
