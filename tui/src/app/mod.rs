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
use kimun_core::NoteVault;
use ratatui::Terminal;
use ratatui::prelude::Backend;

use crate::app_screen::browse::BrowseScreen;
use crate::app_screen::editor::EditorScreen;
use crate::app_screen::onboarding::OnboardingScreen;
use crate::app_screen::preferences::PreferencesScreen;
use crate::app_screen::start::StartScreen;
use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::events::InputEvent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::logic::message::{AppMessage, AppTx, FileOp, ScreenEvent, UpdateFlow};
use crate::settings::{AppSettings, SharedSettings};
use clap::Parser;
use color_eyre::Result;
use events::{AppEvent, EventHandler};
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

/// File logging in the app's log directory, set up as the TUI's own; for
/// another frontend (the GUI). Keep the guard alive for the process's life
/// so the log is flushed. `None` (reported on stderr) when it cannot start.
pub fn init_file_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    bootstrap::init_logging(&kimun_core::system::log_dir().into_path_buf())
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
/// `VaultError::user_message` (plus the CLI's own [`crate::cli::UserError`]
/// for requests core has no error for); the boundary lives here so every CLI
/// command propagates the typed error (via `?`) and renders identically.
async fn run_cli_command(
    command: crate::cli::CliCommand,
    config: Option<PathBuf>,
) -> Result<ExitCode> {
    match crate::cli::run_cli(command, config).await {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(report) => {
            // `wiki check` printed its report already: only the exit code is left.
            if report.downcast_ref::<crate::cli::CheckFailed>().is_some() {
                return Ok(ExitCode::from(1));
            }
            let user_message = report
                .downcast_ref::<kimun_core::error::VaultError>()
                .and_then(|ve| ve.user_message())
                .or_else(|| {
                    report
                        .downcast_ref::<crate::cli::UserError>()
                        .map(ToString::to_string)
                });
            if let Some(msg) = user_message {
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
        let s = app.settings.read().expect("settings lock poisoned");
        (s.mouse(), s.ctrl_h)
    };
    let mut session = terminal::TerminalSession::enter(mouse_capture)?;
    // Resolved after `enter`: whether the kitty flags went out is half of the
    // answer, and only the live session knows it.
    let ctrl_h = ctrl_h::CtrlHPolicy::resolve(ctrl_h_setting, session.keyboard_enhanced());
    let mut events = EventHandler::new(ctrl_h);
    app.parked_flashes.extend(unreachable_binding_notice(
        &app,
        &session,
        ctrl_h_setting,
        ctrl_h,
    ));

    spawn_update_check(&app, events.app_sender());
    respawn_rag(&mut app, &events.app_sender());

    let outcome = run_app(session.terminal_mut(), &mut app, &mut events).await;
    drop(session);

    if let Err(e) = outcome {
        tracing::error!("fatal error: {e}");
        return Err(e.into());
    }
    crate::logic::editor::widener_metrics::dump_if_enabled();
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

    /// Whether `vault` is usable: set from every indexing outcome, reset to
    /// Ready whenever `vault` is rebuilt.
    pub vault_status: crate::logic::startup::VaultStatus,

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

    /// One-shot flashes waiting for a screen that can show them, oldest
    /// first: the notice that some key binding cannot reach this terminal,
    /// and whatever [`AppMessage::ParkFlash`] sent (Start's "your last note is
    /// gone").
    ///
    /// Parked rather than sent, because they are decided while **Start** is
    /// the live screen, and `FlashMessage` is only handled by the editor.
    /// `switch_screen` drains them the first time it opens an editor, so each
    /// is shown once and not re-flashed on every later screen swap (unlike
    /// `update`, which is a standing indicator and is re-seeded).
    pub parked_flashes: Vec<String>,

    /// The background RAG sync task for the current vault, when a server is
    /// configured. Aborted and respawned when the vault is rebuilt.
    pub rag_sync_task: crate::logic::task_slot::TaskSlot,

    /// Latest RAG status from the background task, held app-globally so a
    /// freshly-opened editor can be seeded immediately (like `update`) instead
    /// of showing nothing until the next sync tick.
    pub rag_status: crate::rag::RagStatus,
    /// Newer server release reported by the server, if any. Seeded into each
    /// screen alongside `rag_status`.
    pub server_update: Option<crate::server_client::sync::ServerUpdate>,
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
        let vault = crate::logic::startup::open_vault(&settings).await;
        Self {
            current_screen: Box::new(StartScreen::new(settings.clone(), vault.clone())),
            settings,
            vault,
            vault_status: Default::default(),
            screen_generation: 0,
            update: None,
            parked_flashes: Vec::new(),
            rag_sync_task: Default::default(),
            rag_status: crate::rag::RagStatus::Disabled,
            server_update: None,
        }
    }
}

/// (Re)starts the background RAG sync for the current vault: aborts any prior
/// task and spawns a fresh one bound to the current `app.vault`. A no-op sync
/// (no server configured, or no vault) leaves the task `None`.
fn respawn_rag(app: &mut App, tx: &crate::logic::message::AppTx) {
    // Stop the old loop before the new one starts: two for one vault would
    // replace each other's index observer.
    app.rag_sync_task.stop();
    let task = app
        .vault
        .clone()
        .and_then(|vault| crate::rag::spawn_rag_sync(vault, &app.settings, tx.clone()));
    app.rag_sync_task.replace(task);
}

async fn switch_screen(app: &mut App, tx: &AppTx, new_screen: ScreenEvent) {
    app.current_screen.on_exit(tx).await;

    // Decided before `new_screen` is consumed below. Only the editor handles
    // `FlashMessage`; every other screen drops it on the floor.
    let shows_flashes = matches!(new_screen, ScreenEvent::OpenEditor(..));

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
            .handle_app_message(AppMessage::Update(UpdateFlow::Available(status)), tx)
            .await;
    }
    // Drained, not cloned: a flash is a one-shot, and re-firing it on every
    // screen swap would turn a warning into a nag. Drained only for a screen
    // that shows flashes — Start can route through Onboarding or Browse
    // first, and handing the notice to one of those loses it for good.
    // Joined into one flash: the footer holds a single one, so delivering
    // them separately would leave only the last on screen.
    if shows_flashes && !app.parked_flashes.is_empty() {
        let msg = std::mem::take(&mut app.parked_flashes).join(" · ");
        screen
            .handle_app_message(AppMessage::FlashMessage(msg), tx)
            .await;
    }
    screen
        .handle_app_message(AppMessage::RagStatus(app.rag_status), tx)
        .await;
    screen
        .handle_app_message(AppMessage::ServerUpdate(app.server_update.clone()), tx)
        .await;
    app.current_screen = screen;
    // Bumped here (not at every swap site) because every swap goes through
    // this function. The main loop watches this counter to break its inner
    // event drain whenever the screen identity changes, so the new screen is
    // drawn before any event still queued is delivered to it.
    app.screen_generation = app.screen_generation.wrapping_add(1);
}

/// Tell the user about a binding their terminal cannot deliver.
///
/// Their own, with one exception. The default keymap always keeps a chord
/// that survives the weakest terminal kimün supports — `settings`' invariant
/// test holds it to that — so anything found here came out of a
/// `[key_bindings]` section and is theirs to change. The exception is the
/// Ctrl-H rewrite: under a Backspace policy `FocusSidebar` loses its only
/// default chord, and the notice then names `ctrl_h` rather than the
/// terminal (see `notice_text`). That is the whole reason this reports rather
/// than silently repairing: rebinding under them is exactly the surprise
/// moving formatting to the leader was meant to avoid.
///
/// Returns the notice rather than sending it: at this point **Start** is the
/// live screen and only the editor handles `FlashMessage`, so it is parked on
/// [`App::parked_flashes`] for `switch_screen` to deliver once an editor is
/// opened.
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
fn unreachable_binding_notice(
    app: &App,
    session: &terminal::TerminalSession,
    setting: crate::settings::CtrlHSetting,
    ctrl_h: ctrl_h::CtrlHPolicy,
) -> Option<String> {
    use crate::keys::reachability::{TerminalKeys, unreachable_actions};

    let keys = TerminalKeys::detected(
        session.keyboard_enhanced(),
        // `FocusSidebar`'s only default chord is `Ctrl+H`, so under the
        // rewrite it is stranded. The rule for when that is worth saying
        // lives with the policy.
        //
        // Deliberately not `policy == CtrlHPolicy::Backspace` — what
        // `doctor.rs` passes for this same argument. Under an explicit
        // `ctrl_h = "backspace"` `loss_is_worth_reporting` is false, so this
        // startup scan reports Ctrl+H as reachable while `kimun doctor`
        // still reports it stranded: the flash is for surprises, quiet on a
        // setting the user picked on purpose, while doctor always shows the
        // full picture on request.
        ctrl_h.loss_is_worth_reporting(setting),
    );
    let stranded = {
        let settings = app.settings.read().expect("settings lock poisoned");
        unreachable_actions(&settings.key_bindings, keys)
    };
    if stranded.is_empty() {
        return None;
    }
    for u in &stranded {
        for (combo, fate) in &u.combos {
            tracing::warn!("key binding {combo} for {}: {fate}", u.action);
        }
    }
    Some(notice_text(&stranded, setting, ctrl_h))
}

/// The footer renders this flash as a single unwrapped, centre-aligned
/// `Paragraph` (`components::footer_bar.rs`). Ratatui's centre offset is
/// `(area/2).saturating_sub(line/2)`, which is `0` once the line is wider
/// than the area — so a line past this many columns on an 80-column footer
/// renders from the left and the TAIL is what gets clipped. Kept as a
/// constant (rather than inlined into the test) so the budget has one place
/// to change and a comment explaining why it exists.
///
/// Only asserted by `the_single_clause_notice_fits_the_flash_width_budget`
/// below, not consulted by `notice_text` itself: the two-clause case reports
/// two independent facts and cannot always fit it, so there is no single
/// formatting rule this constant could gate at runtime.
#[allow(
    dead_code,
    reason = "read by the width-budget test, not by notice_text"
)]
const FLASH_WIDTH_BUDGET: usize = 72;

/// More than this many actions in one clause are named, then folded into an
/// "and N more" tail — the flash has to stay on `FLASH_WIDTH_BUDGET`, not
/// grow with however many bindings a user managed to strand.
const FLASH_NAME_LIMIT: usize = 2;

/// The footer line for a stranded-binding scan.
///
/// Partitioned by *cause*, not by policy. Under a Backspace policy the scan
/// can turn up two different kinds of entry in the same call: the bare
/// `Ctrl+H` chord that the rewrite itself shadowed with Backspace — kimün's
/// own doing, `auto` read the tty's erase character or the user set
/// `backspace` — and, independently, anything else the user's own
/// `[key_bindings]` bound to a chord this terminal cannot deliver (a
/// `SearchNotes = ["ctrl&I"]` colliding with Tab is always-on and has nothing
/// to do with `ctrl_h`). Wrapping *that* action's name in "Ctrl+H is
/// Backspace" would send the user off to change the wrong setting, so only
/// the entries the rewrite actually caused get the ctrl_h wording; everything
/// else keeps the plain "no usable key" line. Both clauses appear,
/// semicolon-joined, when one scan turns up both causes.
///
/// Deliberately terse: this is a flash, not the report. It names the action,
/// the `ctrl_h` setting responsible (when that is the cause), and points at
/// `kimun doctor` — `stranded_advice` there is what actually says "rebind or
/// set ctrl_h", in more words than a footer line can afford. Kept at or under
/// [`FLASH_WIDTH_BUDGET`] for the common single-clause case so the pointer to
/// `kimun doctor` is never the part that gets clipped.
fn notice_text(
    stranded: &[crate::keys::reachability::Unreachable],
    setting: crate::settings::CtrlHSetting,
    policy: ctrl_h::CtrlHPolicy,
) -> String {
    use crate::keys::key_combo::{KeyCombo, KeyModifiers};
    use crate::keys::key_strike::KeyStrike;
    use crate::keys::reachability::Reach;

    // The exact pair the rewrite itself produces: `reach()` shadows the bare
    // chord with the plain key, nothing else. Checking the pair (not just the
    // combo) is what keeps a hand-built entry that merely mentions Ctrl+H
    // under some other reach — or a real collision that happens to share the
    // combo — out of this group.
    let ctrl_h_combo = KeyCombo::new(KeyModifiers::new().and_ctrl(), KeyStrike::KeyH);
    let backspace_key = KeyCombo::new(KeyModifiers::new(), KeyStrike::Backspace);
    let is_ctrl_h_loss = |u: &&crate::keys::reachability::Unreachable| {
        policy == ctrl_h::CtrlHPolicy::Backspace
            && u.combos.iter().any(|(combo, reach)| {
                *combo == ctrl_h_combo && *reach == Reach::Shadowed(backspace_key)
            })
    };
    let (ctrl_h_group, plain_group): (Vec<_>, Vec<_>) = stranded.iter().partition(is_ctrl_h_loss);

    // Names two actions at most; anything past that is folded into a count so
    // a heavily-remapped keymap cannot blow the width budget.
    let join_names = |group: &[&crate::keys::reachability::Unreachable]| {
        let names: Vec<String> = group.iter().map(|u| u.action.to_string()).collect();
        if names.len() <= FLASH_NAME_LIMIT {
            names.join(", ")
        } else {
            format!(
                "{} and {} more",
                names[..FLASH_NAME_LIMIT].join(", "),
                names.len() - FLASH_NAME_LIMIT
            )
        }
    };

    let ctrl_h_clause = (!ctrl_h_group.is_empty()).then(|| {
        let names = join_names(&ctrl_h_group);
        let setting = format!("{setting:?}").to_lowercase();
        format!("{names}: Ctrl+H is Backspace (ctrl_h = \"{setting}\")")
    });
    let plain_clause = (!plain_group.is_empty())
        .then(|| format!("{}: no usable key here", join_names(&plain_group)));

    match (ctrl_h_clause, plain_clause) {
        (Some(a), None) => format!("{a} — run `kimun doctor`"),
        (None, Some(b)) => format!("{b} — run `kimun doctor`"),
        (Some(a), Some(b)) => format!("{a}; {b} — run `kimun doctor`"),
        (None, None) => {
            unreachable!("unreachable_binding_notice never calls this with an empty scan")
        }
    }
}

/// Kick off the background update check (gated on the user's `update_check`
/// preference). All network/filesystem work runs on `spawn_blocking` inside
/// `update::check_now`; a found update is surfaced via `AppMessage::Update(UpdateFlow::Available)`. Failures are logged and
/// swallowed — the check never blocks startup or interaction.
fn spawn_update_check(app: &App, tx: AppTx) {
    if !app
        .settings
        .read()
        .expect("settings lock poisoned")
        .update_check()
    {
        return;
    }
    let Ok(config_dir) = crate::settings::config_dir() else {
        return;
    };
    tokio::spawn(async move {
        match crate::update::check_now(config_dir, false).await {
            Ok(Some(status)) if status.should_notify() => {
                let _ = tx.send(AppMessage::Update(UpdateFlow::Available(status)));
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
                AppEvent::Message(AppMessage::Quit) => {
                    app.current_screen.on_exit(&tx).await;
                    return Ok(());
                }
                AppEvent::Message(AppMessage::Redraw) => {
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
                                    let s = app.settings.read().expect("settings lock poisoned");
                                    tracing::debug!(
                                        "COMBO: {} → {:?}",
                                        combo,
                                        s.key_bindings.get_action(&combo)
                                    );
                                    s.key_bindings.get_action(&combo)
                                };
                                let handled_global = match action {
                                    Some(ActionShortcuts::Quit) => {
                                        tx.send(AppMessage::Quit).ok();
                                        true
                                    }
                                    Some(ActionShortcuts::OpenPreferences) => {
                                        let already_on_settings = app.current_screen.get_kind()
                                            == ScreenKind::Preferences;
                                        if !already_on_settings {
                                            tx.send(AppMessage::OpenScreen(
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
                AppEvent::Message(msg) => {
                    // Capture screen identity around handle_app_message so we
                    // can detect a synchronous screen swap (OpenScreen,
                    // VaultIndexed(Conflict)). Use `screen_generation` rather than
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

async fn handle_app_message(msg: AppMessage, app: &mut App, tx: &AppTx) -> io::Result<()> {
    use crate::logic::startup::{IndexOutcome, VaultStatus};
    match msg {
        AppMessage::Redraw => {}
        AppMessage::OpenScreen(screen) => {
            switch_screen(app, tx, screen).await;
        }
        AppMessage::ParkFlash(msg) => app.parked_flashes.push(msg),
        AppMessage::OpenPath { path, emphasis } => {
            // We either handle the new path within the current screen, or we switch to a new screen for this path
            let unhandled = app.current_screen.try_open_path(path, emphasis, tx).await;
            if let Some(path) = unhandled {
                if let Some(vault) = app.vault.clone() {
                    if path.is_note() {
                        tx.send(AppMessage::OpenScreen(ScreenEvent::OpenEditor(vault, path)))
                            .ok();
                    } else {
                        tx.send(AppMessage::OpenScreen(ScreenEvent::OpenBrowse(vault, path)))
                            .ok();
                    }
                } else {
                    // No vault: the app is unconfigured, or its vault has a
                    // conflict (dropped by the `VaultIndexed` arm, workspace
                    // entry kept). A conflicted vault goes back to Preferences
                    // with the error, because the workspace is still set and
                    // rerun Onboarding cannot change it: that route is a dead
                    // end. Unconfigured goes to the guided setup. Saving,
                    // finishing Onboarding or switching workspace resets the
                    // status to Ready.
                    let screen = match app.vault_status.conflict() {
                        Some(msg) => ScreenEvent::OpenPreferencesWithError(msg.to_string()),
                        None => ScreenEvent::OpenOnboarding,
                    };
                    tx.send(AppMessage::OpenScreen(screen)).ok();
                }
            }
        }
        AppMessage::OpenAttachment(path) => {
            // The editor screen shows it in its attachment view; any other
            // screen routes through OpenEditor first, then the attachment opens
            // there. (In practice this is sent from the editor's FILES drawer.)
            let unhandled = app.current_screen.try_open_attachment(path, tx).await;
            if let Some(path) = unhandled
                && let Some(vault) = app.vault.clone()
            {
                tx.send(AppMessage::OpenScreen(ScreenEvent::OpenEditor(vault, path)))
                    .ok();
            }
        }
        AppMessage::OpenJournal => {
            // Resolve today's journal entry (creating it if needed) once, then
            // route it like any other note via OpenPath so it works from every
            // screen — the current screen opens it inline or the loop switches
            // to the editor.
            // Notify the current screen's sidebar when freshly created, then
            // open it — works from every screen via OpenPath.
            if let Some(vault) = app.vault.clone() {
                crate::logic::file_ops::open_journal(&vault, tx).await;
            }
        }
        AppMessage::PreferencesSaved | AppMessage::OnboardingFinished => {
            // Rebuild the vault so workspace path and inbox_path changes take effect.
            app.vault = crate::logic::startup::open_vault(&app.settings).await;
            app.vault_status = VaultStatus::default();
            respawn_rag(app, tx);
            tx.send(AppMessage::OpenScreen(ScreenEvent::Start)).ok();
        }
        AppMessage::ClosePreferences => {
            tx.send(AppMessage::OpenScreen(ScreenEvent::Start)).ok();
        }
        AppMessage::VaultIndexed(outcome) => {
            let was_conflicted = app.vault_status.is_conflicted();
            app.vault_status.apply(&outcome);
            if let IndexOutcome::Conflict(msg) = outcome {
                // The vault has structural conflicts (e.g. case-insensitive
                // path clashes). Drop the unusable vault and show Preferences
                // with the error overlay. The workspace entry stays, in memory
                // and on disk: the vault opens again once the clash is fixed.
                // Not forwarded to the current screen: the swap replaces it.
                app.vault = None;
                respawn_rag(app, tx);
                switch_screen(app, tx, ScreenEvent::OpenPreferencesWithError(msg)).await;
            } else {
                // A clean pass after a conflict (the user fixed the folder and
                // reindexed from Preferences) ran on a throwaway vault: the
                // app's own vault was dropped, so rebuild it. Otherwise Start
                // finds no vault and sends the user to Onboarding. If the
                // clash is still there, the next Start pass reports it again.
                if was_conflicted && matches!(outcome, IndexOutcome::Done(_)) && app.vault.is_none()
                {
                    app.vault = crate::logic::startup::open_vault(&app.settings).await;
                    respawn_rag(app, tx);
                }
                // Done and Failed belong to the screen that started the pass
                // (Start restores the session, Preferences finishes a Save).
                app.current_screen
                    .handle_app_message(AppMessage::VaultIndexed(outcome), tx)
                    .await;
            }
        }
        AppMessage::WorkspaceSwitched(name) => {
            {
                let mut s = app.settings.write().expect("settings lock poisoned");
                if let Some(ref mut wc) = s.workspace_config {
                    wc.global.current_workspace = name;
                }
                s.save_to_disk().ok();
            }
            app.vault = crate::logic::startup::open_vault(&app.settings).await;
            app.vault_status = VaultStatus::default();
            respawn_rag(app, tx);
            tx.send(AppMessage::OpenScreen(ScreenEvent::Start)).ok();
        }
        AppMessage::Update(flow) => {
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
                .handle_app_message(AppMessage::Update(flow), tx)
                .await;
        }
        AppMessage::RagStatus(status) => {
            // Same pattern as update: keep app-globally for screen seeding, and
            // forward for immediate display.
            app.rag_status = status;
            app.current_screen
                .handle_app_message(AppMessage::RagStatus(status), tx)
                .await;
        }
        AppMessage::ServerUpdate(latest) => {
            app.server_update = latest.clone();
            app.current_screen
                .handle_app_message(AppMessage::ServerUpdate(latest), tx)
                .await;
        }
        AppMessage::FileOp(op) => {
            // History is app-wide bookkeeping; keep it in step here, whichever
            // screen hosted the operation, then let the screen react.
            match &op {
                FileOp::Renamed { from, to } | FileOp::Moved { from, to } => {
                    app.settings
                        .read()
                        .expect("settings lock poisoned")
                        .follow_rename_in_history(from, to);
                }
                FileOp::Deleted(path) => {
                    app.settings
                        .read()
                        .expect("settings lock poisoned")
                        .follow_delete_in_history(path);
                }
                _ => {}
            }
            app.current_screen
                .handle_app_message(AppMessage::FileOp(op), tx)
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

    /// The startup key warning is parked until a screen that can show it.
    /// Start can route to Onboarding or Browse before any editor exists;
    /// consuming the flash there loses the only notice the user gets.
    #[tokio::test]
    async fn the_key_warning_waits_for_a_screen_that_shows_flashes() {
        use crate::logic::message::ScreenEvent;
        use kimun_core::nfs::VaultPath;
        use kimun_core::{NoteVault, VaultConfig};

        let settings = Arc::new(RwLock::new(AppSettings::default()));
        let mut app = App::from_settings(settings).await;
        app.parked_flashes.push("stranded".to_string());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        super::switch_screen(&mut app, &tx, ScreenEvent::OpenOnboarding).await;
        assert_eq!(
            app.parked_flashes,
            vec!["stranded".to_string()],
            "onboarding cannot show a flash, so the warning must still be parked"
        );

        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(
            NoteVault::new(VaultConfig::new(crate::test_support::sys(dir.path())))
                .await
                .unwrap(),
        );
        super::switch_screen(
            &mut app,
            &tx,
            ScreenEvent::OpenEditor(vault, VaultPath::root()),
        )
        .await;
        assert!(
            app.parked_flashes.is_empty(),
            "the editor shows flashes, so the one-shot is delivered and gone"
        );
    }

    /// An app whose current workspace's history is `history`, newest first,
    /// kept in `scratch` (the config file too).
    async fn app_with_history(history: &[&str], scratch: &tempfile::TempDir) -> App {
        let workspace = crate::test_support::sys(scratch.path());
        let settings = AppSettings::for_test_workspace("ws", &workspace, workspace.clone());
        settings.seed_test_history(history);
        App::from_settings(Arc::new(RwLock::new(settings))).await
    }

    async fn file_op(app: &mut App, op: crate::logic::message::FileOp) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        super::handle_app_message(crate::logic::message::AppMessage::FileOp(op), app, &tx)
            .await
            .unwrap();
    }

    /// The history as plain strings, form-independent (entries may be stored
    /// relative or absolute).
    fn history_of(app: &App) -> Vec<String> {
        app.settings
            .read()
            .unwrap()
            .current_last_paths()
            .iter()
            .map(|p| {
                let mut p = p.clone();
                p.to_relative();
                p.to_string()
            })
            .collect()
    }

    /// A rename done inside kimün must follow through to the history, or the
    /// next launch finds the old path gone and blames an outside change.
    #[tokio::test]
    async fn renaming_a_note_rewrites_it_in_the_history() {
        use crate::logic::message::FileOp;
        use kimun_core::nfs::VaultPath;

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&["a.md", "other.md"], &scratch).await;
        file_op(
            &mut app,
            FileOp::Renamed {
                from: VaultPath::new("a.md"),
                to: VaultPath::new("b.md"),
            },
        )
        .await;
        assert_eq!(history_of(&app), vec!["b.md", "other.md"]);
    }

    #[tokio::test]
    async fn moving_a_directory_rewrites_the_notes_beneath_it() {
        use crate::logic::message::FileOp;
        use kimun_core::nfs::VaultPath;

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&["dir/a.md", "dirx/b.md"], &scratch).await;
        file_op(
            &mut app,
            FileOp::Moved {
                from: VaultPath::new("dir"),
                to: VaultPath::new("archive/dir"),
            },
        )
        .await;
        assert_eq!(history_of(&app), vec!["archive/dir/a.md", "dirx/b.md"]);
    }

    #[tokio::test]
    async fn deleting_drops_the_note_or_everything_beneath_the_directory() {
        use crate::logic::message::FileOp;
        use kimun_core::nfs::VaultPath;

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app =
            app_with_history(&["a.md", "dir/b.md", "dirx/c.md", "keep.md"], &scratch).await;
        file_op(&mut app, FileOp::Deleted(VaultPath::new("a.md"))).await;
        file_op(&mut app, FileOp::Deleted(VaultPath::new("dir"))).await;
        assert_eq!(history_of(&app), vec!["dirx/c.md", "keep.md"]);
    }

    /// The footer holds one flash at a time, so notices parked together must
    /// reach it together — delivered one by one, each overwrites the last
    /// before a frame is ever drawn.
    #[tokio::test]
    async fn every_parked_flash_reaches_the_footer() {
        use crate::logic::message::{AppMessage, ScreenEvent};
        use kimun_core::nfs::VaultPath;

        let settings = Arc::new(RwLock::new(AppSettings::default()));
        let mut app = App::from_settings(settings).await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        for notice in ["first notice", "second notice"] {
            super::handle_app_message(AppMessage::ParkFlash(notice.to_string()), &mut app, &tx)
                .await
                .unwrap();
        }

        let vault = crate::test_support::temp_vault("app").await;
        super::switch_screen(
            &mut app,
            &tx,
            ScreenEvent::OpenEditor(vault, VaultPath::root()),
        )
        .await;

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 30)).unwrap();
        terminal.draw(|f| app.current_screen.render(f)).unwrap();
        let flat: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(flat.contains("first notice"), "first notice lost");
        assert!(flat.contains("second notice"), "second notice lost");
    }

    /// Feed `msg` to the app handler.
    async fn handle(app: &mut App, msg: crate::logic::message::AppMessage) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        super::handle_app_message(msg, app, &tx).await.unwrap();
    }

    fn conflict(msg: &str) -> crate::logic::message::AppMessage {
        crate::logic::message::AppMessage::VaultIndexed(
            crate::logic::startup::IndexOutcome::Conflict(msg.to_string()),
        )
    }

    fn workspace_path_of(app: &App) -> Option<kimun_core::system::SystemPath> {
        app.settings.read().unwrap().resolve_workspace_path()
    }

    /// The config file of an app built by `app_with_history`, written once
    /// so there are bytes to compare, plus a hand-written comment: any
    /// rewrite drops it, even one of unchanged settings.
    fn saved_config(app: &App, scratch: &tempfile::TempDir) -> (std::path::PathBuf, Vec<u8>) {
        app.settings.read().unwrap().save_to_disk().unwrap();
        let path = scratch.path().join("config.toml");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n# hand edit\n");
        std::fs::write(&path, &bytes).unwrap();
        (path, bytes)
    }

    /// A conflict says the vault is unusable, not that the user's workspace
    /// entry is wrong: the entry stays so the vault opens again once fixed.
    #[tokio::test]
    async fn a_vault_conflict_keeps_the_workspace_and_opens_preferences_with_the_error() {
        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        let before = workspace_path_of(&app);
        assert!(before.is_some());

        handle(&mut app, conflict("boom")).await;

        assert_eq!(workspace_path_of(&app), before, "the workspace stays");
        assert_eq!(
            app.settings.read().unwrap().current_workspace_name(),
            Some("ws".to_string())
        );
        assert!(app.vault.is_none(), "the unusable vault is dropped");
        assert_eq!(app.vault_status.conflict(), Some("boom"));
        assert_eq!(app.current_screen.get_kind(), ScreenKind::Preferences);
    }

    #[tokio::test]
    async fn a_vault_conflict_does_not_write_the_settings_file() {
        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        let (path, before) = saved_config(&app, &scratch);

        handle(&mut app, conflict("boom")).await;

        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    /// Feed every queued message back to the app handler, the way the loop
    /// does, until the screen is `target` (or the queue runs dry).
    async fn pump_until(
        app: &mut App,
        tx: &crate::logic::message::AppTx,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::logic::message::AppMessage>,
        target: ScreenKind,
    ) {
        for _ in 0..100 {
            if app.current_screen.get_kind() == target {
                return;
            }
            let Ok(msg) = rx.try_recv() else { return };
            super::handle_app_message(msg, app, tx).await.unwrap();
        }
    }

    /// Closing Preferences after a conflict goes through Start, which has no
    /// vault. The status is conflicted, so it returns to Preferences with the
    /// error: rerun Onboarding cannot change the workspace.
    #[tokio::test]
    async fn closing_preferences_after_a_conflict_returns_to_preferences_with_the_error() {
        use crate::logic::message::AppMessage;

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        super::handle_app_message(conflict("boom"), &mut app, &tx)
            .await
            .unwrap();
        assert_eq!(app.current_screen.get_kind(), ScreenKind::Preferences);

        super::handle_app_message(AppMessage::ClosePreferences, &mut app, &tx)
            .await
            .unwrap();
        // Start is the intermediate screen: wait for it, then let its
        // OpenPath reach the app handler.
        pump_until(&mut app, &tx, &mut rx, ScreenKind::Start).await;
        assert_eq!(app.current_screen.get_kind(), ScreenKind::Start);
        for _ in 0..100 {
            let Ok(msg) = rx.try_recv() else { break };
            super::handle_app_message(msg, &mut app, &tx).await.unwrap();
            if app.current_screen.get_kind() != ScreenKind::Start {
                break;
            }
        }

        assert_eq!(app.current_screen.get_kind(), ScreenKind::Preferences);
        assert!(workspace_path_of(&app).is_some(), "the workspace stays");
        assert!(app.vault.is_none());
        assert_eq!(app.vault_status.conflict(), Some("boom"));
    }

    #[tokio::test]
    async fn no_vault_with_a_ready_status_still_reaches_onboarding() {
        use crate::logic::message::AppMessage;

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        app.vault = None;
        assert!(!app.vault_status.is_conflicted());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        super::handle_app_message(
            AppMessage::open(kimun_core::nfs::VaultPath::root()),
            &mut app,
            &tx,
        )
        .await
        .unwrap();
        pump_until(&mut app, &tx, &mut rx, ScreenKind::Onboarding).await;

        assert_eq!(app.current_screen.get_kind(), ScreenKind::Onboarding);
    }

    #[tokio::test]
    async fn after_a_conflict_saving_preferences_clears_the_conflicted_route() {
        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        handle(&mut app, conflict("boom")).await;
        assert!(app.vault_status.is_conflicted());

        handle(
            &mut app,
            crate::logic::message::AppMessage::PreferencesSaved,
        )
        .await;

        assert_eq!(app.vault_status, crate::logic::startup::VaultStatus::Ready);
    }

    fn indexed(outcome: crate::logic::startup::IndexOutcome) -> crate::logic::message::AppMessage {
        crate::logic::message::AppMessage::VaultIndexed(outcome)
    }

    /// Fix the folder, reindex from Preferences: the clean pass must bring
    /// the vault back, or Esc lands on Onboarding.
    #[tokio::test]
    async fn a_clean_reindex_after_a_conflict_reopens_the_vault() {
        use crate::logic::message::AppMessage;
        use crate::logic::startup::{IndexOutcome, VaultStatus};

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        super::handle_app_message(conflict("boom"), &mut app, &tx)
            .await
            .unwrap();
        assert!(app.vault.is_none());
        super::handle_app_message(
            indexed(IndexOutcome::Done(std::time::Duration::ZERO)),
            &mut app,
            &tx,
        )
        .await
        .unwrap();
        assert!(app.vault.is_some(), "the vault is rebuilt");
        assert_eq!(app.vault_status, VaultStatus::Ready);

        super::handle_app_message(AppMessage::ClosePreferences, &mut app, &tx)
            .await
            .unwrap();
        pump_until(&mut app, &tx, &mut rx, ScreenKind::Onboarding).await;
        for _ in 0..100 {
            let Ok(msg) = rx.try_recv() else { break };
            super::handle_app_message(msg, &mut app, &tx).await.unwrap();
        }
        assert_ne!(app.current_screen.get_kind(), ScreenKind::Onboarding);
    }

    #[tokio::test]
    async fn a_done_with_a_ready_status_does_not_replace_the_vault() {
        use crate::logic::startup::IndexOutcome;

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        let before = app.vault.clone().expect("an open vault");

        handle(
            &mut app,
            indexed(IndexOutcome::Done(std::time::Duration::ZERO)),
        )
        .await;

        assert!(Arc::ptr_eq(&before, app.vault.as_ref().unwrap()));
    }

    #[tokio::test]
    async fn a_failed_pass_after_a_conflict_leaves_the_vault_dropped() {
        use crate::logic::startup::IndexOutcome;

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        handle(&mut app, conflict("boom")).await;

        handle(&mut app, indexed(IndexOutcome::Failed("x".into()))).await;

        assert!(app.vault.is_none());
        assert!(app.vault_status.is_conflicted());
    }

    async fn conflicted_app(scratch: &tempfile::TempDir) -> App {
        let mut app = app_with_history(&[], scratch).await;
        app.vault_status = crate::logic::startup::VaultStatus::Conflicted("old".into());
        app
    }

    #[tokio::test]
    async fn saving_preferences_resets_the_status_to_ready() {
        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = conflicted_app(&scratch).await;
        handle(
            &mut app,
            crate::logic::message::AppMessage::PreferencesSaved,
        )
        .await;
        assert_eq!(app.vault_status, crate::logic::startup::VaultStatus::Ready);
    }

    #[tokio::test]
    async fn finishing_onboarding_resets_the_status_to_ready() {
        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = conflicted_app(&scratch).await;
        handle(
            &mut app,
            crate::logic::message::AppMessage::OnboardingFinished,
        )
        .await;
        assert_eq!(app.vault_status, crate::logic::startup::VaultStatus::Ready);
    }

    #[tokio::test]
    async fn switching_workspace_resets_the_status_to_ready() {
        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = conflicted_app(&scratch).await;
        handle(
            &mut app,
            crate::logic::message::AppMessage::WorkspaceSwitched("ws".into()),
        )
        .await;
        assert_eq!(app.vault_status, crate::logic::startup::VaultStatus::Ready);
    }

    /// A Save that moves the workspace onto a case-clashing folder reindexes
    /// it, hits the conflict, and must keep what the user entered: the
    /// workspace still points at the new folder in memory, and the settings
    /// file is not rewritten (the save waits for a clean index).
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_save_that_hits_a_conflict_keeps_the_entered_workspace() {
        use crate::logic::message::{AppMessage, ScreenEvent};
        use crate::logic::startup::IndexOutcome;
        use crate::test_support::key_event;
        use ratatui::crossterm::event::KeyCode;

        let scratch = tempfile::TempDir::new().unwrap();
        let clash = tempfile::TempDir::new().unwrap();
        std::fs::write(clash.path().join("note.md"), "a").unwrap();
        std::fs::write(clash.path().join("Note.md"), "b").unwrap();

        let mut app = app_with_history(&[], &scratch).await;
        let (path, before) = saved_config(&app, &scratch);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        super::switch_screen(&mut app, &tx, ScreenEvent::OpenPreferences).await;

        app.settings
            .write()
            .unwrap()
            .set_workspace_path("ws", clash.path().to_path_buf());
        let moved = workspace_path_of(&app);
        app.current_screen
            .handle_input(&key_event(KeyCode::Esc), &tx);
        app.current_screen
            .handle_input(&key_event(KeyCode::Enter), &tx);

        // Run the loop's half until the reindex outcome has been handled.
        loop {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
                .await
                .expect("timed out waiting for the reindex outcome")
                .expect("channel closed");
            let done = matches!(msg, AppMessage::VaultIndexed(_));
            if let AppMessage::VaultIndexed(ref outcome) = msg {
                assert!(
                    matches!(outcome, IndexOutcome::Conflict(_)),
                    "expected a conflict, got {outcome:?}"
                );
            }
            super::handle_app_message(msg, &mut app, &tx).await.unwrap();
            if done {
                break;
            }
        }

        assert_eq!(workspace_path_of(&app), moved, "the entered path stays");
        assert_eq!(
            app.settings.read().unwrap().current_workspace_name(),
            Some("ws".to_string()),
            "the workspace is not cleared"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before, "nothing was saved");
        assert!(app.vault.is_none());
        assert!(app.vault_status.is_conflicted());
        assert_eq!(app.current_screen.get_kind(), ScreenKind::Preferences);
    }

    /// Drive a Start screen that holds a real vault, waiting for its pass.
    /// The pass runs on a throwaway channel so only the injected `outcome`
    /// reaches the app. Returns once the screen has asked to open a path, the
    /// way the loop would see it, and the app has moved off Start.
    async fn start_receives_through_the_app(outcome: crate::logic::startup::IndexOutcome) {
        use crate::logic::message::{AppMessage, ScreenEvent};

        let scratch = tempfile::TempDir::new().unwrap();
        let mut app = app_with_history(&[], &scratch).await;
        assert!(app.vault.is_some());
        let (throwaway_tx, _throwaway_rx) = tokio::sync::mpsc::unbounded_channel();
        super::switch_screen(&mut app, &throwaway_tx, ScreenEvent::Start).await;
        assert_eq!(app.current_screen.get_kind(), ScreenKind::Start);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        super::handle_app_message(indexed(outcome), &mut app, &tx)
            .await
            .unwrap();

        // Start answers with OpenPath only if the app forwarded the outcome.
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("Start never received the outcome: it would hang on the spinner")
            .expect("channel closed");
        assert!(matches!(msg, AppMessage::OpenPath { .. }), "got {msg:?}");
        super::handle_app_message(msg, &mut app, &tx).await.unwrap();
        for _ in 0..100 {
            if app.current_screen.get_kind() != ScreenKind::Start {
                break;
            }
            let Ok(Some(msg)) =
                tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await
            else {
                break;
            };
            super::handle_app_message(msg, &mut app, &tx).await.unwrap();
        }
        assert_ne!(app.current_screen.get_kind(), ScreenKind::Start);
    }

    /// The app must hand a clean pass to the screen that started it, or the
    /// Start spinner never ends and eats all input.
    #[tokio::test]
    async fn a_done_pass_is_forwarded_to_the_start_screen() {
        start_receives_through_the_app(crate::logic::startup::IndexOutcome::Done(
            std::time::Duration::ZERO,
        ))
        .await;
    }

    /// Start restores the session on a failed pass too.
    #[tokio::test]
    async fn a_failed_pass_is_forwarded_to_the_start_screen() {
        start_receives_through_the_app(crate::logic::startup::IndexOutcome::Failed("x".into()))
            .await;
    }

    /// A Save that moves the workspace reindexes it; the clean pass must
    /// reach Preferences through the app, or the Save never finishes.
    #[tokio::test]
    async fn a_done_pass_is_forwarded_to_preferences_during_a_save() {
        use crate::logic::message::{AppMessage, ScreenEvent};
        use crate::test_support::key_event;
        use ratatui::crossterm::event::KeyCode;

        let scratch = tempfile::TempDir::new().unwrap();
        let other = tempfile::TempDir::new().unwrap();
        std::fs::write(other.path().join("note.md"), "a").unwrap();

        let mut app = app_with_history(&[], &scratch).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        super::switch_screen(&mut app, &tx, ScreenEvent::OpenPreferences).await;
        app.settings
            .write()
            .unwrap()
            .set_workspace_path("ws", other.path().to_path_buf());
        app.current_screen
            .handle_input(&key_event(KeyCode::Esc), &tx);
        app.current_screen
            .handle_input(&key_event(KeyCode::Enter), &tx);

        // The loop's half: feed messages to the app until the screen reports
        // the Save finished.
        // One deadline for the whole wait: progress ticks keep arriving, so a
        // per-message timeout would never fire if the outcome were dropped.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let msg = tokio::time::timeout_at(deadline, rx.recv())
                .await
                .expect("timed out: Preferences never finished the Save")
                .expect("channel closed");
            let saved = matches!(msg, AppMessage::PreferencesSaved);
            if let AppMessage::VaultIndexed(ref o) = msg {
                assert!(
                    matches!(o, crate::logic::startup::IndexOutcome::Done(_)),
                    "{o:?}"
                );
            }
            if saved {
                break;
            }
            super::handle_app_message(msg, &mut app, &tx).await.unwrap();
        }
    }

    /// When `auto` chose Backspace the stranded action is kimün's own
    /// default, and the flash has to say which setting did it — "run doctor"
    /// alone reads as a broken install.
    #[test]
    fn the_notice_names_ctrl_h_when_auto_chose_backspace() {
        use crate::app::ctrl_h::CtrlHPolicy;
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;
        use crate::keys::reachability::{Reach, Unreachable};
        use crate::settings::CtrlHSetting;

        let ctrl_h = KeyCombo::new(KeyModifiers::new().and_ctrl(), KeyStrike::KeyH);
        let stranded = vec![Unreachable {
            action: ActionShortcuts::FocusSidebar,
            combos: vec![(
                ctrl_h,
                Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Backspace)),
            )],
        }];

        let auto = super::notice_text(&stranded, CtrlHSetting::Auto, CtrlHPolicy::Backspace);
        assert!(auto.contains("FocusSidebar"), "{auto}");
        assert!(auto.contains("ctrl_h"), "{auto}");
        assert!(auto.contains("kimun doctor"), "{auto}");

        // A stranding the rewrite had no part in keeps the plain wording.
        let theirs = super::notice_text(&stranded, CtrlHSetting::Auto, CtrlHPolicy::Chord);
        assert!(!theirs.contains("ctrl_h"), "{theirs}");
        assert!(theirs.contains("no usable key"), "{theirs}");
    }

    /// A single scan can turn up both causes at once: the rewrite's own loss
    /// and an unrelated collision out of the user's own `[key_bindings]`.
    /// Wrapping that second action's name in the ctrl_h wording would send
    /// the user off to change the wrong setting, so the two clauses must
    /// stay apart in the string, not just both appear somewhere in it.
    #[test]
    fn the_notice_keeps_an_unrelated_stranding_out_of_the_ctrl_h_clause() {
        use crate::app::ctrl_h::CtrlHPolicy;
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;
        use crate::keys::reachability::{Reach, Unreachable};
        use crate::settings::CtrlHSetting;

        let ctrl_h = KeyCombo::new(KeyModifiers::new().and_ctrl(), KeyStrike::KeyH);
        let ctrl_i = KeyCombo::new(KeyModifiers::new().and_ctrl(), KeyStrike::KeyI);
        let stranded = vec![
            Unreachable {
                action: ActionShortcuts::FocusSidebar,
                combos: vec![(
                    ctrl_h,
                    Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Backspace)),
                )],
            },
            Unreachable {
                action: ActionShortcuts::QuickNote,
                combos: vec![(
                    ctrl_i,
                    Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Tab)),
                )],
            },
        ];

        let text = super::notice_text(&stranded, CtrlHSetting::Auto, CtrlHPolicy::Backspace);
        let ctrl_h_clause = text
            .split_once("; ")
            .map(|(first, _)| first)
            .unwrap_or_else(|| panic!("expected the ctrl_h clause joined with '; ': {text}"));
        assert!(
            ctrl_h_clause.contains("FocusSidebar"),
            "ctrl_h clause: {ctrl_h_clause}"
        );
        assert!(
            !ctrl_h_clause.contains("QuickNote"),
            "ctrl_h clause: {ctrl_h_clause}"
        );
        assert!(text.contains("QuickNote"), "{text}");
    }

    /// Pins the width budget: the footer renders this flash as an unwrapped,
    /// centre-aligned `Paragraph` (`components::footer_bar.rs`), so a line
    /// wider than the terminal is clipped from the *right* — losing the
    /// `kimun doctor` pointer this flash exists to deliver. The common
    /// scenario (the shipped default, `auto` chose Backspace) must fit an
    /// 80-column footer with room to spare. A regression here means someone
    /// widened the wording without checking it still fits.
    #[test]
    fn the_single_clause_notice_fits_the_flash_width_budget() {
        use crate::app::ctrl_h::CtrlHPolicy;
        use crate::keys::key_combo::{KeyCombo, KeyModifiers};
        use crate::keys::key_strike::KeyStrike;
        use crate::keys::reachability::{Reach, Unreachable};
        use crate::settings::CtrlHSetting;

        let ctrl_h = KeyCombo::new(KeyModifiers::new().and_ctrl(), KeyStrike::KeyH);
        let stranded = vec![Unreachable {
            action: ActionShortcuts::FocusSidebar,
            combos: vec![(
                ctrl_h,
                Reach::Shadowed(KeyCombo::new(KeyModifiers::new(), KeyStrike::Backspace)),
            )],
        }];

        let text = super::notice_text(&stranded, CtrlHSetting::Auto, CtrlHPolicy::Backspace);
        assert!(
            text.chars().count() <= super::FLASH_WIDTH_BUDGET,
            "{} chars, over the {}-column budget: {text}",
            text.chars().count(),
            super::FLASH_WIDTH_BUDGET
        );
    }
}
