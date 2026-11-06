//! Onboarding (guided setup) rules shared by every frontend: the step order,
//! the staged [`Draft`], the model a setup screen drives
//! ([`OnboardingModel`]) and the [`commit`] that applies a finished draft to
//! the settings. No rendering, no key handling: a frontend maps its own input
//! onto these calls and decides how to show their results.

use std::path::{Path, PathBuf};

use crate::settings::config_migration::CURRENT_CONFIG_VERSION;
use crate::settings::themes::Theme;
use crate::settings::workspace_config::{WorkspaceConfig, WorkspaceConfigError};
use crate::settings::{AppSettings, EditorBackendSetting, SharedSettings};

// ── Steps ────────────────────────────────────────────────────────────────────

/// One step of the setup flow, in [`OnbStep::ORDER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnbStep {
    Welcome,
    Workspace,
    NerdFonts,
    Theme,
    Backend,
    Updates,
    Summary,
}

impl OnbStep {
    /// Every step, in the order the flow walks them.
    pub const ORDER: [OnbStep; 7] = [
        OnbStep::Welcome,
        OnbStep::Workspace,
        OnbStep::NerdFonts,
        OnbStep::Theme,
        OnbStep::Backend,
        OnbStep::Updates,
        OnbStep::Summary,
    ];

    /// Zero-based position in [`OnbStep::ORDER`].
    pub fn index(self) -> usize {
        Self::ORDER.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The following step, `None` on the last one.
    pub fn next(self) -> Option<OnbStep> {
        Self::ORDER.get(self.index() + 1).copied()
    }

    /// The preceding step, `None` on the first one.
    pub fn prev(self) -> Option<OnbStep> {
        self.index().checked_sub(1).map(|i| Self::ORDER[i])
    }
}

// ── Draft ────────────────────────────────────────────────────────────────────

/// Staged choices — applied to shared settings only by [`commit`].
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// `Some((name, path))` only on first run; rerun never mutates workspaces.
    pub workspace: Option<(String, PathBuf)>,
    pub use_nerd_fonts: bool,
    pub update_check: bool,
    pub theme_name: String,
    pub editor_backend: EditorBackendSetting,
}

// ── Backends ─────────────────────────────────────────────────────────────────

/// The editor backends the Backend step offers: `(backend, name, description)`.
// Descriptions stay short enough for one TUI dialog row — wrapped continuation
// lines would break out of the marker column.
pub const BACKENDS: [(EditorBackendSetting, &str, &str); 3] = [
    (
        EditorBackendSetting::Plain,
        "plain",
        "simple editing, no modes (the default)",
    ),
    (
        EditorBackendSetting::Vim,
        "vim",
        "built-in vim emulation, no external programs",
    ),
    (
        EditorBackendSetting::Nvim,
        "nvim",
        "your real Neovim embedded; requires nvim",
    ),
];

// ── Free functions ───────────────────────────────────────────────────────────

/// Derive a workspace name from a directory: basename, lowercased. Falls back
/// to "notes" when the basename is empty or invalid.
pub fn suggest_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if kimun_core::nfs::filename::validate_filename(&name).is_ok() && !name.is_empty() {
        name
    } else {
        "notes".to_string()
    }
}

/// `nvim` reachable? Explicit configured path wins; otherwise scan PATH.
///
/// Blocking: it stats the configured path or every `PATH` entry. A GUI caller
/// runs it (or [`OnboardingModel::new`], which calls it) off the UI thread.
pub fn nvim_on_path(configured: Option<&Path>) -> bool {
    if let Some(p) = configured {
        return p.is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    let exe = if cfg!(windows) { "nvim.exe" } else { "nvim" };
    std::env::split_paths(&paths).any(|d| d.join(exe).is_file())
}

// ── Model ────────────────────────────────────────────────────────────────────

/// The state of one run of the setup flow, independent of how it is drawn.
#[derive(Debug, Clone)]
pub struct OnboardingModel {
    /// The step currently shown.
    pub step: OnbStep,
    /// No workspace configured yet: the flow must register one, and leaving
    /// it means quitting the app.
    pub first_run: bool,
    /// The staged choices.
    pub draft: Draft,
    /// Every theme the Theme step lists.
    pub themes: Vec<Theme>,
    /// Selected row of [`OnboardingModel::themes`].
    pub theme_idx: usize,
    /// Selected row of [`OnboardingModel::backends`] (all of [`BACKENDS`]
    /// unless a frontend dropped some with
    /// [`OnboardingModel::without_backend`]). Meaningless while the
    /// configured backend is one of the dropped ones: see
    /// [`OnboardingModel::selected_backend`].
    pub backend_idx: usize,
    /// Whether the nvim backend can be chosen.
    pub nvim_available: bool,
    /// The user explicitly committed a name edit — directory picks stop
    /// overwriting the name with the basename suggestion.
    pub name_edited: bool,
    /// The steps this run walks, in order: [`OnbStep::ORDER`] unless a
    /// frontend dropped some with [`OnboardingModel::without_step`].
    steps: Vec<OnbStep>,
    /// The backends the Backend step offers, in order: [`BACKENDS`] unless a
    /// frontend dropped some with [`OnboardingModel::without_backend`].
    backends: Vec<(EditorBackendSetting, &'static str, &'static str)>,
}

impl OnboardingModel {
    /// Build the model from the current settings: first-run detection, theme
    /// list and selection, draft defaults.
    ///
    /// Blocking (it probes for nvim, see [`nvim_on_path`]); GUI callers build
    /// it off the UI thread.
    pub fn new(s: &AppSettings) -> Self {
        let first_run = s.resolve_workspace_path().is_none();
        let themes = s.theme_list();
        let current_theme_name = s.effective_theme_name();
        let theme_idx = themes
            .iter()
            .position(|t| t.name == current_theme_name)
            .unwrap_or(0);
        let draft = Draft {
            workspace: if first_run {
                AppSettings::default_workspace_suggestion().map(|p| (suggest_name(&p), p))
            } else {
                None
            },
            use_nerd_fonts: s.use_nerd_fonts,
            update_check: s.update_check(),
            // Keep the configured name even when its theme file is missing
            // from theme_list() — the draft must never silently rewrite a
            // setting the user didn't touch (dirty()/commit() would persist
            // the substitution). Same for a configured-but-unavailable nvim
            // backend below: the row renders disabled, the setting survives.
            theme_name: current_theme_name,
            editor_backend: s.editor_backend,
        };
        let backend_idx = BACKENDS
            .iter()
            .position(|(b, _, _)| *b == draft.editor_backend)
            .unwrap_or(0);
        let nvim_available = nvim_on_path(s.nvim_path.as_deref());
        Self {
            step: OnbStep::Welcome,
            first_run,
            draft,
            themes,
            theme_idx,
            backend_idx,
            nvim_available,
            name_edited: false,
            steps: OnbStep::ORDER.to_vec(),
            backends: BACKENDS.to_vec(),
        }
    }

    /// The same model without `backend` on the Backend step (the desktop app
    /// has no nvim engine). A configured `backend` stays in the draft, so a
    /// [`commit`] leaves it as it was unless another row is chosen; until
    /// then no row is selected ([`OnboardingModel::selected_backend`] is
    /// `None`).
    pub fn without_backend(mut self, backend: EditorBackendSetting) -> Self {
        if self.backends.len() <= 1 {
            return self;
        }
        self.backends.retain(|(b, _, _)| *b != backend);
        self.backend_idx = self
            .backends
            .iter()
            .position(|(b, _, _)| *b == self.draft.editor_backend)
            .unwrap_or(0);
        self
    }

    /// The backends the Backend step offers: `(backend, name, description)`.
    pub fn backends(&self) -> &[(EditorBackendSetting, &'static str, &'static str)] {
        &self.backends
    }

    /// The selected row of [`OnboardingModel::backends`]; `None` while the
    /// draft holds a backend that is not offered (a configured one this
    /// frontend dropped).
    pub fn selected_backend(&self) -> Option<usize> {
        self.backends
            .iter()
            .any(|(b, _, _)| *b == self.draft.editor_backend)
            .then_some(self.backend_idx)
    }

    /// Choose backend row `idx`. `false` (no change) when out of range or
    /// it is nvim and nvim is missing.
    pub fn select_backend(&mut self, idx: usize) -> bool {
        let Some((backend, _, _)) = self.backends.get(idx).copied() else {
            return false;
        };
        if backend == EditorBackendSetting::Nvim && !self.nvim_available {
            return false;
        }
        self.backend_idx = idx;
        self.draft.editor_backend = backend;
        true
    }

    /// The same model without `step` in its walk (the desktop app has no Nerd
    /// Fonts step). Dropping the step shown moves to the following one (the
    /// preceding one when it was the last). The draft keeps its value for the
    /// dropped step, so a [`commit`] leaves that setting as it was.
    pub fn without_step(mut self, step: OnbStep) -> Self {
        if self.steps.len() <= 1 {
            return self;
        }
        if self.step == step {
            let i = self.step_index();
            self.step = self
                .steps
                .get(i + 1)
                .or_else(|| i.checked_sub(1).and_then(|p| self.steps.get(p)))
                .copied()
                .unwrap_or(OnbStep::Welcome);
        }
        self.steps.retain(|s| *s != step);
        self
    }

    /// The steps this run walks, in order.
    pub fn steps(&self) -> &[OnbStep] {
        &self.steps
    }

    /// Zero-based position of the current step in [`OnboardingModel::steps`].
    pub fn step_index(&self) -> usize {
        self.steps.iter().position(|s| *s == self.step).unwrap_or(0)
    }

    /// How many steps this run walks.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Advance one step. `false` when already on the last step.
    pub fn go_next(&mut self) -> bool {
        match self.steps.get(self.step_index() + 1).copied() {
            Some(n) => {
                self.step = n;
                true
            }
            None => false,
        }
    }

    /// Go back one step. `false` when already on the first step.
    pub fn go_prev(&mut self) -> bool {
        let prev = self
            .step_index()
            .checked_sub(1)
            .and_then(|i| self.steps.get(i).copied());
        match prev {
            Some(p) => {
                self.step = p;
                true
            }
            None => false,
        }
    }

    /// Whether leaving the current step forward is allowed. A first run must
    /// choose a directory before it gets past the Workspace step.
    pub fn can_advance(&self) -> bool {
        !(self.step == OnbStep::Workspace && self.first_run && self.draft.workspace.is_none())
    }

    /// The draft differs from `settings` in a way [`commit`] would persist.
    pub fn dirty(&self, settings: &AppSettings) -> bool {
        let effective_theme = settings.effective_theme_name();
        settings.use_nerd_fonts != self.draft.use_nerd_fonts
            || settings.editor_backend != self.draft.editor_backend
            || settings.update_check() != self.draft.update_check
            || (!self.draft.theme_name.is_empty() && effective_theme != self.draft.theme_name)
    }

    /// Select theme row `idx`; returns the theme to preview, `None` (and no
    /// change) when out of range.
    pub fn select_theme(&mut self, idx: usize) -> Option<&Theme> {
        let t = self.themes.get(idx)?;
        self.theme_idx = idx;
        self.draft.theme_name = t.name.clone();
        Some(t)
    }

    /// Move the backend selection by `delta` rows, hopping over nvim when it
    /// is unavailable and staying put at the edges. With no row selected
    /// (see [`OnboardingModel::selected_backend`]) any move selects the
    /// first row that can be chosen.
    pub fn move_backend(&mut self, delta: isize) {
        let len = self.backends.len() as isize;
        let (mut idx, delta) = match self.selected_backend() {
            Some(i) => (i as isize, delta),
            None => (-1, 1),
        };
        loop {
            idx += delta;
            if idx < 0 || idx >= len {
                return; // stay at the edges
            }
            let (backend, _, _) = self.backends[idx as usize];
            if backend == EditorBackendSetting::Nvim && !self.nvim_available {
                continue; // hop over the disabled entry
            }
            self.backend_idx = idx as usize;
            self.draft.editor_backend = backend;
            return;
        }
    }

    /// Display name of the selected backend, or of the draft's backend
    /// when it is not offered here (a configured one left as it was).
    pub fn backend_name(&self) -> &'static str {
        match self.selected_backend() {
            Some(i) => self.backends[i].1,
            None => BACKENDS
                .iter()
                .find(|(b, _, _)| *b == self.draft.editor_backend)
                .map_or("", |(_, name, _)| *name),
        }
    }

    /// Commit an edited workspace name: trimmed, lowercased, validated.
    /// `false` (draft untouched) when the name is empty or invalid. A valid
    /// name with no directory chosen yet has nowhere to go and is dropped.
    pub fn set_workspace_name(&mut self, raw: &str) -> bool {
        let name = raw.trim().to_lowercase();
        if name.is_empty() || kimun_core::nfs::filename::validate_filename(&name).is_err() {
            return false;
        }
        if let Some((n, _)) = self.draft.workspace.as_mut() {
            *n = name;
            self.name_edited = true;
        }
        true
    }

    /// Adopt `chosen` as the workspace directory. A name the user explicitly
    /// edited survives re-picking the directory; only un-edited names follow
    /// the basename suggestion.
    pub fn choose_directory(&mut self, chosen: PathBuf) {
        let name = match &self.draft.workspace {
            Some((n, _)) if self.name_edited => n.clone(),
            _ => suggest_name(&chosen),
        };
        self.draft.workspace = Some((name, chosen));
    }

    /// Where a directory browser starts: the parent of the chosen directory,
    /// else the parent of the default suggestion, else the browse root.
    pub fn browse_start(&self) -> Option<PathBuf> {
        self.draft
            .workspace
            .as_ref()
            .and_then(|(_, p)| p.parent().map(|p| p.to_path_buf()))
            .or_else(|| {
                AppSettings::default_workspace_suggestion()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            })
            .or_else(|| {
                kimun_core::system::browse_root()
                    .ok()
                    .map(|p| p.into_path_buf())
            })
    }
}

// ── Commit ───────────────────────────────────────────────────────────────────

/// Why [`commit`] refused. Every variant sends the flow back to
/// [`CommitError::return_step`]; its `Display` is the message to show.
#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    /// A first run reached commit without a workspace in the draft.
    #[error("no workspace configured")]
    NoWorkspace,
    /// The workspace directory could not be created.
    #[error("cannot create {}: {source}", path.display())]
    CreateDir {
        path: PathBuf,
        source: kimun_core::system::SystemError,
    },
    /// The workspace could not be registered (invalid or duplicate name).
    #[error(transparent)]
    Register(#[from] WorkspaceConfigError),
}

impl CommitError {
    /// The step the flow returns to so the user can fix the problem.
    pub fn return_step(&self) -> OnbStep {
        OnbStep::Workspace
    }
}

/// Apply a finished draft: on a first run create and register the workspace
/// (rolling back a directory this call created if registration fails), then
/// apply fonts, backend, update check and theme, and persist. A failed save is
/// logged, not returned — the in-memory settings are already applied.
///
/// Synchronous and blocking (directory creation, config write): GUI callers
/// run it off the UI thread.
pub fn commit(
    settings: &SharedSettings,
    draft: &Draft,
    first_run: bool,
) -> Result<(), CommitError> {
    apply_draft(settings, draft, first_run)?;
    if let Err(e) = settings.read().unwrap().save_to_disk() {
        tracing::error!("failed to save settings after onboarding: {e}");
    }
    Ok(())
}

/// [`commit`] without the save: the directory and the in-memory settings
/// only. For a frontend that persists the settings through its own write
/// path (the GUI's never overwrites a config it could not read).
///
/// Synchronous and blocking (directory creation): run it off the UI thread.
pub fn apply_draft(
    settings: &SharedSettings,
    draft: &Draft,
    first_run: bool,
) -> Result<(), CommitError> {
    // Filesystem work happens before the settings lock is taken — a slow
    // mount must not stall settings readers for its duration.
    let workspace = if first_run {
        let Some((name, path)) = draft.workspace.clone() else {
            return Err(CommitError::NoWorkspace);
        };
        let existed = path.is_dir();
        if let Err(source) = kimun_core::system::create_dir(&path) {
            // `create_dir` can fail *after* creating the directory — it
            // canonicalizes as a second step — so this needs the same
            // rollback as the `add_workspace` failure below. Non-recursive,
            // so a pre-existing or non-empty directory is never touched.
            if !existed {
                kimun_core::system::remove_empty_dir(&path).ok();
            }
            return Err(CommitError::CreateDir { path, source });
        }
        Some((name, path, existed))
    } else {
        None
    };
    let mut s = settings.write().unwrap();
    if let Some((name, path, existed)) = workspace {
        let wc = s
            .workspace_config
            .get_or_insert_with(WorkspaceConfig::new_empty);
        if let Err(e) = wc.add_workspace(name, path.clone()) {
            drop(s);
            // Roll back a directory this run created. Non-recursive, so a
            // pre-existing or non-empty directory is never touched.
            if !existed {
                kimun_core::system::remove_empty_dir(&path).ok();
            }
            return Err(e.into());
        }
        s.config_version = CURRENT_CONFIG_VERSION;
    }
    s.use_nerd_fonts = draft.use_nerd_fonts;
    s.editor_backend = draft.editor_backend;
    s.workspace_config
        .get_or_insert_with(WorkspaceConfig::new_empty)
        .global
        .update_check = draft.update_check;
    s.set_theme(draft.theme_name.clone());
    Ok(())
}

// ── Directory chooser ────────────────────────────────────────────────────────

/// Why a new folder could not be made in the directory chooser.
#[derive(Debug, thiserror::Error)]
pub enum NewFolderError {
    /// Nothing but whitespace was typed.
    #[error("directory name is empty")]
    Empty,
    /// The name breaks the cross-platform file name rules.
    #[error("{0}")]
    Invalid(String),
    /// The filesystem refused.
    #[error(transparent)]
    Create(#[from] kimun_core::system::SystemError),
}

/// The subdirectories directly inside `dir`, sorted by name ignoring case
/// (ties by the exact name). Hidden directories are listed too, as the
/// TUI's directory browser lists them. Files are left out.
///
/// Blocking (a directory read and one stat per entry): run it off the UI
/// thread.
pub fn list_subdirs(
    dir: &kimun_core::system::SystemPath,
) -> Result<Vec<kimun_core::system::SystemPath>, kimun_core::system::SystemError> {
    let mut dirs: Vec<_> = kimun_core::system::read_dir(dir)?
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    let key = |p: &kimun_core::system::SystemPath| {
        let name = p
            .as_path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        (name.to_lowercase(), name)
    };
    dirs.sort_by_cached_key(key);
    Ok(dirs)
}

/// Make the folder `name` inside `dir` (the directory chooser's "New
/// folder"), with the rules of the TUI's browser: trimmed, not empty, a
/// valid file name on every platform. Returns the created directory.
///
/// Blocking: run it off the UI thread.
pub fn create_subdir(
    dir: &kimun_core::system::SystemPath,
    name: &str,
) -> Result<kimun_core::system::SystemPath, NewFolderError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NewFolderError::Empty);
    }
    kimun_core::nfs::filename::validate_filename(name)
        .map_err(|e| NewFolderError::Invalid(e.to_string()))?;
    Ok(kimun_core::system::create_dir(dir.join(name))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn shared_defaults() -> SharedSettings {
        Arc::new(RwLock::new(AppSettings::default()))
    }

    fn shared_with_workspace() -> SharedSettings {
        let mut s = AppSettings::default();
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace(
            "notes".to_string(),
            std::env::temp_dir().join("kimun_onb_ws"),
        )
        .unwrap();
        s.workspace_config = Some(wc);
        Arc::new(RwLock::new(s))
    }

    fn model(shared: &SharedSettings) -> OnboardingModel {
        OnboardingModel::new(&shared.read().unwrap())
    }

    #[test]
    fn step_order_next_prev_within_bounds() {
        assert_eq!(OnbStep::Welcome.index(), 0);
        assert_eq!(OnbStep::Summary.index(), OnbStep::ORDER.len() - 1);
        assert_eq!(OnbStep::Welcome.prev(), None);
        assert_eq!(OnbStep::Welcome.next(), Some(OnbStep::Workspace));
        assert_eq!(OnbStep::Summary.next(), None);
        assert_eq!(OnbStep::Summary.prev(), Some(OnbStep::Updates));
        let mut m = model(&shared_with_workspace());
        assert_eq!(m.step, OnbStep::Welcome);
        assert!(!m.go_prev());
        for _ in 0..6 {
            assert!(m.go_next());
        }
        assert_eq!(m.step, OnbStep::Summary);
        assert!(!m.go_next());
        assert_eq!(m.steps(), OnbStep::ORDER);
        assert_eq!(m.step_count(), 7);
        assert_eq!(m.step_index(), 6);
    }

    #[test]
    fn a_dropped_step_is_skipped_both_ways() {
        let mut m = model(&shared_with_workspace()).without_step(OnbStep::NerdFonts);
        assert_eq!(m.step_count(), 6);
        assert!(!m.steps().contains(&OnbStep::NerdFonts));
        assert_eq!(m.step, OnbStep::Welcome);
        assert!(m.go_next());
        assert_eq!(m.step, OnbStep::Workspace);
        assert_eq!(m.step_index(), 1);
        assert!(m.go_next());
        assert_eq!(
            m.step,
            OnbStep::Theme,
            "next from Workspace skips Nerd Fonts"
        );
        assert_eq!(m.step_index(), 2);
        assert!(m.go_prev());
        assert_eq!(
            m.step,
            OnbStep::Workspace,
            "prev from Theme skips Nerd Fonts"
        );
        let mut walked = vec![OnbStep::Welcome];
        m.step = OnbStep::Welcome;
        while m.go_next() {
            walked.push(m.step);
        }
        assert_eq!(walked, m.steps());
        assert_eq!(m.step, OnbStep::Summary);
        assert_eq!(m.step_index(), 5);
        // The draft keeps the dropped step's setting as configured.
        assert_eq!(
            m.draft.use_nerd_fonts,
            AppSettings::default().use_nerd_fonts
        );
    }

    #[test]
    fn dropping_the_shown_step_moves_on() {
        let mut m = model(&shared_with_workspace());
        m.step = OnbStep::NerdFonts;
        let m = m.without_step(OnbStep::NerdFonts);
        assert_eq!(m.step, OnbStep::Theme);
        let mut m = model(&shared_with_workspace());
        m.step = OnbStep::Summary;
        let m = m.without_step(OnbStep::Summary);
        assert_eq!(m.step, OnbStep::Updates);
        assert_eq!(m.step_count(), 6);
    }

    #[test]
    fn first_run_detected_from_missing_workspace() {
        assert!(model(&shared_defaults()).first_run);
        assert!(!model(&shared_with_workspace()).first_run);
    }

    #[test]
    fn first_run_workspace_step_prefills_suggestion() {
        let m = model(&shared_defaults());
        let (name, path) = m.draft.workspace.clone().expect("suggestion expected");
        assert!(path.ends_with("kimun-notes"));
        assert_eq!(name, "kimun-notes");
    }

    #[test]
    fn rerun_draft_has_no_workspace() {
        assert!(model(&shared_with_workspace()).draft.workspace.is_none());
    }

    #[test]
    fn first_run_cannot_advance_past_workspace_without_directory() {
        let mut m = model(&shared_defaults());
        m.step = OnbStep::Workspace;
        assert!(m.can_advance());
        m.draft.workspace = None;
        assert!(!m.can_advance());
        m.step = OnbStep::Welcome;
        assert!(m.can_advance(), "rule applies to the Workspace step only");
        let mut r = model(&shared_with_workspace());
        r.step = OnbStep::Workspace;
        assert!(r.can_advance(), "rerun never needs a directory");
    }

    #[test]
    fn suggest_name_lowercases_and_falls_back() {
        assert_eq!(suggest_name(Path::new("My-Vault")), "my-vault");
        assert_eq!(suggest_name(Path::new("")), "notes");
        assert_eq!(suggest_name(Path::new("con")), "notes");
    }

    #[test]
    fn nvim_configured_path_wins() {
        assert!(!nvim_on_path(Some(Path::new("/nonexistent/nvim-binary"))));
    }

    #[test]
    fn missing_theme_keeps_configured_name_and_stays_clean() {
        let shared = shared_with_workspace();
        shared.write().unwrap().theme = "ghost-theme".to_string();
        let m = model(&shared);
        assert_eq!(
            m.draft.theme_name, "ghost-theme",
            "draft must not substitute a fallback for a missing theme"
        );
        assert!(
            !m.dirty(&shared.read().unwrap()),
            "untouched model must not report changes"
        );
    }

    #[test]
    fn unavailable_nvim_keeps_configured_backend_and_stays_clean() {
        let shared = shared_with_workspace();
        {
            let mut s = shared.write().unwrap();
            s.editor_backend = EditorBackendSetting::Nvim;
            s.nvim_path = Some(PathBuf::from("/nonexistent/nvim-binary"));
        }
        let m = model(&shared);
        assert!(!m.nvim_available);
        assert_eq!(
            m.draft.editor_backend,
            EditorBackendSetting::Nvim,
            "constructor must not rewrite the configured backend"
        );
        assert!(
            !m.dirty(&shared.read().unwrap()),
            "untouched model must not report changes"
        );
    }

    #[test]
    fn dirty_tracks_each_committed_field() {
        let shared = shared_with_workspace();
        let s = shared.read().unwrap();
        let mut m = OnboardingModel::new(&s);
        assert!(!m.dirty(&s));
        m.draft.use_nerd_fonts = !m.draft.use_nerd_fonts;
        assert!(m.dirty(&s));
        let mut m = OnboardingModel::new(&s);
        m.draft.update_check = !m.draft.update_check;
        assert!(m.dirty(&s));
        let mut m = OnboardingModel::new(&s);
        m.draft.editor_backend = EditorBackendSetting::Vim;
        assert!(m.dirty(&s));
    }

    #[test]
    fn theme_selection_updates_draft() {
        let mut m = model(&shared_with_workspace());
        assert!(m.themes.len() >= 2, "need at least two builtin themes");
        let second = m.select_theme(1).map(|t| t.name.clone());
        assert_eq!(second.as_deref(), Some(m.draft.theme_name.as_str()));
        assert_eq!(m.theme_idx, 1);
        let len = m.themes.len();
        assert!(m.select_theme(len).is_none());
        assert_eq!(m.theme_idx, 1, "out of range leaves the selection");
    }

    #[test]
    fn backend_selection_skips_unavailable_nvim() {
        let mut m = model(&shared_with_workspace());
        m.nvim_available = false;
        m.backend_idx = 1; // vim
        m.draft.editor_backend = EditorBackendSetting::Vim;
        m.move_backend(1);
        assert_eq!(m.draft.editor_backend, EditorBackendSetting::Vim);
        assert_eq!(m.backend_name(), "vim");
        m.nvim_available = true;
        m.move_backend(1);
        assert_eq!(m.draft.editor_backend, EditorBackendSetting::Nvim);
        m.move_backend(1);
        assert_eq!(m.backend_idx, 2, "stays at the bottom edge");
        m.backend_idx = 0;
        m.move_backend(-1);
        assert_eq!(m.backend_idx, 0, "stays at the top edge");
        assert_eq!(m.backends(), BACKENDS, "the full model offers all three");
        m.nvim_available = false;
        assert!(!m.select_backend(2), "missing nvim cannot be chosen");
        assert!(!m.select_backend(3));
        assert!(m.select_backend(1));
        assert_eq!(m.draft.editor_backend, EditorBackendSetting::Vim);
    }

    fn shared_with_backend(backend: EditorBackendSetting) -> SharedSettings {
        let shared = shared_with_workspace();
        shared.write().unwrap().editor_backend = backend;
        shared
    }

    #[test]
    fn a_dropped_backend_is_not_offered() {
        let shared = shared_with_backend(EditorBackendSetting::Vim);
        let mut m = model(&shared).without_backend(EditorBackendSetting::Nvim);
        m.nvim_available = true;
        let names: Vec<_> = m.backends().iter().map(|(_, n, _)| *n).collect();
        assert_eq!(names, ["plain", "vim"]);
        assert_eq!(m.selected_backend(), Some(1));
        assert_eq!(m.backend_name(), "vim");
        m.move_backend(1);
        assert_eq!(m.backend_idx, 1, "stays at the bottom edge: no nvim row");
        assert_eq!(m.draft.editor_backend, EditorBackendSetting::Vim);
        assert!(!m.select_backend(2));
        m.move_backend(-1);
        assert_eq!(m.draft.editor_backend, EditorBackendSetting::Plain);
        assert!(m.dirty(&shared.read().unwrap()));
        // The TUI's model keeps all three.
        assert_eq!(model(&shared).backends().len(), 3);
    }

    #[test]
    fn a_configured_dropped_backend_survives_untouched() {
        let shared = shared_with_backend(EditorBackendSetting::Nvim);
        let mut m = model(&shared).without_backend(EditorBackendSetting::Nvim);
        assert_eq!(m.selected_backend(), None, "no row stands for nvim");
        assert_eq!(m.draft.editor_backend, EditorBackendSetting::Nvim);
        assert_eq!(m.backend_name(), "nvim");
        assert!(!m.dirty(&shared.read().unwrap()));
        apply_draft(&shared, &m.draft.clone(), false).unwrap();
        assert_eq!(
            shared.read().unwrap().editor_backend,
            EditorBackendSetting::Nvim
        );
        // A move picks the first row, from either direction.
        m.move_backend(-1);
        assert_eq!(m.selected_backend(), Some(0));
        assert_eq!(m.draft.editor_backend, EditorBackendSetting::Plain);
        let mut m = model(&shared).without_backend(EditorBackendSetting::Nvim);
        m.move_backend(1);
        assert_eq!(m.draft.editor_backend, EditorBackendSetting::Plain);
    }

    #[test]
    fn workspace_name_validates_and_lowercases() {
        let mut m = model(&shared_defaults());
        assert!(!m.set_workspace_name("  "));
        assert!(!m.set_workspace_name("bad?"));
        assert!(!m.name_edited);
        assert!(m.set_workspace_name(" My-Notes "));
        assert!(m.name_edited);
        assert_eq!(m.draft.workspace.as_ref().unwrap().0, "my-notes");
    }

    #[test]
    fn choose_directory_suggests_name_unless_edited() {
        let base = std::env::temp_dir();
        let mut m = model(&shared_defaults());
        m.choose_directory(base.join("My-Vault"));
        assert_eq!(m.draft.workspace.as_ref().unwrap().0, "my-vault");
        assert!(m.set_workspace_name("kept"));
        m.choose_directory(base.join("Other"));
        let (name, path) = m.draft.workspace.clone().unwrap();
        assert_eq!(name, "kept");
        assert_eq!(path, base.join("Other"));
    }

    #[test]
    fn browse_start_is_parent_of_chosen_directory() {
        let base = std::env::temp_dir();
        let mut m = model(&shared_defaults());
        m.choose_directory(base.join("child"));
        assert_eq!(m.browse_start(), Some(base));
    }

    #[test]
    fn commit_first_run_creates_dir_and_applies_draft() {
        let tmp = std::env::temp_dir().join(format!("kimun_onb_lfin_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        let cfg = std::env::temp_dir().join(format!("kimun_onb_lcfg_{}.toml", std::process::id()));
        let settings = shared_defaults();
        settings.write().unwrap().config_file = Some(cfg.clone());
        let mut m = model(&settings);
        m.draft.workspace = Some(("myws".to_string(), tmp.clone()));
        m.draft.use_nerd_fonts = true;
        m.draft.editor_backend = EditorBackendSetting::Vim;
        commit(&settings, &m.draft, m.first_run).unwrap();
        assert!(tmp.is_dir(), "workspace directory created");
        let s = settings.read().unwrap();
        assert!(s.use_nerd_fonts);
        assert_eq!(s.editor_backend, EditorBackendSetting::Vim);
        assert_eq!(s.current_workspace_name().as_deref(), Some("myws"));
        assert_eq!(s.theme, m.draft.theme_name);
        assert_eq!(s.config_version, CURRENT_CONFIG_VERSION);
        drop(s);
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::remove_file(&cfg).ok();
    }

    #[test]
    fn commit_first_run_without_workspace_returns_to_workspace() {
        let settings = shared_defaults();
        let mut m = model(&settings);
        m.draft.workspace = None;
        let err = commit(&settings, &m.draft, true).unwrap_err();
        assert!(matches!(err, CommitError::NoWorkspace));
        assert_eq!(err.return_step(), OnbStep::Workspace);
        assert_eq!(err.to_string(), "no workspace configured");
    }

    #[test]
    fn rerun_commit_never_touches_workspaces() {
        let settings = shared_with_workspace();
        let cfg =
            std::env::temp_dir().join(format!("kimun_onb_lcfg_r_{}.toml", std::process::id()));
        settings.write().unwrap().config_file = Some(cfg.clone());
        let names = |s: &SharedSettings| -> Vec<String> {
            s.read()
                .unwrap()
                .workspace_config
                .as_ref()
                .unwrap()
                .workspaces
                .keys()
                .cloned()
                .collect()
        };
        let before = names(&settings);
        let mut m = model(&settings);
        m.draft.use_nerd_fonts = true;
        commit(&settings, &m.draft, m.first_run).unwrap();
        assert_eq!(before, names(&settings));
        assert!(
            settings.read().unwrap().use_nerd_fonts,
            "fonts applied on rerun commit"
        );
        std::fs::remove_file(&cfg).ok();
    }

    fn names(dirs: &[kimun_core::system::SystemPath]) -> Vec<String> {
        dirs.iter()
            .map(|d| {
                d.as_path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn list_subdirs_lists_only_directories_sorted_ignoring_case() {
        let tmp = tempfile::TempDir::new().unwrap();
        for d in ["beta", "Alpha", "gamma", ".hidden"] {
            std::fs::create_dir(tmp.path().join(d)).unwrap();
        }
        std::fs::write(tmp.path().join("aaa.txt"), "x").unwrap();
        let dir = kimun_core::system::SystemPath::try_absolute(tmp.path()).unwrap();
        let dirs = list_subdirs(&dir).unwrap();
        assert_eq!(names(&dirs), [".hidden", "Alpha", "beta", "gamma"]);
        assert!(dirs.iter().all(|d| d.parent().as_ref() == Some(&dir)));
    }

    #[test]
    fn list_subdirs_reports_an_unreadable_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let gone = kimun_core::system::SystemPath::try_absolute(tmp.path().join("gone")).unwrap();
        assert!(list_subdirs(&gone).is_err());
    }

    #[test]
    fn create_subdir_makes_the_folder_and_refuses_bad_names() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = kimun_core::system::SystemPath::try_absolute(tmp.path()).unwrap();
        let made = create_subdir(&dir, "  My-Notes ").unwrap();
        assert!(made.is_dir());
        assert_eq!(names(&[made]), ["My-Notes"]);
        assert!(matches!(
            create_subdir(&dir, "   "),
            Err(NewFolderError::Empty)
        ));
        for bad in ["a/b", "a\\b", "..", "con", "notes:"] {
            assert!(
                matches!(create_subdir(&dir, bad), Err(NewFolderError::Invalid(_))),
                "{bad}"
            );
        }
        assert_eq!(names(&list_subdirs(&dir).unwrap()), ["My-Notes"]);
    }

    #[test]
    fn apply_draft_applies_without_saving() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = tmp.path().join("config.toml");
        let settings = shared_defaults();
        settings.write().unwrap().config_file = Some(cfg.clone());
        let mut m = model(&settings);
        let ws = tmp.path().join("ws");
        m.draft.workspace = Some(("ws".to_string(), ws.clone()));
        apply_draft(&settings, &m.draft, true).unwrap();
        assert!(ws.is_dir());
        assert_eq!(
            settings.read().unwrap().current_workspace_name().as_deref(),
            Some("ws")
        );
        assert!(!cfg.exists(), "nothing written");
    }

    #[test]
    fn commit_rolls_back_created_dir_when_registration_fails() {
        let tmp = std::env::temp_dir().join(format!("kimun_onb_lrollback_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // A leftover workspace entry named "notes" with no current workspace:
        // first_run is true, but registering another "notes" must fail.
        let shared = {
            let mut s = AppSettings::default();
            let mut wc = WorkspaceConfig::new_empty();
            wc.add_workspace("notes".to_string(), tmp.join("other"))
                .unwrap();
            wc.global.current_workspace = String::new();
            s.workspace_config = Some(wc);
            Arc::new(RwLock::new(s))
        };
        let mut m = model(&shared);
        assert!(m.first_run);
        let new_dir = tmp.join("fresh");
        m.draft.workspace = Some(("notes".to_string(), new_dir.clone()));
        let err = commit(&shared, &m.draft, m.first_run).unwrap_err();
        assert!(matches!(err, CommitError::Register(_)));
        assert_eq!(err.return_step(), OnbStep::Workspace);
        assert!(
            !new_dir.exists(),
            "directory created by commit must be rolled back"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }
}
