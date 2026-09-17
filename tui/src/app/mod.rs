//! The **App loop** and the state it runs over (see CONTEXT.md § App shell).
//!
//! Submodules are internal seams: `events` (the **Input source**), `terminal`
//! (the **Terminal session**), `bootstrap` (logging and the panic hook),
//! `ctrl_h` (which of two keys a `0x08` byte is).

pub(crate) mod bootstrap;
pub mod ctrl_h;
pub mod events;
pub(crate) mod terminal;

use std::io;
use std::process::ExitCode;
use std::sync::{Arc, RwLock};

use color_eyre::eyre;
use kimun_core::{NoteVault, VaultConfig};
use ratatui::Terminal;
use ratatui::prelude::Backend;

use crate::app_screen::browse::BrowseScreen;
use crate::app_screen::editor::EditorScreen;
use crate::app_screen::onboarding::OnboardingScreen;
use crate::app_screen::preferences::PreferencesScreen;
use crate::app_screen::start::StartScreen;
use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::events::{AppEvent, AppTx, AppTxExt, InputEvent, ScreenEvent, UpdateFlow};
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::settings::{AppSettings, SharedSettings};
use clap::Parser;
use color_eyre::Result;
use events::EventHandler;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "kimun", about = "Kimün notes", version)]
pub struct Cli {
    /// Path to a custom config file
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<crate::cli::CliCommand>,
}

/// The process entry the binary delegates to. Owns the runtime: the nvim
/// backend uses `tokio::task::block_in_place` during construction, which
/// requires the multi-thread flavor — that constraint lives here, next to the
/// code that has it, not in the shim.
pub fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(entry())
}

async fn entry() -> Result<ExitCode> {
    // Computed once, reused by logging and the panic hook. The guard is held
    // to the end of this function, and every exit path returns through here,
    // so the log is always flushed.
    let log_dir: PathBuf = kimun_core::system::log_dir().into_path_buf();
    let _guard = bootstrap::init_logging(&log_dir);
    bootstrap::install_panic_hook(log_dir.join("kimun.log"));

    let cli = Cli::parse();
    match cli.command {
        Some(command) => run_cli_command(command, cli.config).await,
        None => {
            run_tui(cli.config).await?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// A user error (missing/existing note, bad input) prints a clean message and
/// exits with code 2 — distinct from an internal failure, which keeps the full
/// color_eyre report (exit 1). The recoverable/internal split is core's
/// `VaultError::user_message`; the boundary lives here so every CLI command
/// propagates the typed `VaultError` (via `?`) and renders identically.
async fn run_cli_command(
    command: crate::cli::CliCommand,
    config: Option<PathBuf>,
) -> Result<ExitCode> {
    match crate::cli::run_cli(command, config).await {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(report) => {
            if let Some(msg) = report
                .downcast_ref::<kimun_core::error::VaultError>()
                .and_then(|ve| ve.user_message())
            {
                // Returned, not `process::exit`ed: the code unwinds back
                // through `entry`, so the runtime and the log guard are
                // dropped normally on the way out.
                eprintln!("Error: {msg}");
                return Ok(ExitCode::from(2));
            }
            Err(report)
        }
    }
}

/// The TUI: settings and vault first (a startup error prints to a normal
/// terminal), then the **Terminal session**, the background tasks, the loop.
/// The session is left before anything else is printed.
pub async fn run_tui(config_path: Option<PathBuf>) -> Result<()> {
    let mut app = App::new(config_path).await?;
    // Read after App::new since both live in its settings (ADR-0015).
    let (mouse_capture, ctrl_h_setting) = {
        let s = app.settings.read().unwrap();
        (s.mouse(), s.ctrl_h)
    };
    let mut session = terminal::TerminalSession::enter(mouse_capture)?;
    // Resolved after `enter`: whether the kitty flags went out is half of the
    // answer, and only the live session knows it.
    let ctrl_h = ctrl_h::CtrlHPolicy::resolve(ctrl_h_setting, session.keyboard_enhanced());
    let mut events = EventHandler::new(ctrl_h);
    warn_about_unreachable_bindings(&app, &events.app_sender(), &session, ctrl_h);

    spawn_update_check(&app, events.app_sender());
    respawn_rag(&mut app, &events.app_sender());

    let outcome = run_app(session.terminal_mut(), &mut app, &mut events).await;
    drop(session);

    if let Err(e) = outcome {
        tracing::error!("fatal error: {e}");
        return Err(e.into());
    }
    crate::components::text_editor::widener_metrics::dump_if_enabled();
    Ok(())
}

pub struct App {
    /// The live **Screen**. There is always exactly one: the app starts on
    /// **Start** and every transition swaps the box whole in `switch_screen`,
    /// so there is no in-between state to represent.
    pub current_screen: Box<dyn AppScreen>,

    pub settings: SharedSettings,

    /// The active vault. `None` until a workspace path is configured.
    /// Rebuilt only when the workspace path changes in settings.
    pub vault: Option<Arc<NoteVault>>,

    /// Monotonic counter bumped by every screen swap (see `switch_screen`
    /// below). The main event loop breaks its inner drain when this changes,
    /// so the new screen is drawn before any event still queued is delivered
    /// to it. The queued events are not dropped — filtering stale results is
    /// the job of the addressed event families (`OverlayData`, `Ask`) and
    /// per-event guards, not of this counter.
    pub screen_generation: u64,

    /// A newer release found by the background update check at startup, if any.
    /// Seeded into each editor screen so the footer can show the indicator.
    pub update: Option<crate::update::UpdateStatus>,

    /// The background RAG sync task for the current vault, when a server is
    /// configured. Aborted and respawned when the vault is rebuilt.
    pub rag_sync_task: Option<tokio::task::JoinHandle<()>>,

    /// Latest RAG status from the background task, held app-globally so a
    /// freshly-opened editor can be seeded immediately (like `update`) instead
    /// of showing nothing until the next sync tick.
    pub rag_status: crate::rag::RagStatus,
}

impl App {
    /// Load settings from `config_path` (or the default location) and build
    /// the app on them.
    pub async fn new(config_path: Option<std::path::PathBuf>) -> eyre::Result<Self> {
        let loaded_settings = match config_path {
            Some(path) => AppSettings::load_from_file(path)?,
            None => AppSettings::load_from_disk()?,
        };
        Ok(Self::from_settings(Arc::new(RwLock::new(loaded_settings))).await)
    }

    /// The app over already-loaded settings: opens the vault those settings
    /// name (if any) and starts on the **Start** screen. The door tests use —
    /// nothing here touches the config file.
    pub async fn from_settings(settings: SharedSettings) -> Self {
        let vault = Self::open_vault(&settings).await;
        Self {
            current_screen: Box::new(StartScreen::new(settings.clone(), vault.clone())),
            settings,
            vault,
            screen_generation: 0,
            update: None,
            rag_sync_task: None,
            rag_status: crate::rag::RagStatus::Disabled,
        }
    }

    /// A fresh `NoteVault` for whatever workspace the settings currently
    /// resolve to, wired to the configured index and the workspace's inbox.
    /// `None` if no workspace is configured or the vault fails to open.
    ///
    /// Used at startup and every time the workspace changes (preferences
    /// saved, onboarding finished, workspace switched).
    pub async fn open_vault(settings: &SharedSettings) -> Option<Arc<NoteVault>> {
        let (workspace_path, cache_path, inbox_path) = {
            let s = settings.read().unwrap();
            let wp = s.resolve_workspace_path();
            let name = s.current_workspace_name();
            let cache = name.as_ref().map(|n| s.index_for(n));
            let ip = s
                .workspace_config
                .as_ref()
                .and_then(|wc| wc.get_current_workspace())
                .map(|e| e.effective_inbox_path());
            (wp, cache, ip)
        };
        let workspace = workspace_path?;
        let mut config = VaultConfig::new(workspace.clone());
        if let Some(cp) = cache_path {
            config = config.with_index(cp);
        }
        match NoteVault::new(config).await {
            Ok(mut v) => {
                if let Some(ref ip) = inbox_path {
                    v.set_inbox_path(kimun_core::nfs::VaultPath::new(ip));
                }
                Some(Arc::new(v))
            }
            // Don't swallow the cause: since the index self-heal, opening the
            // vault can fail on a cache probe error (e.g. the cache is locked
            // by another kimun process). The app falls back to the no-vault
            // start screen either way, but the reason must reach the log
            // instead of looking like an unconfigured workspace.
            Err(e) => {
                tracing::error!("could not open vault at {}: {e}", workspace);
                None
            }
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
    app.current_screen.on_exit(tx).await;

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
            .handle_app_message(AppEvent::Update(UpdateFlow::Available(status)), tx)
            .await;
    }
    screen
        .handle_app_message(AppEvent::RagStatus(app.rag_status), tx)
        .await;
    app.current_screen = screen;
    // Bumped here (not at every swap site) because every swap goes through
    // this function. The main loop watches this counter to break its inner
    // event drain whenever the screen identity changes, so the new screen is
    // drawn before any event still queued is delivered to it.
    app.screen_generation = app.screen_generation.wrapping_add(1);
}

/// Kick off the background update check (gated on the user's `update_check`
/// preference). All network/filesystem work runs on `spawn_blocking` inside
/// Tell the user about a binding their terminal cannot deliver.
///
/// Only ever their own. The default keymap always keeps a chord that survives
/// the weakest terminal kimün supports — `settings`' invariant test holds it
/// to that — so anything found here came out of a `[key_bindings]` section and
/// is theirs to change. That is the whole reason this reports rather than
/// silently repairing: rebinding under them is exactly the surprise moving
/// formatting to the leader was meant to avoid.
///
/// Rare in practice, and deliberately so: `merge_missing_default_bindings`
/// hands an action its default combo back unless the config gave that combo to
/// something else. So `SearchNotes = ["ctrl&I"]` alone still answers to Ctrl-K
/// and stays reachable; it takes a config that *also* claims Ctrl-K elsewhere
/// to strand it. Quiet is the point — a warning that fired on a working setup
/// would teach people to ignore it.
///
/// Logged as a warning (durable — the troubleshooting docs send people to the
/// log) and flashed once in the footer (visible, and gone in two seconds
/// rather than nagging). `kimun doctor` prints the full picture on demand.
fn warn_about_unreachable_bindings(
    app: &App,
    tx: &AppTx,
    session: &terminal::TerminalSession,
    ctrl_h: ctrl_h::CtrlHPolicy,
) {
    use crate::keys::reachability::{Reach, TerminalKeys, unreachable_actions};

    let keys = TerminalKeys {
        enhanced: session.keyboard_enhanced(),
        ctrl_h_is_backspace: ctrl_h == ctrl_h::CtrlHPolicy::Backspace,
    };
    let stranded = {
        let settings = app.settings.read().unwrap();
        unreachable_actions(&settings.key_bindings, keys)
    };
    if stranded.is_empty() {
        return;
    }
    for u in &stranded {
        for (combo, reach) in &u.combos {
            let fate = match reach {
                Reach::Ok => continue,
                Reach::Shadowed(by) => format!("arrives as {by}"),
                Reach::Untransmitted => "is not sent by this terminal".to_string(),
            };
            tracing::warn!("key binding {combo} for {} {fate}", u.action);
        }
    }
    let names = stranded
        .iter()
        .map(|u| u.action.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    tx.send(AppEvent::FlashMessage(format!(
        "{names}: no usable key on this terminal — run `kimun doctor`"
    )))
    .ok();
}

/// `update::check_now`; a found update is surfaced via `AppEvent::Update(UpdateFlow::Available)`. Failures are logged and
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
                let _ = tx.send(AppEvent::Update(UpdateFlow::Available(status)));
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("update check failed: {e}"),
        }
    });
}

/// The **App loop** (CONTEXT.md § App shell).
///
/// Draw the screen, wait for the next event, drain what is already queued,
/// draw again. The rules that live here and nowhere else:
///
/// - Global shortcuts (`Quit`, `OpenPreferences`) fire before the screen
///   sees the key.
/// - Queued app messages are coalesced: one frame per batch. A real input
///   event always comes through `next()` and so always gets its own draw.
/// - A screen swap mid-drain ends the drain (`App::screen_generation`), so the
///   new screen is drawn before any event still queued is delivered to it.
///   The queued events are *not* dropped — they are read again on the next
///   iteration and reach the new screen. Filtering stale results is the job
///   of the addressed event families (`OverlayData`, `Ask`) and per-event
///   guards, not of this loop.
/// - `Quit` runs the current screen's `on_exit` before returning.
///
/// Generic over the terminal backend and fed by an **Input source**, so it
/// runs headless in tests (`tests/app_loop_test.rs`).
pub async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    events: &mut EventHandler,
) -> io::Result<()>
where
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let tx = events.app_sender();

    app.current_screen.on_enter(&tx).await;

    loop {
        terminal
            .draw(|f| app.current_screen.render(f))
            // A `From` bound into `io::Error` would exclude `TestBackend`
            // (`Error = Infallible`), which the headless loop tests use.
            // Wrapping through `io::Error::other` keeps the source error and
            // needs only the standard error bounds.
            .map_err(io::Error::other)?;

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
                    app.current_screen.on_exit(&tx).await;
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
                                        let already_on_settings = app.current_screen.get_kind()
                                            == ScreenKind::Preferences;
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
                            app.current_screen.handle_input(&InputEvent::Key(key), &tx);
                        }
                        InputEvent::Mouse(mouse_event) => {
                            app.current_screen
                                .handle_input(&InputEvent::Mouse(mouse_event), &tx);
                        }
                        InputEvent::Paste(text) => {
                            app.current_screen
                                .handle_input(&InputEvent::Paste(text), &tx);
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
            let unhandled = app.current_screen.try_open_path(path, emphasis, tx).await;
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
            let unhandled = app.current_screen.try_open_attachment(path, tx).await;
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
            app.vault = App::open_vault(&app.settings).await;
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
            app.vault = App::open_vault(&app.settings).await;
            respawn_rag(app, tx);
            tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
        }
        AppEvent::Update(flow) => {
            // The app-global half of the update lifecycle: remember the
            // notice (so a later-opened editor is seeded in switch_screen),
            // persist a dismissal, clear on dismiss/install. Every flow
            // event is then forwarded — the editor screen owns the display
            // half (indicator, dialog, running the install).
            match &flow {
                UpdateFlow::Available(status) => app.update = Some(status.clone()),
                UpdateFlow::Dismiss(version) => {
                    if let Ok(config_dir) = crate::settings::config_dir()
                        && let Err(e) = crate::update::dismiss(&config_dir, version)
                    {
                        tracing::debug!("could not persist update dismissal: {e}");
                    }
                    app.update = None;
                }
                UpdateFlow::Applied => app.update = None,
                UpdateFlow::Apply | UpdateFlow::ShowDialog => {}
            }
            app.current_screen
                .handle_app_message(AppEvent::Update(flow), tx)
                .await;
        }
        AppEvent::RagStatus(status) => {
            // Same pattern as update: keep app-globally for screen seeding, and
            // forward for immediate display.
            app.rag_status = status;
            app.current_screen
                .handle_app_message(AppEvent::RagStatus(status), tx)
                .await;
        }
        other => {
            app.current_screen.handle_app_message(other, tx).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::App;
    use crate::app_screen::ScreenKind;
    use crate::keys::action_shortcuts::ActionShortcuts;
    use crate::keys::key_event_to_combo;
    use crate::settings::AppSettings;

    #[tokio::test]
    async fn from_settings_without_a_workspace_starts_on_the_start_screen_with_no_vault() {
        let settings = Arc::new(RwLock::new(AppSettings::default()));
        let app = App::from_settings(settings).await;
        assert!(app.vault.is_none());
        assert_eq!(app.current_screen.get_kind(), ScreenKind::Start);
        assert_eq!(app.screen_generation, 0);
    }

    /// Ctrl+, is the global shortcut for OpenPreferences, handled in `run_app`
    /// before any screen.
    #[test]
    fn settings_keybinding_sends_open_settings() {
        let settings = AppSettings::default();
        let key = KeyEvent {
            code: KeyCode::Char(','),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        let combo = key_event_to_combo(&key).expect("Ctrl+, should produce a combo");
        let action = settings.key_bindings.get_action(&combo);
        assert_eq!(action, Some(ActionShortcuts::OpenPreferences));
    }
}
