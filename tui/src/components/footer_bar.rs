use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

pub struct FooterBar {
    key_flash: Option<(String, Instant)>,
    settings_key: String,
    quit_key: String,
    toggle_key: String,
}

impl FooterBar {
    pub fn new(settings_key: String, quit_key: String, toggle_key: String) -> Self {
        Self {
            key_flash: None,
            settings_key,
            quit_key,
            toggle_key,
        }
    }

    /// Show a key-flash message for 2 seconds.
    pub fn flash(&mut self, text: String) {
        self.key_flash = Some((text, Instant::now()));
    }

    pub fn render(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        theme: &Theme,
        focus_label: &str,
        hints: &[(String, String)],
        icons: &Icons,
    ) {
        // Expire stale key flash
        if let Some((_, instant)) = &self.key_flash
            && instant.elapsed() >= std::time::Duration::from_secs(2)
        {
            self.key_flash = None;
        }

        let mut footer = Block::default()
            .title(format!(
                "[{focus_label}]  {}: Preferences |  {}: Toggle sidebar | {}: Quit",
                self.settings_key, self.toggle_key, self.quit_key,
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .style(theme.base_style())
            .title_style(Style::default().fg(theme.fg_secondary.to_ratatui()));
        if let Some((flash, _)) = &self.key_flash {
            footer = footer.title_top(Line::from(format!(" {} ", flash)).right_aligned());
        }
        let footer_inner = footer.inner(rect);
        f.render_widget(footer, rect);

        // Build the hints line with the nvim mode label (empty key) styled
        // distinctly from the regular shortcut hints.
        let secondary = Style::default().fg(theme.fg_secondary.to_ratatui());
        let sep = Span::styled("  │  ", secondary);
        let mut spans = vec![Span::styled(format!(" {} ", icons.info), secondary)];
        for (i, (key, label)) in hints.iter().enumerate() {
            if i > 0 {
                spans.push(sep.clone());
            }
            if key.is_empty() {
                // Mode / command-line label from the nvim backend — make it pop.
                spans.push(Span::styled(
                    format!(" {label} "),
                    Style::default()
                        .fg(theme.accent.to_ratatui())
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(format!("{key}: {label}"), secondary));
            }
        }
        f.render_widget(Paragraph::new(Line::from(spans)), footer_inner);
    }
}
