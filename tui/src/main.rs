pub mod app;
pub mod app_screen;
pub mod components;
pub mod keys;
pub mod settings;
pub mod ui;

use std::sync::Arc;

use color_eyre::Result;
use crossterm::event::{DisableMouseCapture, Event, EventStream};
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};
use futures::StreamExt;
use kimun_core::NoteVault;
use ratatui::Terminal;
use ratatui::crossterm::event::EnableMouseCapture;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::prelude::{Backend, CrosstermBackend};
use std::io;

use crate::app::App;
use crate::app_screen::AppScreen;
use crate::app_screen::browse::BrowseScreen;
use crate::app_screen::editor::EditorScreen;
use crate::app_screen::settings::SettingsScreen;
use crate::app_screen::start::StartScreen;
use crate::components::app_message::AppMessage;
use crate::components::events::AppEvent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new()?;
    run_app(&mut terminal, &mut app).await?;
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()>
where
    io::Error: From<B::Error>,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppMessage>();

    // Run the event stream in a dedicated task and forward events via a channel.
    // This lets the main loop drain buffered events safely with `try_recv()`
    // without touching the async stream's waker state.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<Event>(256);
    tokio::spawn(async move {
        let mut stream = EventStream::new();
        while let Some(Ok(event)) = stream.next().await {
            if event_tx.send(event).await.is_err() {
                break;
            }
        }
    });

    // Run on_enter for the initial screen.
    if let Some(screen) = &mut app.current_screen {
        screen.on_enter(&tx).await;
    }

    loop {
        // Drain all pending messages before drawing.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                AppMessage::Quit => {
                    if let Some(screen) = app.current_screen.as_mut() {
                        screen.on_exit(&tx).await;
                    }
                    return Ok(());
                }
                AppMessage::Redraw => {}
                AppMessage::OpenSettings => {
                    let mut screen: Box<dyn AppScreen> = Box::new(SettingsScreen::new(app.settings.clone()));
                    screen.on_enter(&tx).await;
                    app.current_screen = Some(screen);
                }
                AppMessage::OpenEditor(vault, path) => {
                    let mut screen: Box<dyn AppScreen> = Box::new(EditorScreen::new(
                        Arc::new(vault),
                        path,
                        app.settings.clone(),
                    ));
                    screen.on_enter(&tx).await;
                    app.current_screen = Some(screen);
                }
                AppMessage::OpenBrowse(vault, path) => {
                    let mut screen: Box<dyn AppScreen> =
                        Box::new(BrowseScreen::new(Arc::new(vault), path, app.settings.clone()));
                    screen.on_enter(&tx).await;
                    app.current_screen = Some(screen);
                }
                AppMessage::OpenPath(path) => {
                    let unhandled = if let Some(screen) = app.current_screen.as_mut() {
                        screen
                            .handle_app_message(AppMessage::OpenPath(path), &tx)
                            .await
                    } else {
                        Some(AppMessage::OpenPath(path))
                    };
                    if let Some(AppMessage::OpenPath(path)) = unhandled {
                        if let Some(vault_path) = &app.settings.workspace_dir {
                            let vault = NoteVault::new(vault_path).await.map_err(io::Error::other)?;
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
                    app.settings = new_settings;
                    let mut screen: Box<dyn AppScreen> =
                        Box::new(StartScreen::new(app.settings.clone()));
                    screen.on_enter(&tx).await;
                    app.current_screen = Some(screen);
                }
                AppMessage::CloseSettings => {
                    let mut screen: Box<dyn AppScreen> =
                        Box::new(StartScreen::new(app.settings.clone()));
                    screen.on_enter(&tx).await;
                    app.current_screen = Some(screen);
                }
                other => {
                    if let Some(screen) = app.current_screen.as_mut() {
                        screen.handle_app_message(other, &tx).await;
                    }
                }
            }
        }

        terminal.draw(|f| ui::ui(f, app))?;

        // Wait for either a user input event or an app message (e.g. Redraw
        // sent by a background task like the sidebar loader).
        tokio::select! {
            maybe_event = event_rx.recv() => {
                let Some(first) = maybe_event else { continue };
                // Drain all buffered events so rapid input (e.g. a mouse-wheel
                // spin) is batched into a single redraw.
                let mut raw_events = vec![first];
                while let Ok(e) = event_rx.try_recv() {
                    raw_events.push(e);
                }
                for raw in raw_events {
                    let app_event = match raw {
                        Event::Key(key) if key.kind != crossterm::event::KeyEventKind::Release => {
                            AppEvent::Key(key)
                        }
                        Event::Mouse(mouse) => AppEvent::Mouse(mouse),
                        _ => continue,
                    };
                    // Global Ctrl+Q quit — fires before any screen gets the event.
                    if let AppEvent::Key(key) = &app_event {
                        if let Some(combo) = key_event_to_combo(key) {
                            if app.settings.key_bindings.get_action(&combo)
                                == Some(ActionShortcuts::Quit)
                            {
                                tx.send(AppMessage::Quit).ok();
                                continue;
                            }
                        }
                    }
                    if let Some(screen) = &mut app.current_screen {
                        screen.handle_event(&app_event, &tx);
                    }
                }
            }
            Some(msg) = rx.recv() => {
                match msg {
                    AppMessage::Redraw => {} // just loop to redraw
                    other => {
                        // Re-queue for the drain loop at the top of next iteration.
                        tx.send(other).ok();
                    }
                }
            }
        }
    }
}
