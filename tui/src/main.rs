pub mod app;
pub mod app_screen;
pub mod cli;
pub mod components;
pub mod event_handler;
pub mod keys;
pub mod rag;
pub mod settings;
pub mod ui;
pub mod update;
pub mod util;

#[cfg(test)]
mod test_support;

use clap::Parser;
use color_eyre::Result;
use std::fs;
use std::path::{Path, PathBuf};

use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[command(name = "kimun", about = "Kimün notes", version)]
struct Cli {
    /// Path to a custom config file
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<crate::cli::CliCommand>,
}

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};
use ratatui::Terminal;
use ratatui::crossterm::event::EnableMouseCapture;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, enable_raw_mode, supports_keyboard_enhancement,
};
use ratatui::prelude::{Backend, CrosstermBackend};
use std::io;

use crate::app::App;
use crate::app_screen::browse::BrowseScreen;
use crate::app_screen::editor::EditorScreen;
use crate::app_screen::onboarding::OnboardingScreen;
use crate::app_screen::preferences::PreferencesScreen;
use crate::app_screen::start::StartScreen;
use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::events::{AppEvent, AppTx, AppTxExt, InputEvent, ScreenEvent};
use crate::event_handler::EventHandler;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;

/// Initialises file (and, in debug, stderr) logging.
///
/// Accepts `log_dir` as a parameter so it can be called from tests with a
/// controlled path. Returns `Some(guard)` on success; the caller must keep
/// the guard alive for the duration of the program. Returns `None` and prints
/// a warning to stderr on failure — startup is not aborted.
fn init_logging(log_dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    if let Err(e) = fs::create_dir_all(log_dir) {
        eprintln!("kimun: could not create log directory: {e}");
        return None;
    }

    let log_path = log_dir.join("kimun.log");
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("kimun: could not open log file: {e}");
            return None;
        }
    };

    let (writer, guard) = tracing_appender::non_blocking(file);

    #[cfg(debug_assertions)]
    let file_level_filter = LevelFilter::DEBUG;
    #[cfg(not(debug_assertions))]
    let file_level_filter = LevelFilter::WARN;

    let file_layer: Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync> =
        tracing_subscriber::fmt::layer()
            .compact()
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(file_level_filter)
            .boxed();

    // No stderr layer — writing to stderr corrupts the ratatui alternate screen.
    // Debug logs are captured in the log file at DEBUG level instead.
    let stderr_layer: Option<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>> = None;

    let mut layers: Vec<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>> =
        vec![file_layer];
    if let Some(s) = stderr_layer {
        layers.push(s);
    }

    // try_init instead of init so tests can call this without panicking on the
    // global-subscriber-already-set error.
    let _ = tracing_subscriber::registry().with(layers).try_init();

    // Forward log:: crate events into the tracing pipeline.
    tracing_log::LogTracer::init().ok();

    Some(guard)
}

// The nvim backend uses `tokio::task::block_in_place` during construction,
// which requires a multi-thread runtime. Keep this flavor explicit.
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Compute once, reuse for both init_logging and the panic hook.
    let log_dir: PathBuf = kimun_core::app_log_dir();
    // _guard declared early so it is dropped last (reverse declaration order).
    let _guard = init_logging(&log_dir);

    let log_path: PathBuf = log_dir.join("kimun.log");
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
            crossterm::cursor::SetCursorStyle::DefaultUserShape,
        );

        // Emit through tracing first (subscriber may still be active).
        tracing::error!("panic: {info}");

        // Direct fallback write — independent of the tracing subscriber.
        let parent = log_path.parent().unwrap_or(std::path::Path::new("."));
        let _ = fs::create_dir_all(parent);
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let _ = writeln!(file, "[PANIC] {info}");
                let bt = std::backtrace::Backtrace::force_capture();
                let _ = writeln!(file, "{bt}");
            }
            Err(e) => {
                eprintln!("kimun: could not write panic to log: {e}");
            }
        }

        default_hook(info);
    }));

    let cli = Cli::parse();

    if let Some(command) = cli.command {
        // A user error (missing/existing note, bad input) prints a clean message
        // and exits with code 2 — distinct from an internal failure, which keeps
        // the full color_eyre report (exit 1). The recoverable/internal split is
        // core's `VaultError::is_user_error`; the boundary lives here so every
        // CLI command propagates the typed `VaultError` (via `?`) and renders
        // identically.
        return match crate::cli::run_cli(command, cli.config).await {
            Ok(()) => Ok(()),
            Err(report) => {
                if let Some(msg) = report
                    .downcast_ref::<kimun_core::error::VaultError>()
                    .and_then(|ve| ve.user_message())
                {
                    eprintln!("Error: {msg}");
                    std::process::exit(2);
                }
                Err(report)
            }
        };
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    // Enable enhanced keyboard protocol when the terminal supports it (e.g. Kitty, WezTerm).
    // This is required to correctly receive F-keys and other special keys in those terminals.
    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut events = EventHandler::new();
    let mut app = App::new(cli.config).await?;

    // Mouse reporting is all-or-nothing: enabling it suppresses the terminal's
    // native selection and middle-click paste. Honor the user's opt-out (see
    // adr/0015). Read after App::new since the setting lives in its settings.
    if app.settings.read().unwrap().mouse() {
        let _ = execute!(terminal.backend_mut(), EnableMouseCapture);
    }

    spawn_update_check(&app, events.app_sender());
    respawn_rag(&mut app, &events.app_sender());

    if let Err(e) = run_app(&mut terminal, &mut app, &mut events).await {
        tracing::error!("fatal error: {e}");
        return Err(e.into());
    }

    disable_raw_mode()?;
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
        crossterm::cursor::SetCursorStyle::DefaultUserShape
    )?;
    terminal.show_cursor()?;

    // IMPORTANT: dump via the BIN crate's own module path, not via
    // `kimun_notes::...`. The lib + bin both declare `pub mod
    // components;` in this Cargo package, so `widener_metrics::METRICS`
    // is compiled into TWO separate crates with TWO separate atomic
    // counters. The bin's runtime code (view.rs) bumps the bin's copy;
    // `kimun_notes::...` would dump the lib's copy (always zero).
    crate::components::text_editor::widener_metrics::dump_if_enabled();

    // Pin _guard liveness through end of scope, preventing NLL early-drop.
    let _ = &_guard;
    Ok(())
}

/// Build a fresh `NoteVault` for whatever workspace the settings currently
/// resolve to, wiring the configured cache path and the workspace's inbox
/// path. Returns `None` if no workspace is configured or the vault fails to
/// open.
async fn rebuild_vault(
    settings: &crate::settings::SharedSettings,
) -> Option<std::sync::Arc<kimun_core::NoteVault>> {
    let (workspace_path, cache_path, inbox_path) = {
        let s = settings.read().unwrap();
        let wp = s.resolve_workspace_path();
        let name = s.current_workspace_name();
        let cache = name.as_ref().map(|n| s.cache_path_for(n));
        let ip = s
            .workspace_config
            .as_ref()
            .and_then(|wc| wc.get_current_workspace())
            .map(|e| e.effective_inbox_path());
        (wp, cache, ip)
    };
    let workspace = workspace_path?;
    let mut config = kimun_core::VaultConfig::new(&workspace);
    if let Some(cp) = cache_path {
        config = config.with_db_path(cp);
    }
    match kimun_core::NoteVault::new(config).await {
        Ok(mut v) => {
            if let Some(ref ip) = inbox_path {
                v.set_inbox_path(kimun_core::nfs::VaultPath::new(ip));
            }
            Some(std::sync::Arc::new(v))
        }
        // Don't swallow the cause: since the index self-heal,
        // opening the vault can fail on a cache probe error (e.g. the cache
        // is locked by another kimun process). The app falls back to the
        // no-vault start screen either way, but the reason must reach the
        // log instead of looking like an unconfigured workspace.
        Err(e) => {
            tracing::error!("could not open vault at {}: {e}", workspace.display());
            None
        }
    }
}

/// (Re)starts the background RAG sync for the current vault: aborts any prior
/// task and spawns a fresh one bound to the current `app.vault`. A no-op sync
/// (no server configured, or no vault) leaves the task `None`.
fn respawn_rag(app: &mut App, tx: &crate::components::events::AppTx) {
    if let Some(handle) = app.rag_sync_task.take() {
        handle.abort();
    }
    if let Some(vault) = app.vault.clone() {
        app.rag_sync_task = crate::rag::spawn_rag_sync(vault, &app.settings, tx.clone());
    }
}

async fn switch_screen(app: &mut App, tx: &AppTx, new_screen: ScreenEvent) {
    if let Some(current) = app.current_screen.as_mut() {
        current.on_exit(tx).await;
    }

    let mut screen: Box<dyn AppScreen> = match new_screen {
        ScreenEvent::Start => Box::new(StartScreen::new(app.settings.clone(), app.vault.clone())),
        ScreenEvent::OpenPreferences => Box::new(PreferencesScreen::new(app.settings.clone())),
        ScreenEvent::OpenPreferencesWithError(msg) => {
            Box::new(PreferencesScreen::new_with_error(app.settings.clone(), msg))
        }
        ScreenEvent::OpenEditor(note_vault, vault_path) => Box::new(EditorScreen::new(
            note_vault,
            vault_path,
            app.settings.clone(),
        )),
        ScreenEvent::OpenBrowse(note_vault, vault_path) => Box::new(BrowseScreen::new(
            note_vault,
            vault_path,
            app.settings.clone(),
        )),
        ScreenEvent::OpenOnboarding => Box::new(OnboardingScreen::new(app.settings.clone())),
    };

    screen.on_enter(tx).await;
    // Seed the freshly-created screen with any pending update notice, so the
    // editor footer shows it even though the check finished before this screen
    // existed. Non-editor screens ignore the event.
    if let Some(status) = app.update.clone() {
        screen
            .handle_app_message(AppEvent::UpdateAvailable(status), tx)
            .await;
    }
    screen
        .handle_app_message(AppEvent::RagStatus(app.rag_status), tx)
        .await;
    app.current_screen = Some(screen);
    // Bumped here (not at every swap site) because every swap goes through
    // this function. The main loop watches this counter to break its inner
    // event drain whenever the screen identity changes, so queued events
    // from the previous screen instance do not leak into the new one.
    app.screen_generation = app.screen_generation.wrapping_add(1);
}

// async fn switch_screen(app: &mut App, tx: &AppTx, new_screen: Box<dyn AppScreen>) {
//     if let Some(current) = app.current_screen.as_mut() {
//         current.on_exit(tx).await;
//     }
//     let mut screen = new_screen;
//     screen.on_enter(tx).await;
//     app.current_screen = Some(screen);
// }

/// Kick off the background update check (gated on the user's `update_check`
/// preference). All network/filesystem work runs on `spawn_blocking`; a found
/// update is surfaced via `AppEvent::UpdateAvailable`. Failures are logged and
/// swallowed — the check never blocks startup or interaction.
fn spawn_update_check(app: &App, tx: AppTx) {
    if !app.settings.read().unwrap().update_check() {
        return;
    }
    let Ok(config_dir) = crate::settings::config_dir() else {
        return;
    };
    tokio::spawn(async move {
        match crate::update::check_now(config_dir, false).await {
            Ok(Some(status)) if status.should_notify() => {
                let _ = tx.send(AppEvent::UpdateAvailable(status));
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("update check failed: {e}"),
        }
    });
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    events: &mut EventHandler,
) -> io::Result<()>
where
    io::Error: From<B::Error>,
{
    let tx = events.app_sender();

    if let Some(screen) = &mut app.current_screen {
        screen.on_enter(&tx).await;
    }

    loop {
        terminal.draw(|f| ui::ui(f, app))?;

        // Block until at least one event arrives, then drain everything else
        // that is already queued before drawing again. `Redraw` events are
        // coalesced — the top-of-loop draw paints one frame for the whole
        // batch instead of one frame per pending message. Crossterm input
        // events never come through the mpsc channel, so a real key event
        // always forces a fresh `events.next().await` (and therefore a
        // dedicated draw) on the next iteration.
        let mut event = events.next().await;
        loop {
            match event {
                AppEvent::Quit => {
                    if let Some(screen) = app.current_screen.as_mut() {
                        screen.on_exit(&tx).await;
                    }
                    return Ok(());
                }
                AppEvent::Redraw => {
                    // No-op: top-of-loop draw already happened (or is about to).
                }
                AppEvent::Input(input) => {
                    match input {
                        InputEvent::Key(key) => {
                            tracing::debug!(
                                "KEY: code={:?} mods={:?} kind={:?}",
                                key.code,
                                key.modifiers,
                                key.kind
                            );
                            // Global shortcuts — fire before any screen gets the event.
                            if let Some(combo) = key_event_to_combo(&key) {
                                let action = {
                                    let s = app.settings.read().unwrap();
                                    tracing::debug!(
                                        "COMBO: {} → {:?}",
                                        combo,
                                        s.key_bindings.get_action(&combo)
                                    );
                                    s.key_bindings.get_action(&combo)
                                };
                                let handled_global = match action {
                                    Some(ActionShortcuts::Quit) => {
                                        tx.send(AppEvent::Quit).ok();
                                        true
                                    }
                                    Some(ActionShortcuts::OpenPreferences) => {
                                        let already_on_settings = app
                                            .current_screen
                                            .as_ref()
                                            .map(|s| s.get_kind() == ScreenKind::Preferences)
                                            .unwrap_or(false);
                                        if !already_on_settings {
                                            tx.send(AppEvent::OpenScreen(
                                                ScreenEvent::OpenPreferences,
                                            ))
                                            .ok();
                                        }
                                        true
                                    }
                                    _ => false,
                                };
                                if handled_global {
                                    // Skip screen-level handling for this key.
                                    match events.try_next() {
                                        Some(next) => {
                                            event = next;
                                            continue;
                                        }
                                        None => break,
                                    }
                                }
                            }
                            if let Some(screen) = &mut app.current_screen {
                                screen.handle_input(&InputEvent::Key(key), &tx);
                            }
                        }
                        InputEvent::Mouse(mouse_event) => {
                            if let Some(screen) = &mut app.current_screen {
                                screen.handle_input(&InputEvent::Mouse(mouse_event), &tx);
                            }
                        }
                        InputEvent::Paste(text) => {
                            if let Some(screen) = &mut app.current_screen {
                                screen.handle_input(&InputEvent::Paste(text), &tx);
                            }
                        }
                    }
                }
                msg => {
                    // Capture screen identity around handle_app_message so we
                    // can detect a synchronous screen swap (OpenScreen,
                    // VaultConflict). Use `screen_generation` rather than
                    // `ScreenKind`, because a swap between two screens of the
                    // same kind (e.g. EditorScreen(A) → follow-link →
                    // EditorScreen(B)) still leaks A's queued events into B
                    // if we only compare kinds. Remaining queued events
                    // belong to the OLD screen instance — break the drain so
                    // they get a fresh outer iteration where they are routed
                    // correctly (and the new screen gets its first draw
                    // before further input).
                    let before_gen = app.screen_generation;
                    handle_app_message(msg, app, &tx).await?;
                    if app.screen_generation != before_gen {
                        break;
                    }
                }
            }
            match events.try_next() {
                Some(next) => event = next,
                None => break,
            }
        }
    }
}

async fn handle_app_message(msg: AppEvent, app: &mut App, tx: &AppTx) -> io::Result<()> {
    match msg {
        AppEvent::Redraw => {}
        AppEvent::OpenScreen(screen) => {
            switch_screen(app, tx, screen).await;
        }
        AppEvent::OpenPath { path, emphasis } => {
            // We either handle the new path within the current screen, or we switch to a new screen for this path
            let unhandled = if let Some(screen) = app.current_screen.as_mut() {
                screen.try_open_path(path, emphasis, tx).await
            } else {
                Some(path)
            };
            if let Some(path) = unhandled {
                if let Some(vault) = app.vault.clone() {
                    if path.is_note() {
                        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenEditor(vault, path)))
                            .ok();
                    } else {
                        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(vault, path)))
                            .ok();
                    }
                } else {
                    // No vault → the app is unconfigured. Route to the guided
                    // setup, not Preferences (onboarding replaces the
                    // preferences fallthrough as the no-workspace path).
                    tx.send(AppEvent::OpenScreen(ScreenEvent::OpenOnboarding))
                        .ok();
                }
            }
        }
        AppEvent::OpenAttachment(path) => {
            // The editor screen shows it in its attachment view; any other
            // screen routes through OpenEditor first, then the attachment opens
            // there. (In practice this is sent from the editor's FILES drawer.)
            let unhandled = if let Some(screen) = app.current_screen.as_mut() {
                screen.try_open_attachment(path, tx).await
            } else {
                Some(path)
            };
            if let Some(path) = unhandled
                && let Some(vault) = app.vault.clone()
            {
                tx.send(AppEvent::OpenScreen(ScreenEvent::OpenEditor(vault, path)))
                    .ok();
            }
        }
        AppEvent::OpenJournal => {
            // Resolve today's journal entry (creating it if needed) once, then
            // route it like any other note via OpenPath so it works from every
            // screen — the current screen opens it inline or the loop switches
            // to the editor.
            if let Some(vault) = app.vault.clone()
                && let Ok((details, _, created)) = vault.journal_entry().await
            {
                // Notify the current screen's sidebar when freshly created, then
                // open it — works from every screen via OpenPath.
                tx.announce_and_open(details.path, created);
            }
        }
        AppEvent::PreferencesSaved | AppEvent::OnboardingFinished => {
            // Rebuild the vault so workspace path and inbox_path changes take effect.
            app.vault = rebuild_vault(&app.settings).await;
            respawn_rag(app, tx);
            tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
        }
        AppEvent::ClosePreferences => {
            tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
        }
        AppEvent::VaultConflict(msg) => {
            // The vault has structural conflicts (e.g. case-insensitive path clashes).
            // Clear the workspace so the user is not stuck in a loop, then show
            // the settings screen with the error overlay pre-populated.
            {
                let mut s = app.settings.write().unwrap();
                s.clear_workspace();
                s.save_to_disk().ok();
            }
            app.vault = None;
            respawn_rag(app, tx);
            switch_screen(app, tx, ScreenEvent::OpenPreferencesWithError(msg)).await;
        }
        AppEvent::WorkspaceSwitched(name) => {
            {
                let mut s = app.settings.write().unwrap();
                if let Some(ref mut wc) = s.workspace_config {
                    wc.global.current_workspace = name;
                }
                s.save_to_disk().ok();
            }
            app.vault = rebuild_vault(&app.settings).await;
            respawn_rag(app, tx);
            tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
        }
        AppEvent::UpdateAvailable(status) => {
            // Remember app-globally (so a later-opened editor can be seeded in
            // switch_screen) and forward to the current screen for immediate
            // display. Non-editor screens ignore it.
            app.update = Some(status.clone());
            if let Some(screen) = app.current_screen.as_mut() {
                screen
                    .handle_app_message(AppEvent::UpdateAvailable(status), tx)
                    .await;
            }
        }
        AppEvent::RagStatus(status) => {
            // Same pattern as update: keep app-globally for screen seeding, and
            // forward for immediate display.
            app.rag_status = status;
            if let Some(screen) = app.current_screen.as_mut() {
                screen
                    .handle_app_message(AppEvent::RagStatus(status), tx)
                    .await;
            }
        }
        AppEvent::DismissUpdate(version) => {
            // Persist the skip and clear the app-global notice, then forward so
            // the current screen drops its indicator copy.
            if let Ok(config_dir) = crate::settings::config_dir()
                && let Err(e) = crate::update::dismiss(&config_dir, &version)
            {
                tracing::debug!("could not persist update dismissal: {e}");
            }
            app.update = None;
            if let Some(screen) = app.current_screen.as_mut() {
                screen
                    .handle_app_message(AppEvent::DismissUpdate(version), tx)
                    .await;
            }
        }
        AppEvent::UpdateApplied => {
            // Self-update installed: clear the app-global notice so no
            // later-opened screen is re-seeded with the now-installed version.
            app.update = None;
            if let Some(screen) = app.current_screen.as_mut() {
                screen.handle_app_message(AppEvent::UpdateApplied, tx).await;
            }
        }
        other => {
            if let Some(screen) = app.current_screen.as_mut() {
                screen.handle_app_message(other, tx).await;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    use crate::components::events::{AppEvent, ScreenEvent};
    use crate::keys::action_shortcuts::ActionShortcuts;
    use crate::keys::key_event_to_combo;
    use crate::settings::AppSettings;

    /// Ctrl+P is the global shortcut for OpenSettings, handled in run_app before any screen.
    /// This test verifies that the keybinding lookup resolves to OpenSettings
    /// and that the app-level handler sends OpenScreen(OpenSettings).
    #[test]
    fn settings_keybinding_sends_open_settings() {
        let settings = AppSettings::default();
        // Settings live on Ctrl+, (Ctrl+P is the palette; Ctrl+Shift+P
        // collides with kitty's hints-kitten chord prefix).
        let key = KeyEvent {
            code: KeyCode::Char(','),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        let combo = key_event_to_combo(&key).expect("Ctrl+, should produce a combo");
        let action = settings.key_bindings.get_action(&combo);
        assert_eq!(action, Some(ActionShortcuts::OpenPreferences));

        // Simulate the app-level dispatch: on OpenSettings, send OpenScreen(OpenSettings).
        let (tx, mut rx) = unbounded_channel();
        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenPreferences))
            .ok();
        let msg = rx.try_recv().expect("should have a message");
        assert!(matches!(
            msg,
            AppEvent::OpenScreen(ScreenEvent::OpenPreferences)
        ));
    }

    #[test]
    fn init_logging_returns_none_on_bad_path() {
        use crate::init_logging;
        // /nonexistent/readonly/path cannot be created; init_logging must return None
        // without panicking. This test exercises the early-return path before try_init
        // is called, so the global subscriber singleton is not set by this test.
        let result = init_logging(std::path::Path::new("/nonexistent/readonly/path"));
        assert!(result.is_none());
    }
}
