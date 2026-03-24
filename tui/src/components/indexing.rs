use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::layout::Alignment;
use throbber_widgets_tui::{Throbber, ThrobberState};

use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

pub enum IndexingProgressState {
    Running {
        work: tokio::task::JoinHandle<()>,
        ticker: tokio::task::JoinHandle<()>,
    },
    Done(Duration),
    Failed(String),
}

impl Drop for IndexingProgressState {
    fn drop(&mut self) {
        if let Self::Running { work, ticker } = self {
            work.abort();
            ticker.abort();
        }
    }
}

pub fn spawn_running(work: tokio::task::JoinHandle<()>, tx: &AppTx) -> IndexingProgressState {
    let tx2 = tx.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if tx2.send(AppEvent::Redraw).is_err() {
                break;
            }
        }
    });
    IndexingProgressState::Running { work, ticker }
}

pub fn fixed_centered_rect(width: u16, height: u16, r: Rect) -> Rect {
    let x = r.x + (r.width.saturating_sub(width)) / 2;
    let y = r.y + (r.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(r.width),
        height: height.min(r.height),
    }
}

/// Render a centered indexing progress dialog over the current frame.
///
/// - `running_label`: text shown next to the throbber spinner while running.
///   Both the throbber and the label are centered in the dialog box.
/// - Done/Failed states render centered status text and a `[ OK ]` hint.
pub fn render_indexing_overlay(
    f: &mut Frame,
    state: &IndexingProgressState,
    throbber_state: &mut ThrobberState,
    theme: &Theme,
    running_label: &str,
) {
    let area = fixed_centered_rect(44, 5, f.area());
    f.render_widget(Clear, area);
    let block = Block::default()
        .title("Indexing")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent.to_ratatui()))
        .style(theme.base_style());
    let inner = block.inner(area);
    f.render_widget(block, area);

    match state {
        IndexingProgressState::Running { .. } => {
            throbber_state.calc_next();
            // +2 for the spinner char and the space throbber_widgets_tui inserts before the label
            let content_width = (running_label.chars().count() as u16).saturating_add(2);
            let vert = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1), Constraint::Min(0)])
                .split(inner);
            let horiz = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(0),
                    Constraint::Length(content_width),
                    Constraint::Min(0),
                ])
                .split(vert[1]);
            let throbber = Throbber::default()
                .label(running_label)
                .style(Style::default().fg(theme.fg.to_ratatui()).bg(theme.bg.to_ratatui()));
            f.render_stateful_widget(throbber, horiz[1], throbber_state);
        }
        IndexingProgressState::Done(dur) => {
            f.render_widget(
                Paragraph::new(Text::raw(format!("✓  Done in {}s\n\n[ OK ]", dur.as_secs())))
                    .alignment(Alignment::Center)
                    .style(theme.base_style()),
                inner,
            );
        }
        IndexingProgressState::Failed(msg) => {
            f.render_widget(
                Paragraph::new(Text::raw(format!("✗  {}\n\n[ OK ]", msg)))
                    .alignment(Alignment::Center)
                    .style(theme.base_style()),
                inner,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[tokio::test]
    async fn drop_aborts_running_tasks() {
        let completed = Arc::new(AtomicBool::new(false));
        let completed2 = completed.clone();

        let work = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            completed2.store(true, Ordering::SeqCst);
        });
        let ticker = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });

        let state = IndexingProgressState::Running { work, ticker };
        drop(state);

        // Yield several times: abort() is cooperative, the task needs at least one
        // poll after cancellation is posted before it is marked finished.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        assert!(
            !completed.load(Ordering::SeqCst),
            "work task should be aborted, not completed"
        );
    }
}
