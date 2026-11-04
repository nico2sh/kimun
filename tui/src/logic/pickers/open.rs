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
    /// The `Create: <q>` row: materialise the note through core
    /// ([`file_ops::create_note`](crate::logic::file_ops::create_note) with no
    /// content = `load_or_create_note`, so an existing note is just loaded),
    /// then announce and open it
    /// ([`NoteOutcome::announce_and_open`](crate::logic::file_ops::NoteOutcome::announce_and_open)).
    /// On failure the picker stays open and shows the error.
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
