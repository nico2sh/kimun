//! The window's title bar (M10a): on Linux and Windows the GUI draws its own
//! title bar in the theme's colours (client-side decorations) unless
//! `[gui] window_decorations = "system"` asks for the operating system's;
//! on macOS the native traffic lights stay, over a themed bar that only
//! carries the title and the drag area.
//!
//! **What is the kit's.** The bar itself is `gpui_kit::component::TitleBar`:
//! its layout, the drag area, the double-click, the window menu, the
//! minimize / maximize / restore / close controls with their per-platform
//! behaviour (Windows hit-test regions, Linux click handlers, macOS traffic
//! lights), and the window frame (`Root` wraps every window in the kit's
//! `window_border`). kimün configures it: the theme (`theme::apply_to_kit`
//! gives it the `title_bar*` and `danger*` roles), the background, the
//! title content, and the close hook ([`Workspace::request_close`]). What is
//! kimün's: *whether* the bar shows ([`bar_shown`]) and the window options
//! ([`window_options`]).
//!
//! **Where it sits.** At the very top of the workspace's root, above every
//! view: the panels, the empty states, and the setup assistant and
//! Preferences takeovers. Fullscreen (the operating system's or the
//! compositor's: kimün has no fullscreen key of its own) hides it. GPUI
//! redraws the whole window when it is activated or deactivated and when
//! its bounds change (a fullscreen change arrives as one), so the bar's
//! inactive colour and its fullscreen check are always current.
//!
//! **Close.** Linux: the kit's close button calls the hook, which goes
//! through the window's close guard: unsaved notes are flushed, dirty
//! Preferences ask first, and a refused close keeps the window. Windows:
//! the window itself closes, and asks the same guard.
//!
//! **Keyboard.** The controls are mouse chrome, like the operating system's:
//! not in the Tab order. Their keyboard path is the existing chords (Quit
//! closes through the same guard).
//!
//! **Testing seam.** What kimün itself reads of the window (the
//! decorations, fullscreen) goes through [`WindowChrome`], so tests swap in
//! a fake.

use std::rc::Rc;

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::TitleBar as KitTitleBar;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Decorations, Entity, InteractiveElement, IntoElement,
    ParentElement, Rgba, Role, SharedString, StatefulInteractiveElement, Styled, Window,
    WindowDecorations, WindowOptions, div, img,
};
use kimun_notes::settings::gui::WindowDecorations as DecorationsSetting;
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use crate::{assets, startup, theme};

/// The window's application id (Wayland `app_id`, X11 `WM_CLASS`): what a
/// desktop entry names to group the window and give it its icon. The repo
/// ships one in `assets/packaging/linux/kimun.desktop` (`StartupWMClass` is
/// this id; a test keeps the two in step) for a package to install.
pub(crate) const APP_ID: &str = "kimun";

/// The element ids of our wrapper around the kit's bar, and of the title.
pub(crate) const TITLE_BAR_ID: &str = "window-title-bar";
pub(crate) const TITLE_TEXT_ID: &str = "title-bar-title";

/// Which operating system's window conventions apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Os {
    /// Linux and the other X11 / Wayland desktops.
    Linux,
    Windows,
    Mac,
}

impl Os {
    /// The operating system this binary was built for.
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Who draws the window's title bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chrome {
    /// The operating system: its title bar and frame, no bar of ours.
    System,
    /// kimün: the kit's bar with its own window controls (Linux, Windows).
    Client,
    /// macOS: the native traffic lights over kimün's bar (title and drag).
    MacOverlay,
}

/// The chrome for the `window_decorations` setting on `os`: macOS ignores
/// the setting.
pub(crate) fn chrome_for(setting: DecorationsSetting, os: Os) -> Chrome {
    match (os, setting) {
        (Os::Mac, _) => Chrome::MacOverlay,
        (_, DecorationsSetting::Client) => Chrome::Client,
        (_, DecorationsSetting::System) => Chrome::System,
    }
}

/// The options the window opens with for `chrome`, titled `title`. The
/// icon (X11 only) is `main`'s to add.
pub(crate) fn window_options(chrome: Chrome, title: SharedString) -> WindowOptions {
    let titlebar = match chrome {
        Chrome::System => gpui_kit::TitlebarOptions {
            title: Some(title),
            ..Default::default()
        },
        // `appears_transparent` hides the title bar on Windows (and on
        // macOS); Linux reads `window_decorations` instead.
        Chrome::Client => gpui_kit::TitlebarOptions {
            title: Some(title),
            appears_transparent: true,
            traffic_light_position: None,
        },
        // The kit's own options for its bar: transparent, the traffic
        // lights where its title bar leaves room for them.
        Chrome::MacOverlay => gpui_kit::TitlebarOptions {
            title: Some(title),
            ..KitTitleBar::title_bar_options()
        },
    };
    WindowOptions {
        titlebar: Some(titlebar),
        window_decorations: (chrome == Chrome::Client).then_some(WindowDecorations::Client),
        // The kit's bar moves the window itself (`start_window_move`), so
        // AppKit must not treat it as its own drag region (macOS only).
        app_owns_titlebar_drag: chrome == Chrome::MacOverlay,
        app_id: Some(APP_ID.to_string()),
        ..Default::default()
    }
}

/// Whether the bar shows: never in fullscreen or with the system's chrome;
/// on Linux only while the compositor really granted client decorations
/// (Windows reports none, its bar is ours whenever we asked).
pub(crate) fn bar_shown(
    chrome: Chrome,
    os: Os,
    decorations: Decorations,
    fullscreen: bool,
) -> bool {
    if fullscreen {
        return false;
    }
    match chrome {
        Chrome::System => false,
        Chrome::MacOverlay => true,
        Chrome::Client => os != Os::Linux || matches!(decorations, Decorations::Client { .. }),
    }
}

/// The glyphs of every window control (the kit draws them), for the bundle
/// test.
#[cfg(test)]
pub(crate) const WINDOW_CONTROL_ICONS: [gpui_kit::assets::IconName; 4] = {
    use gpui_kit::assets::IconName;
    [
        IconName::WindowMinimize,
        IconName::WindowMaximize,
        IconName::WindowRestore,
        IconName::WindowClose,
    ]
};

/// The window, as kimün reads and drives it.
pub(crate) trait WindowChrome {
    fn decorations(&self, window: &Window) -> Decorations;
    fn is_fullscreen(&self, window: &Window) -> bool;
}

/// The real window.
struct PlatformWindow;

impl WindowChrome for PlatformWindow {
    fn decorations(&self, window: &Window) -> Decorations {
        window.window_decorations()
    }

    fn is_fullscreen(&self, window: &Window) -> bool {
        window.is_fullscreen()
    }
}

/// The bar's state, kept by the workspace.
pub(crate) struct TitleBar {
    chrome: Chrome,
    os: Os,
    window: Rc<dyn WindowChrome>,
    /// The window title, as the operating system's title shows it too.
    pub(crate) title: SharedString,
}

impl TitleBar {
    /// The system's chrome (no bar) until `main` says otherwise.
    pub(crate) fn new() -> Self {
        Self {
            chrome: Chrome::System,
            os: Os::current(),
            window: Rc::new(PlatformWindow),
            title: startup::APP_NAME.into(),
        }
    }
}

/// The colours kimün itself sets on the bar. The controls' and the hairline
/// are the kit theme's ([`theme::apply_to_kit`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BarColors {
    /// The bar: the rail's and the slots' surface, so the chrome reads as
    /// one band with them.
    pub(crate) surface: Rgba,
    /// The title: the text colour, muted while the window is inactive.
    pub(crate) title: Rgba,
}

/// The bar's colours in `t`, for an `active` window or not.
pub(crate) fn bar_colors(t: &Theme, active: bool) -> BarColors {
    BarColors {
        surface: theme::panel_background(t),
        title: if active {
            theme::foreground(t)
        } else {
            theme::muted(t)
        },
    }
}

/// What the kit's close button runs (Linux): the window's close guard, as
/// the operating system's close button reaches it.
pub(crate) fn close_hook(
    workspace: Entity<Workspace>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        workspace.update(cx, |this, cx| this.request_close(window, cx));
    }
}

impl Workspace {
    /// Draw the window chrome `chrome` says (`main`, from the setting).
    pub(crate) fn set_chrome(&mut self, chrome: Chrome) {
        self.titlebar.chrome = chrome;
    }

    /// Drive `window` instead of the real window (tests).
    #[cfg(test)]
    pub(crate) fn set_window_chrome(&mut self, window: Rc<dyn WindowChrome>) {
        self.titlebar.window = window;
    }

    /// The window title: the bar's, and the operating system's (the task
    /// switcher, the accessibility tree).
    pub(super) fn set_title(&mut self, title: String, window: &mut Window) {
        window.set_window_title(&title);
        self.titlebar.title = title.into();
    }

    /// The bar, when it shows (see [`bar_shown`]): the kit's title bar,
    /// carrying the logo and the title, closing through the close guard.
    pub(super) fn render_title_bar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let bar = &self.titlebar;
        let shown = bar_shown(
            bar.chrome,
            bar.os,
            bar.window.decorations(window),
            bar.window.is_fullscreen(window),
        );
        if !shown {
            return None;
        }
        let colors = bar_colors(&theme::current(cx), window.is_window_active());
        let title = bar.title.clone();
        let workspace = cx.entity();
        let content = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .min_w_0()
            // Decorative: the title names the window.
            .child(img(assets::logo()).size_4().flex_none())
            .child(
                div()
                    .id(TITLE_TEXT_ID)
                    .test_support()
                    .role(Role::Label)
                    .aria_label(title.clone())
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(colors.title)
                    .child(title.clone()),
            );
        let kit_bar = KitTitleBar::new()
            .bg(colors.surface)
            .on_close_window(close_hook(workspace))
            .child(content);
        Some(
            div()
                .id(TITLE_BAR_ID)
                .test_support()
                .role(Role::TitleBar)
                .aria_label(title)
                .flex_none()
                .w_full()
                .child(kit_bar)
                .into_any_element(),
        )
    }
}
