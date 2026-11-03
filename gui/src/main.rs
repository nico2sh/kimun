//! Native GPUI frontend for kimün (ADR-0049). M3: one editor pane, plain or
//! vim as the TUI's `editor_backend` setting says, in the user's theme, with
//! the TUI's markdown rendering (sigils hidden off the caret, revealed under
//! it) and the TUI's find/replace bar (M4), optionally showing a file named
//! on the command line. Nothing is ever saved.

mod editor_element;
mod editor_view;
mod find_bar;
#[cfg(test)]
mod fixture;
mod fonts;
mod grid;
mod input;
mod row_cache;
mod row_paint;
mod theme;
#[cfg(test)]
mod ui_tests;

use gpui_kit::*;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, EditorBackendSetting};

use crate::editor_view::EditorView;

/// The config, read the TUI's way but never written, migrated or renamed.
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
    let settings = load_settings();
    let theme = load_theme(&settings);
    let vim = uses_vim(settings.editor_backend);
    let (text, title) = initial_text();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            editor_view::init(cx);
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some(title),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let view = cx.new(|cx| EditorView::new(text, theme, vim, cx));
                // The editor is the whole window: it starts with the keyboard.
                window.focus(&view.read(cx).focus_handle(cx), cx);
                view
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
