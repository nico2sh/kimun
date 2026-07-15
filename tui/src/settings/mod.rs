use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
use crate::keys::key_strike::KeyStrike;
use crate::settings::config_dir::get_or_create_config_dir;
use crate::settings::themes::Theme;
use crate::settings::workspace_config::WorkspaceConfig;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use std::fs::{self, File};

use color_eyre::eyre;

/// Shared settings handle — all screens and components reference the same instance.
pub type SharedSettings = Arc<RwLock<AppSettings>>;
use kimun_core::nfs::VaultPath;

use crate::keys::KeyBindings;
mod config_dir;
pub(crate) use config_dir::get_home_dir;
pub mod config_migration;
pub mod history;
pub mod icons;
pub mod themes;
pub mod workspace_config;

// ---------------------------------------------------------------------------
// Sort settings types (shared between AppSettings and sorting UI)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortFieldSetting {
    Name,
    Title,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrderSetting {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditorBackendSetting {
    #[default]
    Textarea,
    Nvim,
    Vim,
}

// pub mod theme;

#[cfg(debug_assertions)]
const CONFIG_DIR: &str = "kimun_debug";
#[cfg(not(debug_assertions))]
const CONFIG_DIR: &str = "kimun";

/// Path to kimün's config directory (`~/.config/kimun`, or `kimun_debug` in
/// debug builds), creating it if needed. Single source of truth for the
/// debug/release directory name — used by the update module for the install
/// marker and update-state file.
pub fn config_dir() -> std::io::Result<PathBuf> {
    get_or_create_config_dir(CONFIG_DIR)
}

const BASE_CONFIG_FILE: &str = "config.toml";
const THEMES_DIR: &str = "themes";
const CACHE_FILE_EXT: &str = "kimuncache";
const HISTORY_FILE_EXT: &str = "txt";

const CONFIG_HEADER: &str = "\
# ─── Kimün configuration ────────────────────────────────────────────────────
#
# KEY BINDINGS
# ────────────
# Supported combinations:
#   - ctrl and/or alt (with optional shift) + a letter (a-z)
#   - bare F-key (F1–F12, no modifier required)
# Any combo that does not follow these rules is silently ignored when loaded.
#
# Format per action:
#   ActionName = [\"<modifiers> & <letter>\", ...]
#
# Available modifiers (combine with +):  ctrl   alt   shift
#
# Examples:
#   Quit         = [\"ctrl&Q\"]            # Ctrl+Q
#   SearchNotes  = [\"ctrl&K\"]            # Ctrl+K
#   OpenNote     = [\"ctrl&O\"]            # Ctrl+O  (fuzzy file finder)
#   OpenSettings = [\"F4\", \"ctrl&,\"]     # F4 (Ctrl+, alias)
#   NewJournal   = [\"ctrl&J\"]            # Ctrl+J
#   FileOperations = [\"F2\"]              # F2  (open file-ops menu: delete/rename/move)
#   Leader       = [\"ctrl&G\"]            # Ctrl+G  (leader gateway: Ctrl+G f f, ...)
#   OpenCommandPalette = [\"ctrl&P\"]      # Ctrl+P  (every leader command, fuzzy)
#
# OTHER SETTINGS
# ──────────────
#   theme             = \"Gruvbox Dark\"   # or any built-in / custom theme name
#   leader_timeout_ms = 400               # hesitation before the which-key menu
#
# LEADER TREE OVERRIDES
# ─────────────────────
#   Remap, add, or remove leader sequences ([leader.bind]) and rename group
#   captions ([leader.labels]). Keys are the sequence AFTER the gateway;
#   bind values are action ids (see the cheatsheet) or \"none\" to unbind.
#   [leader.bind]
#   \"o f\" = \"find.files\"     # remap: leader o f now opens the file picker
#   \"x\"   = \"note.daily\"     # add:   leader x opens today's journal
#   \"g p\" = \"none\"           # remove the git-sync stub binding
#   [leader.labels]
#   \"f\"   = \"+search\"        # rename the +find group caption
#
# ─────────────────────────────────────────────────────────────────────────────
";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AppSettings {
    // Phase 2 config
    #[serde(default)]
    pub config_version: u32,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub workspace_config: Option<WorkspaceConfig>,

    // Legacy Phase 1 fields — only kept for migration detection/deserialization.
    // Never written back: workspace_dir is taken by migration, last_paths is
    // moved into workspace_config entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    pub last_paths: Vec<VaultPath>,

    // Preserved fields
    #[serde(default)]
    pub theme: String,
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
    #[serde(skip)]
    cache_dir_resolved: Option<PathBuf>,

    #[serde(default = "default_history_dir")]
    pub history_dir: PathBuf,
    #[serde(skip)]
    history_dir_resolved: Option<PathBuf>,
    #[serde(skip, default = "yes")]
    needs_indexing: bool,
    #[serde(default = "default_keybindings")]
    pub key_bindings: KeyBindings,
    #[serde(default = "default_autosave_interval")]
    pub autosave_interval_secs: u64,
    /// Hesitation timeout (ms) before the which-key overlay reveals itself
    /// during a pending leader sequence. Sequences typed faster never wait.
    #[serde(default = "default_leader_timeout_ms")]
    pub leader_timeout_ms: u64,
    /// Leader-tree customization: `[leader.bind]` sequence→action-id
    /// overrides and `[leader.labels]` group captions. Applied over the
    /// built-in tree.
    #[serde(default)]
    pub leader: LeaderConfig,
    #[serde(default = "default_use_nerd_fonts")]
    pub use_nerd_fonts: bool,
    #[serde(default)]
    pub editor_backend: EditorBackendSetting,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvim_path: Option<std::path::PathBuf>,
    #[serde(default = "default_sort_field")]
    pub default_sort_field: SortFieldSetting,
    #[serde(default = "default_sort_order")]
    pub default_sort_order: SortOrderSetting,
    #[serde(default = "default_journal_sort_field")]
    pub journal_sort_field: SortFieldSetting,
    #[serde(default = "default_journal_sort_order")]
    pub journal_sort_order: SortOrderSetting,
    #[serde(default)]
    pub group_directories: bool,
    /// Custom config file path. `None` means use the default location.
    /// Not serialized — it's a runtime-only override.
    #[serde(skip)]
    pub config_file: Option<PathBuf>,
}

fn default_keybindings() -> KeyBindings {
    let mut kb = KeyBindings::empty();
    kb.batch_add()
        .with_ctrl()
        .add(KeyStrike::KeyK, ActionShortcuts::SearchNotes)
        .add(KeyStrike::KeyO, ActionShortcuts::OpenNote)
        .add(KeyStrike::KeyB, ActionShortcuts::Text(TextAction::Bold))
        .add(KeyStrike::KeyI, ActionShortcuts::Text(TextAction::Italic))
        .add(
            KeyStrike::KeyU,
            ActionShortcuts::Text(TextAction::Underline),
        )
        .add(
            KeyStrike::KeyS,
            ActionShortcuts::Text(TextAction::Strikethrough),
        )
        .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Link))
        .add(
            KeyStrike::KeyT,
            ActionShortcuts::Text(TextAction::ToggleHeader),
        )
        // =============================
        // We add shift to the modifiers
        // =============================
        .with_shift()
        .add(KeyStrike::KeyL, ActionShortcuts::Text(TextAction::Image));

    // TUI navigation shortcuts (always Ctrl — terminal apps don't use Cmd/Meta).
    // NOTE: the `Quit` entry must match `crate::keys::default_quit_combo()`,
    // which the deserialize safety net uses to recover an unreachable app.
    kb.batch_add()
        .with_ctrl()
        // Ctrl-P is the command palette (decision 2026-06-05); settings
        // live on Ctrl+Shift+P.
        .add(KeyStrike::KeyP, ActionShortcuts::OpenCommandPalette)
        .add(KeyStrike::KeyQ, ActionShortcuts::Quit)
        .add(KeyStrike::KeyJ, ActionShortcuts::NewJournal)
        // Drawer toggle. Deliberate spec deviation: the spec's Tier-0 puts
        // this on Ctrl-B, but Ctrl-B stays Bold (decision 2026-06-05) — the
        // drawer toggle lives on Ctrl-T.
        .add(KeyStrike::KeyT, ActionShortcuts::ToggleSidebar)
        .add(KeyStrike::KeyR, ActionShortcuts::OpenSortDialog)
        // Leader gateway. Spec deviation: spec says Ctrl-K, which stays the
        // note browser; the gateway lives on Ctrl-G (decision 2026-06-05).
        .add(KeyStrike::KeyG, ActionShortcuts::Leader)
        // FollowLink's always-works binding; Ctrl+Enter also follows on
        // kitty-protocol terminals (hardcoded in the editor screen).
        .add(KeyStrike::KeyN, ActionShortcuts::FollowLink)
        .add(KeyStrike::KeyH, ActionShortcuts::FocusSidebar)
        .add(KeyStrike::KeyL, ActionShortcuts::FocusEditor)
        .add(KeyStrike::KeyW, ActionShortcuts::QuickNote)
        // Ctrl-E opens (or switches the drawer to) the file browser; the
        // pure drawer toggle is Ctrl-T above. ToggleQueryPanel has no
        // default binding — FIND stays reachable via the rail and leader.
        .add(KeyStrike::KeyE, ActionShortcuts::OpenFileBrowser)
        .add(KeyStrike::KeyF, ActionShortcuts::FindInBuffer);

    // Settings — F4 (no modifier, reliable in all terminals) plus the classic
    // Ctrl+, kept as an alias. Ctrl+, doesn't transmit a distinct code on many
    // terminals outside the kitty protocol, so F4 is the dependable default.
    // (Ctrl+Shift+P collides with kitty's default hints-kitten chord prefix,
    // which holds the screen mid-chord, so it isn't used.)
    kb.batch_add()
        .add(KeyStrike::F4, ActionShortcuts::OpenPreferences);
    kb.batch_add()
        .with_ctrl()
        .add(KeyStrike::Comma, ActionShortcuts::OpenPreferences);

    // File operations menu (F2 — no modifier, reliable in all terminals).
    kb.batch_add()
        .add(KeyStrike::F2, ActionShortcuts::FileOperations);

    kb.batch_add()
        .add(KeyStrike::F3, ActionShortcuts::OpenSavedSearches);

    // RAG answer overlay (F6 — free key; the feature is inert without a server).
    kb.batch_add()
        .add(KeyStrike::F6, ActionShortcuts::OpenRagAnswer);

    // Workspace switcher — F5 (moved off F4, which is now Settings).
    kb.batch_add()
        .add(KeyStrike::F5, ActionShortcuts::SwitchWorkspace);

    // Ctrl+D — save the current query to saved searches. Ctrl-only by design:
    // Ctrl+Shift is unreliable on some terminals, Ctrl+S is taken by
    // Strikethrough, and Ctrl+{A,C,X,Z} are claimed by the editor. Ctrl+D is
    // the only free, terminal-safe Ctrl combo.
    kb.batch_add()
        .with_ctrl()
        .add(KeyStrike::KeyD, ActionShortcuts::SaveCurrentQuery);

    kb
}

fn yes() -> bool {
    true
}

fn default_autosave_interval() -> u64 {
    5
}

fn default_leader_timeout_ms() -> u64 {
    400
}

/// The `[leader]` config section: binding overrides + group captions.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LeaderConfig {
    /// `[leader.bind]`: sequence (after the gateway, e.g. `"o f"` / `"x"`) →
    /// action id (see the cheatsheet) or `"none"` to unbind.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub bind: std::collections::BTreeMap<String, String>,
    /// `[leader.labels]`: group sequence (e.g. `"f"`) → caption shown in the
    /// which-key overlay and cheatsheet.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub labels: std::collections::BTreeMap<String, String>,
}

impl AppSettings {
    /// Suggested directory for a first workspace (`~/kimun-notes`). `None`
    /// when the home directory cannot be determined.
    pub fn default_workspace_suggestion() -> Option<PathBuf> {
        config_dir::get_home_dir()
            .ok()
            .map(|h| h.join("kimun-notes"))
    }

    /// The leader tree with this config's `[leader]` overrides applied — the
    /// ONE constructor every surface (engine, which-key, cheatsheet, palette)
    /// must use, so they can never disagree.
    pub fn leader_tree(&self) -> crate::keys::leader::LeaderNode {
        let tree = crate::keys::leader::apply_overrides(
            crate::keys::leader::leader_tree(),
            self.leader
                .bind
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        );
        crate::keys::leader::apply_labels(
            tree,
            self.leader
                .labels
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        )
    }
}

fn default_cache_dir() -> PathBuf {
    PathBuf::from(".")
}

fn default_history_dir() -> PathBuf {
    PathBuf::from("history")
}

fn default_use_nerd_fonts() -> bool {
    false
}

fn default_sort_field() -> SortFieldSetting {
    SortFieldSetting::Name
}

fn default_sort_order() -> SortOrderSetting {
    SortOrderSetting::Ascending
}

fn default_journal_sort_field() -> SortFieldSetting {
    SortFieldSetting::Name
}

fn default_journal_sort_order() -> SortOrderSetting {
    SortOrderSetting::Descending
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            config_version: 0,
            workspace_config: None,
            last_paths: vec![],
            workspace_dir: None,
            theme: Default::default(),
            cache_dir: default_cache_dir(),
            cache_dir_resolved: None,
            history_dir: default_history_dir(),
            history_dir_resolved: None,
            needs_indexing: true,
            key_bindings: default_keybindings(),
            autosave_interval_secs: default_autosave_interval(),
            leader_timeout_ms: default_leader_timeout_ms(),
            leader: LeaderConfig::default(),
            use_nerd_fonts: false,
            editor_backend: EditorBackendSetting::Textarea,
            nvim_path: None,
            default_sort_field: default_sort_field(),
            default_sort_order: default_sort_order(),
            journal_sort_field: default_journal_sort_field(),
            journal_sort_order: default_journal_sort_order(),
            group_directories: false,
            config_file: None,
        }
    }
}

impl AppSettings {
    pub fn theme_list(&self) -> Vec<Theme> {
        let mut list = Theme::builtins();
        list.append(&mut Self::load_custom_themes());
        // Merge the user's default.toml override if present.
        if let Ok(custom_default) = Self::load_default_theme() {
            list.push(custom_default);
        }
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    fn default_config_file_path() -> eyre::Result<PathBuf> {
        let config_home = get_or_create_config_dir(CONFIG_DIR)?;
        Ok(config_home.join(BASE_CONFIG_FILE))
    }

    fn get_config_file_path(&self) -> eyre::Result<PathBuf> {
        if let Some(ref path) = self.config_file {
            Ok(path.clone())
        } else {
            Self::default_config_file_path()
        }
    }

    fn get_themes_path() -> eyre::Result<PathBuf> {
        let config_home = get_or_create_config_dir(CONFIG_DIR)?;
        Ok(config_home.join(THEMES_DIR))
    }

    fn load_theme_from_path(path: &std::path::Path) -> eyre::Result<Theme> {
        let theme_string = fs::read_to_string(path)?;
        match toml::from_str::<Theme>(&theme_string) {
            Ok(theme) => Ok(theme),
            Err(e) => {
                // Never delete a user-authored file over a typo — warn and
                // skip, exactly like load_custom_themes does.
                tracing::warn!("Skipping unparsable theme file {:?}: {}", path, e);
                Err(eyre::eyre!("corrupt theme file: {}", e))
            }
        }
    }

    fn load_default_theme() -> eyre::Result<Theme> {
        let theme_path = AppSettings::get_themes_path()?.join("default.toml");
        Self::load_theme_from_path(&theme_path)
    }

    fn load_custom_themes() -> Vec<Theme> {
        let mut themes = Vec::new();

        // Get themes directory, return empty vec if it fails
        let themes_path = match Self::get_themes_path() {
            Ok(path) => path,
            Err(_) => return themes,
        };

        // Read directory entries, return empty vec if it fails
        let entries = match fs::read_dir(&themes_path) {
            Ok(entries) => entries,
            Err(_) => return themes,
        };

        // Iterate through all entries in the themes directory
        for entry in entries.flatten() {
            let path = entry.path();

            // Skip if not a file
            if !path.is_file() {
                continue;
            }

            // Skip if not a .toml file
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }

            // Skip default.toml
            if path.file_name().and_then(|s| s.to_str()) == Some("default.toml") {
                continue;
            }

            // Try to read and deserialize the theme file
            match fs::read_to_string(&path)
                .and_then(|s| toml::from_str::<Theme>(&s).map_err(std::io::Error::other))
            {
                Ok(theme) => themes.push(theme),
                Err(e) => tracing::warn!("Skipping theme file {:?}: {}", path, e),
            }
        }

        themes
    }

    /// Whether the startup update check is enabled. Lives in `GlobalConfig`;
    /// defaults to on when no workspace config exists yet. Single source for
    /// the four read sites (startup, preferences, onboarding).
    pub fn update_check(&self) -> bool {
        self.workspace_config
            .as_ref()
            .map(|wc| wc.global.update_check)
            .unwrap_or(true)
    }

    /// Whether kimün captures the mouse for in-app use; defaults on when no
    /// workspace config exists yet. Read at startup (main.rs) and in preferences.
    pub fn mouse(&self) -> bool {
        self.workspace_config
            .as_ref()
            .map(|wc| wc.global.mouse)
            .unwrap_or(true)
    }

    pub fn save_to_disk(&self) -> eyre::Result<()> {
        tracing::debug!("Saving settings to disk");
        let settings_file_path = self.get_config_file_path()?;
        let mut file = File::create(settings_file_path)?;
        file.write_all(CONFIG_HEADER.as_bytes())?;
        let toml = toml::to_string(&self)?;
        file.write_all(toml.as_bytes())?;
        Ok(())
    }

    pub fn load_from_disk() -> eyre::Result<Self> {
        let settings_file_path = Self::default_config_file_path()?;

        if !settings_file_path.exists() {
            let default_settings = Self::default();
            default_settings.save_to_disk()?;
            Ok(default_settings)
        } else {
            let mut settings_file = File::open(&settings_file_path)?;

            let mut toml = String::new();
            settings_file.read_to_string(&mut toml)?;

            match toml::from_str::<AppSettings>(toml.as_ref()) {
                Ok(mut setting) => {
                    setting.config_file = Some(settings_file_path.clone());
                    let config_dir = settings_file_path
                        .parent()
                        .unwrap_or(std::path::Path::new("."));
                    setting.resolve_paths(config_dir);
                    if config_migration::ConfigMigration::run(&mut setting)? {
                        setting.save_to_disk()?;
                    }
                    setting.merge_missing_default_bindings();
                    Ok(setting)
                }
                Err(e) => {
                    tracing::warn!(
                        "Config file at {:?} could not be parsed ({}). \
                         Renaming to .corrupt and starting with defaults.",
                        settings_file_path,
                        e
                    );
                    let corrupt_path = settings_file_path.with_extension("toml.corrupt");
                    let _ = fs::rename(&settings_file_path, &corrupt_path);
                    let defaults = Self::default();
                    defaults.save_to_disk()?;
                    Ok(defaults)
                }
            }
        }
    }

    pub fn load_from_file(path: PathBuf) -> eyre::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            let default_settings = Self {
                config_file: Some(path),
                ..Self::default()
            };
            default_settings.save_to_disk()?;
            return Ok(default_settings);
        }
        let mut toml_str = String::new();
        File::open(&path)?.read_to_string(&mut toml_str)?;
        match toml::from_str::<AppSettings>(&toml_str) {
            Ok(mut setting) => {
                setting.config_file = Some(path.clone());

                // Resolve ~ and relative paths against the config file's directory.
                let config_dir = path.parent().unwrap_or(std::path::Path::new("."));
                setting.resolve_paths(config_dir);

                // Run config migrations (e.g. Phase 1 → Phase 2 workspace_dir).
                if config_migration::ConfigMigration::run(&mut setting)? {
                    setting.save_to_disk()?;
                }

                setting.merge_missing_default_bindings();
                Ok(setting)
            }
            Err(e) => {
                tracing::warn!(
                    "Config file at {:?} could not be parsed ({}). \
                     Renaming to .corrupt and starting with defaults.",
                    path,
                    e
                );
                let corrupt_path = path.with_extension("toml.corrupt");
                let _ = fs::rename(&path, &corrupt_path);
                let defaults = Self {
                    config_file: Some(path),
                    ..Self::default()
                };
                defaults.save_to_disk()?;
                Ok(defaults)
            }
        }
    }

    /// Fills in defaults from `default_keybindings()` that are absent in the
    /// loaded config: actions with no binding at all, plus default combos
    /// added in newer versions (e.g. Ctrl-B for the drawer toggle) — as long
    /// as the combo is not already bound to *any* action. Existing
    /// user-customised bindings are never overwritten.
    fn merge_missing_default_bindings(&mut self) {
        let defaults = default_keybindings().to_hashmap();
        let mut current = self.key_bindings.to_hashmap();
        let mut bound: std::collections::HashSet<_> = current.values().flatten().cloned().collect();
        for (action, combos) in defaults {
            match current.entry(action) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    // Never steal a combo the user has bound to something
                    // else — insert only the free ones, and claim them so a
                    // later default in this pass cannot double-bind.
                    let free: Vec<_> = combos.into_iter().filter(|c| !bound.contains(c)).collect();
                    if !free.is_empty() {
                        bound.extend(free.iter().copied());
                        e.insert(free);
                    }
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    for combo in combos {
                        if !bound.contains(&combo) && !e.get().contains(&combo) {
                            bound.insert(combo);
                            e.get_mut().push(combo);
                        }
                    }
                }
            }
        }
        self.key_bindings = KeyBindings::from_hashmap(current);
    }

    // We set a new workspace to work with, remember to save the data
    // to persist it in disk
    pub fn set_workspace(&mut self, workspace_path: &PathBuf) {
        if let Some(current_workspace_dir) = &self.workspace_dir
            && workspace_path != current_workspace_dir
        {
            self.needs_indexing = true;
        }

        self.workspace_dir = Some(workspace_path.to_owned());
    }

    /// Removes the active workspace path so the user is prompted to choose a new one.
    /// Handles both Phase 1 (workspace_dir) and Phase 2 (workspace_config) config formats.
    ///
    /// For Phase 2: only the currently active workspace entry is removed; other workspace
    /// entries in the config are preserved. After this call, `workspace_config` remains
    /// `Some` but `get_current_workspace()` returns `None`.
    pub fn clear_workspace(&mut self) {
        // Phase 1
        if self.workspace_dir.is_some() {
            self.workspace_dir = None;
            self.needs_indexing = true;
        }
        // Phase 2
        if let Some(wc) = &mut self.workspace_config {
            let key = wc.global.current_workspace.clone();
            if !key.is_empty() {
                wc.workspaces.remove(&key);
            }
            wc.global.current_workspace = String::new();
        }
    }

    /// Resolve the active workspace path from Phase 2 (workspace_config) or
    /// Phase 1 (workspace_dir). Returns `None` if no workspace is configured.
    pub fn resolve_workspace_path(&self) -> Option<PathBuf> {
        self.workspace_config
            .as_ref()
            .and_then(|wc| wc.get_current_workspace())
            .map(|entry| entry.effective_path().clone())
            .or_else(|| self.workspace_dir.clone())
    }

    /// Resolve `~` and relative paths in workspace entries.
    /// Relative paths are resolved against `base` (typically the config file's
    /// parent directory). Called once after deserialization.
    fn resolve_paths(&mut self, base: &std::path::Path) {
        // Legacy workspace_dir — resolve in place (it's a legacy field that
        // gets consumed by migration anyway).
        if let Some(ref mut p) = self.workspace_dir {
            *p = Self::expand_path(p, base);
        }
        // Phase 2 workspace entries — populate resolved_path, keep original path intact.
        if let Some(ref mut wc) = self.workspace_config {
            for entry in wc.workspaces.values_mut() {
                let resolved = Self::expand_path(&entry.path, base);
                if resolved != entry.path {
                    entry.resolved_path = Some(resolved);
                }
            }
        }
        self.cache_dir_resolved = Some(Self::expand_path(&self.cache_dir, base));
        self.history_dir_resolved = Some(Self::expand_path(&self.history_dir, base));
    }

    /// Expand `~` to the home directory and resolve relative paths against `base`.
    /// Returns an absolute path. If the resolved path exists on disk, it is
    /// canonicalized to remove `.` and `..` components.
    fn expand_path(path: &std::path::Path, base: &std::path::Path) -> PathBuf {
        let s = path.to_string_lossy();
        let expanded = if s.starts_with("~/") || s == "~" {
            if let Ok(home) = config_dir::get_home_dir() {
                home.join(s.strip_prefix("~/").unwrap_or(""))
            } else {
                path.to_path_buf()
            }
        } else {
            path.to_path_buf()
        };
        let absolute = if expanded.is_relative() {
            base.join(expanded)
        } else {
            expanded
        };
        // Canonicalize if the path exists, otherwise return as-is.
        absolute.canonicalize().unwrap_or(absolute)
    }

    pub fn set_theme(&mut self, theme: String) {
        self.theme = theme;
    }

    pub fn report_indexed(&mut self) {
        self.needs_indexing = false;
    }

    pub fn needs_indexing(&self) -> bool {
        self.needs_indexing
    }

    pub fn add_path_history(&mut self, note_path: &VaultPath) {
        if !note_path.is_note() {
            return;
        }
        let Some(workspace_name) = self.current_workspace_name() else {
            return;
        };
        let file_path = self.history_path_for(&workspace_name);
        if let Err(e) = history::push_history(&file_path, note_path) {
            tracing::warn!("failed to write history {:?}: {}", file_path, e);
        }
    }

    pub fn current_workspace_name(&self) -> Option<String> {
        self.workspace_config
            .as_ref()
            .map(|wc| wc.global.current_workspace.clone())
            .filter(|s| !s.is_empty())
    }

    pub fn cache_dir_resolved(&self) -> Option<&Path> {
        self.cache_dir_resolved.as_deref()
    }

    pub fn history_dir_resolved(&self) -> Option<&Path> {
        self.history_dir_resolved.as_deref()
    }

    /// Path to the SQLite cache file for the named workspace.
    /// Caller must have already validated `workspace_name` via
    /// `kimun_core::nfs::filename::validate_filename`.
    pub fn cache_path_for(&self, workspace_name: &str) -> PathBuf {
        Self::workspace_file(
            self.cache_dir_resolved.as_ref().unwrap_or(&self.cache_dir),
            workspace_name,
            CACHE_FILE_EXT,
        )
    }

    /// Path to the history file for the named workspace.
    /// Caller must have already validated `workspace_name`.
    pub fn history_path_for(&self, workspace_name: &str) -> PathBuf {
        Self::workspace_file(
            self.history_dir_resolved
                .as_ref()
                .unwrap_or(&self.history_dir),
            workspace_name,
            HISTORY_FILE_EXT,
        )
    }

    fn workspace_file(dir: &Path, workspace_name: &str, ext: &str) -> PathBuf {
        dir.join(format!("{workspace_name}.{ext}"))
    }

    /// Returns the last-visited paths for the current workspace.
    pub fn current_last_paths(&self) -> Vec<VaultPath> {
        let Some(name) = self.current_workspace_name() else {
            return Vec::new();
        };
        let file_path = self.history_path_for(&name);
        history::load_history(&file_path)
    }

    /// Build the icon set for the current `use_nerd_fonts` setting.
    pub fn icons(&self) -> icons::Icons {
        icons::Icons::new(self.use_nerd_fonts)
    }

    /// Name of the theme the app is effectively using: the configured name,
    /// or the default theme's name when none is configured. Single owner of
    /// the empty-name fallback rule — use this instead of re-deriving it.
    pub fn effective_theme_name(&self) -> String {
        if self.theme.is_empty() {
            Theme::default().name
        } else {
            self.theme.clone()
        }
    }

    /// Resolve the active theme by name, falling back to the default.
    ///
    /// The resolved theme is adapted to the terminal's color depth (truecolor
    /// themes are quantized on 256-color terminals and mapped to role-semantic
    /// ANSI slots on 16-color terminals).
    pub fn get_theme(&self) -> Theme {
        let theme = if self.theme.is_empty() {
            Theme::default()
        } else {
            self.theme_list()
                .into_iter()
                .find(|t| t.name == self.theme)
                .unwrap_or_default()
        };
        theme.adapt_to_terminal()
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn default_workspace_suggestion_is_under_home() {
        let suggestion = AppSettings::default_workspace_suggestion();
        if let Some(p) = suggestion {
            assert!(p.ends_with("kimun-notes"));
            assert!(p.is_absolute());
        }
        // None is acceptable only when the platform has no home dir.
    }

    #[test]
    fn load_theme_from_nonexistent_path_returns_err_without_creating_file() {
        // RED: fails to compile because load_theme_from_path doesn't exist.
        // GREEN: method exists, returns Err, and does NOT create the file.
        let path = std::env::temp_dir().join("kimun_tdd_test_theme_absent.toml");
        let _ = std::fs::remove_file(&path); // ensure clean state

        let result = AppSettings::load_theme_from_path(&path);

        assert!(result.is_err(), "should return Err when file is absent");
        assert!(!path.exists(), "must not create the file as a side effect");
    }

    #[test]
    fn load_theme_from_corrupt_path_returns_err_without_recreating_file() {
        // After a corrupt file is removed, no replacement must be written.
        let path = std::env::temp_dir().join("kimun_tdd_test_theme_corrupt.toml");
        std::fs::write(&path, b"not valid toml {{{{").unwrap();

        let result = AppSettings::load_theme_from_path(&path);

        assert!(result.is_err(), "should return Err for corrupt TOML");
        // The user's file must SURVIVE a parse error (a typo must never
        // delete a hand-authored theme).
        assert!(path.exists(), "corrupt theme file must not be deleted");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn default_keybindings_quit_matches_canonical_combo() {
        let kb = default_keybindings();
        let combo = crate::keys::default_quit_combo();
        assert_eq!(
            kb.get_action(&combo),
            Some(ActionShortcuts::Quit),
            "default_keybindings() must bind default_quit_combo() to Quit so the \
             deserialize safety net can recover an unreachable app"
        );
    }

    #[test]
    fn autosave_interval_defaults_to_five() {
        let settings = AppSettings::default();
        assert_eq!(settings.autosave_interval_secs, 5);
    }

    #[test]
    fn autosave_interval_deserializes_from_toml() {
        let toml = "autosave_interval_secs = 30\n";
        let settings: AppSettings = toml::from_str(toml).unwrap();
        assert_eq!(settings.autosave_interval_secs, 30);
    }

    #[test]
    fn autosave_interval_defaults_when_missing_from_toml() {
        let toml = ""; // no autosave_interval_secs key
        let settings: AppSettings = toml::from_str(toml).unwrap();
        assert_eq!(settings.autosave_interval_secs, 5);
    }

    /// Verify the full load path: TOML with FileOperations = ["F2"] → keybinding lookup.
    #[test]
    fn f2_file_operations_survives_toml_deserialize() {
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        let toml = r#"
[key_bindings]
FileOperations = ["F2"]
"#;
        let settings: AppSettings = toml::from_str(toml).unwrap();
        let f2 = KeyCombo::new(KeyModifiers::default(), KeyStrike::F2);
        let action = settings.key_bindings.get_action(&f2);
        assert_eq!(
            action,
            Some(ActionShortcuts::FileOperations),
            "F2 should survive deserialization and map to FileOperations"
        );
    }

    /// Verify merge_missing_default_bindings adds F2 when absent from config.
    #[test]
    fn merge_adds_f2_when_absent() {
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;

        // Settings with no FileOperations binding
        let toml = r#"
[key_bindings]
Quit = ["ctrl&Q"]
"#;
        let mut settings: AppSettings = toml::from_str(toml).unwrap();
        settings.merge_missing_default_bindings();

        let f2 = KeyCombo::new(KeyModifiers::default(), KeyStrike::F2);
        let action = settings.key_bindings.get_action(&f2);
        assert_eq!(
            action,
            Some(ActionShortcuts::FileOperations),
            "merge_missing_default_bindings should add F2 → FileOperations"
        );
    }

    #[test]
    fn clear_workspace_phase1_clears_workspace_dir() {
        let mut settings = AppSettings::default();
        settings.workspace_dir = Some(PathBuf::from("/tmp/vault"));
        settings.needs_indexing = false;
        settings.clear_workspace();
        assert!(
            settings.workspace_dir.is_none(),
            "workspace_dir should be None"
        );
        assert!(
            settings.needs_indexing,
            "needs_indexing should be reset to true"
        );
    }

    #[test]
    fn clear_workspace_phase2_removes_current_workspace_entry() {
        let mut settings = AppSettings::default();
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("vault1".to_string(), PathBuf::from("/tmp/vault1"))
            .unwrap();
        settings.workspace_config = Some(wc);
        // Assert precondition: add_workspace auto-selects the first workspace
        assert_eq!(
            settings
                .workspace_config
                .as_ref()
                .unwrap()
                .global
                .current_workspace,
            "vault1"
        );
        settings.clear_workspace();
        let wc = settings.workspace_config.as_ref().unwrap();
        assert!(
            wc.workspaces.is_empty(),
            "workspace entry should be removed"
        );
        assert!(
            wc.global.current_workspace.is_empty(),
            "current_workspace should be empty"
        );
    }

    #[test]
    fn clear_workspace_both_phases_active() {
        // When Phase 1 and Phase 2 fields are both populated (e.g. during migration),
        // clear_workspace must clear both independently.
        let mut settings = AppSettings::default();
        settings.workspace_dir = Some(PathBuf::from("/tmp/vault"));
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("vault1".to_string(), PathBuf::from("/tmp/vault1"))
            .unwrap();
        settings.workspace_config = Some(wc);
        settings.clear_workspace();
        assert!(
            settings.workspace_dir.is_none(),
            "phase1 workspace_dir should be cleared"
        );
        let wc = settings.workspace_config.as_ref().unwrap();
        assert!(
            wc.workspaces.is_empty(),
            "phase2 workspace entry should be removed"
        );
        assert!(
            wc.global.current_workspace.is_empty(),
            "phase2 current_workspace should be empty"
        );
    }

    #[test]
    fn clear_workspace_phase2_preserves_other_workspaces() {
        let mut settings = AppSettings::default();
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace("vault1".to_string(), PathBuf::from("/tmp/vault1"))
            .unwrap();
        wc.add_workspace("vault2".to_string(), PathBuf::from("/tmp/vault2"))
            .unwrap();
        wc.global.current_workspace = "vault1".to_string();
        settings.workspace_config = Some(wc);
        settings.clear_workspace();
        let wc = settings.workspace_config.as_ref().unwrap();
        assert!(
            !wc.workspaces.contains_key("vault1"),
            "active workspace should be removed"
        );
        assert!(
            wc.workspaces.contains_key("vault2"),
            "other workspaces should be preserved"
        );
        assert!(
            wc.global.current_workspace.is_empty(),
            "current_workspace should be empty"
        );
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;

    #[test]
    fn default_backend_is_textarea() {
        let settings = AppSettings::default();
        assert!(matches!(
            settings.editor_backend,
            EditorBackendSetting::Textarea
        ));
    }

    #[test]
    fn nvim_backend_round_trips_toml() {
        let toml = "editor_backend = \"nvim\"\n";
        let parsed: AppSettings = toml::from_str(toml).unwrap();
        assert!(matches!(parsed.editor_backend, EditorBackendSetting::Nvim));
    }

    #[test]
    fn editor_backend_vim_roundtrips_through_toml() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct W {
            editor_backend: EditorBackendSetting,
        }
        let w = W {
            editor_backend: EditorBackendSetting::Vim,
        };
        let s = toml::to_string(&w).unwrap();
        assert!(s.contains("editor_backend = \"vim\""), "serialized: {s}");
        let back: W = toml::from_str(&s).unwrap();
        assert_eq!(back.editor_backend, EditorBackendSetting::Vim);
    }

    // ── expand_path tests ──────────────────────────────────────────────

    #[test]
    fn expand_path_absolute_unchanged() {
        let base = PathBuf::from("/config/dir");
        let result = AppSettings::expand_path(std::path::Path::new("/absolute/path/notes"), &base);
        assert!(result.is_absolute());
        assert!(result.to_string_lossy().contains("absolute"));
    }

    #[test]
    fn expand_path_relative_resolved_against_base() {
        let base = tempfile::TempDir::new().unwrap();
        let notes = base.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();

        let result = AppSettings::expand_path(std::path::Path::new("notes"), base.path());
        assert!(result.is_absolute());
        assert_eq!(result, notes.canonicalize().unwrap());
    }

    #[test]
    fn expand_path_relative_with_dotdot() {
        let base = tempfile::TempDir::new().unwrap();
        let sibling = base.path().join("sibling");
        std::fs::create_dir_all(&sibling).unwrap();
        let sub = base.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        let result = AppSettings::expand_path(std::path::Path::new("../sibling"), &sub);
        assert!(result.is_absolute());
        assert_eq!(result, sibling.canonicalize().unwrap());
    }

    #[test]
    fn expand_path_nonexistent_relative_still_absolute() {
        let base = PathBuf::from("/some/config/dir");
        let result = AppSettings::expand_path(std::path::Path::new("my-notes"), &base);
        assert!(result.is_absolute());
        assert_eq!(result, PathBuf::from("/some/config/dir/my-notes"));
    }

    #[test]
    #[cfg(unix)]
    fn expand_path_tilde_uses_home_unix() {
        let home = std::env::var("HOME").expect("HOME must be set on Unix");
        let base = PathBuf::from("/irrelevant");
        let result = AppSettings::expand_path(std::path::Path::new("~/Documents/notes"), &base);
        assert!(result.is_absolute());
        assert!(
            result.starts_with(&home),
            "expected path to start with HOME={}, got {:?}",
            home,
            result
        );
        assert!(result.to_string_lossy().contains("Documents/notes"));
    }

    #[test]
    #[cfg(unix)]
    fn expand_path_tilde_alone_is_home_unix() {
        let home = std::env::var("HOME").expect("HOME must be set on Unix");
        let base = PathBuf::from("/irrelevant");
        let result = AppSettings::expand_path(std::path::Path::new("~"), &base);
        assert!(result.is_absolute());
        // canonicalize may resolve symlinks, so compare canonicalized forms
        let expected = PathBuf::from(&home)
            .canonicalize()
            .unwrap_or(PathBuf::from(&home));
        assert_eq!(result, expected);
    }

    #[test]
    #[cfg(windows)]
    fn expand_path_tilde_uses_userprofile_windows() {
        let home = std::env::var("USERPROFILE").expect("USERPROFILE must be set on Windows");
        let base = PathBuf::from("C:\\irrelevant");
        let result = AppSettings::expand_path(std::path::Path::new("~/Documents/notes"), &base);
        assert!(result.is_absolute());
        assert!(
            result.starts_with(&home),
            "expected path to start with USERPROFILE={}, got {:?}",
            home,
            result
        );
    }

    #[test]
    fn resolve_paths_populates_resolved_path() {
        let base = tempfile::TempDir::new().unwrap();
        let notes = base.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();

        let toml = r#"
config_version = 2
[global]
current_workspace = "test"
[workspaces.test]
path = "notes"
last_paths = []
created = "2026-01-01T00:00:00Z"
"#
        .to_string();
        let mut settings: AppSettings = toml::from_str(&toml).unwrap();
        settings.resolve_paths(base.path());

        let wc = settings.workspace_config.as_ref().unwrap();
        let entry = wc.workspaces.get("test").unwrap();
        // Original path preserved
        assert_eq!(entry.path, PathBuf::from("notes"));
        // Resolved path is absolute
        assert!(entry.resolved_path.is_some());
        assert!(entry.effective_path().is_absolute());
    }

    #[test]
    fn resolve_paths_absolute_no_resolved_path() {
        let toml = r#"
config_version = 2
[global]
current_workspace = "test"
[workspaces.test]
path = "/absolute/notes"
last_paths = []
created = "2026-01-01T00:00:00Z"
"#;
        let mut settings: AppSettings = toml::from_str(toml).unwrap();
        settings.resolve_paths(std::path::Path::new("/config"));

        let wc = settings.workspace_config.as_ref().unwrap();
        let entry = wc.workspaces.get("test").unwrap();
        // No resolved_path needed for already-absolute paths
        assert!(entry.resolved_path.is_none());
        assert_eq!(*entry.effective_path(), PathBuf::from("/absolute/notes"));
    }
}

#[cfg(test)]
mod sort_settings_tests {
    use super::*;

    #[test]
    fn group_directories_defaults_off() {
        let s = AppSettings::default();
        assert!(!s.group_directories);
    }

    #[test]
    fn open_sort_dialog_is_bound_by_default() {
        let s = AppSettings::default();
        let map = s.key_bindings.to_hashmap();
        assert!(
            map.contains_key(&ActionShortcuts::OpenSortDialog),
            "OpenSortDialog must have a default binding"
        );
    }
}
