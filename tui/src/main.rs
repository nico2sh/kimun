pub mod app;
pub mod app_screen;
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
}

use crossterm::event::{
    DisableMouseCapture, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};
use ratatui::Terminal;
use ratatui::crossterm::event::EnableMouseCapture;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::prelude::{Backend, CrosstermBackend};
use std::io;

use crate::app::App;
use crate::app_screen::{AppScreen, ScreenKind};
use crate::app_screen::browse::BrowseScreen;
use crate::app_screen::editor::EditorScreen;
use crate::app_screen::settings::SettingsScreen;
use crate::app_screen::start::StartScreen;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::events::AppEvent;
use crate::event_handler::{EventHandler, TuiEvent};
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    color_eyre::install()?;
    #[cfg(debug_assertions)]
    {
        use simplelog::*;
        let log_file = std::fs::File::create("/tmp/kimun-keys.log").unwrap();
        WriteLogger::init(LevelFilter::Debug, Config::default(), log_file).unwrap();
    }
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    // Best-effort: terminals that support the kitty keyboard protocol will honour this
    // and report Ctrl+symbol combos (e.g. Ctrl+,) correctly. Terminals that don't
    // support it safely ignore the escape sequence, so we send it unconditionally.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
        )
    );
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

async fn switch_screen(app: &mut App, tx: &AppTx, new_screen: Box<dyn AppScreen>) {
    if let Some(current) = app.current_screen.as_mut() {
        current.on_exit(tx).await;
    }
    let mut screen = new_screen;
    screen.on_enter(tx).await;
    app.current_screen = Some(screen);
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App, events: &mut EventHandler) -> io::Result<()>
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
            TuiEvent::App(AppMessage::Quit) => {
                if let Some(screen) = app.current_screen.as_mut() {
                    screen.on_exit(&tx).await;
                }
                return Ok(());
            }
            TuiEvent::App(msg) => handle_app_message(msg, app, &tx).await?,
            TuiEvent::Crossterm(raw) => {
                let app_event = match raw {
                    Event::Key(key) if key.kind != crossterm::event::KeyEventKind::Release => {
                        log::debug!("KEY: code={:?} mods={:?} kind={:?}", key.code, key.modifiers, key.kind);
                        AppEvent::Key(key)
                    }
                    Event::Mouse(mouse) => AppEvent::Mouse(mouse),
                    _ => continue,
                };
                // Global shortcuts — fire before any screen gets the event.
                if let AppEvent::Key(key) = &app_event {
                    if let Some(combo) = key_event_to_combo(key) {
                        log::debug!("COMBO: {} → {:?}", combo, app.settings.key_bindings.get_action(&combo));
                        match app.settings.key_bindings.get_action(&combo) {
                            Some(ActionShortcuts::Quit) => {
                                tx.send(AppMessage::Quit).ok();
                                continue;
                            }
                            Some(ActionShortcuts::OpenSettings) => {
                                let already_on_settings = app.current_screen
                                    .as_ref()
                                    .map(|s| s.get_kind() == ScreenKind::Settings)
                                    .unwrap_or(false);
                                if !already_on_settings {
                                    tx.send(AppMessage::OpenSettings).ok();
                                }
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(screen) = &mut app.current_screen {
                    screen.handle_event(&app_event, &tx);
                }
            }
        }
    }
}

async fn handle_app_message(msg: AppMessage, app: &mut App, tx: &AppTx) -> io::Result<()> {
    match msg {
        AppMessage::Redraw => {}
        AppMessage::OpenSettings => {
            switch_screen(app, tx, Box::new(SettingsScreen::new(app.settings.clone()))).await;
        }
        AppMessage::OpenEditor(vault, path) => {
            switch_screen(app, tx, Box::new(EditorScreen::new(vault, path, app.settings.clone()))).await;
        }
        AppMessage::OpenBrowse(vault, path) => {
            switch_screen(app, tx, Box::new(BrowseScreen::new(vault, path, app.settings.clone()))).await;
        }
        AppMessage::OpenPath(path) => {
            let unhandled = if let Some(screen) = app.current_screen.as_mut() {
                screen.handle_app_message(AppMessage::OpenPath(path), tx).await
            } else {
                Some(AppMessage::OpenPath(path))
            };
            if let Some(AppMessage::OpenPath(path)) = unhandled {
                if let Some(vault) = app.vault.clone() {
                    if path.is_note() {
                        tx.send(AppMessage::OpenEditor(vault, path)).ok();
                    } else {
                        tx.send(AppMessage::OpenBrowse(vault, path)).ok();
                    }
                } else {
                    tx.send(AppMessage::OpenSettings).ok();
                }
            }
        }
        AppMessage::SettingsSaved(new_settings) => {
            if new_settings.workspace_dir != app.settings.workspace_dir {
                app.vault = if let Some(ref workspace) = new_settings.workspace_dir {
                    kimun_core::NoteVault::new(workspace).await.ok().map(std::sync::Arc::new)
                } else {
                    None
                };
            }
            app.settings = new_settings;
            switch_screen(app, tx, Box::new(StartScreen::new(app.settings.clone()))).await;
        }
        AppMessage::CloseSettings => {
            switch_screen(app, tx, Box::new(StartScreen::new(app.settings.clone()))).await;
        }
        other => {
            if let Some(screen) = app.current_screen.as_mut() {
                screen.handle_app_message(other, tx).await;
            }
        }
    }
    Ok(())
}
