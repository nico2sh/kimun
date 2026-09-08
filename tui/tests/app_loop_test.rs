//! The **App loop**, driven headless: a `TestBackend` for the terminal and a
//! scripted **Input source** for the keys (CONTEXT.md § App shell).
//!
//! Each test pins one rule the loop owns. The screens are the real ones; the
//! vault is a real temp-dir vault; only the terminal and the keyboard are fake.
//!
//! Temp directories are leaked (`keep()`) on purpose, as `test_support` does:
//! screens spawn tasks that hold the vault, and on Windows a directory with an
//! open index cannot be removed — `tempfile` would swallow that silently.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use futures::stream;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use kimun_core::nfs::VaultPath;
use kimun_notes::app::events::EventHandler;
use kimun_notes::app::{App, run_app};
use kimun_notes::app_screen::ScreenKind;
use kimun_notes::app_screen::editor::EditorScreen;
use kimun_notes::app_screen::preferences::PreferencesScreen;
use kimun_notes::components::events::AppEvent;
use kimun_notes::keys::action_shortcuts::ActionShortcuts;
use kimun_notes::keys::key_event_to_combo;
use kimun_notes::settings::{AppSettings, SharedSettings};

mod common;
use common::{buffer_text, input, press, write_config};

struct Fixture {
    app: App,
    settings: SharedSettings,
    workspace: PathBuf,
    config_path: PathBuf,
}

async fn fixture(name: &str) -> Fixture {
    let workspace = tempfile::Builder::new()
        .prefix(&format!("kimun_loop_{name}_ws_"))
        .tempdir()
        .unwrap()
        .keep();
    let config_dir = tempfile::Builder::new()
        .prefix(&format!("kimun_loop_{name}_cfg_"))
        .tempdir()
        .unwrap()
        .keep();
    let config_path = config_dir.join("config.toml");
    write_config(&config_path, &workspace);

    let settings: SharedSettings = Arc::new(RwLock::new(
        AppSettings::load_from_file(config_path.clone()).expect("settings load"),
    ));
    let app = App::from_settings(settings.clone()).await;
    app.vault
        .as_ref()
        .expect("the config names a workspace")
        .validate_and_init()
        .await
        .expect("index init");
    Fixture {
        app,
        settings,
        workspace,
        config_path,
    }
}

fn terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(120, 40)).unwrap()
}

fn kind(app: &App) -> Option<ScreenKind> {
    app.current_screen.as_ref().map(|s| s.get_kind())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exhausted_input_source_quits_the_loop() {
    let mut fx = fixture("exhausted").await;
    fx.app.current_screen = Some(Box::new(PreferencesScreen::new(fx.settings.clone())));
    let mut events = EventHandler::from_input(stream::empty());
    let mut term = terminal();

    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_app(&mut term, &mut fx.app, &mut events),
    )
    .await
    .expect("the loop must return when input ends")
    .expect("the loop returns Ok when input ends");

    assert_eq!(kind(&fx.app), Some(ScreenKind::Preferences));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_quit_shortcut_fires_before_the_screen_and_on_exit_saves_the_note() {
    let mut fx = fixture("quit").await;
    let vault = fx.app.vault.clone().unwrap();
    let note = VaultPath::note_path_from("loop-note");
    vault.create_note(&note, "hello").await.unwrap();
    fx.app.current_screen = Some(Box::new(EditorScreen::new(
        vault,
        note.clone(),
        fx.settings.clone(),
    )));

    // Precondition, so a rebinding shows up here and not as a mystery hang.
    let quit = press(KeyCode::Char('q'), KeyModifiers::CONTROL);
    let combo = key_event_to_combo(&quit).unwrap();
    assert_eq!(
        fx.settings.read().unwrap().key_bindings.get_action(&combo),
        Some(ActionShortcuts::Quit)
    );

    // The trailing 'y' after Ctrl+Q is what makes the assertion below
    // discriminate: if the global shortcut fires first the loop returns
    // before 'y' is ever read and the length assertion still holds; if the
    // editor ever ate Ctrl+Q instead, 'y' would land in the buffer and the
    // assertion would fail.
    let mut events = EventHandler::from_input(stream::iter([
        input(press(KeyCode::Char('x'), KeyModifiers::NONE)),
        input(quit),
        input(press(KeyCode::Char('y'), KeyModifiers::NONE)),
    ]));
    let mut term = terminal();

    run_app(&mut term, &mut fx.app, &mut events).await.unwrap();

    let on_disk = std::fs::read_to_string(note.to_pathbuf(&fx.workspace)).unwrap();
    assert!(
        on_disk.contains('x') && on_disk.len() == "hello".len() + 1,
        "the key reached the editor and on_exit saved it: {on_disk:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_preferences_shortcut_switches_screens_before_the_editor_sees_it() {
    let mut fx = fixture("prefs").await;
    let vault = fx.app.vault.clone().unwrap();
    let note = VaultPath::note_path_from("prefs-note");
    vault.create_note(&note, "hello").await.unwrap();
    fx.app.current_screen = Some(Box::new(EditorScreen::new(
        vault,
        note.clone(),
        fx.settings.clone(),
    )));

    let prefs = press(KeyCode::Char(','), KeyModifiers::CONTROL);
    let combo = key_event_to_combo(&prefs).unwrap();
    assert_eq!(
        fx.settings.read().unwrap().key_bindings.get_action(&combo),
        Some(ActionShortcuts::OpenPreferences)
    );
    let gen_before = fx.app.screen_generation;

    // The trailing key is what makes the assertion discriminate: if the
    // editor ever consumed Ctrl+, the 'y' would reach the buffer and the
    // saved note would change.
    let mut events = EventHandler::from_input(stream::iter([
        input(prefs),
        input(press(KeyCode::Char('y'), KeyModifiers::NONE)),
    ]));
    let mut term = terminal();

    run_app(&mut term, &mut fx.app, &mut events).await.unwrap();

    assert_eq!(kind(&fx.app), Some(ScreenKind::Preferences));
    assert_eq!(
        fx.app.screen_generation,
        gen_before + 1,
        "one swap: editor → preferences"
    );
    let on_disk = std::fs::read_to_string(note.to_pathbuf(&fx.workspace)).unwrap();
    assert_eq!(
        on_disk, "hello",
        "the shortcut never reached the editor; on_exit saved an unchanged note"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vault_conflict_clears_the_workspace_and_opens_preferences_with_the_error() {
    let mut fx = fixture("conflict").await;
    fx.app.current_screen = Some(Box::new(PreferencesScreen::new(fx.settings.clone())));
    let mut events = EventHandler::from_input(stream::empty());
    events
        .app_sender()
        .send(AppEvent::VaultConflict("case clash: a vs A".into()))
        .unwrap();
    let mut term = terminal();
    let gen_before = fx.app.screen_generation;

    run_app(&mut term, &mut fx.app, &mut events).await.unwrap();

    assert_eq!(
        fx.app.screen_generation,
        gen_before + 1,
        "VaultConflict swapped the screen exactly once"
    );
    assert!(fx.app.vault.is_none(), "the unusable vault is dropped");
    assert_eq!(kind(&fx.app), Some(ScreenKind::Preferences));
    assert!(
        fx.settings
            .read()
            .unwrap()
            .current_workspace_name()
            .is_none(),
        "the workspace entry is cleared in memory"
    );
    let reloaded = AppSettings::load_from_file(fx.config_path.clone()).unwrap();
    assert!(
        reloaded.current_workspace_name().is_none(),
        "…and the cleared config was saved to disk"
    );
    assert!(
        buffer_text(&term).contains("case clash: a vs A"),
        "the last frame shows the error overlay"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unhandled_note_path_switches_to_the_editor() {
    let mut fx = fixture("route").await;
    let vault = fx.app.vault.clone().unwrap();
    let note = VaultPath::note_path_from("routed");
    vault.create_note(&note, "routed").await.unwrap();
    // The Start screen does send `AppEvent::open(..)` itself, on
    // `IndexingDone` — but it never overrides `try_open_path`, so it cannot
    // *handle* an `OpenPath` it is asked to open; the loop is what routes
    // this one to the editor. (Left to itself, the Start screen's own open
    // targets the last path or the vault root, neither of which is a note,
    // so that path would route to Browse instead.)
    assert_eq!(kind(&fx.app), Some(ScreenKind::Start));

    let mut events = EventHandler::from_input(stream::empty());
    events.app_sender().send(AppEvent::open(note)).unwrap();
    let mut term = terminal();

    run_app(&mut term, &mut fx.app, &mut events).await.unwrap();

    assert_eq!(kind(&fx.app), Some(ScreenKind::Editor));
}
