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

use gpui_kit::{AnyWindowHandle, App, Context, Global, WeakEntity, Window, actions};

use crate::Launch;
use crate::workspace::Workspace;

actions!(
    kimun,
    [
        QuitApp,
        NewWindow,
        CloseWindow,
        HideApp,
        HideOthers,
        MinimizeWindow,
        ZoomWindow
    ]
);

/// The main window, for the app-level quit to reach its workspace.
#[derive(Default)]
struct MainWindow(Option<(AnyWindowHandle, WeakEntity<Workspace>)>);

impl Global for MainWindow {}

/// Remember `workspace`, in `window`, as the main window.
pub(crate) fn set_main(cx: &mut App, window: AnyWindowHandle, workspace: WeakEntity<Workspace>) {
    cx.set_global(MainWindow(Some((window, workspace))));
}

/// Run `f` on the main window's workspace, once the action that asked is over. An
/// app-level action runs inside the active window's own update, where that window is
/// taken out of the app: a nested `window.update` would fail. `fallback` runs when the
/// window is gone or out of reach, so the request is never silently dropped.
fn on_main_workspace(
    cx: &mut App,
    f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    fallback: impl FnOnce(&mut App) + 'static,
) {
    let main = cx.try_global::<MainWindow>().and_then(|m| m.0.clone());
    match main {
        Some((window, workspace)) if cx.windows().contains(&window) => {
            cx.defer(move |cx| {
                let reached = window.update(cx, |_, window, cx| {
                    workspace
                        .update(cx, |workspace, cx| f(workspace, window, cx))
                        .is_ok()
                });
                if !matches!(reached, Ok(true)) {
                    fallback(cx);
                }
            });
        }
        _ => fallback(cx),
    }
}

/// Quit the app: through the main window's workspace when there is one, so the open
/// note is saved first; at once when there is none.
pub(crate) fn quit_app(cx: &mut App) {
    on_main_workspace(
        cx,
        |workspace, window, cx| workspace.request_quit(window, cx),
        |cx| cx.quit(),
    );
}

/// Close the main window as its close button does: the open note is saved first, and
/// the app stays in the dock.
pub(crate) fn close_window(cx: &mut App) {
    on_main_workspace(
        cx,
        |workspace, window, cx| workspace.request_close(window, cx),
        |_| {},
    );
}

/// Run `f` on the active window, once the action that asked is over.
fn on_active_window(cx: &mut App, f: impl FnOnce(&mut Window) + 'static) {
    cx.defer(move |cx| {
        if let Some(window) = cx.active_window() {
            window.update(cx, |_, window, _| f(window)).ok();
        }
    });
}

/// The menu bar, Cmd+Q, and "New Window". macOS only (see the module docs).
pub(crate) fn install(cx: &mut App, launch: &Launch) {
    if !cfg!(target_os = "macos") {
        return;
    }
    cx.on_action(|_: &QuitApp, cx| quit_app(cx));
    let launch = launch.clone();
    cx.on_action(move |_: &NewWindow, cx| crate::reopen(&launch, cx));
    cx.on_action(|_: &CloseWindow, cx| close_window(cx));
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &MinimizeWindow, cx| on_active_window(cx, |w| w.minimize_window()));
    cx.on_action(|_: &ZoomWindow, cx| on_active_window(cx, |w| w.zoom_window()));
    cx.bind_keys([
        gpui_kit::KeyBinding::new("cmd-q", QuitApp, None),
        gpui_kit::KeyBinding::new("cmd-w", CloseWindow, None),
        gpui_kit::KeyBinding::new("cmd-h", HideApp, None),
        gpui_kit::KeyBinding::new("cmd-alt-h", HideOthers, None),
        gpui_kit::KeyBinding::new("cmd-m", MinimizeWindow, None),
    ]);
    cx.set_menus([
        gpui_kit::Menu {
            name: "Kimün".into(),
            items: vec![
                gpui_kit::MenuItem::action("Hide Kimün", HideApp),
                gpui_kit::MenuItem::action("Hide Others", HideOthers),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Quit Kimün", QuitApp),
            ],
            disabled: false,
        },
        gpui_kit::Menu {
            name: "File".into(),
            items: vec![
                gpui_kit::MenuItem::action("New Window", NewWindow),
                gpui_kit::MenuItem::action("Close Window", CloseWindow),
            ],
            disabled: false,
        },
        gpui_kit::Menu {
            name: "Window".into(),
            items: vec![
                gpui_kit::MenuItem::action("Minimize", MinimizeWindow),
                gpui_kit::MenuItem::action("Zoom", ZoomWindow),
            ],
            disabled: false,
        },
    ]);
}
