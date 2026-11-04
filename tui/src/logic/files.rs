//! The FILES listing, toolkit-free: the row data ([`FileListEntry`]), its order
//! ([`entry_order`]), the streamed directory source ([`DirListingSource`]) and
//! [`FilesModel`] — the browsed directory, the open-note marker, the sort and
//! what activating a row means ([`Activation`]). A UI draws the model's
//! [`ListCore`] and performs the activations; the TUI's `SidebarComponent`
//! wraps a `FilesModel` over its `SearchList`, a GUI can use one over a bare
//! `ListCore` (the default). See CONTEXT.md (FILES) and adr/0049.

use std::cmp::Ordering;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use futures::StreamExt;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, NotesValidation, ResultType, SearchResult, VaultBrowseOptionsBuilder};

use crate::logic::list::{
    Emit, Filter, ListCore, ListHolder, ListRow, ListSetup, OrderFn, Polled, RowSource, YankTarget,
};
use crate::logic::message::{AppTx, AppTxExt, redraw_callback};
pub use crate::logic::sort::PropertyValues;
use crate::logic::sort::{
    PropertySort, SortField, SortOrder, SortState, is_blank_property, property_key,
};
use crate::settings::AppSettings;

// ---------------------------------------------------------------------------
// Sort options
// ---------------------------------------------------------------------------

/// The row order for a listing sorted by `field` / `order`. `Up` always comes
/// first, then directories when `group_dirs` is on. A property sort puts the
/// rows with a value (from `values`) first, ordered by it, and everything
/// without one — directories, attachments, notes missing the key or not yet
/// indexed, or every row while `values` is `None` — after them in name order,
/// in both directions. Grouped directories sort among themselves by name.
pub fn entry_order(
    field: SortField,
    order: SortOrder,
    group_dirs: bool,
    values: Option<PropertyValues>,
) -> OrderFn<FileListEntry> {
    let directed = move |ord: Ordering| match order {
        SortOrder::Ascending => ord,
        SortOrder::Descending => ord.reverse(),
    };
    Arc::new(move |a: &FileListEntry, b: &FileListEntry| {
        let rank = |e: &FileListEntry| match e {
            FileListEntry::Up { .. } => 0,
            FileListEntry::Directory { .. } if group_dirs => 1,
            _ => 2,
        };
        rank(a).cmp(&rank(b)).then_with(|| {
            let by_name = || a.sort_key(&field).cmp(&b.sort_key(&field));
            if !matches!(field, SortField::Property(_)) {
                return directed(by_name());
            }
            let value = |e: &FileListEntry| match e {
                FileListEntry::Note { path, .. } => values.as_ref().and_then(|v| v.get(path)),
                _ => None,
            };
            match (value(a), value(b)) {
                (Some(x), Some(y)) => directed(x.cmp(y)).then_with(by_name),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => by_name(),
            }
        })
    })
}

// ---------------------------------------------------------------------------
// FileListEntry
// ---------------------------------------------------------------------------

/// One row of a file listing (FILES, the file finder, search results).
#[derive(Clone, Debug)]
pub enum FileListEntry {
    Up {
        parent: VaultPath,
    },
    Note {
        path: VaultPath,
        title: String,
        filename: String,
        journal_date: Option<String>,
        /// `true` when this is the note currently open in the editor. Drives the
        /// open-note marker (accent glyph). Stamped by [`FilesModel`] after each
        /// load; always `false` from the row source and on non-FILES surfaces.
        is_open: bool,
    },
    Directory {
        path: VaultPath,
        name: String,
    },
    Attachment {
        path: VaultPath,
        filename: String,
        /// `true` when this is the attachment the editor area shows (the
        /// GUI's attachment view keeps the open-note marker on it). The TUI
        /// clears the marker on an attachment, so it never sets this.
        is_open: bool,
    },
    CreateNote {
        filename: String,
        path: VaultPath,
    },
}

impl FileListEntry {
    pub fn from_result(result: SearchResult, journal_date: Option<String>) -> Self {
        let filename = result.path.get_parent_path().1;
        match result.rtype {
            ResultType::Note(data) => Self::Note {
                path: result.path,
                title: Self::display_title(data.title),
                filename,
                journal_date,
                is_open: false,
            },
            ResultType::Directory => Self::Directory {
                path: result.path,
                name: filename,
            },
            ResultType::Attachment => Self::Attachment {
                path: result.path,
                filename,
                is_open: false,
            },
        }
    }

    /// Map a raw note title to its display form, substituting a placeholder
    /// for an empty/whitespace title. Shared by listing construction and the
    /// live title updates so they never diverge.
    pub fn display_title(raw: String) -> String {
        if raw.trim().is_empty() {
            "<no title>".to_string()
        } else {
            raw
        }
    }

    pub fn path(&self) -> &VaultPath {
        match self {
            Self::Up { parent } => parent,
            Self::Note { path, .. } => path,
            Self::Directory { path, .. } => path,
            Self::Attachment { path, .. } => path,
            Self::CreateNote { path, .. } => path,
        }
    }

    /// Sort key for the given field.
    pub(crate) fn sort_key(&self, field: &SortField) -> String {
        match self {
            Self::Up { .. } => String::new(),
            Self::Note {
                title, filename, ..
            } => match field {
                SortField::Title => title.to_lowercase(),
                // A property sort's rows without a value fall back to the
                // file name (see `entry_order`).
                SortField::Name | SortField::Property(_) => filename.to_lowercase(),
            },
            Self::Directory { name, .. } => name.to_lowercase(),
            Self::Attachment { filename, .. } => filename.to_lowercase(),
            Self::CreateNote { filename, .. } => filename.to_lowercase(),
        }
    }

    /// What activating this row does. `Up` and directories navigate, notes
    /// open, attachments open as attachments, `Create: …` creates then opens.
    pub fn activation(&self) -> Activation {
        match self {
            Self::Up { parent } => Activation::Navigate(parent.clone()),
            Self::Directory { path, .. } => Activation::Navigate(path.clone()),
            Self::Note { path, .. } => Activation::Open(path.clone()),
            Self::Attachment { path, .. } => Activation::OpenAttachment(path.clone()),
            Self::CreateNote { path, .. } => Activation::Create(path.clone()),
        }
    }
}

impl ListRow for FileListEntry {
    fn match_text(&self) -> Option<&str> {
        match self {
            Self::Note { filename, .. } | Self::CreateNote { filename, .. } => Some(filename),
            // Directories participate in the fuzzy filter (matched on their
            // name). `Up` stays exempt.
            Self::Directory { name, .. } => Some(name),
            _ => None,
        }
    }

    fn yank_target(&self) -> Option<YankTarget> {
        match self {
            // `Up` navigates to the parent and `CreateNote` names a note that
            // does not exist yet — neither is a path worth putting on the
            // clipboard.
            Self::Up { .. } | Self::CreateNote { .. } => None,
            Self::Note { path, .. }
            | Self::Directory { path, .. }
            | Self::Attachment { path, .. } => Some(YankTarget::path(path.to_string())),
        }
    }
}

/// Format a `NaiveDate` as a human-readable string with day-of-week.
/// Example: "Wednesday, March 17, 2026"
pub fn format_journal_date(date: NaiveDate) -> String {
    date.format("%A, %B %-d, %Y").to_string()
}

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

/// What activating a FILES row asks the host to do. The model decides; the
/// host acts (the TUI routes `Navigate` / `Open` through `AppMessage::open`,
/// so its screens apply their own open rules).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activation {
    /// Show this directory (an `Up` row names the parent).
    Navigate(VaultPath),
    /// Open this note in the editor.
    Open(VaultPath),
    /// Show / hand off this attachment.
    OpenAttachment(VaultPath),
    /// Create this note through core, then open it
    /// ([`spawn_create_and_open`]).
    Create(VaultPath),
}

/// Materialise the note at `path` through core (`load_or_create_note`) on the
/// tokio runtime, then announce it (`FileOp::Created`, only when it is new)
/// and ask for it to be opened (`OpenPath`) on `tx`. The host's half of
/// [`Activation::Create`]. A failure is logged.
pub fn spawn_create_and_open(vault: Arc<NoteVault>, path: VaultPath, tx: AppTx) {
    tokio::spawn(async move {
        match vault.load_or_create_note(&path, None).await {
            Ok((_, created)) => tx.announce_and_open(path, created),
            Err(e) => {
                tracing::warn!("create note failed for {path}: {e}");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// DirListingSource
// ---------------------------------------------------------------------------

/// Streamed `RowSource` over one directory's listing. Pushes an `Up` row first
/// (when not at root) so it is always present, then forwards each entry the
/// moment `browse_vault_stream` yields it — rows show up while a slow drive is
/// still being read. Ordering is the engine's job (`order`, see
/// [`entry_order`]), so nothing is re-delivered at the end. Loads once; a
/// local `Filter::Fuzzy` narrows the set and `leading_row` provides the
/// "Create: …" affordance.
pub struct DirListingSource {
    vault: Arc<NoteVault>,
    dir: VaultPath,
}

impl DirListingSource {
    pub fn new(vault: Arc<NoteVault>, dir: VaultPath) -> Self {
        Self { vault, dir }
    }
}

#[async_trait]
impl RowSource<FileListEntry> for DirListingSource {
    async fn load(&self, _query: &str, emit: Emit<FileListEntry>) {
        // Up row first (if not root) — pushed so it's always present.
        if !self.dir.is_root_or_empty() {
            emit.push(FileListEntry::Up {
                parent: self.dir.get_parent_path().0,
            });
        }

        let options = VaultBrowseOptionsBuilder::new(&self.dir)
            .recursive(false)
            .validation(NotesValidation::Full)
            .build();

        let mut stream = std::pin::pin!(self.vault.browse_vault_stream(options));
        while let Some(item) = stream.next().await {
            let result = match item {
                Ok(result) => result,
                Err(e) => {
                    tracing::warn!("sidebar directory listing failed: {e}");
                    break;
                }
            };
            // `is_like` ignores relative/absolute form: skip the current dir's
            // own "." entry whichever form each side carries.
            if matches!(result.rtype, ResultType::Directory) && result.path.is_like(&self.dir) {
                continue;
            }
            let journal_date = self
                .vault
                .journal_date(&result.path)
                .map(format_journal_date);
            emit.push(FileListEntry::from_result(result, journal_date));
        }
        emit.done();
    }

    fn leading_row(&self, query: &str) -> Option<FileListEntry> {
        if query.is_empty() {
            None
        } else {
            let path = self.dir.append(&VaultPath::note_path_from(query)).flatten();
            Some(FileListEntry::CreateNote {
                filename: path.to_string(),
                path,
            })
        }
    }

    fn reload_on_query(&self) -> bool {
        // Load the directory once; the local fuzzy filter narrows it and
        // `leading_row` keeps the create affordance in sync per keystroke.
        false
    }
}

// ---------------------------------------------------------------------------
// FilesModel
// ---------------------------------------------------------------------------

/// Builds the list a [`FilesModel`] holds for one directory: the source, the
/// filter / order / query setup, and the redraw callback the load fires. A
/// bare [`ListCore`] by default ([`FilesModel::new`]); the TUI builds its
/// `SearchList` ([`FilesModel::with_list`]).
pub type ListFactory<L> = Box<
    dyn Fn(DirListingSource, ListSetup<FileListEntry>, Arc<dyn Fn() + Send + Sync>) -> L
        + Send
        + Sync,
>;

/// The FILES view's state and rules, with no UI in it: the browsed directory,
/// the note open in the editor (the marker), the sort (per-directory
/// defaults, grouping, property values), and the listing itself — a
/// [`ListCore`] of [`FileListEntry`] inside `L` (see [`ListHolder`]).
///
/// The listing loads asynchronously on the tokio runtime ([`Self::navigate`]);
/// the owner calls [`Self::poll`] each frame (or on the redraw the load
/// sends) to apply the rows, the property values and the marker. Row reads,
/// selection and the filter text go through [`Self::core`] /
/// [`Self::core_mut`]; [`Self::activate`] says what the selected row asks for.
pub struct FilesModel<L = ListCore<FileListEntry>> {
    vault: Arc<NoteVault>,
    current_dir: VaultPath,
    /// The note currently open in the editor, if any — drives the open-note
    /// marker. `None` on the Browse screen (it never opens notes). Matched
    /// against `FileListEntry::Note` rows by `is_like`.
    open_note: Option<VaultPath>,
    list: Option<L>,
    make_list: ListFactory<L>,
    default_sort_field: SortField,
    default_sort_order: SortOrder,
    journal_sort_field: SortField,
    journal_sort_order: SortOrder,
    /// Sort field/order of the active listing. Set per `navigate` from the
    /// per-dir defaults (kept across a refresh of the same directory); the
    /// sort dialog changes it via `apply_sort`, which hands the engine a new
    /// order.
    sort: (SortField, SortOrder),
    /// Index values a property sort orders by, fetched per key.
    property_sort: PropertySort,
    /// "Group directories first" for the active listing; see `sort`.
    group_dirs: bool,
}

impl FilesModel {
    /// A model whose listing is a bare [`ListCore`] — for a UI that draws the
    /// rows itself. Sort defaults and grouping come from `settings`.
    pub fn new(vault: Arc<NoteVault>, settings: &AppSettings) -> Self {
        Self::with_list(
            vault,
            settings,
            Box::new(|source, setup, redraw| ListCore::new(Arc::new(source), redraw, setup)),
        )
    }
}

impl<L: ListHolder<FileListEntry>> FilesModel<L> {
    /// A model whose listing `make_list` builds — the TUI wraps the core in
    /// its `SearchList`. Sort defaults and grouping come from `settings`.
    pub fn with_list(
        vault: Arc<NoteVault>,
        settings: &AppSettings,
        make_list: ListFactory<L>,
    ) -> Self {
        let default_sort_field = SortField::from(settings.default_sort_field);
        let default_sort_order = SortOrder::from(settings.default_sort_order);
        Self {
            vault,
            current_dir: VaultPath::root(),
            open_note: None,
            list: None,
            make_list,
            default_sort_field: default_sort_field.clone(),
            default_sort_order,
            journal_sort_field: SortField::from(settings.journal_sort_field),
            journal_sort_order: SortOrder::from(settings.journal_sort_order),
            sort: (default_sort_field, default_sort_order),
            property_sort: PropertySort::default(),
            group_dirs: settings.group_directories,
        }
    }

    pub fn vault(&self) -> &Arc<NoteVault> {
        &self.vault
    }

    /// The directory the listing shows (or will show on the first
    /// [`navigate`](Self::navigate)).
    pub fn current_dir(&self) -> &VaultPath {
        &self.current_dir
    }

    /// Seed the directory before the first `navigate`. Lets a screen open at
    /// a non-root path while keeping `current_dir` the single source of truth
    /// for the browsed directory.
    pub fn set_current_dir(&mut self, dir: VaultPath) {
        self.current_dir = dir;
    }

    /// The note the editor has open, as last told by
    /// [`set_open_note`](Self::set_open_note).
    pub fn open_note(&self) -> Option<&VaultPath> {
        self.open_note.as_ref()
    }

    /// `true` until a directory has been loaded (no listing yet). The editor
    /// uses this to decide whether to issue the first-open navigation.
    pub fn is_empty(&self) -> bool {
        self.list.is_none()
    }

    /// The listing's holder, once a directory was navigated to.
    pub fn list(&self) -> Option<&L> {
        self.list.as_ref()
    }

    pub fn list_mut(&mut self) -> Option<&mut L> {
        self.list.as_mut()
    }

    /// The listing's engine: rows, visible rows, selection, filter text.
    pub fn core(&self) -> Option<&ListCore<FileListEntry>> {
        self.list.as_ref().map(|l| l.core())
    }

    /// Mutable engine access: `set_query` (the filter text, which also
    /// refreshes the `Create: …` row), `select*`. A UI that keeps its own
    /// copy of the filter text (the TUI input) updates it through its holder
    /// instead.
    pub fn core_mut(&mut self) -> Option<&mut ListCore<FileListEntry>> {
        self.list.as_mut().map(|l| l.core_mut())
    }

    /// Sort field/order to apply for `dir` (journal dirs get their own).
    fn sort_for(&self, dir: &VaultPath) -> (SortField, SortOrder) {
        if dir.is_like(self.vault.journal_path()) {
            (self.journal_sort_field.clone(), self.journal_sort_order)
        } else {
            (self.default_sort_field.clone(), self.default_sort_order)
        }
    }

    /// (Re)build the listing for `dir`, replacing any prior one. This is the
    /// single directory-navigation entry point: changing directory = rebuild
    /// the engine with a fresh [`DirListingSource`] for the new dir, sorted by
    /// that directory's default. The load runs on the tokio runtime and sends
    /// `AppMessage::Redraw` on `tx` as rows arrive.
    pub fn navigate(&mut self, dir: VaultPath, tx: &AppTx) {
        self.sort = self.sort_for(&dir);
        self.current_dir = dir;
        self.build_listing(tx);
    }

    /// A fresh engine over `current_dir`, ordered by the active sort. A
    /// property sort uses the values cached for its key and fetches them
    /// again, so edits since the last fetch show up.
    fn build_listing(&mut self, tx: &AppTx) {
        let source = DirListingSource::new(self.vault.clone(), self.current_dir.clone());
        let setup = ListSetup {
            filter: Filter::Fuzzy,
            order: Some(self.order()),
            query: String::new(),
        };
        self.list = Some((self.make_list)(source, setup, redraw_callback(tx.clone())));
        self.fetch_property_values(tx);
    }

    /// Start a fetch of the active property sort's values, if it is one;
    /// any other sort forgets the cached values.
    fn fetch_property_values(&mut self, tx: &AppTx) {
        match property_key(&self.sort.0) {
            Some(key) => self.property_sort.fetch(&self.vault, key, tx),
            None => self.property_sort.clear(),
        }
    }

    /// The engine order for the active sort. A property sort whose values
    /// have not arrived yet orders by name (see [`entry_order`]).
    fn order(&self) -> OrderFn<FileListEntry> {
        let (field, order) = self.sort.clone();
        let values = self.property_sort.values_for_field(&field);
        entry_order(field, order, self.group_dirs, values)
    }

    /// Re-sort the loaded rows by the active sort (no reload). A property
    /// sort still waiting for its values keeps the current order.
    fn reorder(&mut self) {
        if self.property_sort.is_awaiting(&self.sort.0) {
            return;
        }
        let order = self.order();
        if let Some(core) = self.core_mut() {
            core.set_order(Some(order));
        }
    }

    /// Property values fetched for `key` landed: re-sort if they belong to
    /// the active sort. `false` when they are stale and were dropped.
    pub fn on_property_sort_values(&mut self, key: &str, values: PropertyValues) -> bool {
        let accepted = self.property_sort.receive(key, values);
        if accepted {
            self.reorder();
        }
        accepted
    }

    /// Apply what arrived since the last call: streamed rows, property sort
    /// values, then re-stamp the open-note marker (a reload rebuilds rows
    /// without it). Call it each frame, or on every redraw. Returns what the
    /// listing's poll did.
    pub fn poll(&mut self) -> Polled {
        let polled = self.list.as_mut().map(|l| l.poll()).unwrap_or_default();
        if let Some((key, values)) = self.property_sort.poll() {
            self.on_property_sort_values(&key, values);
        }
        self.stamp_open_marker();
        polled
    }

    /// Rebuild the listing only when it is currently showing `dir`, so a
    /// create/rename/delete/move in that directory is reflected without yanking
    /// the user away from an unrelated directory they browsed to. The active
    /// sort is kept (a property sort refetches its values). A no-op
    /// otherwise.
    pub fn refresh_if_showing(&mut self, dir: &VaultPath, tx: &AppTx) {
        if dir.is_like(&self.current_dir) {
            self.build_listing(tx);
        }
    }

    /// Set (or clear) the note the editor currently has open, then re-stamp the
    /// marker on the live rows. The editor calls this on every open and on an
    /// open-note rename.
    pub fn set_open_note(&mut self, path: Option<VaultPath>) {
        self.open_note = path;
        self.stamp_open_marker();
    }

    /// Re-apply `is_open` to the rows so exactly the open note's row is marked
    /// (or the shown attachment's: the GUI passes one to [`Self::set_open_note`]).
    /// Idempotent: a full reload rebuilds rows without the flag, so this runs
    /// again after each load (see [`poll`](Self::poll)).
    fn stamp_open_marker(&mut self) {
        let open = self.open_note.clone();
        if let Some(core) = self.core_mut() {
            core.update_rows(|row| {
                if let FileListEntry::Note { path, is_open, .. }
                | FileListEntry::Attachment { path, is_open, .. } = row
                {
                    let want = open.as_ref().is_some_and(|o| path.is_like(o));
                    if *is_open != want {
                        *is_open = want;
                        return true;
                    }
                }
                false
            });
        }
    }

    /// Update the title of the row whose note path matches `path`, if it is in
    /// the current listing. Called when a note is saved and its title (first
    /// body line) may have changed. Position is left unchanged (no re-sort).
    pub fn update_note_row(&mut self, path: &VaultPath, new_title: &str) {
        if let Some(core) = self.core_mut() {
            core.update_rows(|row| {
                if let FileListEntry::Note {
                    path: row_path,
                    title,
                    ..
                } = row
                    && row_path.is_like(path)
                    && title != new_title
                {
                    *title = new_title.to_string();
                    return true;
                }
                false
            });
        }
    }

    /// Move the row at `from` to `to` (path + filename + journal_date) in
    /// place, for a same-directory note rename. The row is re-sorted into its
    /// new spot under the active order (a Name sort reads the very field this
    /// changes) and keeps the highlight if it had it. `journal_date` is
    /// recomputed so a rename into/out of a `YYYY-MM-DD` name under the
    /// journal directory flips the glyph and the secondary date line correctly.
    pub fn rename_note_row(&mut self, from: &VaultPath, to: &VaultPath) {
        let new_filename = to.get_parent_path().1;
        let new_journal_date = self.vault.journal_date(to).map(format_journal_date);
        if let Some(core) = self.core_mut() {
            core.update_rows(|row| {
                if let FileListEntry::Note {
                    path,
                    filename,
                    journal_date,
                    ..
                } = row
                    && path.is_like(from)
                {
                    *path = to.clone();
                    *filename = new_filename.clone();
                    *journal_date = new_journal_date.clone();
                    return true;
                }
                false
            });
        }
    }

    /// Current sort field/order for the active listing.
    pub fn current_sort(&self) -> (SortField, SortOrder) {
        self.sort.clone()
    }

    /// Current "group directories first" flag.
    pub fn group_dirs(&self) -> bool {
        self.group_dirs
    }

    /// What the sort dialog shows for this listing.
    pub fn sort_state(&self) -> SortState {
        let (field, order) = self.current_sort();
        SortState {
            field,
            order,
            group_dirs: Some(self.group_dirs),
        }
    }

    /// Re-order the rows already loaded by `field` / `order` / `group_dirs` —
    /// no second walk of the directory. A property sort orders by the values
    /// cached for its key; without them the current order stays until they
    /// arrive (see [`apply_sort_state`](Self::apply_sort_state), which also
    /// fetches them).
    pub fn apply_sort(&mut self, field: SortField, order: SortOrder, group_dirs: bool) {
        self.sort = (field, order);
        self.group_dirs = group_dirs;
        self.reorder();
    }

    /// Apply a selection from the sort dialog. A `None` group flag keeps the
    /// current grouping. A property sort on a new key fetches its values from
    /// the index and re-sorts when they land; on the same key (an Order /
    /// Group toggle) it reuses the cached values. One with no key yet is
    /// ignored.
    pub fn apply_sort_state(&mut self, state: &SortState, tx: &AppTx) {
        if is_blank_property(&state.field) {
            return;
        }
        let group_dirs = state.group_dirs.unwrap_or(self.group_dirs);
        self.apply_sort(state.field.clone(), state.order, group_dirs);
        self.property_sort.sync(&self.vault, &state.field, tx);
    }

    /// `true` when the active directory is the journal (so its sort default is
    /// the journal one). Lets the caller persist to the matching settings.
    pub fn is_current_journal(&self) -> bool {
        self.current_dir.is_like(self.vault.journal_path())
    }

    /// Record `field` / `order` as the in-session default for the active
    /// context (journal vs. normal) without re-sorting — "save as default",
    /// for callers that already applied the sort. Without it the cached
    /// per-context defaults that `navigate` reads stay at their
    /// construction-time values until restart. Never called with a property
    /// sort (it can't be a default).
    pub fn remember_default(&mut self, field: SortField, order: SortOrder) {
        if self.is_current_journal() {
            self.journal_sort_field = field;
            self.journal_sort_order = order;
        } else {
            self.default_sort_field = field;
            self.default_sort_order = order;
        }
    }

    /// Number of note rows currently visible (excludes Up / dirs / create).
    pub fn note_count(&self) -> usize {
        match self.core() {
            None => 0,
            Some(core) => core
                .visible_rows()
                .iter()
                .filter(|e| matches!(e, FileListEntry::Note { .. }))
                .count(),
        }
    }

    /// What activating the selected row asks for; `None` with no listing or
    /// no selection. The host performs it.
    pub fn activate(&self) -> Option<Activation> {
        self.core()?.selected_row().map(FileListEntry::activation)
    }

    /// The property-sort cache, for tests that pin when a fetch happens.
    #[cfg(test)]
    pub(crate) fn property_sort(&self) -> &PropertySort {
        &self.property_sort
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn display_title_substitutes_placeholder_for_empty() {
        assert_eq!(
            FileListEntry::display_title("   ".to_string()),
            "<no title>"
        );
        assert_eq!(FileListEntry::display_title("Real".to_string()), "Real");
    }

    #[test]
    fn sort_field_cycle_gates_property() {
        assert_eq!(SortField::Name.cycle(false), SortField::Title);
        assert_eq!(SortField::Title.cycle(false), SortField::Name);
        assert_eq!(
            SortField::Property(String::new()).cycle(false),
            SortField::Name
        );
        let p = SortField::Name.cycle(true).cycle(true);
        assert_eq!(p, SortField::Property(String::new()));
        assert_eq!(p.cycle(true), SortField::Name);
    }

    fn note(name: &str) -> FileListEntry {
        FileListEntry::Note {
            path: VaultPath::note_path_from(name),
            title: name.to_string(),
            filename: format!("{name}.md"),
            journal_date: None,
            is_open: false,
        }
    }

    fn dir(name: &str) -> FileListEntry {
        FileListEntry::Directory {
            path: VaultPath::new(name),
            name: name.to_string(),
        }
    }

    fn names(rows: &[FileListEntry]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                FileListEntry::Up { .. } => "..".to_string(),
                FileListEntry::Note { title, .. } => title.clone(),
                FileListEntry::Directory { name, .. } => format!("{name}/"),
                FileListEntry::Attachment { filename, .. } => filename.clone(),
                FileListEntry::CreateNote { filename, .. } => filename.clone(),
            })
            .collect()
    }

    /// `rows` sorted by a property sort on `values`.
    fn sorted(order: SortOrder, group_dirs: bool, values: Option<PropertyValues>) -> Vec<String> {
        let mut rows = vec![
            note("delta"),
            dir("zdir"),
            note("alpha"),
            FileListEntry::Attachment {
                path: VaultPath::new("pic.png"),
                filename: "pic.png".into(),
                is_open: false,
            },
            note("charlie"),
            FileListEntry::Up {
                parent: VaultPath::root(),
            },
            dir("adir"),
            note("bravo"),
        ];
        let cmp = entry_order(
            SortField::Property("rank".into()),
            order,
            group_dirs,
            values,
        );
        rows.sort_by(|a, b| cmp(a, b));
        names(&rows)
    }

    fn rank_values() -> PropertyValues {
        use kimun_core::PropertySortValue::{Number, Text};
        Arc::new(HashMap::from([
            (VaultPath::note_path_from("delta"), Number(1.0)),
            (VaultPath::note_path_from("bravo"), Number(5.0)),
            (VaultPath::note_path_from("charlie"), Text("x".into())),
        ]))
    }

    #[test]
    fn property_order_ascending_puts_missing_last_in_name_order() {
        assert_eq!(
            sorted(SortOrder::Ascending, false, Some(rank_values())),
            [
                "..", "delta", "bravo", "charlie", "adir/", "alpha", "pic.png", "zdir/"
            ]
        );
    }

    #[test]
    fn property_order_descending_still_puts_missing_last() {
        assert_eq!(
            sorted(SortOrder::Descending, false, Some(rank_values())),
            [
                "..", "charlie", "bravo", "delta", "adir/", "alpha", "pic.png", "zdir/"
            ]
        );
    }

    #[test]
    fn property_order_groups_directories_first_by_name() {
        assert_eq!(
            sorted(SortOrder::Descending, true, Some(rank_values())),
            [
                "..", "adir/", "zdir/", "charlie", "bravo", "delta", "alpha", "pic.png"
            ]
        );
    }

    #[test]
    fn property_order_without_values_is_name_order() {
        assert_eq!(
            sorted(SortOrder::Descending, false, None),
            [
                "..", "adir/", "alpha", "bravo", "charlie", "delta", "pic.png", "zdir/"
            ]
        );
    }

    #[test]
    fn name_order_still_follows_the_direction() {
        let mut rows = vec![note("alpha"), dir("bdir"), note("charlie")];
        let cmp = entry_order(SortField::Name, SortOrder::Descending, false, None);
        rows.sort_by(|a, b| cmp(a, b));
        assert_eq!(names(&rows), ["charlie", "bdir/", "alpha"]);
    }

    #[test]
    fn directory_match_text_is_some_name() {
        let dir = FileListEntry::Directory {
            path: VaultPath::note_path_from("projects"),
            name: "projects".to_string(),
        };
        assert_eq!(ListRow::match_text(&dir), Some("projects"));
    }

    #[test]
    fn up_match_text_is_none() {
        let up = FileListEntry::Up {
            parent: VaultPath::root(),
        };
        assert_eq!(ListRow::match_text(&up), None);
    }

    #[test]
    fn sort_field_setting_roundtrip() {
        use crate::settings::SortFieldSetting;
        assert_eq!(
            SortFieldSetting::from(SortField::Name),
            SortFieldSetting::Name
        );
        assert_eq!(
            SortFieldSetting::from(SortField::Title),
            SortFieldSetting::Title
        );
        assert_eq!(SortField::from(SortFieldSetting::Title), SortField::Title);
    }

    #[test]
    fn sort_order_setting_roundtrip() {
        use crate::settings::SortOrderSetting;
        assert_eq!(
            SortOrderSetting::from(SortOrder::Ascending),
            SortOrderSetting::Ascending
        );
        assert_eq!(
            SortOrderSetting::from(SortOrder::Descending),
            SortOrderSetting::Descending
        );
    }

    // ── FilesModel over a bare ListCore (the GUI's shape) ────────────────

    /// Each row kind asks for its own activation.
    #[test]
    fn each_row_kind_has_its_activation() {
        let p = |s: &str| VaultPath::note_path_from(s);
        let up = FileListEntry::Up {
            parent: VaultPath::new("a"),
        };
        assert_eq!(up.activation(), Activation::Navigate(VaultPath::new("a")));
        assert_eq!(
            dir("sub").activation(),
            Activation::Navigate(VaultPath::new("sub"))
        );
        assert_eq!(note("alpha").activation(), Activation::Open(p("alpha")));
        let att = FileListEntry::Attachment {
            path: VaultPath::new("pic.png"),
            filename: "pic.png".into(),
            is_open: false,
        };
        assert_eq!(
            att.activation(),
            Activation::OpenAttachment(VaultPath::new("pic.png"))
        );
        let create = FileListEntry::CreateNote {
            filename: "x".into(),
            path: p("x"),
        };
        assert_eq!(create.activation(), Activation::Create(p("x")));
    }

    /// A vault with notes `alpha`, `bravo` at the root and `sub/inner` in a
    /// subdirectory, and a model over it.
    async fn model() -> (FilesModel, AppTx) {
        let vault = crate::test_support::temp_vault("files-model").await;
        vault.validate_and_init().await.unwrap();
        for name in ["alpha", "bravo", "sub/inner"] {
            vault
                .create_note(&VaultPath::note_path_from(name), "body")
                .await
                .unwrap();
        }
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        (FilesModel::new(vault, &AppSettings::default()), tx)
    }

    async fn poll_to_idle(model: &mut FilesModel) {
        for _ in 0..200 {
            model.poll();
            if model.core().is_some_and(|c| !c.is_loading()) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        model.poll();
    }

    /// Visible rows in display order; notes by name (their titles are all
    /// "body" here). Attachments (the temp vault's own index file) are left
    /// out.
    fn labels(model: &FilesModel) -> Vec<String> {
        model
            .core()
            .unwrap()
            .visible_rows()
            .into_iter()
            .filter(|r| !matches!(r, FileListEntry::Attachment { .. }))
            .map(|r| match r {
                FileListEntry::Note { path, .. } => path.get_clean_name(),
                other => names(std::slice::from_ref(other)).remove(0),
            })
            .collect()
    }

    fn select(model: &mut FilesModel, pred: impl Fn(&FileListEntry) -> bool) {
        let core = model.core_mut().unwrap();
        let pos = core.visible_rows().iter().position(|r| pred(r)).unwrap();
        core.select(pos);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn activate_follows_the_selected_row() {
        let (mut model, tx) = model().await;
        assert!(model.is_empty());
        assert_eq!(model.activate(), None, "no listing yet");
        model.navigate(VaultPath::root(), &tx);
        poll_to_idle(&mut model).await;
        assert_eq!(model.note_count(), 2);

        select(&mut model, |r| matches!(r, FileListEntry::Note { .. }));
        assert!(
            matches!(model.activate(), Some(Activation::Open(p)) if p.is_like(&VaultPath::note_path_from("alpha")))
        );
        select(&mut model, |r| matches!(r, FileListEntry::Directory { .. }));
        assert!(
            matches!(model.activate(), Some(Activation::Navigate(p)) if p.is_like(&VaultPath::new("sub")))
        );
        // The temp vault's own index file lists as an attachment.
        select(&mut model, |r| {
            matches!(r, FileListEntry::Attachment { .. })
        });
        assert!(matches!(
            model.activate(),
            Some(Activation::OpenAttachment(_))
        ));

        model.navigate(VaultPath::new("sub"), &tx);
        poll_to_idle(&mut model).await;
        assert_eq!(model.current_dir(), &VaultPath::new("sub"));
        assert!(
            matches!(
                model.core().unwrap().selected_row(),
                Some(FileListEntry::Up { .. })
            ),
            "the untouched highlight sits on Up"
        );
        assert!(matches!(model.activate(), Some(Activation::Navigate(p)) if p.is_root_or_empty()));
    }

    /// A reload rebuilds the rows without the marker; the next poll puts it
    /// back on the open note.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_open_note_marker_survives_a_reload() {
        let (mut model, tx) = model().await;
        model.navigate(VaultPath::root(), &tx);
        poll_to_idle(&mut model).await;
        model.set_open_note(Some(VaultPath::note_path_from("alpha")));
        let is_open = |m: &FilesModel, name: &str| {
            m.core().unwrap().rows().iter().any(|r| {
                matches!(r, FileListEntry::Note { path, is_open: true, .. }
                    if path.is_like(&VaultPath::note_path_from(name)))
            })
        };
        assert!(is_open(&model, "alpha"));
        assert!(!is_open(&model, "bravo"));

        model.refresh_if_showing(&VaultPath::root(), &tx);
        poll_to_idle(&mut model).await;
        assert_eq!(model.note_count(), 2, "the reload landed");
        assert!(is_open(&model, "alpha"), "re-stamped after the reload");
        assert_eq!(model.open_note(), Some(&VaultPath::note_path_from("alpha")));

        // Another directory is not refreshed.
        let before = model.core().unwrap().is_loading();
        model.refresh_if_showing(&VaultPath::new("sub"), &tx);
        assert_eq!(model.core().unwrap().is_loading(), before);
    }

    /// The marker may sit on an attachment (the GUI's attachment view): its
    /// row is marked, the notes are not, and a reload keeps it.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_open_marker_may_mark_an_attachment() {
        let (mut model, tx) = model().await;
        let pic = VaultPath::new("pic.png").absolute();
        model.vault().save_attachment(&pic, b"png").await.unwrap();
        model.navigate(VaultPath::root(), &tx);
        poll_to_idle(&mut model).await;
        let marked = |m: &FilesModel| -> Vec<String> {
            m.core()
                .unwrap()
                .rows()
                .iter()
                .filter(|r| {
                    matches!(
                        r,
                        FileListEntry::Note { is_open: true, .. }
                            | FileListEntry::Attachment { is_open: true, .. }
                    )
                })
                .map(|r| r.path().to_string())
                .collect()
        };
        model.set_open_note(Some(VaultPath::note_path_from("alpha")));
        model.set_open_note(Some(pic.clone()));
        assert_eq!(marked(&model), vec![pic.to_string()]);
        model.refresh_if_showing(&VaultPath::root(), &tx);
        poll_to_idle(&mut model).await;
        assert_eq!(marked(&model), vec![pic.to_string()], "after a reload");
        model.set_open_note(None);
        assert!(marked(&model).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn another_directory_is_not_refreshed() {
        let (mut model, tx) = model().await;
        model.navigate(VaultPath::root(), &tx);
        poll_to_idle(&mut model).await;
        let before = model.core().unwrap().is_loading();
        model.refresh_if_showing(&VaultPath::new("sub"), &tx);
        assert_eq!(model.core().unwrap().is_loading(), before);
    }

    /// In a subdirectory the `Up` row survives any filter text, and the
    /// filter text offers a `Create: …` row for a note in that directory.
    #[tokio::test(flavor = "multi_thread")]
    async fn up_is_filter_exempt_and_the_filter_offers_create() {
        let (mut model, tx) = model().await;
        model.navigate(VaultPath::new("sub"), &tx);
        poll_to_idle(&mut model).await;
        assert_eq!(labels(&model), ["..", "inner"]);

        model.core_mut().unwrap().set_query("zzz");
        let expected = VaultPath::new("sub")
            .append(&VaultPath::note_path_from("zzz"))
            .flatten();
        assert_eq!(
            labels(&model),
            [expected.to_string(), "..".to_string()],
            "Create row first, Up kept, `inner` filtered out"
        );
        let Some(FileListEntry::CreateNote { filename, path }) = model.core().unwrap().leading()
        else {
            panic!("expected a Create row");
        };
        assert_eq!(filename, &expected.to_string());
        assert_eq!(path, &expected);
        assert_eq!(model.activate(), Some(Activation::Create(expected.clone())));

        // The host's half: create through core, then announce and open.
        let (tx2, mut rx) = tokio::sync::mpsc::unbounded_channel();
        spawn_create_and_open(model.vault().clone(), expected.clone(), tx2);
        let mut got = Vec::new();
        while got.len() < 2 {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("create announced")
                .expect("channel open");
            got.push(msg);
        }
        use crate::logic::message::{AppMessage, FileOp};
        assert!(matches!(&got[0], AppMessage::FileOp(FileOp::Created(p)) if p == &expected));
        assert!(matches!(&got[1], AppMessage::OpenPath { path, .. } if path == &expected));
    }

    /// A sort change re-orders the loaded rows in place; navigating back
    /// picks up the per-directory default again.
    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_reorders_the_loaded_rows() {
        let (mut model, tx) = model().await;
        model.navigate(VaultPath::root(), &tx);
        poll_to_idle(&mut model).await;
        assert_eq!(labels(&model), ["alpha", "bravo", "sub/"]);

        model.apply_sort(SortField::Name, SortOrder::Descending, true);
        assert!(!model.core().unwrap().is_loading(), "no reload");
        assert_eq!(labels(&model), ["sub/", "bravo", "alpha"]);
        assert_eq!(
            model.sort_state(),
            SortState {
                field: SortField::Name,
                order: SortOrder::Descending,
                group_dirs: Some(true),
            }
        );

        // A blank property sort is ignored.
        model.apply_sort_state(
            &SortState {
                field: SortField::Property(" ".into()),
                order: SortOrder::Ascending,
                group_dirs: None,
            },
            &tx,
        );
        assert_eq!(
            model.current_sort(),
            (SortField::Name, SortOrder::Descending)
        );

        model.navigate(VaultPath::root(), &tx);
        poll_to_idle(&mut model).await;
        assert_eq!(
            model.current_sort(),
            (SortField::Name, SortOrder::Ascending)
        );
    }
}
