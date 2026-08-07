use chrono::{DateTime, Utc};
use kimun_core::nfs::filename::{InvalidFilenameError, validate_filename};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
    /// What this workspace's index and history files are named after, when that
    /// is no longer its name.
    ///
    /// `None` means "my name", which is what every workspace starts as and
    /// what keeps `work.kimuncache` readable in a directory listing. It is
    /// pinned to the old name by [`rename_workspace`] and never changes again,
    /// so a rename becomes a pure config edit.
    ///
    /// That is the point: deriving a *file name* from a *user-editable label*
    /// is what forced a rename to move an open SQLite database, which Windows
    /// refuses to do while any handle is on it — the failure this field exists
    /// to make impossible rather than to retry.
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

        let entry = WorkspaceEntry {
            path,
            last_paths: Vec::new(),
            created: Utc::now(),
            quick_note_path: None,
            inbox_path: None,
            resolved_path: None,
            file_key: self.free_file_key(&name),
        };

        self.workspaces.insert(name.clone(), entry);

        // Set as current if there is no valid current workspace (first
        // workspace, or the previous current was removed/cleared)
        if !self.workspaces.contains_key(&self.global.current_workspace) {
            self.global.current_workspace = name.clone();
        }

        Ok(())
    }

    /// A file key no existing workspace is already using.
    ///
    /// `None` — meaning "my own name" — whenever the name is free, so a fresh
    /// workspace's files stay readable as `work.kimuncache`. It is *not* always
    /// free: renaming `work` to `job` leaves that workspace still using the
    /// `work` files while freeing the name `work` for a new workspace, and
    /// letting the newcomer take it too would point two workspaces at one
    /// SQLite index — each reindexing over the other's notes.
    ///
    /// Terminates: `taken` is finite, so some suffix is always free.
    fn free_file_key(&self, name: &str) -> Option<String> {
        let taken: std::collections::HashSet<String> = self
            .workspaces
            .iter()
            .map(|(key, entry)| entry.file_key_or(key))
            .collect();
        if !taken.contains(name) {
            return None;
        }
        (2..)
            .map(|n| format!("{name}-{n}"))
            .find(|candidate| !taken.contains(candidate))
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

    /// A rename pins what the workspace's files are called, so nothing on disk
    /// has to move — and so a later lookup still finds the index that already
    /// exists rather than naming one that does not.
    #[test]
    fn rename_pins_the_file_key_to_the_original_name() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        // Until renamed, files are named after the workspace — `work.kimuncache`
        // reads better in a directory listing than an opaque key.
        assert_eq!(wc.workspaces["work"].file_key_or("work"), "work");

        assert!(wc.rename_workspace("work", "job".to_string()));

        let entry = &wc.workspaces["job"];
        assert_eq!(entry.file_key.as_deref(), Some("work"));
        assert_eq!(entry.file_key_or("job"), "work");
        assert_eq!(wc.global.current_workspace, "job");
        assert!(!wc.workspaces.contains_key("work"));
    }

    /// The key must not follow an intermediate name: after two renames the
    /// files are still the ones created under the *first* name.
    #[test]
    fn renaming_twice_keeps_the_first_file_key() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("first".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();

        assert!(wc.rename_workspace("first", "second".to_string()));
        assert!(wc.rename_workspace("second", "third".to_string()));

        assert_eq!(wc.workspaces["third"].file_key_or("third"), "first");
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

    /// Renaming frees the old *name* but not the old *files* — the renamed
    /// workspace still uses them. A new workspace claiming that name must get
    /// its own, or two workspaces share one SQLite index and each reindexes
    /// over the other's notes.
    #[test]
    fn a_new_workspace_reusing_a_freed_name_gets_its_own_files() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.rename_workspace("work", "job".to_string());

        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();

        assert_eq!(wc.workspaces["job"].file_key_or("job"), "work");
        assert_eq!(wc.workspaces["work"].file_key_or("work"), "work-2");
    }

    /// The suffix keeps climbing rather than colliding a second time.
    #[test]
    fn repeated_reuse_of_a_name_keeps_finding_a_free_key() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.rename_workspace("work", "first".to_string());
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();
        wc.rename_workspace("work", "second".to_string());
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/c"))
            .unwrap();

        let mut keys: Vec<String> = wc
            .workspaces
            .iter()
            .map(|(name, entry)| entry.file_key_or(name))
            .collect();
        keys.sort();
        assert_eq!(keys, ["work", "work-2", "work-3"]);
    }

    /// An untouched name still gets the readable form — the suffix is only for
    /// genuine collisions, not for every workspace after the first rename.
    #[test]
    fn an_unclaimed_name_keeps_its_own_files() {
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("work".to_string(), PathBuf::from("/tmp/a"))
            .unwrap();
        wc.rename_workspace("work", "job".to_string());
        wc.add_workspace("other".to_string(), PathBuf::from("/tmp/b"))
            .unwrap();

        assert_eq!(wc.workspaces["other"].file_key, None);
        assert_eq!(wc.workspaces["other"].file_key_or("other"), "other");
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
