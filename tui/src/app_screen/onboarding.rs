//! The Onboarding screen — Kimün's guided setup. One screen, five steps
//! (workspace → nerd fonts → theme → editor backend → summary), rendered as
//! a centered dialog floating over a blank backdrop so it reads as a setup
//! assistant running *for* the app rather than a screen *of* the app.
//!
//! Choices are staged in a local [`Draft`] and committed only when the user
//! finishes the summary step (`AppEvent::OnboardingFinished`); Esc discards.
//! Theme and nerd-font selections preview live on the dialog itself.

use async_trait::async_trait;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::dir_browser::FileBrowserState;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent};
use crate::components::single_line_input::SingleLineInput;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;
use crate::settings::{AppSettings, EditorBackendSetting, SharedSettings};

// ── Step enum ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnbStep {
    Workspace,
    NerdFonts,
    Theme,
    Backend,
    Summary,
}

impl OnbStep {
    pub(crate) const ORDER: [OnbStep; 5] = [
        OnbStep::Workspace,
        OnbStep::NerdFonts,
        OnbStep::Theme,
        OnbStep::Backend,
        OnbStep::Summary,
    ];

    pub(crate) fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|s| *s == self)
            .unwrap_or(0)
    }

    fn next(self) -> Option<OnbStep> {
        Self::ORDER.get(self.index() + 1).copied()
    }

    fn prev(self) -> Option<OnbStep> {
        self.index().checked_sub(1).map(|i| Self::ORDER[i])
    }
}

// ── Draft ────────────────────────────────────────────────────────────────────

/// Staged choices — applied to shared settings only on Finish.
struct Draft {
    /// `Some((name, path))` only on first run; rerun never mutates workspaces.
    workspace: Option<(String, std::path::PathBuf)>,
    use_nerd_fonts: bool,
    theme_name: String,
    editor_backend: EditorBackendSetting,
}

// ── Overlay ───────────────────────────────────────────────────────────────────

/// Modal sub-states layered over the current step.
// used from Task 5 on
#[allow(dead_code)]
enum OnbOverlay {
    None,
    Browser(FileBrowserState),
    NewDir(FileBrowserState, SingleLineInput),
    ConfirmQuit,
    ConfirmDiscard,
}

// ── BACKENDS constant ─────────────────────────────────────────────────────────

const BACKENDS: [(EditorBackendSetting, &str, &str); 3] = [
    (
        EditorBackendSetting::Textarea,
        "textarea",
        "Simple editing, no modes. The default — pick this if unsure.",
    ),
    (
        EditorBackendSetting::Vim,
        "vim",
        "Built-in vim emulation (modal editing). No external programs needed.",
    ),
    (
        EditorBackendSetting::Nvim,
        "nvim",
        "Embeds your real Neovim: your config, your plugins. Requires nvim installed.",
    ),
];

// ── Screen struct ─────────────────────────────────────────────────────────────

pub struct OnboardingScreen {
    settings: SharedSettings,
    theme: Theme,
    icons: Icons,
    pub(crate) step: OnbStep,
    pub(crate) first_run: bool,
    draft: Draft,
    themes: Vec<Theme>,
    theme_idx: usize,
    backend_idx: usize,
    nvim_available: bool,
    name_input: SingleLineInput,
    name_editing: bool,
    overlay: OnbOverlay,
    flash: Option<String>,
}

// ── Constructor ───────────────────────────────────────────────────────────────

impl OnboardingScreen {
    pub fn new(settings: SharedSettings) -> Self {
        let s = settings.read().unwrap();
        let first_run = s.resolve_workspace_path().is_none();
        let themes = s.theme_list();
        let current_theme_name = if s.theme.is_empty() {
            Theme::default().name.clone()
        } else {
            s.theme.clone()
        };
        let theme_idx = themes
            .iter()
            .position(|t| t.name == current_theme_name)
            .unwrap_or(0);
        let draft = Draft {
            workspace: if first_run {
                AppSettings::default_workspace_suggestion()
                    .map(|p| (suggest_name(&p), p))
            } else {
                None
            },
            use_nerd_fonts: s.use_nerd_fonts,
            theme_name: themes
                .get(theme_idx)
                .map(|t| t.name.clone())
                .unwrap_or_default(),
            editor_backend: s.editor_backend,
        };
        let backend_idx = BACKENDS
            .iter()
            .position(|(b, _, _)| *b == draft.editor_backend)
            .unwrap_or(0);
        let theme = s.get_theme();
        let icons = Icons::new(draft.use_nerd_fonts);
        let nvim_available = nvim_on_path(s.nvim_path.as_deref());
        let name_input = SingleLineInput::with_value(
            draft
                .workspace
                .as_ref()
                .map(|(n, _)| n.clone())
                .unwrap_or_default(),
        );
        drop(s);
        Self {
            settings,
            theme,
            icons,
            step: OnbStep::Workspace,
            first_run,
            draft,
            themes,
            theme_idx,
            backend_idx,
            nvim_available,
            name_input,
            name_editing: false,
            overlay: OnbOverlay::None,
            flash: None,
        }
    }
}

// ── Free functions ────────────────────────────────────────────────────────────

/// Derive a workspace name from a directory: basename, lowercased. Falls back
/// to "notes" when the basename is empty or invalid.
fn suggest_name(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if kimun_core::nfs::filename::validate_filename(&name).is_ok() && !name.is_empty() {
        name
    } else {
        "notes".to_string()
    }
}

/// `nvim` reachable? Explicit configured path wins; otherwise scan PATH.
fn nvim_on_path(configured: Option<&std::path::Path>) -> bool {
    if let Some(p) = configured {
        return p.is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    let exe = if cfg!(windows) { "nvim.exe" } else { "nvim" };
    std::env::split_paths(&paths).any(|d| d.join(exe).is_file())
}

// ── AppScreen impl ────────────────────────────────────────────────────────────

#[async_trait(?Send)]
impl AppScreen for OnboardingScreen {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        if self.handle_overlay_key(key, tx) {
            tx.send(AppEvent::Redraw).ok();
            return EventState::Consumed;
        }
        match key.code {
            // While the name field is in edit mode, Esc must exit the edit
            // (handled by workspace_step_key in Task 5), not cancel the flow.
            KeyCode::Esc if !self.name_editing => self.on_cancel(tx),
            KeyCode::Left | KeyCode::BackTab if !self.name_editing => self.go_prev(),
            KeyCode::Right | KeyCode::Tab if !self.name_editing => self.go_next(),
            _ => self.handle_step_key(key, tx),
        }
        tx.send(AppEvent::Redraw).ok();
        EventState::Consumed
    }

    fn render(&mut self, f: &mut Frame) {
        self.render_dialog(f);
    }

    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Onboarding
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

impl OnboardingScreen {
    fn go_next(&mut self) {
        if let Some(n) = self.step.next() {
            self.step = n;
            self.name_editing = false;
        }
    }

    fn go_prev(&mut self) {
        if let Some(p) = self.step.prev() {
            self.step = p;
            self.name_editing = false;
        }
    }

    fn dirty(&self) -> bool {
        let s = self.settings.read().unwrap();
        s.use_nerd_fonts != self.draft.use_nerd_fonts
            || s.editor_backend != self.draft.editor_backend
            || (!self.draft.theme_name.is_empty() && s.theme != self.draft.theme_name)
    }

    fn on_cancel(&mut self, tx: &AppTx) {
        if self.first_run {
            self.overlay = OnbOverlay::ConfirmQuit;
        } else if self.dirty() {
            self.overlay = OnbOverlay::ConfirmDiscard;
        } else {
            tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
        }
    }

    // Step content lands in Tasks 5-8; stubs keep the skeleton compiling.
    fn handle_step_key(&mut self, _key: &KeyEvent, _tx: &AppTx) {}
    fn handle_overlay_key(&mut self, _key: &KeyEvent, _tx: &AppTx) -> bool {
        false
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

impl OnboardingScreen {
    fn render_dialog(&mut self, f: &mut Frame) {
        // Backdrop: a flat, empty surface in the preview theme. Nothing of
        // the app shows through — the dialog is the only thing on screen.
        f.render_widget(Block::default().style(self.theme.base_style()), f.area());

        let area = crate::components::centered_rect(62, 75, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(" Kimün Setup ")
            .borders(Borders::ALL)
            .border_style(
                Style::default().fg(self.theme.accent.to_ratatui()),
            )
            .style(self.theme.base_style());
        let inner = block.inner(area);
        f.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // header: step title + progress
                Constraint::Min(0),    // step body
                Constraint::Length(1), // flash line
                Constraint::Length(1), // key hints
            ])
            .split(inner);

        self.render_header(f, rows[0]);
        match self.step {
            OnbStep::Workspace => self.render_workspace_step(f, rows[1]),
            OnbStep::NerdFonts => self.render_nerd_fonts_step(f, rows[1]),
            OnbStep::Theme => self.render_theme_step(f, rows[1]),
            OnbStep::Backend => self.render_backend_step(f, rows[1]),
            OnbStep::Summary => self.render_summary_step(f, rows[1]),
        }
        if let Some(msg) = &self.flash {
            f.render_widget(
                Paragraph::new(format!(" {msg}"))
                    .style(Style::default().fg(self.theme.accent.to_ratatui())),
                rows[2],
            );
        }
        self.render_hints(f, rows[3]);
        let dialog_area = area;
        self.render_overlay(f, dialog_area);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let idx = self.step.index();
        let dots: String = (0..OnbStep::ORDER.len())
            .map(|i| if i == idx { "●" } else { "○" })
            .collect::<Vec<_>>()
            .join(" ");
        let title = match self.step {
            OnbStep::Workspace => "Workspace",
            OnbStep::NerdFonts => "Nerd Fonts",
            OnbStep::Theme => "Theme",
            OnbStep::Backend => "Editor Backend",
            OnbStep::Summary => "Summary",
        };
        f.render_widget(
            Paragraph::new(format!(
                " {title}   {dots}   {} / {}",
                idx + 1,
                OnbStep::ORDER.len()
            ))
            .style(
                Style::default()
                    .fg(self.theme.accent.to_ratatui())
                    .add_modifier(Modifier::BOLD),
            ),
            area,
        );
    }

    fn render_hints(&self, f: &mut Frame, area: Rect) {
        let hints = match self.step {
            OnbStep::Workspace if self.first_run => {
                " Enter: accept  b: browse  e: edit name  ←/→: steps  Esc: cancel"
            }
            OnbStep::Summary => " Enter: finish  ←: back  Esc: cancel",
            _ => " ↑/↓: select  Enter/→: next  ←: back  Esc: cancel",
        };
        f.render_widget(
            Paragraph::new(hints)
                .style(Style::default().fg(self.theme.fg_secondary.to_ratatui())),
            area,
        );
    }

    // Step renderers land in Tasks 5-8.
    fn render_workspace_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_nerd_fonts_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_theme_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_backend_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_summary_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_overlay(&mut self, _f: &mut Frame, _dialog_area: Rect) {}
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use crate::test_support::key_event;
    use ratatui::crossterm::event::KeyCode;
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc::unbounded_channel;

    fn shared_defaults() -> crate::settings::SharedSettings {
        Arc::new(RwLock::new(AppSettings::default()))
    }

    fn shared_with_workspace() -> crate::settings::SharedSettings {
        use crate::settings::workspace_config::WorkspaceConfig;
        let mut s = AppSettings::default();
        let mut wc = WorkspaceConfig::new_empty();
        wc.add_workspace(
            "notes".to_string(),
            std::env::temp_dir().join("kimun_onb_ws"),
        )
        .unwrap();
        s.workspace_config = Some(wc);
        Arc::new(RwLock::new(s))
    }

    #[test]
    fn first_run_detected_from_missing_workspace() {
        let screen = OnboardingScreen::new(shared_defaults());
        assert!(screen.first_run);
        let screen = OnboardingScreen::new(shared_with_workspace());
        assert!(!screen.first_run);
    }

    #[test]
    fn kind_is_onboarding_and_starts_on_workspace_step() {
        let screen = OnboardingScreen::new(shared_defaults());
        assert_eq!(screen.get_kind() as u8, ScreenKind::Onboarding as u8);
        assert_eq!(screen.step, OnbStep::Workspace);
    }

    #[test]
    fn left_right_navigate_steps_within_bounds() {
        let (tx, _rx) = unbounded_channel();
        // Rerun screen: workspace step is informational, so plain Right
        // advances without needing a valid draft.
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        screen.handle_input(&key_event(KeyCode::Right), &tx);
        assert_eq!(screen.step, OnbStep::NerdFonts);
        screen.handle_input(&key_event(KeyCode::Left), &tx);
        assert_eq!(screen.step, OnbStep::Workspace);
        screen.handle_input(&key_event(KeyCode::Left), &tx);
        assert_eq!(screen.step, OnbStep::Workspace);
    }

    #[test]
    fn renders_dialog_with_progress_header() {
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        let backend = ratatui::backend::TestBackend::new(100, 32);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| screen.render(f)).unwrap();
        let flat: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(flat.contains("Kimün Setup"));
        assert!(flat.contains("1 / 5"));
    }
}
