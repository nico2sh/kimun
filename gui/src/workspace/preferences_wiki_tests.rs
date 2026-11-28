//! UI integration tests for Preferences > **Wiki** ([`wiki`]): a real
//! `Workspace` over a real temp vault whose config file is the one saved
//! to, driven by keys and clicks.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the section reads the
//! declaration and applies it off the UI thread.
//!
//! [`wiki`]: super::preferences::wiki
#![cfg(test)]

use std::path::PathBuf;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use kimun_core::WikiInitOptions;
use kimun_notes::logic::wiki_settings::{Loaded, WikiState};
use tokio::runtime::Runtime;

use super::preferences::wiki::{
    WIKI_APPLY_ID, WIKI_EFFECT_ID, WIKI_ERROR_ID, WIKI_INERT_ID, WIKI_LOG_ID, WIKI_PROPERTIES_ID,
    WIKI_RESULT_ID, WIKI_SCHEMA_ID, WIKI_SOURCES_ID, WIKI_STATE_ID, WIKI_WIKI_ID, WikiField,
};
use super::preferences::{Section, section_id};
use super::preferences_tests::{on_vault, settle, vault};
use super::preferences_workspaces_tests::no_workspace;
use super::reopen_tests::files_rows;
use super::start_tests::{Started, drive, runtime};
use crate::test_vault::{NAME, TestVault};

/// The window on a fresh vault, Preferences open on Wiki, the declaration
/// read.
fn wiki_section(cx: &mut TestAppContext, rt: &Runtime, v: &TestVault) -> Started {
    let s = on_vault(cx, rt, v);
    s.pf_route(cx);
    s.pf_loaded(rt, cx);
    s.pf_section(cx, Section::Wiki);
    until_read(&s, rt, cx);
    s
}

/// Wait until the section has read the declaration.
fn until_read(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| s.pf_read(cx, |v| !v.wiki_loading()));
}

/// Wait until the apply under way has ended and the section read again.
fn until_applied(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| {
        s.pf_read(cx, |v| !v.wiki_running() && !v.wiki_loading())
    });
}

/// The vault's folder on disk.
fn root(v: &TestVault) -> PathBuf {
    v.workspace.clone().into_path_buf()
}

/// The accessible label of the element `id`, `None` when it is not drawn.
fn label(s: &Started, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|e| e.label().map(str::to_string))
    })
    .unwrap()
}

/// Whether the element `id` has the keyboard.
fn focused(s: &Started, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .unwrap()
}

/// The text of `field`.
fn value(s: &Started, field: WikiField, cx: &mut TestAppContext) -> String {
    let input = s.pf_read(cx, |v| v.wiki_input(field));
    cx.read(|cx| input.read(cx).value().to_string())
}

/// Whether `field` is drawn disabled.
fn field_off(s: &Started, field: WikiField, cx: &mut TestAppContext) -> bool {
    let input = s.pf_read(cx, |v| v.wiki_input(field));
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        !input.read(cx).is_editable()
    })
    .unwrap()
}

/// Give `field` the keyboard and replace its text by typing.
fn type_into(s: &Started, field: WikiField, text: &str, cx: &mut TestAppContext) {
    let input = s.pf_read(cx, |v| v.wiki_input(field));
    cx.update_window(s.handle, |_, window, cx| {
        input.update(cx, |i, cx| i.set_value("", window, cx));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    })
    .unwrap();
    cx.run_until_parked();
    s.pf_typed(text, cx);
}

/// Make the vault a wiki with the default layout, through core.
fn make_wiki(rt: &Runtime, v: &TestVault) {
    rt.block_on(v.vault.wiki_init(WikiInitOptions::default()))
        .expect("wiki init");
}

/// Point the current workspace at another folder, in memory: the window
/// now names another vault.
fn move_vault(v: &TestVault, to: &std::path::Path) {
    v.settings
        .write()
        .unwrap()
        .workspace_config
        .as_mut()
        .unwrap()
        .workspaces
        .get_mut(NAME)
        .unwrap()
        .path = to.to_path_buf();
}

/// The section list has ten sections and Wiki follows Indexing; the
/// section is listed and opens.
#[gpui_kit::test]
fn the_section_list_has_ten_sections_and_wiki_follows_indexing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = wiki_section(cx, &rt, &v);
    assert_eq!(Section::ALL.len(), 10);
    let ix = Section::ALL
        .iter()
        .position(|s| *s == Section::Indexing)
        .unwrap();
    assert_eq!(Section::ALL[ix + 1], Section::Wiki);
    assert!(s.pf_shows(section_id(Section::Wiki), cx));
    assert!(s.pf_shows(WIKI_STATE_ID, cx));
    settle(&rt, cx);
}

/// A vault that is not a wiki: the state line, the default layout in the
/// fields, and "Set up wiki".
#[gpui_kit::test]
fn a_vault_that_is_not_a_wiki_shows_the_state_line_the_defaults_and_set_up_wiki(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = wiki_section(cx, &rt, &v);
    assert_eq!(
        label(&s, WIKI_STATE_ID, cx).as_deref(),
        Some("This vault is not a wiki.")
    );
    assert_eq!(value(&s, WikiField::Sources, cx), "/sources");
    assert_eq!(value(&s, WikiField::Wiki, cx), "/wiki");
    assert_eq!(value(&s, WikiField::Log, cx), "/wiki/log.md");
    assert_eq!(value(&s, WikiField::Schema, cx), "/wiki/schema.md");
    assert!(!value(&s, WikiField::Properties, cx).is_empty());
    assert_eq!(label(&s, WIKI_APPLY_ID, cx).as_deref(), Some("Set up wiki"));
    assert!(!s.pf_disabled(WIKI_APPLY_ID, cx));
    assert!(WikiField::ALL.iter().all(|f| !field_off(&s, *f, cx)));
    assert!(!s.pf_shows(WIKI_INERT_ID, cx));
    assert!(!s.pf_shows(WIKI_EFFECT_ID, cx), "nothing changes yet");
    settle(&rt, cx);
}

/// A wiki: its declaration in the fields, and "Apply".
#[gpui_kit::test]
fn a_wiki_shows_its_declaration_and_apply(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    rt.block_on(v.vault.wiki_init(WikiInitOptions {
        sources: Some(kimun_core::nfs::VaultPath::new("/raw")),
        ..WikiInitOptions::default()
    }))
    .expect("wiki init");
    let s = wiki_section(cx, &rt, &v);
    assert_eq!(
        label(&s, WIKI_STATE_ID, cx).as_deref(),
        Some("This vault is a wiki.")
    );
    assert_eq!(value(&s, WikiField::Sources, cx), "/raw");
    assert_eq!(label(&s, WIKI_APPLY_ID, cx).as_deref(), Some("Apply"));
    assert!(!s.pf_disabled(WIKI_APPLY_ID, cx), "a repair is allowed");
    settle(&rt, cx);
}

/// A broken declaration: its message as the state line, the defaults in
/// the fields, and the button that sets up a good one.
#[gpui_kit::test]
fn a_broken_declaration_shows_its_message_and_the_defaults(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    std::fs::create_dir_all(root(&v).join(".kimun")).unwrap();
    std::fs::write(root(&v).join(".kimun").join("wiki.toml"), "not = [valid").unwrap();
    let s = wiki_section(cx, &rt, &v);
    let state = label(&s, WIKI_STATE_ID, cx).unwrap();
    assert!(state.contains("wiki.toml"), "{state}");
    assert_eq!(value(&s, WikiField::Sources, cx), "/sources");
    assert_eq!(label(&s, WIKI_APPLY_ID, cx).as_deref(), Some("Set up wiki"));
    assert!(!s.pf_disabled(WIKI_APPLY_ID, cx));
    settle(&rt, cx);
}

/// Apply makes the folders and notes and lists them; the draft of the
/// other sections stays clean.
#[gpui_kit::test]
fn apply_creates_the_folders_and_shows_the_result_lines(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = wiki_section(cx, &rt, &v);
    s.pf_click(WIKI_APPLY_ID, cx);
    until_applied(&s, &rt, cx);
    assert!(root(&v).join("sources").is_dir());
    assert!(root(&v).join("wiki").join("log.md").is_file());
    let result = label(&s, WIKI_RESULT_ID, cx).unwrap();
    assert!(result.contains("Created folder /sources."), "{result}");
    assert!(!s.pf_read(cx, |v| v.is_dirty()), "the draft stays clean");
    // FILES lists the new folders without a reopen.
    drive(&rt, cx, |cx| {
        files_rows(&s, cx).contains(&"sources/".to_string())
    });
    settle(&rt, cx);
}

/// Someone deleted the wiki folder: Apply with the unchanged draft makes
/// it again and says so.
#[gpui_kit::test]
fn a_repair_apply_recreates_a_deleted_wiki_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    rt.block_on(v.vault.wiki_init(WikiInitOptions::default()))
        .unwrap();
    std::fs::remove_dir_all(root(&v).join("wiki")).unwrap();
    let s = wiki_section(cx, &rt, &v);
    s.pf_click(WIKI_APPLY_ID, cx);
    until_applied(&s, &rt, cx);
    assert!(root(&v).join("wiki").is_dir());
    assert!(root(&v).join("wiki").join("log.md").is_file());
    let result = label(&s, WIKI_RESULT_ID, cx).unwrap();
    assert!(result.contains("Created folder /wiki."), "{result}");
    settle(&rt, cx);
}

/// A failed apply shows the core's error, writes no declaration and keeps
/// the typed text.
#[gpui_kit::test]
fn a_failed_apply_shows_the_error_and_changes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    std::fs::write(root(&v).join("other"), "a plain file").unwrap();
    let s = wiki_section(cx, &rt, &v);
    type_into(&s, WikiField::Sources, "/other", cx);
    s.pf_click(WIKI_APPLY_ID, cx);
    until_applied(&s, &rt, cx);
    assert!(s.pf_read(cx, |v| v.wiki_failed()), "the result is an error");
    assert!(label(&s, WIKI_RESULT_ID, cx).is_some_and(|t| !t.is_empty()));
    assert!(!root(&v).join(".kimun").join("wiki.toml").exists());
    assert_eq!(
        label(&s, WIKI_STATE_ID, cx).as_deref(),
        Some("This vault is not a wiki.")
    );
    assert_eq!(value(&s, WikiField::Sources, cx), "/other");
    settle(&rt, cx);
}

/// Without a vault the fields and the button are off and say why; Apply
/// does nothing.
#[gpui_kit::test]
fn without_a_vault_the_fields_and_the_button_are_disabled(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (s, _settings, _root, _dir) = no_workspace(cx);
    s.pf_press("f4", cx);
    s.pf_loaded(&rt, cx);
    s.pf_section(cx, Section::Wiki);
    until_read(&s, &rt, cx);
    assert!(s.pf_disabled(WIKI_APPLY_ID, cx), "the button is off");
    for field in WikiField::ALL {
        assert!(field_off(&s, field, cx), "{field:?} is off");
    }
    assert!(s.pf_shows(WIKI_INERT_ID, cx), "says why");
    s.pf_with_view(cx, |v, window, cx| v.wiki_apply(window, cx));
    assert!(!s.pf_read(cx, |v| v.wiki_running()));
    settle(&rt, cx);
}

/// A second press while an apply runs does nothing: the draft changed
/// after the first press is not applied.
#[gpui_kit::test]
fn a_second_press_while_an_apply_runs_does_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = wiki_section(cx, &rt, &v);
    s.pf_click(WIKI_APPLY_ID, cx);
    assert!(s.pf_read(cx, |v| v.wiki_running()));
    assert!(s.pf_disabled(WIKI_APPLY_ID, cx), "the button waits");
    s.pf_with_view(cx, |v, window, cx| {
        v.wiki_settings_mut().set_sources("/second");
        v.wiki_apply(window, cx);
    });
    until_applied(&s, &rt, cx);
    assert!(root(&v).join("sources").is_dir());
    assert!(!root(&v).join("second").exists(), "no second apply");
    settle(&rt, cx);
}

/// A result for a vault the window no longer names is dropped: an apply
/// that lands after the vault changed, and a read that does.
#[gpui_kit::test]
fn a_result_that_arrives_after_the_vault_changed_is_dropped(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let other = tempfile::TempDir::new().unwrap();
    let other_root = other.path().canonicalize().unwrap();
    let s = wiki_section(cx, &rt, &v);
    let first = v.workspace.clone();
    s.pf_click(WIKI_APPLY_ID, cx);
    assert!(s.pf_read(cx, |v| v.wiki_running()));
    move_vault(&v, &other_root);
    until_applied(&s, &rt, cx);
    assert!(
        label(&s, WIKI_RESULT_ID, cx).is_none(),
        "the old vault's result is not shown"
    );
    // That frame saw the new vault: the section reads it.
    until_read(&s, &rt, cx);
    assert_eq!(
        label(&s, WIKI_STATE_ID, cx).as_deref(),
        Some("This vault is not a wiki."),
        "the section shows the new vault"
    );
    // A late read of the old vault: dropped too.
    s.pf_with_view(cx, |v, window, cx| {
        v.wiki_loaded(
            first,
            Loaded {
                state: WikiState::Wiki(kimun_core::WikiDeclaration::defaults()),
                schema_exists: true,
            },
            window,
            cx,
        )
    });
    assert_eq!(
        label(&s, WIKI_STATE_ID, cx).as_deref(),
        Some("This vault is not a wiki.")
    );
    settle(&rt, cx);
}

/// After an apply the section reads the vault again: it is a wiki now.
#[gpui_kit::test]
fn the_section_reads_again_after_an_apply(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = wiki_section(cx, &rt, &v);
    s.pf_click(WIKI_APPLY_ID, cx);
    until_applied(&s, &rt, cx);
    assert_eq!(
        label(&s, WIKI_STATE_ID, cx).as_deref(),
        Some("This vault is a wiki.")
    );
    assert_eq!(label(&s, WIKI_APPLY_ID, cx).as_deref(), Some("Apply"));
    assert!(label(&s, WIKI_RESULT_ID, cx).is_some(), "the lines stay");
    settle(&rt, cx);
}

/// A new sources folder shows which folder stops being protected and
/// which starts, before the apply. An invalid text shows the error and
/// blocks Apply.
#[gpui_kit::test]
fn changing_the_sources_folder_shows_the_protection_note_before_the_apply(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    make_wiki(&rt, &v);
    let s = wiki_section(cx, &rt, &v);
    assert!(!s.pf_shows(WIKI_EFFECT_ID, cx));
    type_into(&s, WikiField::Sources, "raw", cx);
    let effect = label(&s, WIKI_EFFECT_ID, cx).unwrap();
    assert!(
        effect.contains("Notes in /sources stay where they are and are no longer protected.")
            && effect.contains("/raw is the new protected folder."),
        "{effect}"
    );
    assert!(!root(&v).join("raw").exists(), "nothing is applied yet");
    type_into(&s, WikiField::Sources, "/wiki/raw", cx);
    assert!(s.pf_shows(WIKI_ERROR_ID, cx));
    assert!(s.pf_disabled(WIKI_APPLY_ID, cx));
    settle(&rt, cx);
}

/// The log and schema follow a valid wiki folder, and keep their text while
/// the wiki folder text is invalid.
#[gpui_kit::test]
fn the_log_and_schema_fields_follow_the_wiki_field(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = wiki_section(cx, &rt, &v);
    type_into(&s, WikiField::Wiki, "atlas", cx);
    assert_eq!(value(&s, WikiField::Log, cx), "/atlas/log.md");
    assert_eq!(value(&s, WikiField::Schema, cx), "/atlas/schema.md");
    type_into(&s, WikiField::Wiki, "bad?name", cx);
    assert_eq!(value(&s, WikiField::Log, cx), "/atlas/log.md");
    assert!(s.pf_shows(WIKI_ERROR_ID, cx));
    settle(&rt, cx);
}

/// Tab walks the five fields, then the button; Enter in a field applies.
#[gpui_kit::test]
fn tab_visits_the_five_fields_then_the_button_and_enter_applies(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = vault(&rt);
    let s = wiki_section(cx, &rt, &v);
    let input = s.pf_read(cx, |v| v.wiki_input(WikiField::Sources));
    cx.update_window(s.handle, |_, window, cx| {
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(focused(&s, WIKI_SOURCES_ID, cx));
    for id in [
        WIKI_WIKI_ID,
        WIKI_LOG_ID,
        WIKI_SCHEMA_ID,
        WIKI_PROPERTIES_ID,
    ] {
        s.pf_press("tab", cx);
        assert!(focused(&s, id, cx), "Tab reaches {id}");
    }
    s.pf_press("tab", cx);
    assert!(focused(&s, WIKI_APPLY_ID, cx), "then the button");
    s.pf_press("shift-tab", cx);
    assert!(focused(&s, WIKI_PROPERTIES_ID, cx));
    s.pf_press("enter", cx);
    assert!(s.pf_read(cx, |v| v.wiki_running()), "Enter applies");
    until_applied(&s, &rt, cx);
    assert!(root(&v).join("sources").is_dir());
    assert!(s.pf_is_open(cx), "the view stays");
    assert_eq!(s.pf_read(cx, |v| v.section()), Section::Wiki);
    settle(&rt, cx);
}
