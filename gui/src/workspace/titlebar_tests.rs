//! Tests for the window's title bar (M10a, [`titlebar`]): the pure choices
//! (chrome, window options, when the bar shows), and UI integration over a
//! real `Workspace` with the window state faked. The bar's own behaviour
//! (drag, double-click, window menu, the controls' per-platform handling) is
//! the kit's `TitleBar`, not tested here: only what kimün configures is.
//!
//! [`titlebar`]: super::titlebar
#![cfg(test)]

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, ClickEvent, Decorations, Role, TestAppContext, Tiling, Window,
    WindowDecorations, px,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::settings::gui::WindowDecorations as DecorationsSetting;
use kimun_notes::settings::themes::Theme;

use super::LayoutStore;
use super::onboarding::ONBOARDING_ID;
use super::preferences::{DISCARD_ID, Section, UPDATE_CHECK_ID};
use super::preferences_tests::{on_vault, vault};
use super::reopen_tests::{BETA, open_on_a, pair, set_current};
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use super::titlebar::{
    APP_ID, Chrome, Os, TITLE_BAR_ID, TITLE_TEXT_ID, WindowChrome, bar_colors, bar_shown,
    chrome_for, close_hook, window_options,
};
use crate::startup::{FirstNote, Start};
use crate::status_row::ROW_ID;
use crate::test_vault::{NAME, test_vault};
use crate::theme;

// ── the pure choices ─────────────────────────────────────────────────────

#[test]
fn macos_ignores_the_setting_and_the_others_follow_it() {
    for setting in [DecorationsSetting::Client, DecorationsSetting::System] {
        assert_eq!(chrome_for(setting, Os::Mac), Chrome::MacOverlay);
    }
    for os in [Os::Linux, Os::Windows] {
        assert_eq!(chrome_for(DecorationsSetting::Client, os), Chrome::Client);
        assert_eq!(chrome_for(DecorationsSetting::System, os), Chrome::System);
    }
    // The default setting draws our bar.
    assert_eq!(
        chrome_for(DecorationsSetting::default(), Os::Linux),
        Chrome::Client
    );
}

#[test]
fn the_window_options_follow_the_chrome() {
    let system = window_options(Chrome::System, "t".into());
    let titlebar = system.titlebar.unwrap();
    assert_eq!(titlebar.title.as_deref(), Some("t"));
    assert!(!titlebar.appears_transparent, "the OS title bar stays");
    assert_eq!(system.window_decorations, None);
    assert!(!system.app_owns_titlebar_drag);

    let client = window_options(Chrome::Client, "t".into());
    assert!(client.titlebar.as_ref().unwrap().appears_transparent);
    assert_eq!(client.titlebar.unwrap().traffic_light_position, None);
    assert_eq!(client.window_decorations, Some(WindowDecorations::Client));
    assert!(!client.app_owns_titlebar_drag);

    let mac = window_options(Chrome::MacOverlay, "t".into());
    let titlebar = mac.titlebar.unwrap();
    assert!(titlebar.appears_transparent);
    assert!(titlebar.traffic_light_position.is_some());
    assert_eq!(mac.window_decorations, None);
    assert!(
        mac.app_owns_titlebar_drag,
        "the bar moves the window itself"
    );

    for chrome in [Chrome::System, Chrome::Client, Chrome::MacOverlay] {
        assert_eq!(
            window_options(chrome, "t".into()).app_id.as_deref(),
            Some(APP_ID)
        );
    }
}

#[test]
fn the_bar_shows_only_where_it_is_ours() {
    let client = Decorations::Client {
        tiling: Tiling::default(),
    };
    let server = Decorations::Server;
    // Linux: only when the compositor granted client decorations.
    assert!(bar_shown(Chrome::Client, Os::Linux, client, false));
    assert!(!bar_shown(Chrome::Client, Os::Linux, server, false));
    // Windows reports server decorations; the bar is ours all the same.
    assert!(bar_shown(Chrome::Client, Os::Windows, server, false));
    assert!(bar_shown(Chrome::MacOverlay, Os::Mac, server, false));
    for os in [Os::Linux, Os::Windows, Os::Mac] {
        assert!(!bar_shown(Chrome::System, os, client, false), "{os:?}");
        // Fullscreen hides it whatever the chrome.
        for chrome in [Chrome::Client, Chrome::MacOverlay] {
            assert!(!bar_shown(chrome, os, client, true), "{chrome:?} {os:?}");
        }
    }
}

#[test]
fn the_colours_come_from_the_theme_roles() {
    let t = Theme::default();
    let active = bar_colors(&t, true);
    let inactive = bar_colors(&t, false);
    assert_eq!(active.surface, theme::panel_background(&t));
    assert_eq!(active.title, theme::foreground(&t));
    assert_eq!(inactive.title, theme::muted(&t), "inactive: muted");
}

// ── the fake window ──────────────────────────────────────────────────────

/// Plays the window's state back.
struct FakeWindow {
    decorations: Cell<Decorations>,
    fullscreen: Cell<bool>,
}

impl FakeWindow {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            decorations: Cell::new(Decorations::Client {
                tiling: Tiling::default(),
            }),
            fullscreen: Cell::new(false),
        })
    }
}

impl WindowChrome for FakeWindow {
    fn decorations(&self, _: &Window) -> Decorations {
        self.decorations.get()
    }

    fn is_fullscreen(&self, _: &Window) -> bool {
        self.fullscreen.get()
    }
}

impl Started {
    /// Draw our own bar over the fake window, as `main` does on Linux.
    fn tb_client(&self, cx: &mut TestAppContext) -> Rc<FakeWindow> {
        let fake = FakeWindow::new();
        let window: Rc<dyn WindowChrome> = fake.clone();
        self.workspace.update(cx, |w, cx| {
            w.set_chrome(Chrome::Client);
            w.set_window_chrome(window);
            cx.notify();
        });
        cx.run_until_parked();
        fake
    }

    fn tb_on_window<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window) -> R) -> R {
        let r = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                f(window)
            })
            .unwrap();
        cx.run_until_parked();
        r
    }

    /// The title the bar shows.
    fn tb_title(&self, cx: &mut TestAppContext) -> Option<String> {
        self.tb_on_window(cx, |window| {
            window
                .try_find(TITLE_TEXT_ID)
                .filter(|e| e.visible())
                .and_then(|e| e.label().map(str::to_string))
        })
    }

    /// The kit's close button was pressed: its hook runs.
    fn tb_close(&self, cx: &mut TestAppContext) {
        let workspace = self.workspace.clone();
        cx.update_window(self.handle, |_, window, cx| {
            close_hook(workspace)(&ClickEvent::default(), window, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn tb_closed(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, _, _| ()).is_err()
    }
}

/// A window with no vault (it would not open): the bar's bare window.
fn bare(cx: &mut TestAppContext) -> Started {
    open_started(
        cx,
        Start::OpenFailed("no vault here".into()),
        Default::default(),
    )
}

// ── presence ─────────────────────────────────────────────────────────────

/// The system's chrome draws no bar (every other test window runs so).
#[gpui_kit::test]
fn the_system_chrome_draws_no_bar(cx: &mut TestAppContext) {
    let started = bare(cx);
    assert!(!started.shows(TITLE_BAR_ID, cx));
}

/// Client chrome: the bar is at the top, titled, a title bar for assistive
/// technology; the body takes the rest.
#[gpui_kit::test]
fn the_client_chrome_draws_the_bar_on_top(cx: &mut TestAppContext) {
    let started = bare(cx);
    started.tb_client(cx);
    assert!(started.shows(TITLE_BAR_ID, cx));
    assert_eq!(started.tb_title(cx).as_deref(), Some("kimün"));
    let (bar, role, footer, height) = started.tb_on_window(cx, |window| {
        let bar = window.find(TITLE_BAR_ID);
        (
            bar.bounds(),
            bar.role(),
            window.find(ROW_ID).bounds(),
            window.viewport_size().height,
        )
    });
    assert_eq!(role, Some(Role::TitleBar));
    assert_eq!(bar.origin.y, px(0.), "at the very top");
    assert!(footer.origin.y >= bar.bottom(), "the body is below the bar");
    assert!(
        footer.bottom() <= height + px(0.5),
        "and still fits the window: {footer:?} in {height:?}"
    );
}

/// On Linux a compositor that refused client decorations draws its own bar:
/// ours stays away.
#[gpui_kit::test]
fn no_bar_when_the_compositor_draws_one(cx: &mut TestAppContext) {
    let started = bare(cx);
    let fake = started.tb_client(cx);
    fake.decorations.set(Decorations::Server);
    assert!(!started.shows(TITLE_BAR_ID, cx));
}

/// Fullscreen from outside (the compositor, the window manager) hides the
/// bar; leaving it brings the bar back. F11 is no longer kimün's: it
/// changes nothing.
#[gpui_kit::test]
fn fullscreen_hides_the_bar_and_f11_is_not_ours(cx: &mut TestAppContext) {
    let started = bare(cx);
    let fake = started.tb_client(cx);
    fake.fullscreen.set(true);
    assert!(!started.shows(TITLE_BAR_ID, cx), "fullscreen: no bar");
    fake.fullscreen.set(false);
    assert!(started.shows(TITLE_BAR_ID, cx));
    cx.update_window(started.handle, |_, window, cx| window.press("f11", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!fake.fullscreen.get(), "F11 toggles nothing");
    assert!(
        cx.update_window(started.handle, |_, window, _| !window.is_fullscreen())
            .unwrap()
    );
    assert!(started.shows(TITLE_BAR_ID, cx));
}

/// The bar sits over the setup assistant too.
#[gpui_kit::test]
fn the_bar_is_over_the_setup_assistant(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let started = open_started(cx, Start::NoWorkspace, Default::default());
    started.tb_client(cx);
    assert!(started.shows(ONBOARDING_ID, cx));
    assert!(started.shows(TITLE_BAR_ID, cx));
    assert_eq!(started.tb_title(cx).as_deref(), Some("kimün"));
}

// ── the title ────────────────────────────────────────────────────────────

/// The bar's title is the workspace's, as the OS title is, and follows a
/// switch to another workspace.
#[gpui_kit::test]
fn the_title_follows_a_workspace_switch(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let started = open_on_a(cx, &rt, &p, LayoutStore::Off);
    started.tb_client(cx);
    assert_eq!(started.tb_title(cx).as_deref(), Some("a - kimün"));
    set_current(&p.settings, "b");
    started.sw_reopen_to(&rt, Some(BETA), cx);
    assert_eq!(started.tb_title(cx).as_deref(), Some("b - kimün"));
    assert_eq!(started.sw_title(cx).as_deref(), Some("b - kimün"));
}

// ── the close control ────────────────────────────────────────────────────
//
// The kit draws its controls from the real window's decorations, and the
// test window reports server ones: no button to click. The tests run the
// hook the kit's close button calls instead.

/// Close with nothing to save closes the window.
#[gpui_kit::test]
fn close_with_nothing_dirty_closes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "alpha")], &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    started.tb_client(cx);
    assert_eq!(
        started.tb_title(cx),
        Some(format!("{NAME} - kimün")),
        "titled by the workspace"
    );
    started.tb_close(cx);
    assert!(started.tb_closed(cx));
}

/// Close with unsaved edits goes through the close guard: the window waits
/// for the save, then closes.
#[gpui_kit::test]
fn close_with_a_dirty_note_saves_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "alpha")], &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| started.text(cx) == "alpha");
    started.tb_client(cx);
    cx.update_window(started.handle, |_, window, cx| window.input("X", cx))
        .unwrap();
    cx.run_until_parked();
    started.tb_close(cx);
    assert!(!started.tb_closed(cx), "the close waits for the save");
    assert_eq!(started.flash(cx).as_deref(), Some("Saving a.md…"));
    drive(&rt, cx, |cx| started.tb_closed(cx));
    let on_disk = rt
        .block_on(v.vault.get_note_text(&VaultPath::new("a.md")))
        .unwrap();
    assert_eq!(on_disk, "Xalpha");
}

/// Over Preferences: the bar is there, and close with a changed draft asks
/// first and keeps the window; Discard then closes it.
#[gpui_kit::test]
fn close_over_dirty_preferences_asks_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = on_vault(cx, &rt, &v);
    s.pf_route(cx);
    s.pf_loaded(&rt, cx);
    s.tb_client(cx);
    assert!(s.shows(TITLE_BAR_ID, cx), "over Preferences too");
    s.pf_section(cx, Section::Display);
    s.pf_click(UPDATE_CHECK_ID, cx);
    s.tb_close(cx);
    assert!(!s.tb_closed(cx));
    assert!(s.pf_is_open(cx));
    assert_eq!(
        s.pf_read(cx, |v| v.confirm()).map(|c| c.close_window),
        Some(true)
    );
    s.pf_click(DISCARD_ID, cx);
    drive(&rt, cx, |cx| s.tb_closed(cx));
}
