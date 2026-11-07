//! Native GPUI frontend for kimün (ADR-0049). The editor screen's shell
//! (M5b-1, ADR-0050): the activity rail, the left drawer (placeholder views)
//! and one editor pane, plain or vim as the TUI's `editor_backend` setting
//! says, in the user's theme, with the TUI's markdown rendering and
//! find/replace bar. It starts on the configured vault (M5b-2 G1), indexes
//! it in the background and opens the last note of the history, or the note
//! named on the command line. The open note is saved through core (G2): on
//! the autosave tick, on the Save chord, before another note opens and
//! before the window closes. The drawer's layout is saved to the `[gui]`
//! table.

mod assets;
mod editor_element;
mod editor_view;
mod find_bar;
#[cfg(test)]
mod fixture;
mod fonts;
mod grid;
mod input;
mod keymap;
mod row_cache;
mod row_paint;
mod shell_layout;
mod startup;
mod status_row;
mod tasks;
#[cfg(test)]
mod test_vault;
mod theme;
#[cfg(test)]
mod ui_tests;
mod workspace;

use std::sync::{Arc, RwLock};

use gpui_kit::*;
use kimun_core::system::SystemPath;
use kimun_notes::logic::startup::try_open_vault;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, EditorBackendSetting, SharedSettings};

use crate::assets::AppAssets;
use crate::editor_view::EditorView;
use crate::keymap::{Keymap, Platform};
use crate::workspace::{LayoutStore, Workspace};

/// The config, read the TUI's way and migrated in memory, but never written
/// or renamed. The GUI writes only its `[gui]` table, and never through this
/// copy: see `LayoutStore`, which saves through the TUI's migrated load. The
/// history is the exception the TUI shares: it lives in its own file, which
/// the restore prunes through the settings' history API.
fn load_settings() -> AppSettings {
    let mut settings = AppSettings::load_read_only_migrated().unwrap_or_else(|e| {
        eprintln!("kimun-gui: could not read the config ({e}); using the defaults");
        AppSettings::default()
    });
    sanitize(&mut settings);
    settings
}

/// Fix, in memory only, values a hand-edited config can hold that would
/// break the GUI: an autosave interval of 0 makes the tick's timer panic
/// (the TUI's preferences never write one).
fn sanitize(settings: &mut AppSettings) {
    settings.autosave_interval_secs = settings.autosave_interval_secs.max(1);
}

/// Same file, same theme lookup as the TUI (ADR-0049 §6). The only possible
/// side effect is `theme_list()` creating an empty config directory. Not
/// `get_theme()`: that applies `adapt_to_terminal()`, which is a terminal-only
/// concern.
fn load_theme(settings: &AppSettings) -> Theme {
    if settings.theme.is_empty() {
        Default::default()
    } else {
        settings
            .theme_list()
            .into_iter()
            .find(|t| t.name == settings.theme)
            .unwrap_or_default()
    }
}

/// Whether the editor runs the built-in vim engine, from the TUI's own
/// `editor_backend` setting (default plain). The GUI has no nvim backend
/// (ADR-0049 §5), so `nvim` gets what the TUI falls back to when nvim cannot
/// start: the plain editor.
pub(crate) fn uses_vim(backend: EditorBackendSetting) -> bool {
    match backend {
        EditorBackendSetting::Vim => true,
        EditorBackendSetting::Plain | EditorBackendSetting::Nvim => false,
    }
}

/// This platform's keymap with the `[gui.keys]` overrides. What the loader
/// rejects is reported and skipped; the rest of the keymap still loads.
/// (Preferences > Keyboard will show these, ADR-0050 §6; until then, the log
/// file — a GUI started from a launcher has no visible stderr — and stderr.)
fn load_keymap(settings: &AppSettings) -> Keymap {
    let (keymap, errors) = Keymap::load(Platform::current(), &settings.gui.key_overrides());
    for error in errors {
        tracing::warn!("{error}");
        eprintln!("kimun-gui: {error}");
    }
    keymap
}

/// Where kimün was started from, for a relative note on the command line.
fn current_dir() -> Option<SystemPath> {
    std::env::current_dir()
        .ok()
        .and_then(|dir| SystemPath::try_absolute(dir).ok())
}

fn main() {
    // The logic layer's background jobs (`SingleSlotTask`) call `tokio::spawn`,
    // which panics outside a runtime. GPUI's executor is not tokio, so the GUI
    // owns a runtime and keeps it entered on the main thread for its whole life
    // (ADR-0049). The guard and the runtime outlive `run`.
    let runtime = tokio::runtime::Runtime::new().expect("failed to start the tokio runtime");
    let _guard = runtime.enter();
    // The TUI's log file; kept open (and flushed) for the process's life.
    let _log = kimun_notes::app::init_file_logging();
    let settings = load_settings();
    let theme = load_theme(&settings);
    let vim = uses_vim(settings.editor_backend);
    let keymap = load_keymap(&settings);
    let layout = shell_layout::restore(&settings.gui);
    let name = settings.current_workspace_name();
    let settings: SharedSettings = Arc::new(RwLock::new(settings));
    // Opening the vault (the index file) is quick; indexing, which is not,
    // runs in the background once the window is up.
    let opened = runtime.block_on(try_open_vault(&settings));
    let arg = std::env::args_os().nth(1);
    let start = startup::decide(opened, name, arg.as_deref(), current_dir().as_ref());
    let title: SharedString = startup::window_title(&start).into();
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            // The kit's components (the palette dialog, the FILES filter) read the
            // kit theme, which is not kimün's: hand them kimün's colours. The
            // dialog's fade is dropped: a palette should appear and go at once.
            // `update`, not `global_mut`: it carries the colours over to the kit's
            // per-component tokens too (the dialog surface reads those).
            gpui_kit::component::Theme::update(cx, |kit| {
                theme::apply_to_kit(&theme, kit);
                kit.motion.duration_normal = std::time::Duration::ZERO;
            });
            editor_view::init(cx);
            keymap::init(cx, keymap);
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some(title),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                // The window's one app channel: the editor reports on it, the
                // workspace drains it.
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let editor = cx.new(|cx| EditorView::on_channel(vim, tx.clone(), cx));
                let workspace = cx.new(|cx| {
                    let mut workspace =
                        Workspace::new(editor, theme, layout, LayoutStore::UserConfig, window, cx);
                    workspace.start(start, settings, tx, rx, window, cx);
                    workspace
                });
                // The layout starts with the editor focused. Closing the
                // window saves the open note first.
                workspace.update(cx, |workspace, cx| {
                    workspace.apply_focus(window, cx);
                    workspace.guard_close(window, cx);
                });
                workspace
            })
            .expect("failed to open window");
        });
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the glob brings in `gpui_kit::test`, which shadows `#[test]`.
    use super::{AppSettings, EditorBackendSetting, sanitize, uses_vim};

    #[test]
    fn a_zero_autosave_interval_is_raised_to_one_second() {
        let mut settings = AppSettings::default();
        settings.autosave_interval_secs = 0;
        sanitize(&mut settings);
        assert_eq!(settings.autosave_interval_secs, 1);
        settings.autosave_interval_secs = 30;
        sanitize(&mut settings);
        assert_eq!(settings.autosave_interval_secs, 30);
    }

    #[test]
    fn only_the_vim_backend_runs_vim() {
        assert!(uses_vim(EditorBackendSetting::Vim));
        assert!(!uses_vim(EditorBackendSetting::Plain));
        // No nvim in the GUI: the TUI's own fallback, the plain editor.
        assert!(!uses_vim(EditorBackendSetting::Nvim));
        assert!(!uses_vim(AppSettings::default().editor_backend));
    }
}
