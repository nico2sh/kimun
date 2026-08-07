use chrono::{DateTime, Utc};
use kimun_core::nfs::filename::{InvalidFilenameError, validate_filename};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum WorkspaceConfigError {
    DuplicateWorkspace {
        name: String,
        existing_path: PathBuf,
    },
    InvalidName {
        name: String,
        error: InvalidFilenameError,
    },
}

impl std::fmt::Display for WorkspaceConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceConfigError::DuplicateWorkspace {
                name,
                existing_path,
            } => {
                write!(
                    f,
                    "Workspace '{}' already exists at {:?}",
                    name, existing_path
                )
            }
            WorkspaceConfigError::InvalidName { error, .. } => {
                write!(f, "Workspace {error}")
            }
        }
    }
}

impl std::error::Error for WorkspaceConfigError {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    pub current_workspace: String,
    /// Whether kimün may contact GitHub to check for a newer release. User-owned
    /// (toggled in onboarding and preferences); defaults on. All machine-managed
    /// update state lives separately in `update_state.toml`, never here.
    #[serde(default = "default_update_check")]
    pub update_check: bool,
    /// Whether kimün captures the mouse for in-app use (divider drag, list
    /// scroll, click-to-focus). When off, the mouse is left to the terminal so
    /// its native selection and middle-click paste work; mouse reporting is
    /// all-or-nothing, so there is no per-button middle ground.
    /// Read only at startup. Defaults on (today's behavior).
    #[serde(default = "default_mouse")]
    pub mouse: bool,
    /// Base URL of the optional RAG server (e.g. `http://localhost:7573`). When
    /// set and reachable, kimün enables semantic search and Q&A.
    /// Global (one server serves many vaults, each as its own collection);
    /// `None` means the feature is off.
    #[serde(default)]
    pub kimun_server_url: Option<String>,
    /// Bearer token for the RAG server, when it requires one.
    #[serde(default)]
    pub kimun_server_token: Option<String>,
}

fn default_update_check() -> bool {
    true
}

fn default_mouse() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceEntry {
    pub path: PathBuf,
    #[serde(default, skip_serializing)]
    pub last_paths: Vec<String>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub quick_note_path: Option<String>,
    #[serde(default)]
    pub inbox_path: Option<String>,
    /// Absolute resolved path for runtime use. Not serialized — `path` is
    /// written to disk as the user configured it (relative, ~/..., or absolute).
    #[serde(skip)]
    pub resolved_path: Option<PathBuf>,
    /// What this workspace's index and history files are named after.
    ///
    /// A short opaque key — twelve hex characters, minted at creation — for
    /// anything a current kimün made, deliberately *not* the workspace's name:
    /// a name is a label the user owns and can change, and letting it reach a
    /// filename is what forced a rename to move an open SQLite database, which
    /// Windows refuses while any handle is on it.
    ///
    /// `None` means "my name", and exists only for configs written before this
    /// field did. Those workspaces keep resolving to `work.kimuncache` exactly
    /// as they always have, so upgrading costs no migration and no reindex;
    /// [`rename_workspace`] pins the name in before it can change.
    ///
    /// [`rename_workspace`]: WorkspaceConfig::rename_workspace
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_key: Option<String>,
}

impl WorkspaceEntry {
    /// What this workspace's files are named after: [`Self::file_key`] once
    /// pinned, otherwise the name it is filed under.
    pub fn file_key_or(&self, name: &str) -> String {
        self.file_key.clone().unwrap_or_else(|| name.to_string())
    }

    /// Returns the resolved absolute path if available, otherwise the original path.
    pub fn effective_path(&self) -> &PathBuf {
        self.resolved_path.as_ref().unwrap_or(&self.path)
    }

    pub fn effective_quick_note_path(&self) -> String {
        self.quick_note_path
            .clone()
            .unwrap_or_else(|| kimun_core::nfs::VaultPath::root().to_string())
    }

    pub fn effective_inbox_path(&self) -> String {
        self.inbox_path
            .clone()
            .unwrap_or_else(|| kimun_core::DEFAULT_INBOX_PATH.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceConfig {
    pub global: GlobalConfig,
    /// Keyed by workspace name. `BTreeMap` (not `HashMap`) so serialization
    /// order is deterministic — otherwise every config save reshuffles the
    /// `[workspaces.*]` sections in the TOML file.
    pub workspaces: BTreeMap<String, WorkspaceEntry>,
}

impl WorkspaceConfig {
    pub fn new_empty() -> Self {
        Self {
            global: GlobalConfig {
                current_workspace: String::new(),
                update_check: true,
                mouse: true,
                kimun_server_url: None,
                kimun_server_token: None,
            },
            workspaces: BTreeMap::new(),
        }
    }

    pub fn add_workspace(
        &mut self,
        name: String,
        path: PathBuf,
    ) -> Result<(), WorkspaceConfigError> {
        if let Err(error) = validate_filename(&name) {
            return Err(WorkspaceConfigError::InvalidName {
                name: name.clone(),
                error,
            });
        }
        if self.workspaces.contains_key(&name) {
            return Err(WorkspaceConfigError::DuplicateWorkspace {
                name: name.clone(),
                existing_path: self.workspaces[&name].path.clone(),
            });
        }

        let created = Utc::now();
        let entry = WorkspaceEntry {
            file_key: Some(self.fresh_file_key(&name, &path, created)),
            path,
            last_paths: Vec::new(),
            created,
            quick_note_path: None,
            inbox_path: None,
            resolved_path: None,
        };

        self.workspaces.insert(name.clone(), entry);

        // Set as current if there is no valid current workspace (first
        // workspace, or the previous current was removed/cleared)
        if !self.workspaces.contains_key(&self.global.current_workspace) {
            self.global.current_workspace = name.clone();
        }

        Ok(())
    }

    /// Every file key currently spoken for.
    fn file_keys_in_use(&self) -> std::collections::HashSet<String> {
        self.workspaces
            .iter()
            .map(|(name, entry)| entry.file_key_or(name))
            .collect()
    }

    /// A short, unused key for a new workspace's index and history files.
    ///
    /// Not the workspace's name. A name is a label the user owns — it can be
    /// renamed, freed and handed to a different workspace — and none of that
    /// should reach a file on disk. Naming files after it made a rename move
    /// an open SQLite database, and then made a reused name collide with the
    /// files of the workspace that had been renamed away from it.
    ///
    /// Derived rather than random, so a config entry always explains its own
    /// filename: SHA-256 over the name, path and creation instant, truncated
    /// to twelve hex characters. The instant is in there so that removing a
    /// workspace and recreating it identically does not land on the previous
    /// key — and so inherit an index whose delete quietly failed.
    ///
    /// Twelve hex characters is 48 bits, far past what a handful of workspaces
    /// needs, but the collision check is here regardless: `salt` bumps until
    /// the key is free, which also covers the case of a key that survives in
    /// the config from an older kimün.
    fn fresh_file_key(&self, name: &str, path: &Path, created: DateTime<Utc>) -> String {
        use sha2::{Digest, Sha256};

        let taken = self.file_keys_in_use();
        for salt in 0u32.. {
            let mut hasher = Sha256::new();
            // Length-delimited, so ("ab", "c") and ("a", "bc") cannot hash the
            // same — a path and a name are both attacker-free here, but the
            // habit costs nothing.
            for part in [
                name.as_bytes(),
                path.to_string_lossy().as_bytes(),
                created.to_rfc3339().as_bytes(),
                &salt.to_le_bytes(),
            ] {
                hasher.update((part.len() as u64).to_le_bytes());
                hasher.update(part);
            }
            let key: String = hasher.finalize()[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            if !taken.contains(&key) {
                return key;
            }
        }
        unreachable!("a 48-bit key space is not exhausted by one config's workspaces")
    }

    /// Re-files the workspace at `old_name` under `new_name`, pinning what its
    /// index and history files are called so neither has to move.
    ///
    /// This is the whole rename: no file on disk is touched. Renaming used to
    /// move `<name>.kimuncache` — a SQLite database — and Windows refuses to
    /// move a file while any handle is on it, which cost a workspace its index
    /// on roughly one run in three. Pinning [`WorkspaceEntry::file_key`] to the
    /// name the files were created under removes the move rather than
    /// retrying it.
    ///
    /// Returns `false` if `old_name` is not there. The caller is responsible
    /// for validating `new_name` and for rejecting a collision — this only
    /// re-files.
    pub fn rename_workspace(&mut self, old_name: &str, new_name: String) -> bool {
        let Some(mut entry) = self.workspaces.remove(old_name) else {
            return false;
        };
        // Only on the first rename: after that the key is already pinned to
        // whatever the files are actually called, and must not drift to the
        // intermediate name.
        entry.file_key = Some(entry.file_key_or(old_name));
        self.workspaces.insert(new_name.clone(), entry);
        if self.global.current_workspace == old_name {
            self.global.current_workspace = new_name;
        }
        true
    }

    pub fn get_current_workspace(&self) -> Option<&WorkspaceEntry> {
        self.workspaces.get(&self.global.current_workspace)
    }

    pub fn get_workspace(&self, name: &str) -> Option<&WorkspaceEntry> {
        self.workspaces.get(name)
    }

    pub fn from_phase1_migration(workspace_dir: PathBuf, last_paths: Vec<String>) -> Self {
        let mut config = Self::new_empty();

        let entry = WorkspaceEntry {
            path: workspace_dir,
            last_paths,
            created: Utc::now(),
            quick_note_path: None,
            inbox_path: None,
            resolved_path: None,
            file_key: None,
        };

        config.workspaces.insert("default".to_string(), entry);
        config.global.current_workspace = "default".to_string();

        config
    }
}

#[cfg(test)]
mod validate_tests {
    use super::*;

    #[test]
    fn add_workspace_rejects_disallowed_chars() {
        let mut wc = WorkspaceConfig::new_empty();
        let err = wc
            .add_workspace("bad/name".to_string(), PathBuf::from("/tmp/x"))
            .unwrap_err();
        match err {
            WorkspaceConfigError::InvalidName { name, .. } => assert_eq!(name, "bad/name"),
            _ => panic!("expected InvalidName"),
        }
    }

    #[test]
    fn add_workspace_rejects_windows_reserved() {
        let mut wc = WorkspaceConfig::new_empty();
        assert!(
            wc.add_workspace("con".to_string(), PathBuf::from("/tmp/x"))
                .is_err()
        );
    }

    #[test]
    fn add_workspace_accepts_simple_names() {
        let mut wc = WorkspaceConfig::new_empty();
        assert!(
            wc.add_workspace("notes".to_string(), PathBuf::from("/tmp/x"))
                .is_ok()
        );
    }

    #[test]
    fn add_workspace_sets_current_when_first() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("notes".to_string(), PathBuf::from("/tmp/x"))
            .unwrap();
        assert_eq!(wc.global.current_workspace, "notes");
    }

    #[test]
    fn add_workspace_keeps_valid_current() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("first".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.add_workspace("second".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();
        assert_eq!(wc.global.current_workspace, "first");
    }

    /// A new workspace's files are named after an opaque key, never after the
    /// workspace — the name is the user's to change, and a filename is not.
    #[test]
    fn a_new_workspace_gets_an_opaque_file_key() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();

        let key = wc.workspaces["work"].file_key_or("work");
        assert_ne!(key, "work", "the name must not reach the filename");
        assert_eq!(key.len(), 12, "twelve hex characters: {key}");
        assert!(
            key.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "must be a plain lowercase hex key, got {key}"
        );
    }

    /// A rename does not change what the files are called, so nothing on disk
    /// has to move and a later lookup still finds the index that exists.
    #[test]
    fn rename_keeps_the_file_key() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        let before = wc.workspaces["work"].file_key_or("work");

        assert!(wc.rename_workspace("work", "job".to_string()));

        assert_eq!(wc.workspaces["job"].file_key_or("job"), before);
        assert_eq!(wc.global.current_workspace, "job");
        assert!(!wc.workspaces.contains_key("work"));
    }

    /// Two renames, still the same files.
    #[test]
    fn renaming_twice_keeps_the_file_key() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("first".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        let before = wc.workspaces["first"].file_key_or("first");

        assert!(wc.rename_workspace("first", "second".to_string()));
        assert!(wc.rename_workspace("second", "third".to_string()));

        assert_eq!(wc.workspaces["third"].file_key_or("third"), before);
    }

    /// A config written before `file_key` existed has none, and must keep
    /// resolving to its name — upgrading kimün must not orphan an index.
    #[test]
    fn a_legacy_entry_without_a_file_key_still_uses_its_name() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.workspaces.get_mut("work").unwrap().file_key = None;

        assert_eq!(wc.workspaces["work"].file_key_or("work"), "work");

        // And renaming pins it before it can drift.
        assert!(wc.rename_workspace("work", "job".to_string()));
        assert_eq!(wc.workspaces["job"].file_key.as_deref(), Some("work"));
    }

    /// Renaming a workspace that is not the current one must not steal the
    /// current pointer.
    #[test]
    fn rename_leaves_an_unrelated_current_alone() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("first".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.add_workspace("second".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();
        assert_eq!(wc.global.current_workspace, "first");

        assert!(wc.rename_workspace("second", "renamed".to_string()));

        assert_eq!(wc.global.current_workspace, "first");
    }

    /// Reusing a name that a rename freed must not hand the newcomer the
    /// renamed workspace's files — two workspaces on one SQLite index would
    /// each reindex over the other's notes.
    #[test]
    fn workspaces_never_share_a_file_key() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.rename_workspace("work", "first".to_string());
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();
        wc.rename_workspace("work", "second".to_string());
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/c"))
            .unwrap();

        let keys: Vec<String> = wc
            .workspaces
            .iter()
            .map(|(name, entry)| entry.file_key_or(name))
            .collect();
        let unique: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len(), "duplicate file keys: {keys:?}");
        assert_eq!(keys.len(), 3);
    }

    /// The collision check covers a legacy name-shaped key too, not just the
    /// hashes it mints itself.
    #[test]
    fn a_fresh_key_avoids_one_already_taken() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        // Force the next mint to land on a taken key by claiming it first.
        let colliding = wc.fresh_file_key("other", Path::new("/tmp/b"), Utc::now());
        wc.workspaces.get_mut("work").unwrap().file_key = Some(colliding.clone());

        let next = wc.fresh_file_key("other", Path::new("/tmp/b"), Utc::now());

        assert_ne!(next, colliding);
    }

    #[test]
    fn renaming_a_missing_workspace_reports_it() {
        let mut wc = WorkspaceConfig::new_empty();
        assert!(!wc.rename_workspace("nope", "other".to_string()));
        assert!(wc.workspaces.is_empty());
    }

    #[test]
    fn add_workspace_repairs_dangling_current() {
        // After clear_workspace the current entry is removed but other
        // workspaces remain; the next add must become current or the
        // app can never activate a workspace again.
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("other".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.global.current_workspace = String::new();
        wc.add_workspace("fresh".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();
        assert_eq!(wc.global.current_workspace, "fresh");
    }
}
