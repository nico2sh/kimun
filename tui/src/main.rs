pub mod app;
pub mod keys;
pub mod settings;
pub mod ui;

use color_eyre::Result;
use crossterm::event::{self, DisableMouseCapture, Event, KeyCode};
use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};
use ratatui::Terminal;
use ratatui::crossterm::event::EnableMouseCapture;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::prelude::{Backend, CrosstermBackend};
use std::io;

use crate::app::App;

fn main() -> Result<()> {
    color_eyre::install()?;
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new()?;
    let res = run_app(&mut terminal, &mut app)?;
    // if let Err(err) = tui::restore() {
    //     eprintln!(
    //         "failed to restore terminal. Run `reset` or restart your terminal to recover: {err}"
    //     );
    // }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<bool>
where
    io::Error: From<B::Error>,
{
    loop {
        terminal.draw(|f| ui::ui(f, app))?;
        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Release {
                // Skip events that are not KeyEventKind::Press
                continue;
            }
            match &app.current_screen {
                app::CurrentScreen::Starting => match key.code {
                    KeyCode::Char('q') => app.current_screen = app::CurrentScreen::Exiting,
                    _ => {}
                },
                app::CurrentScreen::Settings => todo!(),
                app::CurrentScreen::Editor(note_vault) => todo!(),
                app::CurrentScreen::Exiting => match key.code {
                    KeyCode::Char('y') => {
                        return Ok(true);
                    }
                    KeyCode::Char('n') | KeyCode::Char('q') => {
                        return Ok(false);
                    }
                    _ => {}
                },
            }
            log::debug!("{}", key.code)
        }
    }
}
