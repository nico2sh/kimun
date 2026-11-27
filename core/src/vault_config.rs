//! The configuration a [`NoteVault`](crate::NoteVault) is built from, and the
//! default directories of its journal, inbox, and attachments.

use crate::{IndexFile, SystemPath};

/// Default directory for journal entries, one note per day.
pub const DEFAULT_JOURNAL_PATH: &str = "/journal";
/// Default directory for quick-capture notes (see [`NoteVault::quick_note`](crate::NoteVault::quick_note)).
pub const DEFAULT_INBOX_PATH: &str = "/inbox";
/// Default directory for attachments (see
/// [`NoteVault::default_attachments_path`](crate::NoteVault::default_attachments_path)).
pub const DEFAULT_ASSETS_PATH: &str = "/assets";

/// Configuration passed to [`NoteVault::new`](crate::NoteVault::new).
///
/// `workspace_path` is the OS path to the vault's root directory.
/// `db_path` overrides where the SQLite cache is stored. When `None`,
/// the cache lives at `<workspace_path>/kimun.sqlite` (legacy default).
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// The vault's root directory on this machine.
    pub workspace_path: SystemPath,
    /// Override for the index location. When `None`, the index lives at
    /// `<workspace_path>/kimun.sqlite` (legacy default).
    pub index: Option<IndexFile>,
    /// When `true`, destructive automated edits (overwrite, replace, delete, and
    /// the backlink rewrites of rename/move) copy a note's previous content into
    /// a hidden in-vault backup directory before mutating it. The TUI leaves this
    /// off; the CLI and MCP server turn it on.
    pub backup: bool,
    /// When `true`, automated edits refuse to change an existing note,
    /// directory, or attachment in the sources folder that
    /// `.kimun/wiki.toml` declares. The CLI and MCP server turn it on. The
    /// TUI and GUI leave it off, so the person editing is never bound.
    pub protect_sources: bool,
}

impl VaultConfig {
    /// Builds a config for the vault rooted at `workspace_path`, with the
    /// legacy default cache location and backups disabled.
    ///
    /// Takes a [`SystemPath`] rather than any `Into<PathBuf>` so a path that
    /// the OS cannot use — relative, or carrying a `.` component that Windows
    /// reads literally inside a verbatim path — cannot reach `create_dir_all`
    /// or the SQLite open. Callers holding a raw path resolve it first, and
    /// the failure lands where the path is built, not where it is opened.
    pub fn new(workspace_path: SystemPath) -> Self {
        Self {
            workspace_path,
            index: None,
            backup: false,
            protect_sources: false,
        }
    }

    /// Overrides where the index is stored, instead of the legacy
    /// in-workspace default.
    pub fn with_index(mut self, index: IndexFile) -> Self {
        self.index = Some(index);
        self
    }

    /// Enables or disables backing up note content before destructive edits
    /// (see the [`backup`](Self::backup) field).
    pub fn with_backup(mut self, backup: bool) -> Self {
        self.backup = backup;
        self
    }

    /// Enables or disables source protection (see the
    /// [`protect_sources`](Self::protect_sources) field).
    pub fn with_protected_sources(mut self, protect: bool) -> Self {
        self.protect_sources = protect;
        self
    }
}
