//! **Layout presets** (ADR-0052 §5): named layouts the user authors in
//! `layouts.toml`, beside `config.toml`, as nested spec TOML
//! (`[preset.<name>]`, see `kimun_notes::logic::layout_tree::spec`).
//!
//! The file is read, never written: when the window opens and on "Reload
//! layout presets". A missing or empty file is no presets and no error; a
//! file that does not parse is one error flash at that load (the TOML
//! parser's message, or the node path and key of a malformed node), no
//! presets, and the layout on screen stays.
//!
//! A preset is a starting point. `Apply layout preset: <name>` builds it
//! as the startup restore builds the saved layout (a view the server does
//! not offer yet keeps its place, SEM and ASK included) and copies it
//! into the live layout
//! (`[gui.layout]`), recording `layout_preset = "<name>"`; from then on the
//! live layout is dragged and rearranged on its own, and editing the file
//! changes nothing until the preset is applied again. A preset that does
//! not build changes nothing and says why in one flash. "Reset to preset"
//! applies the recorded preset again, discarding weights dragged and views
//! moved since.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui_kit::{Context, Window};
use kimun_notes::logic::layout_tree::LayoutTree;
use kimun_notes::logic::layout_tree::spec::{self, ChildSpec};
use kimun_notes::logic::nav::DrawerView;

use super::{LayoutStore, Workspace};
use crate::shell_tree;

/// The presets file's name, in the directory of `config.toml`.
pub(crate) const PRESETS_FILE: &str = "layouts.toml";

/// Why "Reset to preset" cannot run: no preset was ever applied.
pub(crate) const NO_ACTIVE_PRESET: &str = "No preset applied";

/// Why "Reset to preset" cannot run: the applied preset is no longer in
/// the file (as last loaded).
pub(crate) const PRESET_GONE: &str = "Preset no longer in layouts.toml";

/// What reading the presets file gave.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct PresetLoad {
    /// The presets by name, empty when the file is missing, empty or broken.
    pub(crate) presets: BTreeMap<String, ChildSpec>,
    /// Why the file could not be read or parsed; `None` when it was fine
    /// or is missing.
    pub(crate) error: Option<String>,
}

/// Read the presets file in `dir`. Missing is no presets and no error.
pub(crate) fn load(dir: &Path) -> PresetLoad {
    let path = dir.join(PRESETS_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PresetLoad::default(),
        Err(e) => {
            return PresetLoad {
                presets: BTreeMap::new(),
                error: Some(format!("could not be read: {e}")),
            };
        }
    };
    match spec::parse_presets(&text) {
        Ok(file) => PresetLoad {
            presets: file.preset,
            error: None,
        },
        Err(message) => PresetLoad {
            presets: BTreeMap::new(),
            error: Some(message),
        },
    }
}

/// A preset command, run from its palette row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PresetCommand {
    /// `Apply layout preset: <name>`.
    Apply(String),
    /// "Reload layout presets".
    Reload,
    /// "Reset to preset": the recorded preset, applied again.
    Reset,
    /// "Reset layout to default": the built-in default layout, always
    /// offered, the way back from any arrangement (Ruling 23).
    ResetDefault,
}

impl PresetCommand {
    /// The palette row's label.
    pub(crate) fn label(&self) -> String {
        match self {
            PresetCommand::Apply(name) => format!("Apply layout preset: {name}"),
            PresetCommand::Reload => "Reload layout presets".to_string(),
            PresetCommand::Reset => "Reset to preset".to_string(),
            PresetCommand::ResetDefault => "Reset layout to default".to_string(),
        }
    }
}

impl LayoutStore {
    /// Where the presets file lives: beside the config file. `None` for a
    /// store that persists nothing, or when the app directory is out of
    /// reach (logged).
    fn presets_dir(&self) -> Option<PathBuf> {
        match self {
            #[cfg(test)]
            LayoutStore::Off => None,
            LayoutStore::UserConfig => match kimun_notes::settings::config_dir() {
                Ok(dir) => Some(dir),
                Err(e) => {
                    tracing::warn!("no app directory for {PRESETS_FILE}: {e}");
                    None
                }
            },
            #[cfg(test)]
            LayoutStore::File(path) => path.parent().map(Path::to_path_buf),
        }
    }

    /// The `layout_preset` the store's `[gui]` records, read fresh.
    fn recorded_preset(&self) -> Option<String> {
        match self {
            #[cfg(test)]
            LayoutStore::Off => None,
            LayoutStore::UserConfig => kimun_notes::settings::AppSettings::load_read_only()
                .ok()?
                .gui
                .layout_preset(),
            #[cfg(test)]
            LayoutStore::File(path) => {
                let text = std::fs::read_to_string(path).ok()?;
                toml::from_str::<kimun_notes::settings::AppSettings>(&text)
                    .ok()?
                    .gui
                    .layout_preset()
            }
        }
    }
}

impl Workspace {
    /// Read the presets file again (when the window opens, and "Reload
    /// layout presets"), and the preset the config records. A file that
    /// does not parse is said once, here; the presets loaded before stay
    /// (none at startup), and the layout on screen is never touched.
    pub(super) fn load_presets(&mut self, cx: &mut Context<Self>) {
        // What the config records is read back: after the writes in flight.
        self.layout_writer.flush();
        self.layout_preset = self.store.recorded_preset();
        let Some(dir) = self.store.presets_dir() else {
            self.presets.clear();
            return;
        };
        let load = load(&dir);
        if load.error.is_none() {
            self.presets = load.presets;
        }
        if let Some(error) = load.error {
            tracing::warn!("{PRESETS_FILE}: {error}");
            self.flash_error(format!("{PRESETS_FILE}: {error}"), cx);
        }
    }

    /// Why "Reset to preset" cannot run now, if it cannot.
    pub(super) fn reset_block(&self) -> Option<&'static str> {
        match &self.layout_preset {
            None => Some(NO_ACTIVE_PRESET),
            Some(name) if !self.presets.contains_key(name) => Some(PRESET_GONE),
            Some(_) => None,
        }
    }

    /// The preset rows of the palette, each with why it cannot run.
    pub(super) fn preset_rows(&self) -> Vec<(PresetCommand, Option<&'static str>)> {
        let mut rows: Vec<_> = self
            .presets
            .keys()
            .map(|name| (PresetCommand::Apply(name.clone()), None))
            .collect();
        rows.push((PresetCommand::Reload, None));
        rows.push((PresetCommand::Reset, self.reset_block()));
        rows.push((PresetCommand::ResetDefault, None));
        rows
    }

    pub(super) fn run_preset_command(
        &mut self,
        command: PresetCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PresetCommand::Apply(name) => self.apply_preset(&name, window, cx),
            PresetCommand::Reload => self.load_presets(cx),
            PresetCommand::Reset => match self.reset_block() {
                Some(reason) => self.flash(reason, cx),
                None => {
                    if let Some(name) = self.layout_preset.clone() {
                        self.apply_preset(&name, window, cx);
                    }
                }
            },
            PresetCommand::ResetDefault => self.install_layout(
                LayoutTree::default_layout(DrawerView::Files),
                None,
                window,
                cx,
            ),
        }
        cx.notify();
    }

    /// Copy the preset `name` into the live layout: built against the
    /// startup restore's caps (`shell_tree::RESTORE_CAPS`: every view,
    /// whatever the server says now), it replaces the tree, the session-only state is
    /// dropped (slots opened empty, the side toggles' memory, collapse and
    /// zoom), the editor gets the keyboard, and the layout and the preset's
    /// name are saved. A preset that does not build changes nothing; one
    /// flash lists its errors. Views left out because the layout cannot hold
    /// them (none today) are listed in one notice after a successful apply.
    fn apply_preset(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preset) = self.presets.get(name) else {
            self.flash_error(format!("No layout preset \"{name}\""), cx);
            return;
        };
        // The startup restore's caps, not the server's of the moment
        // (Ruling 18): a view the server does not offer yet (SEM before
        // the first report) keeps its placement, and the rail offers it
        // once the server does; ASK likewise.
        let (tree, warnings) = match spec::build(preset, shell_tree::RESTORE_CAPS) {
            Ok(built) => built,
            Err(errors) => {
                let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
                self.flash_error(
                    format!(
                        "Layout preset \"{name}\" not applied: {}",
                        spec::list_problems(&errors, spec::MAX_LISTED)
                    ),
                    cx,
                );
                return;
            }
        };
        self.install_layout(tree, Some(name), window, cx);
        if !warnings.is_empty() {
            self.flash(
                format!(
                    "Layout preset \"{name}\": {}",
                    spec::list_problems(&warnings, spec::MAX_LISTED)
                ),
                cx,
            );
        }
    }

    /// Replace the live layout with `tree`, applied from the preset
    /// `preset` (`None`: from no preset, as "Reset layout to default"):
    /// the session-only state is dropped (slots opened empty, the side
    /// toggles' memory, collapse and zoom, which lives in the old tree),
    /// the editor gets the keyboard, and the layout is saved with the
    /// preset's name recorded, or the recorded name cleared.
    fn install_layout(
        &mut self,
        tree: LayoutTree<DrawerView>,
        preset: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layout = tree;
        self.opened_empty.clear();
        self.side_memory = Default::default();
        self.collapsed.clear();
        self.resizing = false;
        self.divider_cells.borrow_mut().clear();
        self.layout_preset = preset.map(str::to_string);
        self.tidy_layout(cx);
        self.apply_focus(window, cx);
        match preset {
            Some(name) => self.save_layout_as_preset(name, cx),
            None => self.save_layout_clearing_preset(cx),
        }
    }
}
