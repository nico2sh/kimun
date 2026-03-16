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
use crate::app_screen::editor::EditorScreen;
use crate::app_screen::settings::SettingsScreen;
use crate::components::app_message::AppMessage;
use crate::components::events::AppEvent;

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
    let mut event_stream = EventStream::new();

    // Run on_enter for the initial screen.
    if let Some(screen) = &mut app.current_screen {
        screen.on_enter(&tx).await;
    }

    loop {
        // Drain all pending messages before drawing.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                AppMessage::Quit => return Ok(()),
                AppMessage::Redraw => {}
                AppMessage::OpenSettings => {
                    let mut screen: Box<dyn AppScreen> = Box::new(SettingsScreen::new());
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
                AppMessage::OpenPath(path) => {
                    if let Some(vault_path) = &app.settings.workspace_dir {
                        if let Some(editor) = app
                            .current_screen
                            .as_mut()
                            .and_then(|s| s.as_any_mut().downcast_mut::<EditorScreen>())
                        {
                            if path.is_note() {
                                editor.open_path(path, tx.clone()).await;
                            } else {
                                editor.navigate_sidebar(path, tx.clone()).await;
                            }
                        } else if path.is_note() {
                            let vault = NoteVault::new(&vault_path)
                                .await
                                .map_err(io::Error::other)?;
                            tx.send(AppMessage::OpenEditor(vault, path)).ok();
                        }
                    } else {
                        tx.send(AppMessage::OpenSettings).ok();
                    }
                }
            }
        }

        terminal.draw(|f| ui::ui(f, app))?;

        // Wait for either a user input event or an app message (e.g. Redraw
        // sent by a background task like the sidebar loader).
        tokio::select! {
            maybe_event = event_stream.next() => {
                let app_event = match maybe_event.and_then(|r| r.ok()) {
                    Some(Event::Key(key)) if key.kind != crossterm::event::KeyEventKind::Release => {
                        AppEvent::Key(key)
                    }
                    Some(Event::Mouse(mouse)) => AppEvent::Mouse(mouse),
                    _ => continue,
                };
                if let Some(screen) = &mut app.current_screen {
                    screen.handle_event(app_event, &tx);
                }
            }
            Some(msg) = rx.recv() => {
                match msg {
                    AppMessage::Quit => return Ok(()),
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
