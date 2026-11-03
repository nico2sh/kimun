//! Native GPUI frontend for kimün (ADR-0049). The editor screen's shell
//! (M5b-1, ADR-0050): the activity rail, the left drawer (placeholder views)
//! and one editor pane, plain or vim as the TUI's `editor_backend` setting
//! says, in the user's theme, with the TUI's markdown rendering and
//! find/replace bar, optionally showing a file named on the command line.
//! No note is ever saved; the drawer's layout is, to the `[gui]` table.

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
mod theme;
#[cfg(test)]
mod ui_tests;
mod workspace;

use gpui_kit::*;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, EditorBackendSetting};

use crate::assets::AppAssets;
use crate::editor_view::EditorView;
use crate::keymap::{Keymap, Platform};
use crate::workspace::{LayoutStore, Workspace};

/// The config, read the TUI's way but never migrated or renamed. The GUI
/// writes only its `[gui]` table, and never through this copy: see
/// `LayoutStore`, which saves through the TUI's migrated load.
fn load_settings() -> AppSettings {
    AppSettings::load_read_only().unwrap_or_else(|e| {
        eprintln!("kimun-gui: could not read the config ({e}); using the defaults");
        AppSettings::default()
    })
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
fn uses_vim(backend: EditorBackendSetting) -> bool {
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

/// The optional first argument: a file to show. Read only, never written; a
/// file that cannot be read is reported and the editor starts empty.
fn initial_text() -> (Option<String>, SharedString) {
    let Some(path) = std::env::args_os().nth(1) else {
        return (None, "kimün".into());
    };
    let shown = std::path::Path::new(&path).display().to_string();
    match std::fs::read_to_string(&path) {
        Ok(text) => (Some(text), format!("{shown} - kimün").into()),
        Err(e) => {
            eprintln!("kimun-gui: could not read {shown}: {e}");
            (None, "kimün".into())
        }
    }
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
    let (text, title) = initial_text();
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
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
                let editor = cx.new(|cx| EditorView::new(text, theme.clone(), vim, cx));
                let workspace =
                    cx.new(|cx| Workspace::new(editor, theme, layout, LayoutStore::UserConfig, cx));
                // The layout starts with the editor focused.
                workspace.update(cx, |workspace, cx| workspace.apply_focus(window, cx));
                workspace
            })
            .expect("failed to open window");
        });
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the glob brings in `gpui_kit::test`, which shadows `#[test]`.
    use super::{AppSettings, EditorBackendSetting, uses_vim};

    #[test]
    fn only_the_vim_backend_runs_vim() {
        assert!(uses_vim(EditorBackendSetting::Vim));
        assert!(!uses_vim(EditorBackendSetting::Plain));
        // No nvim in the GUI: the TUI's own fallback, the plain editor.
        assert!(!uses_vim(EditorBackendSetting::Nvim));
        assert!(!uses_vim(AppSettings::default().editor_backend));
    }
}
