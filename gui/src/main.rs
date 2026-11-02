//! Native GPUI frontend for kimün (ADR-0049). M1: one plain (non-vim) editor
//! pane in the user's theme, optionally showing a file named on the command
//! line. Nothing is ever saved.

mod editor_element;
mod editor_view;
mod grid;
mod input;
mod theme;
#[cfg(test)]
mod ui_tests;

use gpui_kit::*;
use kimun_notes::settings::AppSettings;
use kimun_notes::settings::themes::Theme;

use crate::editor_view::EditorView;

/// Same file, same theme lookup as the TUI (ADR-0049 §6). The config is never
/// written, migrated or renamed here; the only possible side effect is
/// `theme_list()` creating an empty config directory. Not `get_theme()`: that
/// applies `adapt_to_terminal()`, which is a terminal-only concern.
fn load_theme() -> Theme {
    let settings = AppSettings::load_read_only().unwrap_or_else(|e| {
        eprintln!("kimun-gui: could not read the config ({e}); using the default theme");
        AppSettings::default()
    });
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
    let theme = load_theme();
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
                let view = cx.new(|cx| EditorView::new(text, theme, cx));
                // The editor is the whole window: it starts with the keyboard.
                window.focus(&view.read(cx).focus_handle(cx), cx);
                view
            })
            .expect("failed to open window");
        });
}
