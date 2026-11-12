//! Preferences rules shared by every frontend: the editable draft of each
//! setting the Preferences screen offers ([`PreferencesModel`]), the ranges,
//! steps and cycles those settings move through, the workspace-management
//! operations with their validation, the Save plan ([`SavePlan`]) and the
//! reindex runners a save or the Indexing section starts, plus the CFG drawer
//! view's summary ([`ConfigInfo`]) and the theme picker's list
//! ([`ThemeChoices`]). No rendering, no key handling: a frontend maps its own
//! input onto these calls and decides how to show their results.
//!
//! The workspace operations act on an [`AppSettings`] the caller chooses —
//! the TUI hands them its live shared settings (its Discard restores the
//! snapshot taken on open), a frontend with a draft copy hands them that.

use std::path::PathBuf;
use std::time::Duration;

use kimun_core::error::VaultError;
use kimun_core::nfs::filename::validate_filename;
use kimun_core::system::SystemPath;
use kimun_core::{IndexFile, NoteVault, NotesValidation, VaultConfig};

use crate::keys::action_shortcuts::ActionShortcuts;
use crate::logic::layout_tree::RailPosition;
use crate::settings::config_migration::CURRENT_CONFIG_VERSION;
use crate::settings::history::HistoryFile;
use crate::settings::themes::Theme;
use crate::settings::workspace_config::{WorkspaceConfig, WorkspaceConfigError};
use crate::settings::{AppSettings, EditorBackendSetting, SortFieldSetting, SortOrderSetting};

// ── Ranges and steps ─────────────────────────────────────────────────────────

/// Shortest which-key reveal delay: 0 shows the overlay the instant the
/// leader sequence arms.
pub const LEADER_DELAY_MIN_MS: u64 = 0;
/// Longest which-key reveal delay.
pub const LEADER_DELAY_MAX_MS: u64 = 2000;
/// One step of the which-key delay stepper.
pub const LEADER_DELAY_STEP_MS: u64 = 50;

/// Shortest autosave interval.
pub const AUTOSAVE_MIN_SECS: u64 = 5;
/// Longest autosave interval.
pub const AUTOSAVE_MAX_SECS: u64 = 300;
/// One step of the autosave stepper.
pub const AUTOSAVE_STEP_SECS: u64 = 5;

/// One step from `value` onto the `step` grid, clamped to `min..=max`.
/// A configured value off the grid or outside the range (a hand-edited
/// config) lands on the next grid point in the direction pressed, never the
/// wrong way: up from 7 on a 5-grid is 10, down is 5.
fn step_on_grid(value: u64, up: bool, step: u64, min: u64, max: u64) -> u64 {
    let stepped = if up {
        (value / step + 1) * step
    } else {
        value.div_ceil(step).saturating_sub(1) * step
    };
    stepped.clamp(min, max)
}

/// The which-key delay one step up or down from `ms`, on the
/// [`LEADER_DELAY_STEP_MS`] grid, clamped to
/// [`LEADER_DELAY_MIN_MS`]..=[`LEADER_DELAY_MAX_MS`].
pub fn step_leader_delay(ms: u64, up: bool) -> u64 {
    step_on_grid(
        ms,
        up,
        LEADER_DELAY_STEP_MS,
        LEADER_DELAY_MIN_MS,
        LEADER_DELAY_MAX_MS,
    )
}

/// The autosave interval one step up or down from `secs`, on the
/// [`AUTOSAVE_STEP_SECS`] grid, clamped to
/// [`AUTOSAVE_MIN_SECS`]..=[`AUTOSAVE_MAX_SECS`].
pub fn step_autosave(secs: u64, up: bool) -> u64 {
    step_on_grid(
        secs,
        up,
        AUTOSAVE_STEP_SECS,
        AUTOSAVE_MIN_SECS,
        AUTOSAVE_MAX_SECS,
    )
}

// ── Sorting ──────────────────────────────────────────────────────────────────

/// One of the four sort settings the Sorting section edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortSlot {
    DefaultField,
    DefaultOrder,
    JournalField,
    JournalOrder,
}

impl SortSlot {
    /// Whether the slot holds a sort field (else a sort order).
    pub fn is_field(self) -> bool {
        matches!(self, SortSlot::DefaultField | SortSlot::JournalField)
    }

    /// Every slot, in the order the Sorting section lists them.
    pub const ALL: [SortSlot; 4] = [
        SortSlot::DefaultField,
        SortSlot::DefaultOrder,
        SortSlot::JournalField,
        SortSlot::JournalOrder,
    ];
}

/// The sort fields, in the order their choices are listed. Cycling and the
/// frontends' choice lists both go through this, so a new field is offered
/// and cycled to by adding it here (and to its label).
pub const SORT_FIELD_CHOICES: [SortFieldSetting; 2] =
    [SortFieldSetting::Name, SortFieldSetting::Title];
/// The sort orders, in the order their choices are listed.
pub const SORT_ORDER_CHOICES: [SortOrderSetting; 2] =
    [SortOrderSetting::Ascending, SortOrderSetting::Descending];

/// The element after `current` in `choices`, wrapping (the first when
/// `current` is not listed).
fn next_choice<T: Copy + PartialEq>(choices: &[T], current: T) -> T {
    let ix = choices.iter().position(|c| *c == current);
    choices[ix.map_or(0, |i| (i + 1) % choices.len())]
}

/// The sort field after `f` (wrapping through [`SORT_FIELD_CHOICES`]).
pub fn next_sort_field(f: SortFieldSetting) -> SortFieldSetting {
    next_choice(&SORT_FIELD_CHOICES, f)
}

/// The sort order after `o` (wrapping through [`SORT_ORDER_CHOICES`]).
pub fn next_sort_order(o: SortOrderSetting) -> SortOrderSetting {
    next_choice(&SORT_ORDER_CHOICES, o)
}

/// Display name of a sort field.
pub fn sort_field_label(f: SortFieldSetting) -> &'static str {
    match f {
        SortFieldSetting::Name => "Name",
        SortFieldSetting::Title => "Title",
    }
}

/// Display name of a sort order.
pub fn sort_order_label(o: SortOrderSetting) -> &'static str {
    match o {
        SortOrderSetting::Ascending => "Ascending",
        SortOrderSetting::Descending => "Descending",
    }
}

// ── Editor backends ──────────────────────────────────────────────────────────

/// The editor backends the Editor section cycles through, in order, with
/// their display names. A frontend drops the ones it cannot run with
/// [`PreferencesModel::without_backend`].
pub const BACKEND_CHOICES: [(EditorBackendSetting, &str); 3] = [
    (EditorBackendSetting::Plain, "Plain"),
    (EditorBackendSetting::Vim, "Vim (built-in)"),
    (EditorBackendSetting::Nvim, "Nvim (external)"),
];

/// Display name of an editor backend.
pub fn backend_label(b: EditorBackendSetting) -> &'static str {
    BACKEND_CHOICES
        .iter()
        .find(|(backend, _)| *backend == b)
        .map_or("", |(_, label)| *label)
}

// ── Server ───────────────────────────────────────────────────────────────────

/// Shown as the placeholder of an empty server field. Mirrors the server's
/// built-in bind default (`127.0.0.1:7573` in `kimun-server`'s config).
pub const DEFAULT_SERVER_URL_HINT: &str = "http://localhost:7573";

/// The server URL a field's text stands for: trimmed, `None` when empty
/// (server features off — the `None` of `GlobalConfig::kimun_server_url`).
pub fn normalize_server_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Why a server field's text is not a server address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerUrlError {
    /// It does not start with `http://` or `https://`, names no host after
    /// it, or holds a space.
    NotHttp,
}

impl std::fmt::Display for ServerUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerUrlError::NotHttp => write!(
                f,
                "Enter an address that starts with http:// or https://, or leave it empty."
            ),
        }
    }
}

/// [`normalize_server_url`], refusing text the server client could never
/// connect to: anything but empty or an `http://` / `https://` address with
/// a host. (The TUI's field takes any text; a frontend that validates calls
/// this.)
pub fn check_server_url(raw: &str) -> Result<Option<String>, ServerUrlError> {
    let Some(url) = normalize_server_url(raw) else {
        return Ok(None);
    };
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"));
    match rest {
        Some(rest) if !rest.is_empty() && !rest.starts_with('/') && !url.contains(' ') => {
            Ok(Some(url))
        }
        _ => Err(ServerUrlError::NotHttp),
    }
}

// ── Themes ───────────────────────────────────────────────────────────────────

/// The theme the configured name `configured` resolves to in `themes`, the
/// default theme when it names none of them.
pub fn resolve_theme(themes: &[Theme], configured: &str) -> Theme {
    themes
        .iter()
        .find(|t| t.name == configured)
        .cloned()
        .unwrap_or_default()
}

/// The row of `themes` the configured name `configured` selects: the
/// resolved theme's ([`resolve_theme`]), the first when the list has none.
/// The one rule behind where Preferences and the theme picker start.
pub fn active_theme_index(themes: &[Theme], configured: &str) -> usize {
    let active = resolve_theme(themes, configured);
    themes
        .iter()
        .position(|t| t.name == active.name)
        .unwrap_or(0)
}

/// The theme picker's list (leader `v c`): every theme, resolved once, and
/// the row of the theme active when it opened (the one Esc restores, marked
/// as current).
#[derive(Debug, Clone)]
pub struct ThemeChoices {
    /// Every theme, in presentation order.
    pub themes: Vec<Theme>,
    /// Index into [`ThemeChoices::themes`] of the active theme (0 when the
    /// configured one is missing).
    pub original: usize,
}

impl ThemeChoices {
    /// Resolve the theme list (reads custom themes from disk) and find the
    /// effective theme in it.
    pub fn new(settings: &AppSettings) -> Self {
        let themes = settings.theme_list();
        let original = active_theme_index(&themes, &settings.theme);
        Self { themes, original }
    }

    /// `(index, name, is_current)` for each theme, in order.
    pub fn entries(&self) -> impl Iterator<Item = (usize, &str, bool)> {
        self.themes
            .iter()
            .enumerate()
            .map(|(i, t)| (i, t.name.as_str(), i == self.original))
    }

    /// The picker's rows, one per [entry](Self::entries), for a list engine
    /// that fuzzy-filters them by name.
    pub fn rows(&self) -> Vec<ThemeRow> {
        self.entries()
            .map(|(index, name, current)| ThemeRow {
                index,
                name: name.to_string(),
                current,
            })
            .collect()
    }

    /// The theme a picker previews with the theme of row `selected` (an
    /// index into [`ThemeChoices::themes`]) under its cursor: that theme,
    /// or `None` — the theme the window had when the picker opened — when
    /// nothing is selected (the filter matches no theme) or the selection
    /// is that theme's row. So the window never shows a theme that is not
    /// highlighted, and the opening theme is put back as it was.
    pub fn preview_target(&self, selected: Option<usize>) -> Option<usize> {
        selected.filter(|&index| index != self.original && index < self.themes.len())
    }
}

/// One theme picker row: a theme's name, its index into
/// [`ThemeChoices::themes`], and whether it is the theme active when the
/// picker opened. Filtered by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeRow {
    pub index: usize,
    pub name: String,
    pub current: bool,
}

impl crate::logic::list::ListRow for ThemeRow {
    fn match_text(&self) -> Option<&str> {
        Some(&self.name)
    }
}

/// `s` with only its theme changed to `name`: what the theme picker writes,
/// so a pick never touches another setting.
pub fn with_theme(s: &AppSettings, name: &str) -> AppSettings {
    let mut candidate = s.clone();
    candidate.theme = name.to_string();
    candidate
}

// ── Model ────────────────────────────────────────────────────────────────────

/// The draft of every scalar setting the Preferences screen edits, built
/// from the settings on open, written back by [`PreferencesModel::apply`] (or
/// per section, by the `apply_*` methods). The workspace list is edited
/// through the free workspace functions below instead.
#[derive(Debug, Clone)]
pub struct PreferencesModel {
    /// Every theme the Appearance section lists.
    pub themes: Vec<Theme>,
    /// Highlighted row of [`PreferencesModel::themes`].
    pub theme_idx: usize,
    /// The configured theme name: as found on open (possibly empty or
    /// naming a missing theme), then the name of each selected theme.
    pub theme_name: String,
    pub use_nerd_fonts: bool,
    /// Whether kimün checks GitHub for a newer release on startup.
    pub update_check: bool,
    /// Whether the TUI captures the mouse (read at startup only).
    pub mouse: bool,
    /// `[gui] reduce_motion`: the desktop app keeps its decorative motion
    /// still. Desktop-only: the TUI does not show it and carries it
    /// unchanged; it is written only when it differs from the config's
    /// value, so a save that did not touch it never adds the key.
    pub reduce_motion: bool,
    /// `[gui] editor_font_family`: `None` = the theme's font. Desktop-only,
    /// written only when it differs from the config's value.
    pub editor_font_family: Option<String>,
    /// `[gui] editor_font_size` in pixels: `None` = the theme's size.
    /// Desktop-only, written only when it differs from the config's value.
    pub editor_font_size: Option<u8>,
    /// `[gui] rail`: the window edge the desktop app's Activity rail sits
    /// on (ADR-0055). Desktop-only, carried and written like
    /// `reduce_motion`.
    pub rail: RailPosition,
    /// Which-key reveal delay, see [`step_leader_delay`].
    pub leader_timeout_ms: u64,
    pub default_sort_field: SortFieldSetting,
    pub default_sort_order: SortOrderSetting,
    pub journal_sort_field: SortFieldSetting,
    pub journal_sort_order: SortOrderSetting,
    /// See [`step_autosave`].
    pub autosave_interval_secs: u64,
    pub editor_backend: EditorBackendSetting,
    /// `None` = server features off, see [`normalize_server_url`].
    pub server_url: Option<String>,
    /// The backends the Editor section cycles through.
    backends: Vec<EditorBackendSetting>,
}

impl PreferencesModel {
    /// The draft of `s`. Reads the theme list (custom themes come from disk).
    pub fn new(s: &AppSettings) -> Self {
        let themes = s.theme_list();
        let theme_idx = active_theme_index(&themes, &s.theme);
        Self {
            themes,
            theme_idx,
            theme_name: s.theme.clone(),
            use_nerd_fonts: s.use_nerd_fonts,
            update_check: s.update_check(),
            mouse: s.mouse(),
            reduce_motion: s.gui.reduce_motion(),
            editor_font_family: s.gui.editor_font_family(),
            editor_font_size: s.gui.editor_font_size(),
            rail: s.gui.rail(),
            leader_timeout_ms: s.leader_timeout_ms,
            default_sort_field: s.default_sort_field,
            default_sort_order: s.default_sort_order,
            journal_sort_field: s.journal_sort_field,
            journal_sort_order: s.journal_sort_order,
            autosave_interval_secs: s.autosave_interval_secs,
            editor_backend: s.editor_backend,
            server_url: configured_server_url(s),
            backends: BACKEND_CHOICES.iter().map(|(b, _)| *b).collect(),
        }
    }

    /// The same model without `backend` in the Editor section's cycle (the
    /// desktop app has no nvim engine). A configured `backend` stays in the
    /// draft until another one is chosen. Never drops the last backend.
    pub fn without_backend(mut self, backend: EditorBackendSetting) -> Self {
        if self.backends.len() > 1 {
            self.backends.retain(|b| *b != backend);
        }
        self
    }

    /// The backends the Editor section cycles through, in order.
    pub fn backends(&self) -> &[EditorBackendSetting] {
        &self.backends
    }

    /// The highlighted theme.
    pub fn selected_theme(&self) -> &Theme {
        debug_assert!(!self.themes.is_empty(), "the theme list is never empty");
        &self.themes[self.theme_idx.min(self.themes.len().saturating_sub(1))]
    }

    /// Select theme row `idx` (it becomes the draft's theme); returns the
    /// theme to preview, `None` (and no change) when out of range.
    pub fn select_theme(&mut self, idx: usize) -> Option<&Theme> {
        let t = self.themes.get(idx)?;
        self.theme_idx = idx;
        self.theme_name = t.name.clone();
        Some(t)
    }

    /// Move the theme selection by `delta` rows, wrapping at both ends;
    /// returns the theme to preview.
    pub fn move_theme(&mut self, delta: isize) -> Option<&Theme> {
        let len = self.themes.len() as isize;
        if len == 0 {
            return None;
        }
        let idx = (self.theme_idx as isize + delta).rem_euclid(len) as usize;
        self.select_theme(idx)
    }

    /// Step the which-key delay, see [`step_leader_delay`].
    pub fn step_leader_delay(&mut self, up: bool) {
        self.leader_timeout_ms = step_leader_delay(self.leader_timeout_ms, up);
    }

    /// Step the editor font size by one pixel from the draft (or from
    /// `theme_default` while unset), clamped to the allowed range.
    pub fn step_editor_font_size(&mut self, up: bool, theme_default: u8) {
        let current = i64::from(self.editor_font_size.unwrap_or(theme_default));
        let next = if up { current + 1 } else { current - 1 };
        self.editor_font_size = Some(crate::settings::gui::clamp_editor_font_size(next));
    }

    /// Cycle one sort setting to its next value.
    pub fn cycle_sort(&mut self, slot: SortSlot) {
        match slot {
            SortSlot::DefaultField => {
                self.default_sort_field = next_sort_field(self.default_sort_field)
            }
            SortSlot::DefaultOrder => {
                self.default_sort_order = next_sort_order(self.default_sort_order)
            }
            SortSlot::JournalField => {
                self.journal_sort_field = next_sort_field(self.journal_sort_field)
            }
            SortSlot::JournalOrder => {
                self.journal_sort_order = next_sort_order(self.journal_sort_order)
            }
        }
    }

    /// The labels of every value `slot` can take, in listing order (what a
    /// frontend offers as choices; indexes are [`Self::sort_selected`] and
    /// [`Self::set_sort`]'s).
    pub fn sort_choices(&self, slot: SortSlot) -> Vec<&'static str> {
        if slot.is_field() {
            SORT_FIELD_CHOICES.map(sort_field_label).to_vec()
        } else {
            SORT_ORDER_CHOICES.map(sort_order_label).to_vec()
        }
    }

    /// The index in [`Self::sort_choices`] of `slot`'s current value.
    pub fn sort_selected(&self, slot: SortSlot) -> Option<usize> {
        match slot {
            SortSlot::DefaultField => SORT_FIELD_CHOICES
                .iter()
                .position(|f| *f == self.default_sort_field),
            SortSlot::JournalField => SORT_FIELD_CHOICES
                .iter()
                .position(|f| *f == self.journal_sort_field),
            SortSlot::DefaultOrder => SORT_ORDER_CHOICES
                .iter()
                .position(|o| *o == self.default_sort_order),
            SortSlot::JournalOrder => SORT_ORDER_CHOICES
                .iter()
                .position(|o| *o == self.journal_sort_order),
        }
    }

    /// Set `slot` to choice `ix` of [`Self::sort_choices`]; an index past
    /// the list is ignored. `true` when it was set.
    pub fn set_sort(&mut self, slot: SortSlot, ix: usize) -> bool {
        if slot.is_field() {
            let Some(f) = SORT_FIELD_CHOICES.get(ix).copied() else {
                return false;
            };
            match slot {
                SortSlot::DefaultField => self.default_sort_field = f,
                _ => self.journal_sort_field = f,
            }
        } else {
            let Some(o) = SORT_ORDER_CHOICES.get(ix).copied() else {
                return false;
            };
            match slot {
                SortSlot::DefaultOrder => self.default_sort_order = o,
                _ => self.journal_sort_order = o,
            }
        }
        true
    }

    /// Display value of one sort setting.
    pub fn sort_label(&self, slot: SortSlot) -> &'static str {
        match slot {
            SortSlot::DefaultField => sort_field_label(self.default_sort_field),
            SortSlot::DefaultOrder => sort_order_label(self.default_sort_order),
            SortSlot::JournalField => sort_field_label(self.journal_sort_field),
            SortSlot::JournalOrder => sort_order_label(self.journal_sort_order),
        }
    }

    /// Step the autosave interval, see [`step_autosave`].
    pub fn step_autosave(&mut self, up: bool) {
        self.autosave_interval_secs = step_autosave(self.autosave_interval_secs, up);
    }

    /// Cycle the editor backend through [`PreferencesModel::backends`],
    /// wrapping at both ends. A backend that is not offered (a configured one
    /// this frontend dropped) moves to the first (`forward`) or last one.
    pub fn cycle_backend(&mut self, forward: bool) {
        let len = self.backends.len();
        if len == 0 {
            return;
        }
        let next = match self.backends.iter().position(|b| *b == self.editor_backend) {
            Some(i) if forward => (i + 1) % len,
            Some(i) => (i + len - 1) % len,
            None if forward => 0,
            None => len - 1,
        };
        self.editor_backend = self.backends[next];
    }

    /// Commit the server field's text, see [`normalize_server_url`].
    pub fn set_server_url(&mut self, raw: &str) {
        self.server_url = normalize_server_url(raw);
    }

    /// [`set_server_url`](Self::set_server_url) for a frontend that
    /// validates: the draft only takes an address [`check_server_url`]
    /// accepts. Text that stands for the value the draft already holds is
    /// accepted as is, so a value saved earlier (a hand-edited config, the
    /// TUI's permissive field) never blocks a Save the user did not touch
    /// it for.
    pub fn try_set_server_url(&mut self, raw: &str) -> Result<(), ServerUrlError> {
        if normalize_server_url(raw) == self.server_url {
            return Ok(());
        }
        self.server_url = check_server_url(raw)?;
        Ok(())
    }

    /// Whether the draft differs from `initial` (the settings it was built
    /// from) in any setting [`PreferencesModel::apply`] writes. Workspace
    /// edits are not part of the draft: compare the settings they were made
    /// on for those. The theme counts as changed only when the selected
    /// theme is another than the one `initial`'s name resolves to
    /// ([`resolve_theme`]): with no theme configured (or one that is
    /// missing), picking a theme and going back to the one in effect is
    /// clean.
    pub fn dirty(&self, initial: &AppSettings) -> bool {
        self.theme_changed(initial)
            || self.use_nerd_fonts != initial.use_nerd_fonts
            || self.update_check != initial.update_check()
            || self.mouse != initial.mouse()
            || self.reduce_motion != initial.gui.reduce_motion()
            || self.editor_font_family != initial.gui.editor_font_family()
            || self.editor_font_size != initial.gui.editor_font_size()
            || self.rail != initial.gui.rail()
            || self.leader_timeout_ms != initial.leader_timeout_ms
            || self.default_sort_field != initial.default_sort_field
            || self.default_sort_order != initial.default_sort_order
            || self.journal_sort_field != initial.journal_sort_field
            || self.journal_sort_order != initial.journal_sort_order
            || self.autosave_interval_secs != initial.autosave_interval_secs
            || self.editor_backend != initial.editor_backend
            || self.server_url != configured_server_url(initial)
    }

    /// Whether the draft's theme is another than the one in effect for
    /// `initial`: the names differ and so do the themes they resolve to.
    fn theme_changed(&self, initial: &AppSettings) -> bool {
        // Names only: no theme is cloned for the comparison (this runs on
        // every Preferences render).
        let resolved = |configured: &str| -> std::borrow::Cow<'_, str> {
            match self.themes.iter().find(|t| t.name == configured) {
                Some(t) => std::borrow::Cow::Borrowed(t.name.as_str()),
                None => std::borrow::Cow::Owned(Theme::default().name),
            }
        };
        self.theme_name != initial.theme && resolved(&self.theme_name) != resolved(&initial.theme)
    }

    /// Write the Appearance draft: the theme name, only when it differs.
    /// `true` when it was written (the frontend previews the new theme).
    pub fn apply_appearance(&self, s: &mut AppSettings) -> bool {
        if s.theme == self.theme_name {
            return false;
        }
        s.set_theme(self.theme_name.clone());
        true
    }

    /// Write the Display draft. The update check and mouse capture live in
    /// the global workspace config, created empty when there is none yet;
    /// reduced motion in `[gui]`, only when it changed (the rest of `[gui]`
    /// is kept).
    pub fn apply_display(&self, s: &mut AppSettings) {
        s.use_nerd_fonts = self.use_nerd_fonts;
        if s.gui.reduce_motion() != self.reduce_motion {
            s.gui.set_reduce_motion(self.reduce_motion);
        }
        if s.gui.editor_font_family() != self.editor_font_family {
            s.gui
                .set_editor_font_family(self.editor_font_family.as_deref());
        }
        if s.gui.editor_font_size() != self.editor_font_size {
            s.gui.set_editor_font_size(self.editor_font_size);
        }
        if s.gui.rail() != self.rail {
            s.gui.set_rail(self.rail);
        }
        let global = &mut s
            .workspace_config
            .get_or_insert_with(WorkspaceConfig::new_empty)
            .global;
        global.update_check = self.update_check;
        global.mouse = self.mouse;
        s.leader_timeout_ms = self.leader_timeout_ms;
    }

    /// Write the Sorting draft.
    pub fn apply_sorting(&self, s: &mut AppSettings) {
        s.default_sort_field = self.default_sort_field;
        s.default_sort_order = self.default_sort_order;
        s.journal_sort_field = self.journal_sort_field;
        s.journal_sort_order = self.journal_sort_order;
    }

    /// Write the Editor draft.
    pub fn apply_editor(&self, s: &mut AppSettings) {
        s.autosave_interval_secs = self.autosave_interval_secs;
        s.editor_backend = self.editor_backend;
    }

    /// Write the Server draft into the global workspace config (created
    /// empty when there is none yet).
    pub fn apply_server(&self, s: &mut AppSettings) {
        s.workspace_config
            .get_or_insert_with(WorkspaceConfig::new_empty)
            .global
            .kimun_server_url = self.server_url.clone();
    }

    /// Write the whole draft, section by section in sidebar order
    /// (Appearance, Display, Sorting, Editor, Server). `true` when the theme
    /// changed.
    pub fn apply(&self, s: &mut AppSettings) -> bool {
        let theme_changed = self.apply_appearance(s);
        self.apply_display(s);
        self.apply_sorting(s);
        self.apply_editor(s);
        self.apply_server(s);
        theme_changed
    }
}

/// The server URL `s` is configured with.
pub fn configured_server_url(s: &AppSettings) -> Option<String> {
    s.workspace_config
        .as_ref()
        .and_then(|wc| wc.global.kimun_server_url.clone())
}

// ── Workspaces ───────────────────────────────────────────────────────────────

/// The name a new workspace's field starts with when none exists yet.
pub const FIRST_WORKSPACE_NAME: &str = "default";

/// Whether there is a current workspace to reindex (the Indexing section is
/// inert without one).
pub fn has_vault(s: &AppSettings) -> bool {
    s.workspace_config
        .as_ref()
        .is_some_and(|wc| wc.get_current_workspace().is_some())
}

/// Why a workspace operation was refused.
#[derive(Debug, Clone)]
pub enum WorkspaceError {
    /// The name is empty (after trimming).
    EmptyName,
    /// Another workspace already has this name.
    Duplicate(String),
    /// No workspace has this name.
    NotFound(String),
    /// The current workspace cannot be deleted.
    IsCurrent(String),
    /// The workspace config refused to register it (invalid name, duplicate
    /// after lowercasing).
    Register(WorkspaceConfigError),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceError::EmptyName => write!(f, "Workspace name is empty."),
            WorkspaceError::Duplicate(name) => write!(f, "Workspace '{name}' already exists."),
            WorkspaceError::NotFound(name) => write!(f, "Workspace '{name}' does not exist."),
            WorkspaceError::IsCurrent(name) => {
                write!(f, "Workspace '{name}' is the current workspace.")
            }
            WorkspaceError::Register(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

/// A typed workspace name as the config keys it: trimmed, lowercased. The
/// only place that spelling lives; validation is
/// [`normalize_workspace_name`].
fn workspace_key(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// The one rule every workspace name goes through (add, rename, the name
/// check before a folder is picked): trimmed, not empty, lowercased, valid on
/// every filesystem, not taken by another workspace.
fn normalize_workspace_name(s: &AppSettings, raw: &str) -> Result<String, WorkspaceError> {
    let name = workspace_key(raw);
    if name.is_empty() {
        return Err(WorkspaceError::EmptyName);
    }
    if let Err(error) = validate_filename(&name) {
        return Err(WorkspaceError::Register(
            WorkspaceConfigError::InvalidName { name, error },
        ));
    }
    let exists = s
        .workspace_config
        .as_ref()
        .is_some_and(|wc| wc.workspaces.contains_key(&name));
    if exists {
        return Err(WorkspaceError::Duplicate(name));
    }
    Ok(name)
}

/// Check a typed name for a new workspace before its folder is chosen:
/// trimmed, lowercased, not empty, valid, not taken. Returns the normalized
/// name [`add_workspace`] registers.
pub fn check_new_workspace_name(s: &AppSettings, raw: &str) -> Result<String, WorkspaceError> {
    normalize_workspace_name(s, raw)
}

/// Register workspace `name` (lowercased) at `path`, creating the workspace
/// config when there is none yet. The first workspace becomes the current
/// one.
pub fn add_workspace(s: &mut AppSettings, name: &str, path: PathBuf) -> Result<(), WorkspaceError> {
    let name = normalize_workspace_name(s, name)?;
    let wc = s.workspace_config.get_or_insert_with(|| {
        s.config_version = CURRENT_CONFIG_VERSION;
        WorkspaceConfig::new_empty()
    });
    wc.add_workspace(name, path)
        .map_err(WorkspaceError::Register)
}

/// Rename workspace `old` to the trimmed `raw_new`. `Ok(true)` when renamed,
/// `Ok(false)` when there was nothing to do (empty or unchanged name, no such
/// workspace), `Err(Duplicate)` when another workspace has the name.
///
/// Through [`WorkspaceConfig::rename_workspace`], never by re-keying the map:
/// that also pins what the index and history files are called, and a rename
/// that skips it points the workspace at files that were never created.
pub fn rename_workspace(
    s: &mut AppSettings,
    old: &str,
    raw_new: &str,
) -> Result<bool, WorkspaceError> {
    if raw_new.trim().is_empty() || workspace_key(raw_new) == old {
        return Ok(false);
    }
    if s.workspace_config.is_none() {
        return Ok(false);
    }
    let new_name = normalize_workspace_name(s, raw_new)?;
    Ok(s.workspace_config
        .as_mut()
        .is_some_and(|wc| wc.rename_workspace(old, new_name)))
}

/// Remove workspace `name` from the config, returning its index and history
/// files for [`remove_workspace_artifacts`]. The current workspace cannot be
/// removed.
///
/// The artifacts are read while the entry still exists — both file names
/// come from its `file_key` — and are meant to be deleted after the settings
/// lock is released, since a delete can wait on a handle another process
/// holds.
pub fn take_workspace_for_delete(
    s: &mut AppSettings,
    name: &str,
) -> Result<(IndexFile, HistoryFile), WorkspaceError> {
    let Some(wc) = s.workspace_config.as_ref() else {
        return Err(WorkspaceError::NotFound(name.to_string()));
    };
    if name == wc.global.current_workspace {
        return Err(WorkspaceError::IsCurrent(name.to_string()));
    }
    if !wc.workspaces.contains_key(name) {
        return Err(WorkspaceError::NotFound(name.to_string()));
    }
    let artifacts = s.workspace_artifacts(name);
    if let Some(wc) = s.workspace_config.as_mut() {
        wc.workspaces.remove(name);
    }
    Ok(artifacts)
}

/// Delete a removed workspace's index and history (blocking: run it outside
/// the settings lock, off a UI thread). Returns the files that stayed behind,
/// already logged.
pub fn remove_workspace_artifacts(
    name: &str,
    index: &IndexFile,
    history: &HistoryFile,
) -> Vec<String> {
    let leftovers = crate::settings::delete_artifacts(index, history);
    if !leftovers.is_empty() {
        tracing::warn!(
            "workspace '{name}' removed, but these files stayed behind:\n{}",
            leftovers.join("\n")
        );
    }
    leftovers
}

/// Point workspace `name` at `path`. `true` when that moves the vault (the
/// settings are flagged for reindexing); a name with no entry is a no-op.
pub fn set_workspace_path(s: &mut AppSettings, name: &str, path: PathBuf) -> bool {
    let moved = s
        .workspace_config
        .as_ref()
        .and_then(|wc| wc.workspaces.get(name))
        .is_some_and(|entry| *entry.effective_path() != path);
    s.set_workspace_path(name, path);
    moved
}

/// One workspace change made from a Preferences view that persists each
/// change as it is made (the desktop app): applied first to a copy of the
/// settings, which is written, and only once that write succeeded to the
/// live settings — the same call both times, so the two cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceEdit {
    /// Register a new workspace ([`add_workspace`]).
    Add { name: String, path: PathBuf },
    /// Rename `old` ([`rename_workspace`]); an empty name is refused.
    Rename { old: String, new: String },
    /// Remove a workspace that is not the current one
    /// ([`take_workspace_for_delete`]).
    Delete { name: String },
    /// Point a workspace at another folder ([`set_workspace_path`]).
    SetPath { name: String, path: PathBuf },
}

/// What a [`WorkspaceEdit`] did.
#[derive(Debug, Default)]
pub struct EditOutcome {
    /// Something changed (a rename to the same name, or a path to the same
    /// folder, does not).
    pub changed: bool,
    /// The edit was made on the current workspace (rename, path), or made
    /// the new one current (the first one added).
    pub current: bool,
    /// The workspace's folder changed ([`SetPath`](WorkspaceEdit::SetPath)):
    /// its index must be built again.
    pub moved: bool,
    /// The key an [`Add`](WorkspaceEdit::Add) registered or a
    /// [`Rename`](WorkspaceEdit::Rename) gave, as the config spells it.
    pub name: Option<String>,
    /// A deleted workspace's index and history, to remove outside the lock
    /// ([`remove_workspace_artifacts`]).
    pub artifacts: Option<(IndexFile, HistoryFile)>,
}

impl WorkspaceEdit {
    /// Apply the edit to `s`, with the validation the TUI's own calls have,
    /// plus: a rename to an empty name is refused rather than ignored.
    pub fn apply(&self, s: &mut AppSettings) -> Result<EditOutcome, WorkspaceError> {
        let current = s.current_workspace_name();
        let is_current = |name: &str| current.as_deref() == Some(name);
        match self {
            WorkspaceEdit::Add { name, path } => {
                let had_vault = has_vault(s);
                add_workspace(s, name, path.clone())?;
                let name = workspace_key(name);
                Ok(EditOutcome {
                    changed: true,
                    current: !had_vault
                        && s.current_workspace_name().as_deref() == Some(name.as_str()),
                    name: Some(name),
                    ..EditOutcome::default()
                })
            }
            WorkspaceEdit::Rename { old, new } => {
                if new.trim().is_empty() {
                    return Err(WorkspaceError::EmptyName);
                }
                let renamed = rename_workspace(s, old, new)?;
                Ok(EditOutcome {
                    changed: renamed,
                    current: is_current(old),
                    name: Some(workspace_key(new)),
                    ..EditOutcome::default()
                })
            }
            WorkspaceEdit::Delete { name } => {
                let artifacts = take_workspace_for_delete(s, name)?;
                Ok(EditOutcome {
                    changed: true,
                    artifacts: Some(artifacts),
                    ..EditOutcome::default()
                })
            }
            WorkspaceEdit::SetPath { name, path } => {
                if !workspace_exists(s, name) {
                    return Err(WorkspaceError::NotFound(name.clone()));
                }
                let moved = set_workspace_path(s, name, path.clone());
                Ok(EditOutcome {
                    changed: moved,
                    current: is_current(name),
                    moved,
                    ..EditOutcome::default()
                })
            }
        }
    }
}

/// Whether `name` is a registered workspace in `s`.
pub fn workspace_exists(s: &AppSettings, name: &str) -> bool {
    s.workspace_config
        .as_ref()
        .is_some_and(|wc| wc.workspaces.contains_key(name))
}

/// Make sure the folder a new workspace goes in exists (the first
/// workspace's suggested folder may not yet), through core. Returns it as
/// core resolved it. Blocking: run it off a UI thread.
pub fn prepare_workspace_folder(path: &std::path::Path) -> Result<PathBuf, String> {
    kimun_core::system::create_dir(path)
        .map(SystemPath::into_path_buf)
        .map_err(|e| e.to_string())
}

/// What an index rebuild reports when it finished, e.g. "Indexed in 1.2s".
pub fn indexed_in(duration: Duration) -> String {
    format!("Indexed in {duration:.1?}")
}

// ── Save plan and reindex runners ────────────────────────────────────────────

/// What saving the Preferences takes.
#[derive(Debug, Clone, PartialEq)]
pub enum SavePlan {
    /// The vault moved: rebuild its index on a throwaway vault
    /// ([`reindex_new_vault`]) and save only when that succeeds.
    NeedsReindex {
        workspace: SystemPath,
        cache: Option<IndexFile>,
    },
    /// Same vault: save straight away.
    SaveOnly,
}

/// Why a save cannot go ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The vault changed to none at all.
    NoWorkspace,
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NoWorkspace => write!(f, "No workspace set"),
        }
    }
}

impl std::error::Error for SaveError {}

/// What saving `current` takes when the Preferences opened on `initial`.
pub fn save_plan(current: &AppSettings, initial: &AppSettings) -> Result<SavePlan, SaveError> {
    if current.resolve_workspace_path() == initial.resolve_workspace_path() {
        return Ok(SavePlan::SaveOnly);
    }
    match reindex_target(current) {
        Some((workspace, cache)) => Ok(SavePlan::NeedsReindex { workspace, cache }),
        None => Err(SaveError::NoWorkspace),
    }
}

/// The current vault and its index file — what a reindex runs on. `None`
/// without a current workspace; the index is `None` when the current
/// workspace's name is empty (the vault's legacy default location).
pub fn reindex_target(s: &AppSettings) -> Option<(SystemPath, Option<IndexFile>)> {
    let workspace = s.resolve_workspace_path()?;
    let cache = s
        .workspace_config
        .as_ref()
        .map(|wc| wc.global.current_workspace.clone())
        .filter(|n| !n.is_empty())
        .map(|n| s.index_for(&n));
    Some((workspace, cache))
}

/// How a reindex ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReindexOutcome {
    /// Finished in this long.
    Done(Duration),
    /// The vault is structurally unusable (a case conflict): the message
    /// to show.
    Conflict(String),
    /// Anything else went wrong: the message to show.
    Failed(String),
}

fn vault_config(workspace: SystemPath, cache: Option<IndexFile>) -> VaultConfig {
    let config = VaultConfig::new(workspace);
    match cache {
        Some(index) => config.with_index(index),
        None => config,
    }
}

/// Rebuild the index of the vault at `workspace` from scratch (a Save after
/// the vault moved, and Full Reindex) on a throwaway vault with its own
/// pool. Closing it does not touch the app's vault, and leaving it to drop
/// would keep a second handle on the index file open.
pub async fn reindex_new_vault(workspace: SystemPath, cache: Option<IndexFile>) -> ReindexOutcome {
    let vault = match NoteVault::new(vault_config(workspace, cache)).await {
        Ok(vault) => vault,
        Err(e) => return ReindexOutcome::Failed(e.to_string()),
    };
    let result = vault.recreate_index().await;
    vault.close().await;
    match result {
        Ok(report) => ReindexOutcome::Done(report.duration),
        Err(e @ VaultError::CaseConflict { .. }) => ReindexOutcome::Conflict(e.to_string()),
        Err(e) => ReindexOutcome::Failed(e.to_string()),
    }
}

/// Fast (incremental) reindex of the vault at `workspace` on a throwaway
/// vault, like [`reindex_new_vault`]. `Err` carries the message to show.
pub async fn fast_reindex(
    workspace: SystemPath,
    cache: Option<IndexFile>,
) -> Result<Duration, String> {
    let vault = NoteVault::new(vault_config(workspace, cache))
        .await
        .map_err(|e| e.to_string())?;
    let result = vault.index_notes(NotesValidation::Fast).await;
    vault.close().await;
    result.map_err(|e| e.to_string()).map(|r| r.duration)
}

// ── CFG drawer view ──────────────────────────────────────────────────────────

/// Shown for an action with no chord bound.
pub const UNBOUND: &str = "unbound";
/// Shown as the config path when the settings carry none.
pub const DEFAULT_CONFIG_LOCATION: &str = "default location";

/// What the CFG drawer view displays: a read-only settings summary.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ConfigInfo {
    pub theme_name: String,
    pub leader_key: String,
    pub preferences_key: String,
    pub leader_timeout_ms: u64,
    pub config_path: String,
}

/// The CFG view's summary of `s`. Resolves the theme (reads the theme list).
pub fn config_info(s: &AppSettings) -> ConfigInfo {
    let key_of = |a: &ActionShortcuts| {
        s.key_bindings
            .first_combo_for(a)
            .unwrap_or_else(|| UNBOUND.to_string())
    };
    ConfigInfo {
        theme_name: s.get_theme().name,
        leader_key: key_of(&ActionShortcuts::Leader),
        preferences_key: key_of(&ActionShortcuts::OpenPreferences),
        leader_timeout_ms: s.leader_timeout_ms,
        config_path: config_path_label(s),
    }
}

/// Where `s` is saved, as the CFG view shows it: the config file's path, or
/// [`DEFAULT_CONFIG_LOCATION`] when the settings carry none.
pub fn config_path_label(s: &AppSettings) -> String {
    s.config_file
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| DEFAULT_CONFIG_LOCATION.to_string())
}

#[cfg(test)]
mod tests;
