//! UI integration tests for the setup assistant (M8c, [`onboarding`]): a
//! real `Workspace` on a real temp config (no workspace: the first run) or a
//! real temp vault (a re-run), driven by keys and clicks.
//!
//! Like the startup tests, each test owns a current-thread tokio runtime and
//! drives it between GPUI's `run_until_parked` (`start_tests::drive`): the
//! model, the folder listings and the finish run on tokio.
//!
//! [`onboarding`]: super::onboarding
#![cfg(test)]

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Context, Entity, Focusable as _, TestAppContext, VisualTestContext, Window,
    WindowOptions,
};
use kimun_core::system::SystemPath;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::leader_route::LeaderRoute;
use kimun_notes::logic::onboarding::{OnbStep, OnboardingModel};
use kimun_notes::settings::gui::GuiSettings;
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, EditorBackendSetting, SharedSettings};
use tokio::runtime::Runtime;

use super::folder_chooser::{FolderChooser, Listing};
use super::onboarding::{
    BACK_ID, CANCEL_ID, CHOOSE_FOLDER_FIRST, CHOOSE_FOLDER_ID, CONFIRM_NO_ID, Confirm, LOGO_ID,
    NEXT_ID, ONBOARDING_ID, OnboardingView, WORDMARK, WORDMARK_ID, WORDMARK_U_ID, WordmarkParts,
    bounce_offset, chooser_row_id, wordmark_parts,
};
use super::start_tests::{Started, drive, runtime};
use super::{LayoutStore, Workspace};
use crate::editor_view::{self, EditorView};
use crate::keymap::{self, Keymap, Platform};
use crate::shell_layout;
use crate::startup::{FirstNote, Start, VaultStart};
use crate::test_vault::{NAME, TestVault, test_vault};
use crate::theme;

/// A config with no workspace, its cache and history beside it: a first run.
struct Fresh {
    settings: SharedSettings,
    config: PathBuf,
    root: PathBuf,
    _dir: tempfile::TempDir,
}

fn fresh() -> Fresh {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let config = root.join("config.toml");
    std::fs::write(
        &config,
        "config_version = 7\ncache_dir = \"cache\"\nhistory_dir = \"history\"\n",
    )
    .unwrap();
    let settings = Arc::new(RwLock::new(
        AppSettings::load_from_file(config.clone()).unwrap(),
    ));
    Fresh {
        settings,
        config,
        root,
        _dir: dir,
    }
}

/// The window as `main` builds it, on `start`, persisting to `store`.
fn open(
    cx: &mut TestAppContext,
    start: Start,
    settings: SharedSettings,
    store: LayoutStore,
) -> Started {
    cx.update(|cx| {
        gpui_kit::init(cx);
        editor_view::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let layout = shell_layout::restore(&GuiSettings::default());
    let (handle, workspace) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let editor = cx.new(|cx| EditorView::on_channel(false, tx.clone(), cx));
                let workspace = cx.new(|cx| {
                    let mut workspace =
                        Workspace::new(editor, Theme::default(), layout, store, window, cx);
                    workspace.start(start, settings, tx, rx, window, cx);
                    workspace
                });
                workspace.update(cx, |w, cx| {
                    w.apply_focus(window, cx);
                    w.guard_close(window, cx);
                });
                workspace
            })
        })
        .expect("open test window");
    Started { handle, workspace }
}

/// A first run: the window started with no workspace, its assistant read.
fn first_run(cx: &mut TestAppContext, rt: &Runtime, f: &Fresh) -> Started {
    let started = open(
        cx,
        Start::NoWorkspace,
        f.settings.clone(),
        LayoutStore::File(f.config.clone()),
    );
    started.ob_loaded(rt, cx);
    started
}

/// A re-run: the window on `v`, the assistant opened from the leader route.
fn rerun(cx: &mut TestAppContext, rt: &Runtime, v: &TestVault) -> Started {
    let start = Start::Vault(VaultStart {
        vault: v.vault.clone(),
        name: NAME.into(),
        first: FirstNote::Restore,
    });
    let config = v.settings.read().unwrap().config_file.clone().unwrap();
    let started = open(cx, start, v.settings.clone(), LayoutStore::File(config));
    drive(rt, cx, |cx| !started.indexing(cx));
    assert!(!started.ob_is_open(cx), "a vault: no assistant of its own");
    started.ob_on_window(cx, |w, window, cx| {
        assert_eq!(
            w.route_block(&LeaderRoute::OpenOnboarding, Panel::Editor, cx),
            None
        );
        w.perform_route(LeaderRoute::OpenOnboarding, window, cx)
    });
    started.ob_loaded(rt, cx);
    started
}

impl Started {
    fn ob_view(&self, cx: &TestAppContext) -> Entity<OnboardingView> {
        cx.read(|cx| self.workspace.read(cx).onboarding.clone())
            .expect("the assistant is open")
    }

    fn ob_is_open(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).onboarding.is_some())
    }

    fn ob_read_view<R>(&self, cx: &TestAppContext, f: impl FnOnce(&OnboardingView) -> R) -> R {
        let view = self.ob_view(cx);
        cx.read(|cx| f(view.read(cx)))
    }

    /// Read the folder chooser, while it shows.
    fn ob_chooser<R>(&self, cx: &TestAppContext, f: impl FnOnce(&FolderChooser) -> R) -> Option<R> {
        let chooser = self.ob_read_view(cx, |v| v.chooser().cloned())?;
        Some(cx.read(|cx| f(chooser.read(cx))))
    }

    fn ob_model<R>(&self, cx: &TestAppContext, f: impl FnOnce(&OnboardingModel) -> R) -> R {
        self.ob_read_view(cx, |v| f(v.model().expect("the model is read")))
    }

    fn ob_step(&self, cx: &TestAppContext) -> OnbStep {
        self.ob_model(cx, |m| m.step)
    }

    /// Run `f` on the assistant with the window.
    fn ob_with_view(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut OnboardingView, &mut Window, &mut Context<OnboardingView>),
    ) {
        let view = self.ob_view(cx);
        cx.update_window(self.handle, |_, window, cx| {
            view.update(cx, |v, cx| f(v, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Run `f` on the workspace with the window.
    fn ob_on_window<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
    ) -> R {
        let workspace = self.workspace.clone();
        let r = cx
            .update_window(self.handle, |_, window, cx| {
                workspace.update(cx, |w, cx| f(w, window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        r
    }

    fn ob_press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn ob_typed(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn ob_click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn ob_disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).disabled() == Some(true)
        })
        .unwrap()
    }

    /// Drive until the assistant's model is read.
    fn ob_loaded(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| self.ob_read_view(cx, |v| v.model().is_some()));
    }

    /// Go to `step` with the Next key.
    fn ob_go_to(&self, step: OnbStep, cx: &mut TestAppContext) {
        while self.ob_step(cx) != step {
            let before = self.ob_step(cx);
            self.ob_press("right", cx);
            assert_ne!(self.ob_step(cx), before, "stuck on {before:?}");
        }
    }

    /// Choose `dir` as the workspace folder, as the chooser does.
    fn ob_choose(&self, dir: &std::path::Path, cx: &mut TestAppContext) {
        let dir = SystemPath::try_absolute(dir).unwrap();
        self.ob_with_view(cx, |v, window, cx| v.choose_directory(dir, window, cx));
    }

    /// Open the chooser on `dir`, driven until it is listed.
    fn ob_browse(&self, rt: &Runtime, dir: &std::path::Path, cx: &mut TestAppContext) {
        if self.ob_read_view(cx, |v| v.chooser().is_none()) {
            self.ob_press("b", cx);
        }
        let dir = SystemPath::try_absolute(dir).unwrap();
        self.ob_with_view(cx, |v, window, cx| v.browse(dir, window, cx));
        self.ob_listed(rt, cx);
    }

    fn ob_listed(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| {
            self.ob_chooser(cx, |c| !matches!(c.listing(), Listing::Loading))
                .unwrap_or(false)
        });
    }

    fn ob_chooser_dir(&self, cx: &TestAppContext) -> PathBuf {
        self.ob_chooser(cx, |c| c.dir().as_path().to_path_buf())
            .unwrap()
    }

    fn ob_chooser_rows(&self, cx: &TestAppContext) -> Vec<String> {
        self.ob_chooser(cx, |c| match c.listing() {
            Listing::Loaded(rows) => rows
                .iter()
                .map(|r| {
                    r.as_path()
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
            other => panic!("not listed: {other:?}"),
        })
        .unwrap()
    }

    fn ob_name_value(&self, cx: &TestAppContext) -> String {
        let input = self.ob_read_view(cx, |v| v.name_input().clone());
        cx.read(|cx| input.read(cx).value().to_string())
    }

    fn ob_window_theme(&self, cx: &TestAppContext) -> String {
        cx.read(|cx| crate::theme::current(cx).name.clone())
    }

    fn ob_shows_id(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }
}

/// Let the vault's background work (index, list loads, the restore) land
/// while the runtime is still up: nothing may reach tokio after the test.
fn settle(rt: &Runtime, cx: &mut TestAppContext) {
    let mut rounds = 0;
    drive(rt, cx, |_| {
        rounds += 1;
        rounds >= 40
    });
}

fn mkdirs(root: &std::path::Path, names: &[&str]) {
    for n in names {
        std::fs::create_dir_all(root.join(n)).unwrap();
    }
}

// ── entry ───────────────────────────────────────────────────────────────────

/// No workspace: the assistant has the window, on Welcome, as a first run;
/// nothing of the shell is drawn under it, and it has the keyboard.
#[gpui_kit::test]
fn no_workspace_starts_the_first_run(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    assert!(s.shows(ONBOARDING_ID, cx));
    assert!(!s.shows("rail", cx) && !s.shows("editor-panel", cx));
    assert_eq!(s.ob_step(cx), OnbStep::Welcome);
    assert!(s.ob_model(cx, |m| m.first_run));
    let view = s.ob_view(cx);
    let focused = cx.update_window(s.handle, |_, window, cx| {
        view.read(cx).focus_handle(cx).is_focused(window)
    });
    assert!(focused.unwrap(), "the assistant has the keyboard");
    assert!(s.ob_disabled(BACK_ID, cx), "nothing before Welcome");
}

/// From the leader route with a vault: a re-run. Its Workspace step lists
/// the workspaces read-only, with no folder chooser.
#[gpui_kit::test]
fn open_onboarding_with_a_vault_is_a_rerun(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "# A")], &["a.md"]));
    let s = rerun(cx, &rt, &v);
    assert!(s.shows(ONBOARDING_ID, cx));
    assert!(!s.shows("rail", cx));
    assert!(!s.ob_model(cx, |m| m.first_run));
    s.ob_go_to(OnbStep::Workspace, cx);
    assert!(s.ob_shows_id(format!("onboarding-ws-{NAME}"), cx));
    assert!(
        !s.shows(CHOOSE_FOLDER_ID, cx),
        "a re-run never changes workspaces"
    );
    s.ob_press("b", cx);
    assert!(s.ob_read_view(cx, |v| v.chooser().is_none()));
    s.ob_press("enter", cx);
    assert_eq!(
        s.ob_step(cx),
        OnbStep::Theme,
        "a re-run never needs a folder"
    );
    settle(&rt, cx);
}

/// Welcome draws the app logo, a labelled image at its fixed square; no
/// other step does, and it comes back with Welcome. The image's decode is
/// not awaited: the element and its size do not depend on it.
#[gpui_kit::test]
fn only_welcome_draws_the_logo(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    let logo = cx
        .update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            let e = window.find(LOGO_ID);
            (
                e.visible(),
                e.role(),
                e.label().map(str::to_owned),
                e.bounds(),
            )
        })
        .unwrap();
    assert!(logo.0, "Welcome draws the logo");
    assert_eq!(logo.1, Some(gpui_kit::Role::Image));
    assert_eq!(logo.2.as_deref(), Some("kimün logo"));
    assert_eq!(
        logo.3.size,
        gpui_kit::size(gpui_kit::px(112.), gpui_kit::px(112.))
    );
    s.ob_choose(&f.root.join("notes"), cx);
    for _ in 0..5 {
        s.ob_press("right", cx);
        let step = s.ob_step(cx);
        assert!(!s.ob_shows_id(LOGO_ID, cx), "{step:?} draws no logo");
    }
    assert_eq!(s.ob_step(cx), OnbStep::Summary);
    while s.ob_step(cx) != OnbStep::Welcome {
        s.ob_press("left", cx);
    }
    assert!(
        s.ob_shows_id(LOGO_ID, cx),
        "back on Welcome, the logo again"
    );
}

/// The wordmark under the logo: its accessible name the whole word, and
/// the frame that draws it.
fn wordmark(
    s: &Started,
    cx: &mut TestAppContext,
) -> (Option<String>, gpui_kit::Bounds<gpui_kit::Pixels>) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        let e = window.find(WORDMARK_ID);
        (e.label().map(str::to_owned), e.bounds())
    })
    .unwrap()
}

/// Whether Welcome asks for another frame now: the ü's spring is running.
fn bounces(s: &Started, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.simulate_next_frame(cx)
    })
    .unwrap()
        > 0
}

/// "Kimün" reads as one word, and its ü keeps bouncing while Welcome
/// shows: its line box holds while it moves, and it keeps asking for
/// frames (it loops). The test platform draws a frame only when asked and
/// its clock only moves when advanced, so nothing runs on its own.
#[gpui_kit::test]
fn the_wordmark_keeps_bouncing_and_keeps_its_place(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    assert!(!cx.read(|cx| cx.reduce_motion()), "motion is on by default");
    assert!(bounces(&s, cx));
    let (label, at_rest) = wordmark(&s, cx);
    assert_eq!(label.as_deref(), Some(WORDMARK));
    assert_eq!(WORDMARK, "Kimün");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(90));
    assert_eq!(wordmark(&s, cx).1, at_rest, "the line box holds");
    let running = cx
        .update_window(s.handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    assert!(running > 0, "mid-spring it asks for the next frame");
    // A long time on, it still loops.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(30));
    cx.run_until_parked();
    let pending = cx
        .update_window(s.handle, |_, window, cx| {
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            window.simulate_next_frame(cx)
        })
        .unwrap();
    assert!(pending > 0, "it never stops asking for frames");

    s.ob_press("right", cx);
    assert_eq!(s.ob_step(cx), OnbStep::Workspace);
    s.ob_press("left", cx);
    assert_eq!(s.ob_step(cx), OnbStep::Welcome);
    assert!(bounces(&s, cx), "back on Welcome it bounces again");
    assert_eq!(wordmark(&s, cx).0.as_deref(), Some(WORDMARK));
}

/// `[gui] reduce_motion = true` sets GPUI's switch: the wordmark is drawn
/// still, its text unchanged.
#[gpui_kit::test]
fn reduce_motion_keeps_the_wordmark_still(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let mut gui = GuiSettings::default();
    gui.set_reduce_motion(true);
    cx.update(|cx| crate::apply_reduce_motion(gui.reduce_motion(), cx));
    assert!(cx.read(|cx| cx.reduce_motion()));
    let s = first_run(cx, &rt, &f);
    assert!(!bounces(&s, cx));
    assert_eq!(wordmark(&s, cx).0.as_deref(), Some(WORDMARK));
    // Unset again, the default: motion.
    cx.update(|cx| crate::apply_reduce_motion(GuiSettings::default().reduce_motion(), cx));
    assert!(!cx.read(|cx| cx.reduce_motion()));
}

/// The painted wordmark is the label split around its "ü": the text before,
/// the plain "u" under the dots, the text after, which put together with the
/// "ü" again is the label.
#[test]
fn the_painted_wordmark_is_derived_from_the_label() {
    let parts = wordmark_parts(WORDMARK);
    assert_eq!(parts.base, Some("u"));
    assert_eq!(format!("{}ü{}", parts.before, parts.after), WORDMARK);
    assert_eq!((parts.before, parts.after), ("Kim", "n"));
}

/// A word without "ü" is painted whole, with nothing to bounce.
#[test]
fn a_wordmark_without_an_umlaut_is_painted_whole() {
    assert_eq!(
        wordmark_parts("Kimun"),
        WordmarkParts {
            before: "Kimun",
            base: None,
            after: "",
        }
    );
    assert_eq!(wordmark_parts("").base, None);
}

/// The spring rests at both ends (what reduced motion draws is the glyph
/// in place), rises first, and stays within its height.
#[test]
fn the_spring_rests_at_both_ends_and_rises_first() {
    assert_eq!(bounce_offset(0.), 0.);
    assert!(bounce_offset(1.).abs() < 1e-6);
    assert!(bounce_offset(0.2) < 0., "up first");
    let peak = (0..=100)
        .map(|i| bounce_offset(i as f32 / 100.).abs())
        .fold(0., f32::max);
    assert!((0.24..=0.26).contains(&peak), "≈ 0.25 em, got {peak}");
}

// ── steps ───────────────────────────────────────────────────────────────────

/// The six steps (no Nerd Fonts: only the terminal app uses them), forward
/// and back, with the keys and with the buttons.
#[gpui_kit::test]
fn the_six_steps_walk_with_keys_and_buttons(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    s.ob_choose(&f.root.join("notes"), cx);
    assert_eq!(s.ob_model(cx, |m| m.step_count()), 6);
    let mut seen = vec![s.ob_step(cx)];
    for _ in 0..5 {
        s.ob_press("enter", cx);
        seen.push(s.ob_step(cx));
    }
    assert_eq!(
        seen,
        [
            OnbStep::Welcome,
            OnbStep::Workspace,
            OnbStep::Theme,
            OnbStep::Backend,
            OnbStep::Updates,
            OnbStep::Summary,
        ]
    );
    assert_eq!(s.ob_model(cx, |m| m.step_index()), 5);
    s.ob_press("left", cx);
    s.ob_press("left", cx);
    s.ob_press("left", cx);
    assert_eq!(
        s.ob_step(cx),
        OnbStep::Theme,
        "back from Backend lands on Theme"
    );
    s.ob_press("left", cx);
    assert_eq!(
        s.ob_step(cx),
        OnbStep::Workspace,
        "back from Theme skips Nerd Fonts"
    );
    s.ob_go_to(OnbStep::Summary, cx);
    s.ob_press("left", cx);
    assert_eq!(s.ob_step(cx), OnbStep::Updates);
    s.ob_click(BACK_ID, cx);
    assert_eq!(s.ob_step(cx), OnbStep::Backend);
    s.ob_click(NEXT_ID, cx);
    assert_eq!(s.ob_step(cx), OnbStep::Updates);
    s.ob_press("right", cx);
    assert_eq!(s.ob_step(cx), OnbStep::Summary);
    for _ in 0..5 {
        s.ob_press("left", cx);
    }
    assert_eq!(s.ob_step(cx), OnbStep::Welcome);
    s.ob_press("left", cx);
    assert_eq!(s.ob_step(cx), OnbStep::Welcome, "stays at the start");
}

/// The first run cannot leave the Workspace step without a folder: Next is
/// disabled and the keys say why; a chosen folder lets it on.
#[gpui_kit::test]
fn first_run_next_waits_for_a_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    s.ob_with_view(cx, |v, _, _| v.model_mut().unwrap().draft.workspace = None);
    s.ob_go_to(OnbStep::Workspace, cx);
    assert!(s.ob_disabled(NEXT_ID, cx));
    s.ob_press("enter", cx);
    assert_eq!(s.ob_step(cx), OnbStep::Workspace);
    assert_eq!(
        s.ob_read_view(cx, |v| v.message().map(str::to_string))
            .as_deref(),
        Some(CHOOSE_FOLDER_FIRST)
    );
    s.ob_press("right", cx);
    assert_eq!(s.ob_step(cx), OnbStep::Workspace);
    s.ob_choose(&f.root.join("notes"), cx);
    assert!(s.ob_read_view(cx, |v| v.message().is_none()));
    assert!(!s.ob_disabled(NEXT_ID, cx));
    s.ob_press("enter", cx);
    assert_eq!(s.ob_step(cx), OnbStep::Theme);
}

/// Updates: Up/Down pick, Space toggles (both ways).
#[gpui_kit::test]
fn the_toggles_take_up_down_and_space(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    s.ob_choose(&f.root.join("notes"), cx);
    let fonts = s.ob_model(cx, |m| m.draft.use_nerd_fonts);
    s.ob_go_to(OnbStep::Updates, cx);
    s.ob_press("down", cx);
    assert!(!s.ob_model(cx, |m| m.draft.update_check));
    s.ob_press("space", cx);
    assert!(s.ob_model(cx, |m| m.draft.update_check));
    s.ob_press("space", cx);
    assert!(!s.ob_model(cx, |m| m.draft.update_check));
    s.ob_press("up", cx);
    assert!(s.ob_model(cx, |m| m.draft.update_check));
    assert_eq!(
        s.ob_model(cx, |m| m.draft.use_nerd_fonts),
        fonts,
        "no step changes the Nerd Fonts setting"
    );
}

// ── the folder chooser ──────────────────────────────────────────────────────

/// The chooser lists only subfolders, sorted ignoring case; Enter opens the
/// selected one, Left goes up, a click enters a row.
#[gpui_kit::test]
fn the_chooser_lists_subfolders_and_walks_them(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let base = f.root.join("browse");
    mkdirs(&base, &["beta", "Alpha", "gamma", "beta/inner"]);
    std::fs::write(base.join("file.txt"), "not a folder").unwrap();
    let s = first_run(cx, &rt, &f);
    s.ob_go_to(OnbStep::Workspace, cx);
    s.ob_press("b", cx);
    assert!(s.shows(super::onboarding::CHOOSER_ID, cx));
    s.ob_browse(&rt, &base, cx);
    assert_eq!(s.ob_chooser_rows(cx), ["Alpha", "beta", "gamma"]);
    assert_eq!(s.ob_chooser(cx, |c| c.selected()).unwrap(), Some(0));
    s.ob_press("down", cx);
    s.ob_press("enter", cx);
    s.ob_listed(&rt, cx);
    assert_eq!(s.ob_chooser_dir(cx), base.join("beta"));
    assert_eq!(s.ob_chooser_rows(cx), ["inner"]);
    s.ob_press("left", cx);
    s.ob_listed(&rt, cx);
    assert_eq!(s.ob_chooser_dir(cx), base);
    let gamma = SystemPath::try_absolute(base.join("gamma")).unwrap();
    s.ob_click(chooser_row_id(&gamma), cx);
    s.ob_listed(&rt, cx);
    assert_eq!(s.ob_chooser_dir(cx), base.join("gamma"));
    // Esc leaves the chooser, nothing chosen; the step stays.
    let before = s.ob_model(cx, |m| m.draft.workspace.clone());
    s.ob_press("escape", cx);
    assert!(s.ob_read_view(cx, |v| v.chooser().is_none()));
    assert_eq!(s.ob_model(cx, |m| m.draft.workspace.clone()), before);
    assert_eq!(s.ob_step(cx), OnbStep::Workspace);
}

/// `New folder…` makes the folder (through core) and enters it; a name no
/// system accepts is refused in the field.
#[gpui_kit::test]
fn new_folder_creates_and_enters_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let base = f.root.join("browse");
    mkdirs(&base, &[]);
    let s = first_run(cx, &rt, &f);
    s.ob_go_to(OnbStep::Workspace, cx);
    s.ob_browse(&rt, &base, cx);
    s.ob_press("n", cx);
    assert!(s.ob_chooser(cx, |c| c.new_folder_open()).unwrap());
    s.ob_typed("bad:name", cx);
    s.ob_press("enter", cx);
    drive(&rt, cx, |cx| {
        s.ob_chooser(cx, |c| c.new_folder_error().is_some())
            .unwrap()
    });
    assert!(!base.join("bad:name").exists());
    let input = s
        .ob_chooser(cx, |c| c.new_folder_input())
        .flatten()
        .unwrap();
    cx.update_window(s.handle, |_, window, cx| {
        input.update(cx, |i, cx| i.set_value("", window, cx))
    })
    .unwrap();
    s.ob_typed("fresh", cx);
    s.ob_press("enter", cx);
    drive(&rt, cx, |cx| {
        s.ob_chooser(cx, |c| {
            c.dir().as_path() == base.join("fresh") && !matches!(c.listing(), Listing::Loading)
        })
        .unwrap_or(false)
    });
    assert!(base.join("fresh").is_dir());
    assert!(!s.ob_chooser(cx, |c| c.new_folder_open()).unwrap());
    // Chosen: it is the workspace folder, named after it.
    s.ob_press("ctrl-enter", cx);
    assert_eq!(
        s.ob_model(cx, |m| m.draft.workspace.clone()),
        Some(("fresh".to_string(), base.join("fresh")))
    );
}

/// A folder that cannot be listed says so in the chooser.
#[gpui_kit::test]
fn a_listing_error_is_shown(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    s.ob_go_to(OnbStep::Workspace, cx);
    s.ob_browse(&rt, &f.root.join("missing"), cx);
    assert!(
        s.ob_chooser(cx, |c| matches!(c.listing(), Listing::Failed(_)))
            .unwrap()
    );
    assert!(s.shows("onboarding-chooser-error", cx));
}

/// Choosing a folder sets it and the name it suggests; a name the user
/// edited survives the next pick.
#[gpui_kit::test]
fn choosing_suggests_the_name_until_it_is_edited(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let base = f.root.join("browse");
    mkdirs(&base, &["My-Vault", "Other"]);
    let s = first_run(cx, &rt, &f);
    s.ob_go_to(OnbStep::Workspace, cx);
    s.ob_browse(&rt, &base.join("My-Vault"), cx);
    s.ob_click(super::onboarding::CHOOSER_CHOOSE_ID, cx);
    assert!(s.ob_read_view(cx, |v| v.chooser().is_none()));
    assert_eq!(
        s.ob_model(cx, |m| m.draft.workspace.clone()),
        Some(("my-vault".to_string(), base.join("My-Vault")))
    );
    assert_eq!(s.ob_name_value(cx), "my-vault");
    // Edit the name: `e` gives the field the keyboard.
    s.ob_press("e", cx);
    s.ob_typed("-x", cx);
    assert_eq!(
        s.ob_model(cx, |m| m.draft.workspace.clone().unwrap().0),
        "my-vault-x"
    );
    assert!(s.ob_model(cx, |m| m.name_edited));
    s.ob_press("escape", cx);
    s.ob_browse(&rt, &base.join("Other"), cx);
    s.ob_press("ctrl-enter", cx);
    assert_eq!(
        s.ob_model(cx, |m| m.draft.workspace.clone()),
        Some(("my-vault-x".to_string(), base.join("Other")))
    );
    assert_eq!(s.ob_name_value(cx), "my-vault-x");
    // An invalid name is refused in the field, and Next with it.
    s.ob_press("e", cx);
    s.ob_typed("?", cx);
    assert!(s.ob_read_view(cx, |v| v.name_error().is_some()));
    assert!(s.ob_disabled(NEXT_ID, cx));
}

// ── theme and backend ───────────────────────────────────────────────────────

/// A theme row previews on the whole window at once; discarding puts the
/// window's theme back.
#[gpui_kit::test]
fn the_theme_previews_live_and_discard_restores_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "# A")], &["a.md"]));
    let s = rerun(cx, &rt, &v);
    let original = s.ob_window_theme(cx);
    s.ob_go_to(OnbStep::Theme, cx);
    let (idx, len) = s.ob_model(cx, |m| (m.theme_idx, m.themes.len()));
    s.ob_press(if idx + 1 < len { "down" } else { "up" }, cx);
    let chosen = s.ob_model(cx, |m| m.draft.theme_name.clone());
    assert_ne!(chosen, original);
    assert_eq!(s.ob_window_theme(cx), chosen, "the window shows it");
    let picked = s.ob_model(cx, |m| m.themes[m.theme_idx].clone());
    cx.read(|cx| {
        let kit = gpui_kit::component::Theme::global(cx);
        let want: gpui_kit::Hsla = theme::panel_background(&picked).into();
        assert_eq!(kit.background, want, "the kit's components too");
    });
    // A row click previews as well: back on the window's own theme.
    s.ob_click(super::onboarding::theme_row_id(&original), cx);
    assert_eq!(s.ob_window_theme(cx), original);
    assert_eq!(s.ob_model(cx, |m| m.theme_idx), idx);
    s.ob_press(if idx + 1 < len { "down" } else { "up" }, cx);
    assert_eq!(s.ob_window_theme(cx), chosen);
    // Esc: the change would go, so it asks; yes restores the theme.
    s.ob_press("escape", cx);
    assert_eq!(s.ob_read_view(cx, |v| v.confirm()), Some(Confirm::Discard));
    s.ob_press("enter", cx);
    assert!(!s.ob_is_open(cx));
    assert_eq!(s.ob_window_theme(cx), original);
    cx.read(|cx| {
        let kit = gpui_kit::component::Theme::global(cx);
        let want: gpui_kit::Hsla = theme::panel_background(&Theme::default()).into();
        assert_eq!(kit.background, want);
    });
    settle(&rt, cx);
}

/// The desktop app has not enabled nvim: the Backend step offers plain and
/// vim only, the keys stop at vim and nvim cannot be chosen (even when it is
/// installed). The terminal app's model still offers all three.
#[gpui_kit::test]
fn the_backend_step_offers_plain_and_vim_only(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    s.ob_with_view(cx, |v, _, _| v.model_mut().unwrap().nvim_available = true);
    let names = s.ob_model(cx, |m| {
        m.backends()
            .iter()
            .map(|(_, name, _)| *name)
            .collect::<Vec<_>>()
    });
    assert_eq!(names, ["plain", "vim"]);
    assert_eq!(
        OnboardingModel::new(&f.settings.read().unwrap())
            .backends()
            .len(),
        3,
        "the terminal app keeps nvim"
    );
    s.ob_choose(&f.root.join("notes"), cx);
    s.ob_go_to(OnbStep::Backend, cx);
    assert_eq!(s.ob_model(cx, |m| m.selected_backend()), Some(0));
    s.ob_press("down", cx);
    assert_eq!(
        s.ob_model(cx, |m| m.draft.editor_backend),
        EditorBackendSetting::Vim
    );
    s.ob_press("down", cx);
    assert_eq!(
        s.ob_model(cx, |m| (m.draft.editor_backend, m.selected_backend())),
        (EditorBackendSetting::Vim, Some(1)),
        "no nvim row below vim"
    );
    s.ob_with_view(cx, |v, _, cx| v.select_backend(2, cx));
    assert_eq!(
        s.ob_model(cx, |m| m.draft.editor_backend),
        EditorBackendSetting::Vim
    );
    s.ob_press("up", cx);
    assert_eq!(
        s.ob_model(cx, |m| m.draft.editor_backend),
        EditorBackendSetting::Plain
    );
}

/// A config that names nvim keeps it: the Backend step selects no row, the
/// summary says it is unchanged, and Finish does not rewrite it.
#[gpui_kit::test]
fn a_configured_nvim_survives_finish(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "# A")], &["a.md"]));
    v.settings.write().unwrap().editor_backend = EditorBackendSetting::Nvim;
    let s = rerun(cx, &rt, &v);
    s.ob_go_to(OnbStep::Backend, cx);
    assert_eq!(s.ob_model(cx, |m| m.selected_backend()), None);
    assert_eq!(s.ob_model(cx, |m| m.backend_name()), "nvim");
    s.ob_go_to(OnbStep::Summary, cx);
    assert!(s.ob_shows_id("onboarding-summary-Editor backend", cx));
    s.ob_press("enter", cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let w = s.workspace.read(cx);
            w.onboarding.is_none() && !w.reopen_running() && !w.status.indexing
        })
    });
    assert_eq!(
        v.settings.read().unwrap().editor_backend,
        EditorBackendSetting::Nvim
    );
    drive(&rt, cx, |cx| s.text(cx) == "# A");
    settle(&rt, cx);
}

#[gpui_kit::test]
fn finishing_the_first_run_opens_the_new_vault(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    // Not the default, so a draft that reset it would show.
    f.settings.write().unwrap().use_nerd_fonts = true;
    let s = first_run(cx, &rt, &f);
    let ws = f.root.join("notes-here");
    s.ob_choose(&ws, cx);
    s.ob_go_to(OnbStep::Backend, cx);
    s.ob_with_view(cx, |v, _, cx| v.select_backend(1, cx));
    s.ob_go_to(OnbStep::Summary, cx);
    s.ob_press("enter", cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let w = s.workspace.read(cx);
            w.onboarding.is_none()
                && w.vault().is_some()
                && !w.reopen_running()
                && !w.status.indexing
        })
    });
    assert!(ws.is_dir(), "the folder is created");
    assert_eq!(
        f.settings
            .read()
            .unwrap()
            .current_workspace_name()
            .as_deref(),
        Some("notes-here")
    );
    assert!(
        f.settings.read().unwrap().use_nerd_fonts,
        "the desktop setup never changes Nerd Fonts"
    );
    drive(&rt, cx, |_| {
        std::fs::read_to_string(&f.config).is_ok_and(|t| t.contains("notes-here"))
    });
    assert!(
        cx.read(|cx| s.workspace.read(cx).files.is_some()),
        "FILES on it"
    );
    assert!(s.shows("rail", cx) && s.shows("editor-panel", cx));
    assert!(!s.shows(ONBOARDING_ID, cx));
    assert_eq!(
        VisualTestContext::from_window(s.handle, cx)
            .window_title()
            .as_deref(),
        Some("notes-here - kimün")
    );
    let vim = cx.read(|cx| {
        let editor = s.workspace.read(cx).editor.clone();
        editor.read(cx).editor.is_vim()
    });
    assert!(vim, "the chosen editor runs now");
    settle(&rt, cx);
}

/// A folder that cannot be created: the error shows in the assistant, back
/// on the Workspace step, and nothing is registered or left behind.
#[gpui_kit::test]
fn a_failed_finish_returns_to_the_workspace_step(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let file = f.root.join("afile");
    std::fs::write(&file, "a file, not a folder").unwrap();
    let ws = file.join("child");
    let s = first_run(cx, &rt, &f);
    s.ob_choose(&ws, cx);
    s.ob_go_to(OnbStep::Summary, cx);
    s.ob_click(NEXT_ID, cx);
    drive(&rt, cx, |cx| !s.ob_read_view(cx, |v| v.finishing()));
    assert_eq!(s.ob_step(cx), OnbStep::Workspace);
    let message = s
        .ob_read_view(cx, |v| v.message().map(str::to_string))
        .unwrap();
    assert!(message.contains("Couldn't finish"), "{message}");
    assert!(s.shows(super::onboarding::MESSAGE_ID, cx));
    assert!(s.ob_is_open(cx));
    assert!(!ws.exists());
    let settings = f.settings.read().unwrap();
    assert!(settings.current_workspace_name().is_none());
    assert!(
        settings
            .workspace_config
            .as_ref()
            .is_none_or(|wc| wc.workspaces.is_empty())
    );
}

/// A re-run's finish changes the theme and the update check, never the
/// workspaces or the Nerd Fonts setting; the window shows the new theme.
#[gpui_kit::test]
fn a_rerun_finish_never_touches_the_workspaces(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "# A")], &["a.md"]));
    let names = |v: &TestVault| -> Vec<String> {
        v.settings
            .read()
            .unwrap()
            .workspace_config
            .as_ref()
            .unwrap()
            .workspaces
            .keys()
            .cloned()
            .collect()
    };
    let before = names(&v);
    // Flipped from the default, so a draft that reset it would show.
    let fonts = {
        let mut settings = v.settings.write().unwrap();
        settings.use_nerd_fonts = !settings.use_nerd_fonts;
        settings.use_nerd_fonts
    };
    let updates = v.settings.read().unwrap().update_check();
    let s = rerun(cx, &rt, &v);
    s.ob_go_to(OnbStep::Theme, cx);
    let (idx, len) = s.ob_model(cx, |m| (m.theme_idx, m.themes.len()));
    s.ob_press(if idx + 1 < len { "down" } else { "up" }, cx);
    let chosen = s.ob_model(cx, |m| m.draft.theme_name.clone());
    s.ob_go_to(OnbStep::Updates, cx);
    s.ob_press("space", cx);
    s.ob_go_to(OnbStep::Summary, cx);
    s.ob_press("enter", cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let w = s.workspace.read(cx);
            w.onboarding.is_none() && !w.reopen_running() && !w.status.indexing
        })
    });
    assert_eq!(names(&v), before);
    let settings = v.settings.read().unwrap();
    assert_eq!(settings.current_workspace_name().as_deref(), Some(NAME));
    assert_eq!(settings.use_nerd_fonts, fonts, "Nerd Fonts unchanged");
    assert_eq!(settings.update_check(), !updates);
    assert_eq!(settings.theme, chosen);
    drop(settings);
    assert_eq!(s.ob_window_theme(cx), chosen);
    drive(&rt, cx, |cx| s.text(cx) == "# A");
    settle(&rt, cx);
}

// ── leaving ─────────────────────────────────────────────────────────────────

/// The first run asks before quitting: no keeps the setup, yes closes the
/// window.
#[gpui_kit::test]
fn leaving_the_first_run_asks_before_quitting(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    s.ob_press("escape", cx);
    assert_eq!(s.ob_read_view(cx, |v| v.confirm()), Some(Confirm::Quit));
    s.ob_press("escape", cx);
    assert_eq!(s.ob_read_view(cx, |v| v.confirm()), None);
    s.ob_click(CANCEL_ID, cx);
    assert_eq!(s.ob_read_view(cx, |v| v.confirm()), Some(Confirm::Quit));
    s.ob_click(CONFIRM_NO_ID, cx);
    assert_eq!(s.ob_read_view(cx, |v| v.confirm()), None);
    assert!(s.ob_is_open(cx));
    s.ob_press("escape", cx);
    s.ob_press("enter", cx);
    assert!(
        cx.update_window(s.handle, |_, _, _| ()).is_err(),
        "the window is closed"
    );
}

/// A clean re-run closes at once; a changed one asks, and `Back to setup`
/// keeps it.
#[gpui_kit::test]
fn leaving_a_rerun_closes_or_asks(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "# A")], &["a.md"]));
    let s = rerun(cx, &rt, &v);
    s.ob_press("escape", cx);
    assert!(!s.ob_is_open(cx), "nothing changed: it just closes");
    assert!(s.shows("rail", cx));
    s.ob_on_window(cx, |w, window, cx| w.open_onboarding(window, cx));
    s.ob_loaded(&rt, cx);
    s.ob_go_to(OnbStep::Updates, cx);
    s.ob_press("space", cx);
    s.ob_press("escape", cx);
    assert_eq!(s.ob_read_view(cx, |v| v.confirm()), Some(Confirm::Discard));
    s.ob_click(CONFIRM_NO_ID, cx);
    assert!(s.ob_is_open(cx));
    assert_eq!(s.ob_step(cx), OnbStep::Updates);
    s.ob_press("escape", cx);
    s.ob_click(super::onboarding::CONFIRM_YES_ID, cx);
    assert!(!s.ob_is_open(cx));
    let update_check = v.settings.read().unwrap().update_check();
    assert!(update_check, "nothing applied");
    settle(&rt, cx);
}

/// While the assistant shows, the shell's chords do nothing: no drawer
/// toggles, no palette, no leader sequence.
#[gpui_kit::test]
fn shell_chords_are_inert_while_it_shows(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("a.md", "# A")], &["a.md"]));
    let s = rerun(cx, &rt, &v);
    let left = |cx: &TestAppContext| {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .layout
                .is_visible(Panel::Drawer(Side::Left))
        })
    };
    let before = left(cx);
    s.ob_press("ctrl-t", cx);
    assert_eq!(left(cx), before, "the drawer toggle is inert");
    s.ob_press("ctrl-shift-p", cx);
    assert_eq!(s.shown_modal(cx), None, "no palette");
    s.ob_press("ctrl-shift-b", cx);
    assert!(
        cx.read(|cx| !s.workspace.read(cx).leader.is_pending()),
        "no leader"
    );
    assert_eq!(s.ob_step(cx), OnbStep::Welcome);
    assert!(s.ob_is_open(cx));
    settle(&rt, cx);
}

/// The ü really moves: higher mid-spring than at rest, at rest again in
/// the pause, and up again on the next round.
#[gpui_kit::test]
fn the_u_moves_mid_spring(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let s = first_run(cx, &rt, &f);
    let top = |cx: &mut TestAppContext| {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(WORDMARK_U_ID).bounds().origin.y
        })
        .unwrap()
    };
    let ms = std::time::Duration::from_millis;
    let start = top(cx);
    cx.executor().advance_clock(ms(120));
    let mid = top(cx);
    assert!(mid < start, "the ü rises mid-spring: {start:?} -> {mid:?}");
    cx.executor().advance_clock(ms(1200 - 120));
    assert_eq!(top(cx), start, "it rests in the pause");
    cx.executor().advance_clock(ms(2400 - 1200 + 120));
    assert!(top(cx) < start, "and rises again on the next round");
}

// ── the wordmark's dots against Inter's own "ü" ─────────────────────────────

/// Where a glyph's ink sits, in pixels from its pen origin on the baseline
/// (y grows down), measured on the real face through Linux's text backend.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct Ink {
    /// The glyph's advance.
    advance: f32,
    /// The ink's top and bottom.
    top: f32,
    bottom: f32,
    /// The ink's centroid (alpha-weighted).
    cx: f32,
    cy: f32,
}

/// The ink of `ch` in Inter at `weight` and `size` px; with `above`, only
/// the rows above that y (the dots of "ü", over the "u").
#[cfg(target_os = "linux")]
fn inter_ink(ch: char, weight: gpui_kit::FontWeight, size: f32, above: Option<f32>) -> Ink {
    use gpui_kit::{DevicePixels, PlatformTextSystem as _, RenderGlyphParams, point, px};
    let (ts, platform) = crate::ui_font::cosmic_tests::bare_text_system();
    crate::ui_font::register(&ts, crate::ui_font::FACES).unwrap();
    let font_id = ts.resolve_font(&gpui_kit::Font {
        weight,
        ..gpui_kit::font(crate::ui_font::FAMILY)
    });
    let glyph_id = platform.glyph_for_char(font_id, ch).expect("Inter has it");
    let params = RenderGlyphParams {
        font_id,
        glyph_id,
        font_size: px(size),
        subpixel_variant: point(0, 0),
        scale_factor: 1.,
        is_emoji: false,
        subpixel_rendering: false,
        dilation: 0,
    };
    let bounds = platform.glyph_raster_bounds(&params).unwrap();
    let (raster_size, mask) = platform.rasterize_glyph(&params, bounds).unwrap();
    let (w, h) = (raster_size.width.0 as usize, raster_size.height.0 as usize);
    assert_eq!(mask.len(), w * h, "an alpha mask");
    let (ox, oy) = (bounds.origin.x.0 as f32, bounds.origin.y.0 as f32);
    let (mut sum, mut sx, mut sy) = (0f64, 0f64, 0f64);
    let (mut top, mut bottom) = (f32::MAX, f32::MIN);
    for row in 0..h {
        let y = oy + row as f32 + 0.5;
        if above.is_some_and(|limit| y >= limit) {
            continue;
        }
        for col in 0..w {
            let a = mask[row * w + col] as f64;
            if a == 0. {
                continue;
            }
            sum += a;
            sx += a * (ox + col as f32 + 0.5) as f64;
            sy += a * y as f64;
            top = top.min(oy + row as f32);
            bottom = bottom.max(oy + row as f32 + 1.);
        }
    }
    let _ = DevicePixels(0);
    Ink {
        advance: f32::from(ts.advance(font_id, px(size), ch).unwrap().width),
        top,
        bottom,
        cx: (sx / sum) as f32,
        cy: (sy / sum) as f32,
    }
}

/// The resting dots sit like the dots of Inter's own "ü", in the face the
/// wordmark paints (Bold): the "¨" laid out as `render_wordmark` lays it
/// (its box centred on the "u", shifted by the spring's end state) is
/// compared with the ü's ink above the u's top, on the rasterized faces at
/// 1000 px. Above the "u", centred on it, and level with the real dots.
#[cfg(target_os = "linux")]
#[test]
fn the_resting_dots_sit_like_the_dots_of_inters_u() {
    use super::onboarding::{DIAERESIS, DOTTED_BASE, WORDMARK_WEIGHT};
    let size = 1000.;
    let ch = |s: &str| s.chars().next().unwrap();
    let u = inter_ink(ch(DOTTED_BASE), WORDMARK_WEIGHT, size, None);
    let real = inter_ink('ü', WORDMARK_WEIGHT, size, Some(u.top));
    let d = inter_ink(ch(DIAERESIS), WORDMARK_WEIGHT, size, None);
    // `justify_center` over the u's box, `top` = the spring at rest.
    let left = (u.advance - d.advance) / 2.;
    let drop = bounce_offset(1.) * size;
    let (cx, cy) = (left + d.cx, drop + d.cy);
    let (top, bottom) = (drop + d.top, drop + d.bottom);
    let em = |px: f32| px / size;
    assert!(bottom < u.top, "above the u: {bottom} vs {}", u.top);
    assert!(
        em((cx - u.advance / 2.).abs()) < 0.01,
        "centred on the u: {cx} vs {}",
        u.advance / 2.
    );
    assert!(
        em((cx - real.cx).abs()) < 0.005,
        "where the ü has them: {cx} vs {}",
        real.cx
    );
    assert!(
        em((cy - real.cy).abs()) < 0.005,
        "level with the ü's dots: {cy} vs {}",
        real.cy
    );
    assert!(
        em((top - real.top).abs()) < 0.005 && em((bottom - real.bottom).abs()) < 0.005,
        "the same size as the ü's dots: {top}..{bottom} vs {}..{}",
        real.top,
        real.bottom
    );
}

/// The structure the bounce depends on: the dots are their own element,
/// one letter wide, over the "u" (after "Kim", before "n"), so moving them
/// moves nothing else. The test platform's stub font gives every letter one
/// width, so the cells are exact.
#[gpui_kit::test]
fn the_dots_are_their_own_element_over_the_u(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let f = fresh();
    let mut gui = GuiSettings::default();
    gui.set_reduce_motion(true);
    cx.update(|cx| crate::apply_reduce_motion(gui.reduce_motion(), cx));
    let s = first_run(cx, &rt, &f);
    let (word, dots) = cx
        .update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find(WORDMARK_ID).bounds(),
                window.find(WORDMARK_U_ID).bounds(),
            )
        })
        .unwrap();
    let parts = wordmark_parts(WORDMARK);
    // The stub font: every letter 0.6 em; the wordmark is 3 rem (48 px).
    // Layout snaps to whole pixels, hence the pixel of slack.
    let cell = 0.6 * 48.;
    let near = |a: gpui_kit::Pixels, b: f32| (f32::from(a) - b).abs() <= 1.;
    assert!(
        near(dots.size.width, cell),
        "one letter wide: {dots:?} in {word:?}"
    );
    assert!(
        near(
            dots.origin.x - word.origin.x,
            cell * parts.before.chars().count() as f32
        ),
        "over the u, after {:?}: {dots:?} in {word:?}",
        parts.before
    );
    assert_eq!(dots.origin.y, word.origin.y, "at rest, in the line box");
}
