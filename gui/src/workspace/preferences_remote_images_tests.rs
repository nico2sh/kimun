//! UI integration tests for Preferences > **Remote images**
//! ([`remote_images`]): a real `Workspace` on a real temp vault whose
//! config file is the one written, with a note that embeds remote images.
//! The fetcher is a fake and the image cache is a temp folder, so no test
//! uses the network.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the draft, the settings
//! writes, the cache size and the clear run off the UI thread.
//!
//! [`remote_images`]: super::preferences::remote_images
#![cfg(test)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use tokio::runtime::Runtime;

use kimun_notes::logic::attachment::human_size;

use super::LayoutStore;
use super::preferences::remote_images::{
    NO_HOSTS, REMOTE_CLEAR_ID, REMOTE_MESSAGE_ID, REMOTE_NO_HOSTS_ID, REMOTE_REMOVE_ID,
    REMOTE_SIZE_ID, remote_host_row_id, size_text,
};
use super::preferences::{CANCEL_ID, Section};
use super::preferences_tests::{config, on_disk, open, settle};
use super::remote_image_tests::{Env, install, press_button};
use super::start_tests::{Started, drive, runtime};
use crate::editor_view::EditorView;
use crate::image_core::{Actions, RemoteLook};
use crate::remote_image::{FetchFailure, Hop, RemoteImages, RemoteState, fetch_count};
use crate::remote_placeholder::RemoteAction;
use crate::startup::{FirstNote, Start, VaultStart};
use crate::test_vault::{NAME, TestVault, test_vault};
use kimun_notes::settings::themes::Theme;

const HOST: &str = "img.example.com";
const URL: &str = "https://img.example.com/pic.png";
const SECOND: &str = "https://img.example.com/second.png";
const OTHER_HOST: &str = "cdn.example.org";
/// The note: a remote image between two lines of text.
const NOTE: &str = "intro\n![a](https://img.example.com/pic.png)\nafter";
/// The note: two remote images of one host.
const TWO: &str = "intro\n![a](https://img.example.com/pic.png)\nmid\n![b](https://img.example.com/second.png)\nend";

/// A PNG of `w`x`h` pixels.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 10, 10, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A vault holding `a.md` (`text`), with `hosts` allowed, the window on
/// `a.md` and saving to the vault's config. Driven until the text shows and
/// the index is done.
fn open_note(
    cx: &mut TestAppContext,
    rt: &Runtime,
    text: &str,
    hosts: &[&str],
) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(&[("a.md", text)], &[]));
    {
        let mut s = v.settings.write().unwrap();
        s.theme = Theme::default().name;
        s.remote_image_hosts = hosts.iter().map(|h| h.to_string()).collect();
    }
    let start = Start::Vault(VaultStart {
        vault: v.vault.clone(),
        name: NAME.into(),
        first: FirstNote::Note(VaultPath::new("a.md")),
    });
    let s = open(cx, start, v.settings.clone(), LayoutStore::File(config(&v)));
    drive(rt, cx, |cx| {
        render(&s, cx);
        s.text(cx) == text && !s.indexing(cx)
    });
    (v, s)
}

/// Open Preferences on Remote images, the draft read and the size known.
fn show_section(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    s.pf_route(cx);
    s.pf_loaded(rt, cx);
    s.pf_section(cx, Section::RemoteImages);
    drive(rt, cx, |cx| s.pf_read(cx, |v| v.remote_size().is_some()));
}

/// Wait until the section's write or clear has ended.
fn until_idle(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| s.pf_read(cx, |v| !v.remote_working()));
}

/// Leave Preferences (the draft is clean) and draw the window.
fn close_prefs(s: &Started, cx: &mut TestAppContext) {
    s.pf_click(CANCEL_ID, cx);
    assert!(!s.pf_is_open(cx), "Preferences closed");
    render(s, cx);
}

fn render(s: &Started, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.refresh();
        window.render_frame(cx);
    })
    .unwrap();
}

/// Whether the element `id` is drawn now.
fn shows(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
    let id = id.into();
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some_and(|e| e.visible())
    })
    .unwrap()
}

/// The accessible label of the element `id`.
fn label_of(s: &Started, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

fn editor(s: &Started, cx: &TestAppContext) -> Entity<EditorView> {
    cx.read(|cx| s.workspace.read(cx).active_state().editor.clone())
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

fn store(cx: &TestAppContext) -> RemoteImages {
    cx.read(|cx| cx.global::<RemoteImages>().clone())
}

fn fetches(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| fetch_count(cx))
}

/// The placeholder with both buttons: the host is not allowed.
fn both(host: &str) -> RemoteLook {
    RemoteLook::Placeholder {
        host: host.to_string(),
        actions: Actions {
            load: true,
            always: true,
        },
    }
}

fn hosts_in_memory(v: &TestVault) -> Vec<String> {
    v.settings.read().unwrap().remote_image_hosts.clone()
}

/// A saved copy of `url` in the temp cache.
fn saved(env: &Env, url: &str) {
    env.cache.save(url, &png(10, 10)).unwrap();
}

#[gpui_kit::test]
fn the_section_lists_the_allowed_hosts_and_the_cache_size(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(cx, &[]);
    saved(&env, URL);
    let (_v, s) = open_note(cx, &rt, NOTE, &[HOST, OTHER_HOST]);
    show_section(&s, &rt, cx);
    for host in [HOST, OTHER_HOST] {
        assert!(shows(&s, remote_host_row_id(host), cx), "{host} is listed");
    }
    assert!(shows(&s, REMOTE_REMOVE_ID, cx), "Remove is under the list");
    // Alphabetical, whatever the stored order, with the first row selected.
    assert_eq!(
        s.pf_read(cx, |v| v.remote_selected()).as_deref(),
        Some(OTHER_HOST),
        "cdn.example.org sorts before img.example.com"
    );
    assert!(!shows(&s, REMOTE_NO_HOSTS_ID, cx));
    assert!(shows(&s, REMOTE_CLEAR_ID, cx));
    let bytes = env.cache.size();
    assert!(bytes > 0);
    assert_eq!(s.pf_read(cx, |v| v.remote_size()), Some(bytes));
    assert_eq!(
        label_of(&s, REMOTE_SIZE_ID, cx),
        Some(size_text(Some(bytes)))
    );
    assert!(size_text(Some(bytes)).contains(&human_size(bytes)));
    settle(&rt, cx);
}

#[gpui_kit::test]
fn an_empty_list_shows_the_no_hosts_line(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    let (_v, s) = open_note(cx, &rt, "plain", &[]);
    show_section(&s, &rt, cx);
    assert!(shows(&s, REMOTE_NO_HOSTS_ID, cx));
    assert_eq!(
        label_of(&s, REMOTE_NO_HOSTS_ID, cx).as_deref(),
        Some(NO_HOSTS)
    );
    assert_eq!(NO_HOSTS, "No hosts allowed");
    assert_eq!(s.pf_read(cx, |v| v.remote_size()), Some(0));
    settle(&rt, cx);
}

#[gpui_kit::test]
fn remove_forgets_the_host_and_keeps_the_saved_copies(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(cx, &[]);
    saved(&env, URL);
    let (v, s) = open_note(cx, &rt, NOTE, &[HOST, OTHER_HOST]);
    show_section(&s, &rt, cx);
    s.pf_click(remote_host_row_id(HOST), cx);
    s.pf_click(REMOTE_REMOVE_ID, cx);
    until_idle(&s, &rt, cx);
    assert_eq!(hosts_in_memory(&v), [OTHER_HOST.to_string()]);
    assert_eq!(on_disk(&v).remote_image_hosts, [OTHER_HOST.to_string()]);
    assert!(!shows(&s, remote_host_row_id(HOST), cx), "the row went");
    assert!(shows(&s, remote_host_row_id(OTHER_HOST), cx));
    assert!(env.cache.find(URL).is_some(), "the saved copy stays");
    assert!(!store(cx).is_allowed(HOST), "the store knows at once");
    assert!(store(cx).is_allowed(OTHER_HOST));
    assert!(!shows(&s, REMOTE_MESSAGE_ID, cx), "no problem to show");
    // The last host goes: the empty line shows.
    assert!(!s.pf_read(cx, |v| v.remote_working()));
    s.pf_click(REMOTE_REMOVE_ID, cx);
    until_idle(&s, &rt, cx);
    assert!(hosts_in_memory(&v).is_empty());
    assert!(shows(&s, REMOTE_NO_HOSTS_ID, cx));
    // The saved copy still draws in the open editor, with no fetch.
    close_prefs(&s, cx);
    assert!(ready(&s, 1, cx), "the saved copy still draws");
    assert_eq!(fetches(cx), 0);
    settle(&rt, cx);
}

const THIRD_HOST: &str = "zzz.example.net";

/// A click selects a row, Up and Down move the selection (clamped), and
/// Delete removes the selected host. The selection keeps its place, and
/// moves to the last row when the last row goes.
#[gpui_kit::test]
fn the_list_selects_with_clicks_and_keys_and_delete_removes_the_selected_host(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    // Stored out of order: the list shows cdn, img, zzz.
    let (v, s) = open_note(cx, &rt, "plain", &[THIRD_HOST, HOST, OTHER_HOST]);
    show_section(&s, &rt, cx);
    let selected = |s: &Started, cx: &mut TestAppContext| s.pf_read(cx, |v| v.remote_selected());
    assert_eq!(selected(&s, cx).as_deref(), Some(OTHER_HOST));
    s.pf_click(remote_host_row_id(HOST), cx);
    assert_eq!(selected(&s, cx).as_deref(), Some(HOST), "a click selects");
    s.pf_press("down", cx);
    assert_eq!(selected(&s, cx).as_deref(), Some(THIRD_HOST));
    s.pf_press("down", cx);
    assert_eq!(selected(&s, cx).as_deref(), Some(THIRD_HOST), "clamped");
    s.pf_press("up", cx);
    s.pf_press("up", cx);
    s.pf_press("up", cx);
    assert_eq!(selected(&s, cx).as_deref(), Some(OTHER_HOST), "clamped");
    // Delete removes the selected host and the selection keeps its place.
    s.pf_press("down", cx);
    s.pf_press("delete", cx);
    until_idle(&s, &rt, cx);
    assert_eq!(
        on_disk(&v).remote_image_hosts.len(),
        2,
        "one host went, with no question"
    );
    assert!(!hosts_in_memory(&v).contains(&HOST.to_string()));
    assert_eq!(selected(&s, cx).as_deref(), Some(THIRD_HOST), "same place");
    // The last row goes: the selection moves to the new last row.
    s.pf_press("backspace", cx);
    until_idle(&s, &rt, cx);
    assert_eq!(hosts_in_memory(&v), [OTHER_HOST.to_string()]);
    assert_eq!(selected(&s, cx).as_deref(), Some(OTHER_HOST));
    s.pf_press("delete", cx);
    until_idle(&s, &rt, cx);
    assert!(hosts_in_memory(&v).is_empty());
    s.pf_press("delete", cx);
    until_idle(&s, &rt, cx);
    assert!(shows(&s, REMOTE_NO_HOSTS_ID, cx));
    settle(&rt, cx);
}

/// Remove is disabled with no host, and Preferences read "Close" while the
/// draft is clean.
#[gpui_kit::test]
fn remove_waits_for_a_host_and_the_footer_says_close_when_clean(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    let (_v, s) = open_note(cx, &rt, "plain", &[]);
    show_section(&s, &rt, cx);
    assert_eq!(label_of(&s, CANCEL_ID, cx).as_deref(), Some("Close"));
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(REMOTE_REMOVE_ID).disabled(), Some(true));
    })
    .unwrap();
    settle(&rt, cx);
}

/// A write that fails (the config on disk does not parse) shows why in the
/// section, and the list and the store stay as they were.
#[gpui_kit::test]
fn a_failed_remove_keeps_the_list_and_the_store(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[]);
    let (v, s) = open_note(cx, &rt, "plain", &[HOST]);
    show_section(&s, &rt, cx);
    std::fs::write(config(&v), "this is [not toml").unwrap();
    s.pf_click(REMOTE_REMOVE_ID, cx);
    until_idle(&s, &rt, cx);
    assert_eq!(hosts_in_memory(&v), [HOST.to_string()]);
    assert!(shows(&s, remote_host_row_id(HOST), cx), "the row stays");
    assert!(store(cx).is_allowed(HOST), "the store is unchanged");
    let message = label_of(&s, REMOTE_MESSAGE_ID, cx).expect("the reason is shown");
    assert!(
        message.contains("Couldn't save the allowed hosts"),
        "{message}"
    );
    assert_eq!(
        std::fs::read_to_string(config(&v)).unwrap(),
        "this is [not toml"
    );
    settle(&rt, cx);
}

/// "Always load from" when the settings write fails (the config on disk does
/// not parse): the host is not allowed, in the settings or in the store, the
/// error flashes, and the one image the reader asked for still loads.
#[gpui_kit::test]
fn a_failed_always_load_write_leaves_the_host_unallowed_and_still_loads_the_image(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(cx, &[(URL, Hop::Body(png(10, 10)))]);
    let (v, s) = open_note(cx, &rt, NOTE, &[]);
    std::fs::write(config(&v), "this is [not toml").unwrap();
    press_button(&s, URL, RemoteAction::AlwaysLoad, cx);
    drive(&rt, cx, |cx| ready(&s, 1, cx));
    assert_eq!(fetches(cx), 1, "this one image loads");
    assert!(hosts_in_memory(&v).is_empty(), "the settings stay");
    assert!(!store(cx).is_allowed(HOST), "the store does not allow it");
    let flash = s.flash(cx).expect("the error flashes");
    assert!(flash.contains("Couldn't save the settings"), "{flash}");
    assert_eq!(
        std::fs::read_to_string(config(&v)).unwrap(),
        "this is [not toml"
    );
    settle(&rt, cx);
}

#[gpui_kit::test]
fn a_removed_host_loads_no_more_on_its_own_in_an_open_view(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let _env = install(
        cx,
        &[
            (URL, Hop::Body(png(10, 10))),
            (SECOND, Hop::Body(png(10, 10))),
        ],
    );
    let (_v, s) = open_note(cx, &rt, NOTE, &[HOST]);
    // The allowed host loads on its own in the open editor.
    drive(&rt, cx, |cx| ready(&s, 1, cx));
    assert_eq!(fetches(cx), 1);
    show_section(&s, &rt, cx);
    s.pf_click(REMOTE_REMOVE_ID, cx);
    until_idle(&s, &rt, cx);
    // The saved copies go too, so the open editor shows the placeholder.
    s.pf_click(REMOTE_CLEAR_ID, cx);
    until_idle(&s, &rt, cx);
    close_prefs(&s, cx);
    for _ in 0..5 {
        assert_eq!(look(&s, 1, cx), Some(both(HOST)), "Always load shows again");
        settle(&rt, cx);
    }
    assert_eq!(fetches(cx), 1, "no fetch on its own");
    settle(&rt, cx);
}

#[gpui_kit::test]
fn clear_cached_images_empties_the_cache_and_updates_the_size_and_the_open_views(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let env = install(cx, &[]);
    saved(&env, URL);
    // A damaged copy: a PNG cut short, so it does not decode.
    let mut cut = png(10, 10);
    cut.truncate(40);
    env.cache.save(SECOND, &cut).unwrap();
    let failed = "https://img.example.com/dead.png";
    store(cx).set_state_for_test(failed, RemoteState::Failed(FetchFailure::Status(404)));
    let (_v, s) = open_note(cx, &rt, TWO, &[]);
    assert!(ready(&s, 1, cx), "the saved copy draws");
    assert_eq!(
        look(&s, 3, cx),
        None,
        "the damaged copy is not a placeholder"
    );
    show_section(&s, &rt, cx);
    let before = s.pf_read(cx, |v| v.remote_size()).unwrap();
    assert!(before > 0);
    s.pf_click(REMOTE_CLEAR_ID, cx);
    until_idle(&s, &rt, cx);
    assert_eq!(env.cache.size(), 0, "the cache is empty");
    assert!(env.cache.find(URL).is_none());
    assert!(
        env.cache.find(SECOND).is_none(),
        "the damaged copy went too"
    );
    assert_eq!(s.pf_read(cx, |v| v.remote_size()), Some(0));
    assert_eq!(label_of(&s, REMOTE_SIZE_ID, cx), Some(size_text(Some(0))));
    assert!(
        matches!(store(cx).state(failed), RemoteState::Blocked { .. }),
        "the session failure is forgotten"
    );
    close_prefs(&s, cx);
    assert_eq!(look(&s, 1, cx), Some(both(HOST)), "the placeholder is back");
    assert_eq!(
        look(&s, 3, cx),
        Some(both(HOST)),
        "for the damaged copy too"
    );
    assert_eq!(fetches(cx), 0);
    settle(&rt, cx);
}

#[test]
fn the_cache_size_reads_in_a_human_unit() {
    assert_eq!(human_size(0), "0 B");
    assert_eq!(human_size(512), "512 B");
    assert_eq!(human_size(1536), "1.5 KB");
    assert_eq!(human_size(12_900_000), "12.3 MB");
    assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GB");
}
