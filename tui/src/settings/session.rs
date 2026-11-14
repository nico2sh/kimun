//! A workspace's session: what the GUI had open — its tabs, the arrangement of
//! editor groups in each, the note in each group, and the active tab and group.
//!
//! The sibling of [`HistoryFile`], shaped the same
//! way: the type owns its own naming, it lives in the app directory beside the
//! history (never in the vault — an arrangement belongs to one machine and
//! screen and must not sync), and removing a workspace goes through it. It is
//! non-critical: a missing, unreadable or half-written file means a fresh
//! start, so [`SessionFile::load`] never fails, and writes are atomic.
//!
//! The records are plain values. They carry [`VaultPath`]s as strings and never
//! build, edit or normalise them: dropping a stale path is the caller's job,
//! with core's help.
//!
//! On disk it is JSON, `<workspace>.session.json`. JSON rather than TOML
//! because the group tree is recursive and mixes leaves with splits, which TOML
//! expresses badly; and the `.session.json` suffix cannot collide with the
//! history's `<workspace>.txt`.

use kimun_core::nfs::VaultPath;
use kimun_core::system::{self, SystemError, SystemPath};
use serde::{Deserialize, Serialize};

use super::history::HistoryFile;

/// Suffix of a workspace's session file, after the workspace name.
const SESSION_FILE_SUFFIX: &str = "session.json";

/// The direction a split lays its children out in. A record-local mirror of the
/// layout model's axis, so the on-disk format does not depend on an internal
/// type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisRecord {
    /// Children side by side.
    Horizontal,
    /// Children stacked.
    Vertical,
}

impl From<crate::logic::layout::Axis> for AxisRecord {
    fn from(axis: crate::logic::layout::Axis) -> Self {
        match axis {
            crate::logic::layout::Axis::Horizontal => Self::Horizontal,
            crate::logic::layout::Axis::Vertical => Self::Vertical,
        }
    }
}

impl From<AxisRecord> for crate::logic::layout::Axis {
    fn from(axis: AxisRecord) -> Self {
        match axis {
            AxisRecord::Horizontal => Self::Horizontal,
            AxisRecord::Vertical => Self::Vertical,
        }
    }
}

/// The arrangement of editor groups inside one tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GroupTreeRecord {
    /// One editor group: the note it shows, or `None` for an empty group.
    Leaf {
        #[serde(with = "note_as_string", default)]
        note: Option<VaultPath>,
    },
    /// Children laid out along `axis` by relative `weights` (one per child).
    Split {
        axis: AxisRecord,
        weights: Vec<f64>,
        children: Vec<GroupTreeRecord>,
    },
}

impl GroupTreeRecord {
    /// How many groups (leaves) the tree holds.
    pub fn group_count(&self) -> usize {
        match self {
            Self::Leaf { .. } => 1,
            Self::Split { children, .. } => children.iter().map(Self::group_count).sum(),
        }
    }

    /// A split needs children, and one weight per child, each finite and
    /// above zero (a share of the split it can be laid out with).
    fn is_well_formed(&self) -> bool {
        match self {
            Self::Leaf { .. } => true,
            Self::Split {
                weights, children, ..
            } => {
                !children.is_empty()
                    && weights.len() == children.len()
                    && weights.iter().all(|w| w.is_finite() && *w > 0.0)
                    && children.iter().all(Self::is_well_formed)
            }
        }
    }
}

/// One tab: its groups and which of them (counting leaves in tree order) is
/// active.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabRecord {
    pub tree: GroupTreeRecord,
    pub active: usize,
}

/// The format version this build writes and reads ([`SessionRecord::version`]).
pub const SESSION_VERSION: u32 = 1;

/// Everything a restore needs: the tabs and which one is shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// The format the record is written in: [`SESSION_VERSION`] for every
    /// record this build makes. A file of another version (a newer build's)
    /// does not load.
    pub version: u32,
    pub tabs: Vec<TabRecord>,
    pub shown: usize,
}

/// Just the version of a stored record, read before the rest so a record
/// from another build is told apart from a broken one.
#[derive(Deserialize)]
struct VersionProbe {
    version: u32,
}

impl SessionRecord {
    /// A record of this build's version.
    pub fn new(tabs: Vec<TabRecord>, shown: usize) -> Self {
        Self {
            version: SESSION_VERSION,
            tabs,
            shown,
        }
    }

    /// Whether the record can be restored without panicking: at least one tab,
    /// `shown` and every `active` in range, no empty split, no weight that is
    /// not a positive number. A record failing
    /// this is treated as unreadable.
    fn is_valid(&self) -> bool {
        self.shown < self.tabs.len()
            && self
                .tabs
                .iter()
                .all(|t| t.tree.is_well_formed() && t.active < t.tree.group_count())
    }
}

/// A workspace's session file on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionFile {
    path: SystemPath,
}

impl SessionFile {
    /// The session for `workspace_name` inside `dir` — the same directory the
    /// [`HistoryFile`] lives in. `workspace_name` must already be a valid
    /// filename.
    pub fn in_dir(dir: &SystemPath, workspace_name: &str) -> Self {
        Self {
            path: dir.join(format!("{workspace_name}.{SESSION_FILE_SUFFIX}")),
        }
    }

    /// The session file that goes with `history`: same directory, same
    /// workspace name. For callers holding only the history, such as the
    /// deletion of a removed workspace's artifacts.
    pub fn beside(history: &HistoryFile) -> Self {
        let path = history.path();
        let dir = path.parent().unwrap_or_else(|| path.clone());
        let name = path
            .as_path()
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self::in_dir(&dir, &name)
    }

    /// The file's path, for callers that need to report it.
    pub fn path(&self) -> &SystemPath {
        &self.path
    }

    /// Whether the file exists.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// The stored session. Missing, unreadable, malformed, of another
    /// version than [`SESSION_VERSION`] or invalid (see [`SessionRecord`])
    /// all yield `None`; failures other than a missing file are logged.
    pub fn load(&self) -> Option<SessionRecord> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!("failed to read session file {}: {}", self.path, e);
                return None;
            }
        };
        match serde_json::from_slice::<VersionProbe>(&bytes) {
            Ok(probe) if probe.version == SESSION_VERSION => {}
            Ok(probe) => {
                tracing::warn!(
                    "ignoring session file {} of version {} (this build reads {})",
                    self.path,
                    probe.version,
                    SESSION_VERSION
                );
                return None;
            }
            Err(e) => {
                tracing::warn!("ignoring unreadable session file {}: {}", self.path, e);
                return None;
            }
        }
        let record: SessionRecord = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("ignoring unreadable session file {}: {}", self.path, e);
                return None;
            }
        };
        if !record.is_valid() {
            tracing::warn!("ignoring inconsistent session file {}", self.path);
            return None;
        }
        Some(record)
    }

    /// Replaces the file's contents with `record`, atomically: a crash
    /// mid-write leaves the previous session rather than a truncated one.
    pub fn save(&self, record: &SessionRecord) -> Result<(), SystemError> {
        let body = serde_json::to_vec_pretty(record).map_err(|e| SystemError::Io {
            action: "serialize",
            path: self.path.to_string(),
            source: std::io::Error::other(e),
        })?;
        system::replace_atomically(self.path.as_path(), &body)
    }

    /// Deletes this file. A missing file is not an error.
    pub fn remove(&self) -> Result<(), SystemError> {
        system::remove_file(self.path.as_path())
    }
}

impl std::fmt::Display for SessionFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path)
    }
}

/// `Option<VaultPath>` as an optional plain string: `VaultPath` itself has no
/// `Deserialize`, and the record must not reinterpret what it stores.
mod note_as_string {
    use super::VaultPath;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(note: &Option<VaultPath>, s: S) -> Result<S::Ok, S::Error> {
        match note {
            Some(p) => s.serialize_some(&p.to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<VaultPath>, D::Error> {
        Ok(Option::<String>::deserialize(d)?.map(VaultPath::new))
    }
}
