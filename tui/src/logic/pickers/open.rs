//! What Enter on a picker row does.

use kimun_core::nfs::VaultPath;

use crate::logic::files::FileListEntry;

/// What choosing a picker row asks the host to do. Unlike FILES'
/// [`Activation`](crate::logic::files::Activation) a picker never navigates:
/// every existing row (note, directory, attachment, `Up`) is opened by path,
/// and the host's open rules decide what that means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOpen {
    /// Open this path (TUI: `AppMessage::OpenPath`, carrying the browser's
    /// emphasis needles).
    Open(VaultPath),
    /// The `Create: <q>` row for a name no listed note has: ask for the new
    /// note dialog holding that name (`FileOp::ShowNewNote`), where the note
    /// is made on purpose. A name a listed note already has is
    /// [`PickerOpen::Open`] instead
    /// ([`FileListEntry::resolve_create_among`]).
    Create(VaultPath),
}

impl PickerOpen {
    /// The path the row names.
    pub fn path(&self) -> &VaultPath {
        match self {
            Self::Open(path) | Self::Create(path) => path,
        }
    }
}

/// The open rule for a chosen picker row.
pub fn picker_open(entry: &FileListEntry) -> PickerOpen {
    match entry {
        FileListEntry::CreateNote { path, .. } => PickerOpen::Create(path.clone()),
        other => PickerOpen::Open(other.path().clone()),
    }
}
