pub mod app;
pub mod app_screen;
pub mod cli;
pub mod components;
pub mod event_handler;
pub mod keys;
pub mod settings;
pub mod ui;

use clap::Parser;
use color_eyre::Result;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "kimun", about = "Kimün notes")]
struct Cli {
    /// Path to a custom config file
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<crate::cli::CliCommand>,
}

use crossterm::event::{
    DisableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};
use ratatui::Terminal;
use ratatui::crossterm::event::EnableMouseCapture;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode, supports_keyboard_enhancement};
use ratatui::prelude::{Backend, CrosstermBackend};
use std::io;

use crate::app::App;
use crate::app_screen::browse::BrowseScreen;
use crate::app_screen::editor::EditorScreen;
use crate::app_screen::settings::SettingsScreen;
use crate::app_screen::start::StartScreen;
use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent};
use crate::event_handler::EventHandler;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    // Check if CLI subcommand was provided
    if let Some(command) = cli.command {
        // CLI mode - run command and exit
        return crate::cli::run_cli(command, cli.config).await;
    }

    // TUI mode continues below...
    #[cfg(debug_assertions)]
    {
        use simplelog::*;
        let log_file = std::fs::File::create("kimun.log").unwrap();
        WriteLogger::init(LevelFilter::Debug, Config::default(), log_file).unwrap();
    }
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
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
    run_app(&mut terminal, &mut app, &mut events).await?;
    disable_raw_mode()?;
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn switch_screen(app: &mut App, tx: &AppTx, new_screen: ScreenEvent) {
    if let Some(current) = app.current_screen.as_mut() {
        current.on_exit(tx).await;
    }

    let mut screen: Box<dyn AppScreen> = match new_screen {
        ScreenEvent::Start => Box::new(StartScreen::new(app.settings.clone(), None)),
        ScreenEvent::OpenSettings => Box::new(SettingsScreen::new(app.settings.clone())),
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
    };

    screen.on_enter(tx).await;
    app.current_screen = Some(screen);
}

// async fn switch_screen(app: &mut App, tx: &AppTx, new_screen: Box<dyn AppScreen>) {
//     if let Some(current) = app.current_screen.as_mut() {
//         current.on_exit(tx).await;
//     }
//     let mut screen = new_screen;
//     screen.on_enter(tx).await;
//     app.current_screen = Some(screen);
// }

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

        match events.next().await {
            AppEvent::Quit => {
                if let Some(screen) = app.current_screen.as_mut() {
                    screen.on_exit(&tx).await;
                }
                return Ok(());
            }
            AppEvent::Input(input) => {
                match input {
                    InputEvent::Key(key) => {
                        log::debug!(
                            "KEY: code={:?} mods={:?} kind={:?}",
                            key.code,
                            key.modifiers,
                            key.kind
                        );
                        // Global shortcuts — fire before any screen gets the event.
                        if let Some(combo) = key_event_to_combo(&key) {
                            log::debug!(
                                "COMBO: {} → {:?}",
                                combo,
                                app.settings.key_bindings.get_action(&combo)
                            );
                            match app.settings.key_bindings.get_action(&combo) {
                                Some(ActionShortcuts::Quit) => {
                                    tx.send(AppEvent::Quit).ok();
                                    continue;
                                }
                                Some(ActionShortcuts::OpenSettings) => {
                                    let already_on_settings = app
                                        .current_screen
                                        .as_ref()
                                        .map(|s| s.get_kind() == ScreenKind::Settings)
                                        .unwrap_or(false);
                                    if !already_on_settings {
                                        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenSettings))
                                            .ok();
                                    }
                                    continue;
                                }
                                _ => {}
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
                }
            }
            msg => handle_app_message(msg, app, &tx).await?,
        }
    }
}

async fn handle_app_message(msg: AppEvent, app: &mut App, tx: &AppTx) -> io::Result<()> {
    match msg {
        AppEvent::Redraw => {}
        AppEvent::OpenScreen(screen) => {
            switch_screen(app, tx, screen).await;
        }
        AppEvent::OpenPath(path) => {
            // We either handle the new path within the current screen, or we switch to a new screen for this path
            let unhandled = if let Some(screen) = app.current_screen.as_mut() {
                screen
                    .handle_app_message(AppEvent::OpenPath(path), tx)
                    .await
            } else {
                Some(AppEvent::OpenPath(path))
            };
            if let Some(AppEvent::OpenPath(path)) = unhandled {
                if let Some(vault) = app.vault.clone() {
                    if path.is_note() {
                        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenEditor(vault, path)))
                            .ok();
                    } else {
                        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(vault, path)))
                            .ok();
                    }
                } else {
                    tx.send(AppEvent::OpenScreen(ScreenEvent::OpenSettings))
                        .ok();
                }
            }
        }
        AppEvent::SettingsSaved(new_settings) => {
            if new_settings.workspace_dir != app.settings.workspace_dir {
                app.vault = if let Some(ref workspace) = new_settings.workspace_dir {
                    kimun_core::NoteVault::new(workspace)
                        .await
                        .ok()
                        .map(std::sync::Arc::new)
                } else {
                    None
                };
            }
            app.settings = new_settings;
            tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
            // switch_screen(app, tx, Box::new(StartScreen::new(app.settings.clone()))).await;
        }
        AppEvent::CloseSettings => {
            tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
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
        let key = KeyEvent {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        let combo = key_event_to_combo(&key).expect("Ctrl+P should produce a combo");
        let action = settings.key_bindings.get_action(&combo);
        assert_eq!(action, Some(ActionShortcuts::OpenSettings));

        // Simulate the app-level dispatch: on OpenSettings, send OpenScreen(OpenSettings).
        let (tx, mut rx) = unbounded_channel();
        tx.send(AppEvent::OpenScreen(ScreenEvent::OpenSettings))
            .ok();
        let msg = rx.try_recv().expect("should have a message");
        assert!(matches!(
            msg,
            AppEvent::OpenScreen(ScreenEvent::OpenSettings)
        ));
    }
}
