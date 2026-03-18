pub mod app;
pub mod app_screen;
pub mod components;
pub mod keys;
pub mod settings;
pub mod ui;

use color_eyre::Result;
use crossterm::event::{DisableMouseCapture, Event, EventStream};
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};
use futures::StreamExt;
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
    let mut app = App::new().await?;
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

async fn switch_screen(app: &mut App, tx: &crate::components::app_message::AppTx, new_screen: Box<dyn AppScreen>) {
    if let Some(current) = app.current_screen.as_mut() {
        current.on_exit(tx).await;
    }
    let mut screen = new_screen;
    screen.on_enter(tx).await;
    app.current_screen = Some(screen);
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
                    switch_screen(app, &tx, Box::new(SettingsScreen::new(app.settings.clone()))).await;
                }
                AppMessage::OpenEditor(vault, path) => {
                    switch_screen(app, &tx, Box::new(EditorScreen::new(vault, path, app.settings.clone()))).await;
                }
                AppMessage::OpenBrowse(vault, path) => {
                    switch_screen(app, &tx, Box::new(BrowseScreen::new(vault, path, app.settings.clone()))).await;
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
                    // Rebuild vault if the workspace path changed.
                    if new_settings.workspace_dir != app.settings.workspace_dir {
                        app.vault = if let Some(ref workspace) = new_settings.workspace_dir {
                            kimun_core::NoteVault::new(workspace).await.ok().map(std::sync::Arc::new)
                        } else {
                            None
                        };
                    }
                    app.settings = new_settings;
                    switch_screen(app, &tx, Box::new(StartScreen::new(app.settings.clone()))).await;
                }
                AppMessage::CloseSettings => {
                    switch_screen(app, &tx, Box::new(StartScreen::new(app.settings.clone()))).await;
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
