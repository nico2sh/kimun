//! UI integration tests for a **Remote image** in a note's editor
//! (CONTEXT.md): the placeholder, its buttons, the load, the rows it takes,
//! the menu of a loaded image, and the follow. A real `Workspace` on a real
//! temp vault. The fetcher is a fake and the image cache is a temp folder,
//! so no test uses the network.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui_kit::test::{ClickOptions, TestWindowExt};
use gpui_kit::{AppContext as _, Entity, MouseButton, TestAppContext, point, px};
use kimun_core::nfs::VaultPath;
use kimun_core::system::{ImageCache, SystemPath};
use tokio::runtime::Runtime;

use super::row_menu_tests::in_workspace::{menu_labels, pick};
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::editor_element::image_hover_id;
use crate::editor_view::EditorView;
use crate::image_core::{Actions, RemoteLook};
use crate::remote_image::{
    FetchFailure, Fetcher, Hop, RemoteImages, fetch_count, set_cache, set_fetcher,
};
use crate::remote_placeholder::RemoteAction;
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

const HOST: &str = "img.example.com";
const URL: &str = "https://img.example.com/pic.png";
const OTHER: &str = "https://img.example.com/other.png";
/// The note: a remote image between two lines of text.
const NOTE: &str = "intro\n![a](https://img.example.com/pic.png)\nafter";

/// A PNG of `w`x`h` pixels.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 10, 10, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A fetcher that answers from a script (404 for any other address) and
/// logs the calls.
#[derive(Default)]
pub(super) struct Fake {
    script: Mutex<HashMap<String, Hop>>,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    pub(super) fn answer(&self, url: &str, hop: Hop) {
        self.script.lock().unwrap().insert(url.to_string(), hop);
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Fetcher for Fake {
    fn get(&self, url: &str) -> Hop {
        self.calls.lock().unwrap().push(url.to_string());
        self.script
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .unwrap_or(Hop::Failed(FetchFailure::Status(404)))
    }
}

/// The fake fetcher and the temp cache the app's store uses.
pub(super) struct Env {
    _dir: tempfile::TempDir,
    pub(super) cache: ImageCache,
    pub(super) fake: Arc<Fake>,
}

/// Install a fake fetcher that answers `script` and a temp cache in the
/// app's store.
pub(super) fn install(cx: &mut TestAppContext, script: &[(&str, Hop)]) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let cache = ImageCache::at(
        SystemPath::try_absolute(dir.path())
            .unwrap()
            .join("image-cache"),
    );
    let fake = Arc::new(Fake::default());
    for (url, hop) in script {
        fake.answer(url, hop.clone());
    }
    let fetcher: Arc<dyn Fetcher> = fake.clone();
    let c = cache.clone();
    cx.update(|cx| {
        set_fetcher(cx, fetcher);
        set_cache(cx, c);
    });
    Env {
        _dir: dir,
        cache,
        fake,
    }
}

/// A vault holding `a.md` (`text`), with `hosts` allowed, started on
/// `a.md`, driven until the text shows and the index is done.
fn open(cx: &mut TestAppContext, rt: &Runtime, text: &str, hosts: &[&str]) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(&[("a.md", text)], &[]));
    v.settings.write().unwrap().remote_image_hosts = hosts.iter().map(|h| h.to_string()).collect();
    let s = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
    );
    drive(rt, cx, |cx| {
        render(&s, cx);
        s.text(cx) == text && !s.indexing(cx)
    });
    (v, s)
}

fn editor(s: &Started, cx: &TestAppContext) -> Entity<EditorView> {
    cx.read(|cx| s.workspace.read(cx).active_state().editor.clone())
}

fn render(s: &Started, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.refresh();
        window.render_frame(cx);
    })
    .unwrap();
}

fn hit(s: &Started, key: &str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Draw a frame; the remote look painted for row `row`.
fn look(s: &Started, row: usize, cx: &mut TestAppContext) -> Option<RemoteLook> {
    render(s, cx);
    let view = editor(s, cx);
    cx.read(|cx| {
        view.read(cx).painted.as_ref().and_then(|p| {
            p.remote_looks
                .iter()
                .find(|l| l.row == row)
                .map(|l| l.look.clone())
        })
    })
}

/// Draw a frame; whether the image of row `row` is painted with its pixels.
fn ready(s: &Started, row: usize, cx: &mut TestAppContext) -> bool {
    render(s, cx);
    let view = editor(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .painted
            .as_ref()
            .is_some_and(|p| p.images.iter().any(|i| i.row == row && i.ready))
    })
}

/// The lines logical row `row` takes in the layout: its own and its pads.
fn lines_of(s: &Started, row: usize, cx: &TestAppContext) -> usize {
    let view = editor(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .editor
            .session()
            .display
            .layout
            .visual_lines()
            .iter()
            .filter(|l| l.logical_row == row)
            .count()
    })
}

fn fetches(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| fetch_count(cx))
}

fn store(cx: &TestAppContext) -> RemoteImages {
    cx.read(|cx| cx.global::<RemoteImages>().clone())
}

fn both(host: &str) -> RemoteLook {
    RemoteLook::Placeholder {
        host: host.to_string(),
        actions: Actions {
            load: true,
            always: true,
        },
    }
}

/// Press the button of `action` of the placeholder of the remote embed
/// `url`, where the last frame painted it.
pub(super) fn press_button(s: &Started, url: &str, action: RemoteAction, cx: &mut TestAppContext) {
    render(s, cx);
    let view = editor(s, cx);
    let at = cx.read(|cx| {
        view.read(cx)
            .remote_hits
            .iter()
            .filter(|hit| hit.url == url)
            .flat_map(|hit| hit.buttons.iter())
            .find(|(_, a)| *a == action)
            .map(|(bounds, _)| bounds.center())
            .unwrap_or_else(|| panic!("no {action:?} button for {url}"))
    });
    click_at(s, at, MouseButton::Left, cx);
}

/// A press of `button` at window position `at`.
fn click_at(
    s: &Started,
    at: gpui_kit::Point<gpui_kit::Pixels>,
    button: MouseButton,
    cx: &mut TestAppContext,
) {
    cx.update_window(s.handle, |_, window, cx| {
        let editor = window.find("editor").bounds();
        window.click_with_options(
            "editor",
            ClickOptions::new()
                .with_button(button)
                .with_offset(at - editor.origin),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

/// Hand files and URLs to a recorder instead of the system.
fn record_opens(s: &Started, cx: &mut TestAppContext) -> Rc<RefCell<Vec<OsString>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let record = opened.clone();
    cx.update(|cx| {
        s.workspace.update(cx, |workspace, _| {
            workspace.opener = Rc::new(move |what| {
                record.borrow_mut().push(what.to_owned());
                Ok(())
            });
        })
    });
    opened
}

/// Whether any popup menu is open.
fn any_menu(s: &Started, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        !window.find_all("popup-menu").is_empty()
    })
    .unwrap()
}

/// Right-click the loaded image of row 1.
fn right_click_image(s: &Started, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(image_hover_id(1), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The open menu's items: the labels, with the one element item read by
/// its accessible name.
fn remote_menu_labels(s: &Started, cx: &mut TestAppContext) -> Vec<String> {
    let mut labels: Vec<String> = menu_labels(s, cx);
    let clear = cx
        .update_window(s.handle, |_, window, _| {
            window
                .within("popup-menu")
                .try_find_by_label("Clear image cache")
                .is_some()
        })
        .unwrap();
    if clear && let Some(last) = labels.iter_mut().rev().find(|l| l.as_str() == "-") {
        *last = "Clear image cache".to_string();
    }
    labels
}

/// Click the open menu's "Clear image cache" item.
fn pick_clear(s: &Started, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.within("popup-menu").click(3usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// A note with a saved copy of `URL` (`png(10, 200)`), opened and driven
/// until the image is drawn.
fn opened_with_copy(cx: &mut TestAppContext, rt: &Runtime) -> (Env, TestVault, Started) {
    let env = install(cx, &[]);
    env.cache
        .save(URL, &png(10, 200))
        .expect("the copy is saved");
    let (v, s) = open(cx, rt, NOTE, &[]);
    drive(rt, cx, |cx| ready(&s, 1, cx));
    (env, v, s)
}

#[gpui_kit::test]
fn a_note_with_a_remote_embed_on_an_unallowed_host_starts_no_fetch(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(cx, &[(URL, Hop::Body(png(10, 200)))]);
    let text = format!("{NOTE}\n{}", "line\n".repeat(80));
    let (_v, s) = open(cx, &rt, &text, &[]);
    assert_eq!(look(&s, 1, cx), Some(both(HOST)));
    for key in ["down", "down", "ctrl-end", "ctrl-home", "down"] {
        hit(&s, key, cx);
        drive(&rt, cx, |cx| {
            render(&s, cx);
            true
        });
    }
    // A fetch that started shows Loading, so this holds with no timing.
    assert_eq!(look(&s, 1, cx), Some(both(HOST)), "still the placeholder");
    assert_eq!(fetches(cx), 0);
    assert!(env.fake.calls().is_empty());
}

#[gpui_kit::test]
fn a_placeholder_reserves_two_lines_and_the_text_below_does_not_move_until_the_load(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    let (_v, s) = open(cx, &rt, NOTE, &[]);
    assert_eq!(look(&s, 1, cx), Some(both(HOST)));
    assert_eq!(lines_of(&s, 1, cx), 2, "the placeholder takes two lines");
    let view = editor(&s, cx);
    let after = |cx: &TestAppContext| {
        cx.read(|cx| view.read(cx).editor.session().display.layout.row_start(2))
    };
    let before = after(cx);
    for _ in 0..3 {
        render(&s, cx);
        assert_eq!(after(cx), before, "the text below stays");
    }
    store(cx).set_state_for_test(URL, crate::remote_image::RemoteState::Loading);
    assert_eq!(
        look(&s, 1, cx),
        Some(RemoteLook::Loading {
            host: HOST.to_string()
        })
    );
    assert_eq!(lines_of(&s, 1, cx), 2, "loading keeps the two lines");
    assert_eq!(after(cx), before);
}

#[gpui_kit::test]
fn load_fetches_then_the_image_takes_the_rows_of_its_height_and_the_text_shifts_once(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(cx, &[(URL, Hop::Body(png(10, 200)))]);
    let (_v, s) = open(cx, &rt, NOTE, &[]);
    assert_eq!(lines_of(&s, 1, cx), 2);
    press_button(&s, URL, RemoteAction::Load, cx);
    drive(&rt, cx, |cx| ready(&s, 1, cx));
    assert!(env.cache.find(URL).is_some(), "the copy is saved");
    let loaded = lines_of(&s, 1, cx);
    assert!(loaded > 2, "the image takes {loaded} lines");
    let view = editor(&s, cx);
    let finds = cx.read(|cx| view.read(cx).image_finds());
    for _ in 0..3 {
        drive(&rt, cx, |cx| ready(&s, 1, cx));
        assert_eq!(lines_of(&s, 1, cx), loaded, "no change on later frames");
    }
    assert_eq!(cx.read(|cx| view.read(cx).image_finds()), finds);
    assert_eq!(fetches(cx), 1);
    assert_eq!(look(&s, 1, cx), None, "no placeholder");
    // The saved copy is the file the editor pins.
    let shown = cx.read(|cx| view.read(cx).shown_image_files());
    assert_eq!(shown, vec![env.cache.find(URL).unwrap()]);
}

#[gpui_kit::test]
fn a_saved_copy_draws_with_no_fetch(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_env, _v, s) = opened_with_copy(cx, &rt);
    assert!(lines_of(&s, 1, cx) > 2);
    assert_eq!(fetches(cx), 0);
}

#[gpui_kit::test]
fn always_load_adds_the_host_then_loads_and_a_second_embed_of_that_host_loads_on_its_own(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(
        cx,
        &[
            (URL, Hop::Body(png(10, 40))),
            (OTHER, Hop::Body(png(10, 40))),
        ],
    );
    let text = format!("{NOTE}\n![b]({OTHER})\nend");
    let (v, s) = open(cx, &rt, &text, &[]);
    assert_eq!(look(&s, 3, cx), Some(both(HOST)));
    press_button(&s, URL, RemoteAction::AlwaysLoad, cx);
    drive(&rt, cx, |cx| ready(&s, 1, cx) && ready(&s, 3, cx));
    assert_eq!(
        v.settings.read().unwrap().remote_image_hosts,
        vec![HOST.to_string()]
    );
    assert!(store(cx).is_allowed(HOST));
    let mut calls = env.fake.calls();
    calls.sort();
    assert_eq!(calls, vec![OTHER.to_string(), URL.to_string()]);
}

#[gpui_kit::test]
fn an_allowed_host_loads_on_screen_embeds_only(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let far = "https://img.example.com/far.png";
    let env = install(
        cx,
        &[(URL, Hop::Body(png(10, 40))), (far, Hop::Body(png(10, 40)))],
    );
    let text = format!("{NOTE}\n{}![c]({far})\nend", "line\n".repeat(200));
    let (_v, s) = open(cx, &rt, &text, &[HOST]);
    assert!(store(cx).is_allowed(HOST), "the settings reach the store");
    drive(&rt, cx, |cx| ready(&s, 1, cx));
    for _ in 0..5 {
        drive(&rt, cx, |cx| ready(&s, 1, cx));
    }
    assert_eq!(env.fake.calls(), vec![URL.to_string()]);
    assert_eq!(fetches(cx), 1);
}

#[gpui_kit::test]
fn a_dead_address_on_an_allowed_host_is_fetched_once_across_redraws(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(cx, &[]);
    let text = format!("{NOTE}\n{}", "line\n".repeat(80));
    let (_v, s) = open(cx, &rt, &text, &[HOST]);
    drive(&rt, cx, |cx| {
        matches!(look(&s, 1, cx), Some(RemoteLook::Failed { .. }))
    });
    for key in ["down", "down", "ctrl-end", "ctrl-home", "up"] {
        hit(&s, key, cx);
        for _ in 0..3 {
            drive(&rt, cx, |cx| {
                render(&s, cx);
                true
            });
        }
    }
    assert_eq!(env.fake.calls(), vec![URL.to_string()]);
    assert_eq!(fetches(cx), 1);
}

#[gpui_kit::test]
fn a_press_on_a_placeholder_button_does_not_move_the_caret(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[(URL, Hop::Body(png(10, 40)))]);
    let (_v, s) = open(cx, &rt, NOTE, &[]);
    let view = editor(&s, cx);
    assert_eq!(cx.read(|cx| view.read(cx).editor.cursor()), (0, 0));
    press_button(&s, URL, RemoteAction::Load, cx);
    assert_eq!(cx.read(|cx| view.read(cx).editor.cursor()), (0, 0));
    assert!(
        cx.read(|cx| view.read(cx).editor.selection())
            .is_none_or(|(a, b)| a == b),
        "no selection"
    );
    assert!(!cx.read(|cx| view.read(cx).dragging()), "no drag");
    drive(&rt, cx, |cx| ready(&s, 1, cx));
}

#[gpui_kit::test]
fn a_right_click_on_a_placeholder_opens_no_menu(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    let (_v, s) = open(cx, &rt, NOTE, &[]);
    render(&s, cx);
    let view = editor(&s, cx);
    let at = cx.read(|cx| {
        let p = view.read(cx).painted.clone().expect("painted");
        point(
            p.origin.x + px(4.),
            p.origin.y + px(p.cells.y_of(1) + p.cells.height() / 2.),
        )
    });
    click_at(&s, at, MouseButton::Right, cx);
    assert!(!any_menu(&s, cx), "no popup menu");
    let (note_menu, link_menu) = cx.read(|cx| {
        let w = s.workspace.read(cx);
        (w.note_menu.is_some(), w.link_menu.is_some())
    });
    assert!(!note_menu && !link_menu, "no note or link menu");
    assert_eq!(cx.read(|cx| view.read(cx).editor.cursor()), (0, 0));
}

#[gpui_kit::test]
fn the_cursor_row_shows_the_raw_remote_line_with_the_placeholder_below(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    let (_v, s) = open(cx, &rt, NOTE, &[]);
    hit(&s, "down", cx);
    let view = editor(&s, cx);
    assert_eq!(cx.read(|cx| view.read(cx).editor.cursor().0), 1);
    assert_eq!(look(&s, 1, cx), Some(both(HOST)));
    let (raw, top, below) = cx.read(|cx| {
        let p = view.read(cx).painted.clone().expect("painted");
        let raw: String = p.row_paint[1]
            .pieces
            .iter()
            .map(|piece| piece.text.as_str())
            .collect();
        let look = p.remote_looks.iter().find(|l| l.row == 1).unwrap().rect;
        (raw, look.origin.y, p.origin.y + px(p.cells.y_of(2)))
    });
    assert!(raw.contains("https://img.example.com/pic.png"), "{raw:?}");
    assert_eq!(top, below, "the placeholder starts below the raw line");
    assert_eq!(lines_of(&s, 1, cx), 3, "the raw line and two lines below");
}

#[gpui_kit::test]
fn a_failed_load_shows_the_note_and_retry_asks_again(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(cx, &[]);
    let (_v, s) = open(cx, &rt, NOTE, &[]);
    press_button(&s, URL, RemoteAction::Load, cx);
    drive(&rt, cx, |cx| {
        matches!(look(&s, 1, cx), Some(RemoteLook::Failed { .. }))
    });
    assert_eq!(
        look(&s, 1, cx),
        Some(RemoteLook::Failed {
            note: "Load failed: HTTP 404".to_string(),
            retry: true
        })
    );
    assert_eq!(lines_of(&s, 1, cx), 2, "the failure keeps the two lines");
    env.fake.answer(URL, Hop::Body(png(10, 40)));
    press_button(&s, URL, RemoteAction::Retry, cx);
    drive(&rt, cx, |cx| ready(&s, 1, cx));
    assert_eq!(fetches(cx), 2);
}

#[gpui_kit::test]
fn the_redirect_blocked_placeholder_names_the_host_and_load_follows_it_once(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let target = "https://cdn.other.net/pic.png";
    let env = install(
        cx,
        &[
            (URL, Hop::Redirect(target.to_string())),
            (target, Hop::Body(png(10, 40))),
        ],
    );
    let (v, s) = open(cx, &rt, NOTE, &[]);
    press_button(&s, URL, RemoteAction::Load, cx);
    drive(&rt, cx, |cx| {
        matches!(look(&s, 1, cx), Some(RemoteLook::RedirectBlocked { .. }))
    });
    assert_eq!(
        look(&s, 1, cx),
        Some(RemoteLook::RedirectBlocked {
            host: "cdn.other.net".to_string(),
            actions: Actions {
                load: true,
                always: true
            }
        })
    );
    press_button(&s, URL, RemoteAction::Load, cx);
    drive(&rt, cx, |cx| ready(&s, 1, cx));
    assert_eq!(
        env.fake.calls(),
        vec![URL.to_string(), URL.to_string(), target.to_string()]
    );
    assert!(
        v.settings.read().unwrap().remote_image_hosts.is_empty(),
        "the host is not allowed"
    );
}

#[gpui_kit::test]
fn an_http_embed_shows_the_insecure_note_and_no_actions(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    let text = "intro\n![a](http://img.example.com/pic.png)\nafter";
    let (_v, s) = open(cx, &rt, text, &[HOST]);
    assert_eq!(look(&s, 1, cx), Some(RemoteLook::Insecure));
    let view = editor(&s, cx);
    assert!(
        cx.read(|cx| view
            .read(cx)
            .remote_hits
            .iter()
            .all(|h| h.buttons.is_empty())),
        "no buttons"
    );
    assert_eq!(lines_of(&s, 1, cx), 2);
    assert_eq!(fetches(cx), 0);
}

#[gpui_kit::test]
fn the_menu_of_a_loaded_remote_image_has_the_four_items_and_not_the_vault_menu(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_env, _v, s) = opened_with_copy(cx, &rt);
    right_click_image(&s, cx);
    assert_eq!(
        remote_menu_labels(&s, cx),
        [
            "Open in browser",
            "Copy image address",
            "Reload",
            "Clear image cache"
        ]
    );
    assert!(
        cx.read(|cx| s.workspace.read(cx).note_menu.is_none()),
        "no note menu"
    );
    // The item explains what it removes.
    cx.update_window(s.handle, |_, window, cx| {
        crate::editor_element::BUILT_TIPS.with(|b| b.borrow_mut().clear());
        window.hover(crate::editor_view::CLEAR_CACHE_ITEM_ID, cx);
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    render(&s, cx);
    let built = crate::editor_element::BUILT_TIPS.with(|b| b.borrow().clone());
    assert_eq!(
        built.last().map(|t| t.as_ref()),
        Some("Remove the saved copy of this image")
    );
}

#[gpui_kit::test]
fn open_in_browser_double_click_and_followlink_use_the_handler_with_the_address(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_env, _v, s) = opened_with_copy(cx, &rt);
    let opened = record_opens(&s, cx);
    let want = || vec![OsString::from(URL)];

    right_click_image(&s, cx);
    pick(&s, "Open in browser", cx);
    drive(&rt, cx, |_| !opened.borrow().is_empty());
    assert_eq!(*opened.borrow(), want(), "the menu item");

    opened.borrow_mut().clear();
    let view = editor(&s, cx);
    cx.update(|cx| view.update(cx, |v, _| v.set_clock(crate::editor_view::frozen_clock)));
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(image_hover_id(1), cx);
    })
    .unwrap();
    cx.run_until_parked();
    drive(&rt, cx, |_| !opened.borrow().is_empty());
    assert_eq!(*opened.borrow(), want(), "the double-click");

    opened.borrow_mut().clear();
    hit(&s, "escape", cx);
    hit(&s, "ctrl-home", cx);
    hit(&s, "down", cx);
    assert_eq!(cx.read(|cx| view.read(cx).editor.cursor().0), 1);
    hit(&s, "ctrl-enter", cx);
    drive(&rt, cx, |_| !opened.borrow().is_empty());
    assert_eq!(*opened.borrow(), want(), "the FollowLink chord");
    assert_eq!(fetches(cx), 0);
}

/// Reload keeps the saved copy drawn while it fetches. A failed Reload
/// keeps the copy and the image. New bytes replace the copy, and the line
/// takes the new image's rows. "Clear image cache" then gives the line back
/// its placeholder.
#[gpui_kit::test]
fn clear_image_cache_returns_the_line_to_the_placeholder_and_reload_fetches_again(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let (env, _v, s) = opened_with_copy(cx, &rt);
    let tall = lines_of(&s, 1, cx);

    // The fake answers 404: the old copy and its image stay.
    right_click_image(&s, cx);
    pick(&s, "Reload", cx);
    assert_eq!(look(&s, 1, cx), None, "no placeholder during the fetch");
    drive(&rt, cx, |cx| fetches(cx) == 1 && ready(&s, 1, cx));
    assert!(
        env.cache.find(URL).is_some(),
        "the failed Reload keeps the copy"
    );
    assert_eq!(look(&s, 1, cx), None, "no failure note");
    assert_eq!(lines_of(&s, 1, cx), tall, "the text does not move");

    // New bytes, a shorter image: they replace the copy.
    env.fake.answer(URL, Hop::Body(png(10, 40)));
    right_click_image(&s, cx);
    pick(&s, "Reload", cx);
    drive(&rt, cx, |cx| {
        fetches(cx) == 2 && ready(&s, 1, cx) && lines_of(&s, 1, cx) < tall
    });
    let saved = std::fs::read(env.cache.find(URL).unwrap()).unwrap();
    assert_eq!(saved, png(10, 40), "the new bytes replaced the copy");

    right_click_image(&s, cx);
    pick_clear(&s, cx);
    drive(&rt, cx, |cx| look(&s, 1, cx) == Some(both(HOST)));
    assert!(env.cache.find(URL).is_none(), "the copy is gone");
    assert_eq!(lines_of(&s, 1, cx), 2, "the placeholder's two lines");
    assert_eq!(fetches(cx), 2, "clearing asks nothing");
}

#[gpui_kit::test]
fn copy_image_address_puts_the_address_on_the_clipboard(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_env, _v, s) = opened_with_copy(cx, &rt);
    right_click_image(&s, cx);
    pick(&s, "Copy image address", cx);
    drive(&rt, cx, |_| true);
    let text = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(text.as_deref(), Some(URL));
}

#[gpui_kit::test]
fn a_vault_image_beside_a_remote_embed_draws_as_before(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    let text = format!("{NOTE}\n![v](pic.png)\nend");
    let v = rt.block_on(test_vault(&[("a.md", &text)], &[]));
    rt.block_on(
        v.vault
            .save_attachment(&VaultPath::new("pic.png"), &png(10, 60)),
    )
    .expect("the attachment is saved");
    let s = open_started(
        cx,
        vault_start(&v, FirstNote::Note(VaultPath::new("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| {
        s.text(cx) == text && !s.indexing(cx) && ready(&s, 3, cx)
    });
    assert_eq!(look(&s, 1, cx), Some(both(HOST)));
    right_click_image_row(&s, 3, cx);
    assert!(
        menu_labels(&s, cx)
            .iter()
            .any(|l| l == "Open in default app"),
        "the vault image keeps its entry menu"
    );
}

/// Right-click the image of row `row`.
fn right_click_image_row(s: &Started, row: usize, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(image_hover_id(row), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Another view loads the image (here the store, with no call back to this
/// editor): this editor gives it the rows of its height at its next frame.
/// A copy cleared elsewhere gives the line back its two lines.
#[gpui_kit::test]
fn a_copy_loaded_or_cleared_elsewhere_resizes_the_line_here(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[(URL, Hop::Body(png(10, 200)))]);
    let (_v, s) = open(cx, &rt, NOTE, &[]);
    assert_eq!(lines_of(&s, 1, cx), 2);
    cx.update(|cx| cx.global::<RemoteImages>().clone().load(URL, cx, |_| {}));
    drive(&rt, cx, |cx| ready(&s, 1, cx) && lines_of(&s, 1, cx) > 2);
    store(cx).clear_copy(URL);
    drive(&rt, cx, |cx| {
        look(&s, 1, cx) == Some(both(HOST)) && lines_of(&s, 1, cx) == 2
    });
}

/// Type `text` into the focused editor.
fn type_in(s: &Started, text: &str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.input(text, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Typing the address of an embed on an allowed host asks nothing while the
/// caret stays on its line: each keystroke makes a new address, which would
/// fail and flash. The image loads once the caret leaves the line.
#[gpui_kit::test]
fn an_address_typed_on_an_allowed_host_loads_only_after_the_caret_leaves_the_line(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(cx, &[(URL, Hop::Body(png(10, 40)))]);
    let text = "![a](https://img.example.com/p)\nafter";
    let (_v, s) = open(cx, &rt, text, &[HOST]);
    hit(&s, "end", cx);
    hit(&s, "left", cx);
    let mut typed = Vec::new();
    for ch in ["i", "c", ".", "p", "n", "g"] {
        type_in(&s, ch, cx);
        typed.push(s.text(cx));
        drive(&rt, cx, |cx| {
            render(&s, cx);
            true
        });
    }
    assert_eq!(s.text(cx), "![a](https://img.example.com/pic.png)\nafter");
    // A fetch that started shows Loading or a failure: no timing involved.
    for partial in [
        "/p", "/pi", "/pic", "/pic.", "/pic.p", "/pic.pn", "/pic.png",
    ] {
        let url = format!("https://{HOST}{partial}");
        assert_eq!(
            store(cx).state(&url),
            crate::remote_image::RemoteState::Blocked {
                host: HOST.to_string()
            },
            "{url} was not asked"
        );
    }
    assert_eq!(fetches(cx), 0, "no fetch while the caret is on the line");
    hit(&s, "down", cx);
    drive(&rt, cx, |cx| ready(&s, 0, cx));
    assert_eq!(env.fake.calls(), vec![URL.to_string()], "one fetch");
}
