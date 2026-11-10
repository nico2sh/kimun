//! The app level: what the menu bar and the dock ask for, with or without a window.
//!
//! On macOS closing the last window does not end the app: it stays in the dock, and a
//! click on its icon opens a window again ([`crate::reopen`]). Only a quit ends it
//! (Cmd+Q, the palette's quit, the leader's quit). With no window there is no workspace
//! to ask, so the app-level `Quit` here ends the app at once; with a window it goes
//! through the workspace (the open note is saved first).
//!
//! Everywhere else closing the window ends the app, as before, and none of this is
//! installed.

use gpui_kit::{AnyWindowHandle, App, Global, WeakEntity, actions};

use crate::Launch;
use crate::workspace::Workspace;

actions!(kimun, [QuitApp, NewWindow]);

/// The main window, for the app-level quit to reach its workspace.
#[derive(Default)]
struct MainWindow(Option<(AnyWindowHandle, WeakEntity<Workspace>)>);

impl Global for MainWindow {}

/// Remember `workspace`, in `window`, as the main window.
pub(crate) fn set_main(cx: &mut App, window: AnyWindowHandle, workspace: WeakEntity<Workspace>) {
    cx.set_global(MainWindow(Some((window, workspace))));
}

/// Quit the app: through the main window's workspace when there is one, so the open
/// note is saved first; at once when there is none.
pub(crate) fn quit_app(cx: &mut App) {
    let main = cx.try_global::<MainWindow>().and_then(|m| m.0.clone());
    match main {
        Some((window, workspace)) if cx.windows().contains(&window) => {
            window
                .update(cx, |_, window, cx| {
                    workspace
                        .update(cx, |workspace, cx| workspace.request_quit(window, cx))
                        .ok();
                })
                .ok();
        }
        _ => cx.quit(),
    }
}

/// The menu bar, Cmd+Q, and "New Window". macOS only (see the module docs).
pub(crate) fn install(cx: &mut App, launch: &Launch) {
    if !cfg!(target_os = "macos") {
        return;
    }
    cx.on_action(|_: &QuitApp, cx| quit_app(cx));
    let launch = launch.clone();
    cx.on_action(move |_: &NewWindow, cx| crate::reopen(&launch, cx));
    cx.bind_keys([gpui_kit::KeyBinding::new("cmd-q", QuitApp, None)]);
    cx.set_menus([
        gpui_kit::Menu {
            name: "Kimün".into(),
            items: vec![gpui_kit::MenuItem::action("Quit Kimün", QuitApp)],
            disabled: false,
        },
        gpui_kit::Menu {
            name: "File".into(),
            items: vec![gpui_kit::MenuItem::action("New Window", NewWindow)],
            disabled: false,
        },
    ]);
}
