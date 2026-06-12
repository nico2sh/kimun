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
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

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
            KeyCode::Right | KeyCode::Tab if !self.name_editing => {
                if self.step == OnbStep::Workspace
                    && self.first_run
                    && self.draft.workspace.is_none()
                {
                    self.flash = Some("choose a directory first (b to browse)".to_string());
                } else {
                    self.go_next();
                }
            }
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
            self.flash = None;
        }
    }

    fn go_prev(&mut self) {
        if let Some(p) = self.step.prev() {
            self.step = p;
            self.name_editing = false;
            self.flash = None;
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

    fn handle_step_key(&mut self, key: &KeyEvent, tx: &AppTx) {
        match self.step {
            OnbStep::Workspace => self.workspace_step_key(key),
            OnbStep::NerdFonts => self.nerd_fonts_step_key(key),
            _ => {
                // Steps land in Tasks 7-8; Enter advances as a placeholder.
                if key.code == KeyCode::Enter {
                    self.go_next();
                }
            }
        }
        let _ = tx;
    }

    fn nerd_fonts_step_key(&mut self, key: &KeyEvent) {
        match key.code {
            KeyCode::Up => self.set_nerd_fonts(false),
            KeyCode::Down => self.set_nerd_fonts(true),
            KeyCode::Char(' ') => {
                let next = !self.draft.use_nerd_fonts;
                self.set_nerd_fonts(next);
            }
            KeyCode::Enter => self.go_next(),
            _ => {}
        }
    }

    fn set_nerd_fonts(&mut self, on: bool) {
        self.draft.use_nerd_fonts = on;
        self.icons = Icons::new(on); // live preview
    }

    fn workspace_step_key(&mut self, key: &KeyEvent) {
        if !self.first_run {
            if key.code == KeyCode::Enter {
                self.go_next();
            }
            return;
        }
        if self.name_editing {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    let name = self.name_input.value().trim().to_lowercase();
                    if name.is_empty()
                        || kimun_core::nfs::filename::validate_filename(&name).is_err()
                    {
                        self.flash = Some("invalid workspace name".to_string());
                        return;
                    }
                    if let Some((n, _)) = self.draft.workspace.as_mut() {
                        *n = name;
                    }
                    self.name_editing = false;
                    self.flash = None;
                }
                _ => {
                    let _ = self.name_input.handle_key(key);
                }
            }
            return;
        }
        match key.code {
            KeyCode::Enter => {
                if self.draft.workspace.is_some() {
                    self.go_next();
                } else {
                    self.flash = Some("choose a directory first (b to browse)".to_string());
                }
            }
            KeyCode::Char('b') => {
                let start = self
                    .draft
                    .workspace
                    .as_ref()
                    .and_then(|(_, p)| p.parent().map(|p| p.to_path_buf()))
                    .or_else(|| {
                        AppSettings::default_workspace_suggestion()
                            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                    })
                    .unwrap_or_else(|| std::path::PathBuf::from("/"));
                self.overlay = OnbOverlay::Browser(FileBrowserState::load(start));
            }
            KeyCode::Char('e') => {
                let current = self
                    .draft
                    .workspace
                    .as_ref()
                    .map(|(n, _)| n.clone())
                    .unwrap_or_default();
                self.name_input.set_value(current);
                self.name_editing = true;
            }
            _ => {}
        }
    }

    fn handle_overlay_key(&mut self, key: &KeyEvent, tx: &AppTx) -> bool {
        use ratatui::crossterm::event::KeyModifiers;
        match std::mem::replace(&mut self.overlay, OnbOverlay::None) {
            OnbOverlay::None => false,
            OnbOverlay::Browser(mut fb) => {
                let offset = if fb.has_parent { 1 } else { 0 };
                let total = fb.entries.len() + offset;
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Up if total > 0 => {
                        let cur = fb.list_state.selected().unwrap_or(0);
                        fb.list_state.select(Some((cur + total - 1) % total));
                        self.overlay = OnbOverlay::Browser(fb);
                    }
                    KeyCode::Down if total > 0 => {
                        let cur = fb.list_state.selected().unwrap_or(0);
                        fb.list_state.select(Some((cur + 1) % total));
                        self.overlay = OnbOverlay::Browser(fb);
                    }
                    KeyCode::Left => {
                        fb.go_up();
                        self.overlay = OnbOverlay::Browser(fb);
                    }
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.confirm_directory(fb.current_path.clone());
                    }
                    KeyCode::Right | KeyCode::Enter => {
                        if let Some(idx) = fb.list_state.selected() {
                            if fb.has_parent && idx == 0 {
                                fb.go_up();
                            } else if let Some(entry) = fb.entries.get(idx - offset).cloned() {
                                fb.navigate_into(entry);
                            }
                        }
                        self.overlay = OnbOverlay::Browser(fb);
                    }
                    KeyCode::Char('c') => {
                        self.confirm_directory(fb.current_path.clone());
                    }
                    KeyCode::Char('n') => {
                        self.overlay = OnbOverlay::NewDir(fb, SingleLineInput::new());
                    }
                    KeyCode::Char(c) => {
                        fb.jump_to_char(c);
                        self.overlay = OnbOverlay::Browser(fb);
                    }
                    _ => self.overlay = OnbOverlay::Browser(fb),
                }
                true
            }
            OnbOverlay::NewDir(mut fb, mut input) => {
                match key.code {
                    KeyCode::Esc => self.overlay = OnbOverlay::Browser(fb),
                    KeyCode::Enter => match fb.create_dir(input.value()) {
                        Ok(_) => self.overlay = OnbOverlay::Browser(fb),
                        Err(e) => {
                            self.flash = Some(format!("cannot create directory: {e}"));
                            self.overlay = OnbOverlay::NewDir(fb, input);
                        }
                    },
                    _ => {
                        let _ = input.handle_key(key);
                        self.overlay = OnbOverlay::NewDir(fb, input);
                    }
                }
                true
            }
            OnbOverlay::ConfirmQuit => {
                match key.code {
                    KeyCode::Enter => {
                        tx.send(AppEvent::Quit).ok();
                    }
                    KeyCode::Esc => {}
                    _ => self.overlay = OnbOverlay::ConfirmQuit,
                }
                true
            }
            OnbOverlay::ConfirmDiscard => {
                match key.code {
                    KeyCode::Enter => {
                        tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
                    }
                    KeyCode::Esc => {}
                    _ => self.overlay = OnbOverlay::ConfirmDiscard,
                }
                true
            }
        }
    }

    fn confirm_directory(&mut self, chosen: std::path::PathBuf) {
        let name = suggest_name(&chosen);
        self.draft.workspace = Some((name, chosen));
        self.flash = None;
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

    fn render_workspace_step(&mut self, f: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5), Constraint::Min(0)])
            .split(area);

        let desc = if self.first_run {
            "A workspace is where your notes live: one directory on disk,\n\
             holding plain Markdown files. Kimün indexes it for search and\n\
             links. You can add more workspaces later in Preferences."
        } else {
            "Your workspaces. This step is informational — add, rename or\n\
             remove workspaces in Preferences (palette: \"preferences\")."
        };
        f.render_widget(
            Paragraph::new(desc).style(self.theme.base_style()).wrap(Wrap { trim: true }),
            rows[0],
        );

        if self.first_run {
            let (name, path) = match &self.draft.workspace {
                Some((n, p)) => (n.clone(), p.display().to_string()),
                None => ("—".to_string(), "no directory chosen (press b)".to_string()),
            };
            let body = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1)])
                .split(rows[1]);
            f.render_widget(
                Paragraph::new(format!("  Directory:  {path}")).style(self.theme.base_style()),
                body[0],
            );
            if self.name_editing {
                f.render_widget(
                    Paragraph::new("  Name:       ").style(self.theme.base_style()),
                    body[1],
                );
                self.name_input.render(
                    f,
                    body[1],
                    Style::default()
                        .fg(self.theme.accent.to_ratatui())
                        .add_modifier(Modifier::BOLD),
                    14,
                    true,
                );
            } else {
                f.render_widget(
                    Paragraph::new(format!("  Name:       {name}")).style(self.theme.base_style()),
                    body[1],
                );
            }
        } else {
            let s = self.settings.read().unwrap();
            let current = s.current_workspace_name().unwrap_or_default();
            let mut items: Vec<ListItem> = Vec::new();
            if let Some(wc) = s.workspace_config.as_ref() {
                for (name, entry) in &wc.workspaces {
                    let marker = if *name == current { "●" } else { " " };
                    items.push(ListItem::new(format!(
                        " {marker} {name}  —  {}",
                        entry.effective_path().display()
                    )));
                }
            }
            drop(s);
            f.render_widget(List::new(items).style(self.theme.base_style()), rows[1]);
        }
    }

    // Step renderers land in Tasks 6-8.
    fn render_nerd_fonts_step(&mut self, f: &mut Frame, area: Rect) {
        let nerd = Icons::new(true);
        let ascii = Icons::new(false);
        let sample = |i: &Icons| {
            format!(
                "{}  {}  {}  {}  {}",
                i.directory, i.note, i.journal, i.info, i.rail_find
            )
        };
        let selected = self.draft.use_nerd_fonts;
        let mark = |sel: bool| if sel { "▶" } else { " " };
        let text = format!(
            "Nerd Fonts are patched terminal fonts with extra icons. If the\n\
             bottom sample row below shows icons (not boxes or question marks),\n\
             your terminal supports them.\n\n\
             {} Plain ASCII      {}\n\
             {} Nerd Fonts       {}\n",
            mark(!selected),
            sample(&ascii),
            mark(selected),
            sample(&nerd),
        );
        f.render_widget(
            Paragraph::new(text)
                .style(self.theme.base_style())
                .wrap(Wrap { trim: true }),
            area,
        );
    }
    fn render_theme_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_backend_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_summary_step(&mut self, _f: &mut Frame, _area: Rect) {}

    fn render_overlay(&mut self, f: &mut Frame, _dialog_area: Rect) {
        match &mut self.overlay {
            OnbOverlay::None => {}
            OnbOverlay::Browser(fb) | OnbOverlay::NewDir(fb, _) => {
                let area = crate::components::centered_rect(55, 70, f.area());
                f.render_widget(Clear, area);
                let block = Block::default()
                    .title(" Choose Notes Directory ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.accent.to_ratatui()))
                    .style(self.theme.base_style());
                let inner = block.inner(area);
                f.render_widget(block, area);
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Min(0),
                        Constraint::Length(1),
                    ])
                    .split(inner);
                f.render_widget(
                    Paragraph::new(fb.current_path.to_string_lossy().into_owned())
                        .style(self.theme.base_style()),
                    rows[0],
                );
                let mut items: Vec<ListItem> = Vec::new();
                if fb.has_parent {
                    items.push(ListItem::new("  ../"));
                }
                for e in &fb.entries {
                    items.push(ListItem::new(format!(
                        "  {}/",
                        e.file_name().unwrap_or_default().to_string_lossy()
                    )));
                }
                let list = List::new(items)
                    .highlight_symbol("▶ ")
                    .highlight_style(Style::default().add_modifier(Modifier::BOLD));
                f.render_stateful_widget(list, rows[1], &mut fb.list_state);
                f.render_widget(
                    Paragraph::new("Enter: open  c: choose  n: new dir  Esc: back")
                        .style(self.theme.base_style()),
                    rows[2],
                );
            }
            OnbOverlay::ConfirmQuit => {
                render_confirm_box(
                    f,
                    &self.theme,
                    " Quit Setup? ",
                    "No workspace is configured — Kimün cannot run\nwithout one. Quit anyway?\n\n  Enter: quit    Esc: back to setup",
                );
            }
            OnbOverlay::ConfirmDiscard => {
                render_confirm_box(
                    f,
                    &self.theme,
                    " Discard Changes? ",
                    "Your setup changes have not been applied.\n\n  Enter: discard    Esc: back to setup",
                );
            }
        }
        // NewDir input prompt floats over the browser — second borrow scope.
        if let OnbOverlay::NewDir(_, input) = &mut self.overlay {
            let prompt = crate::components::fixed_centered_rect(40, 3, f.area());
            f.render_widget(Clear, prompt);
            let theme = &self.theme;
            let pblock = Block::default()
                .title(" New Directory ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.accent.to_ratatui()))
                .style(theme.base_style());
            let pinner = pblock.inner(prompt);
            f.render_widget(pblock, prompt);
            input.render(f, pinner, theme.base_style(), 0, true);
        }
    }
}

// ── Free rendering helpers ────────────────────────────────────────────────────

fn render_confirm_box(f: &mut Frame, theme: &Theme, title: &str, body: &str) {
    let area = crate::components::fixed_centered_rect(52, 7, f.area());
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(title.to_string())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent.to_ratatui()))
        .style(theme.base_style());
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(body.to_string())
            .style(theme.base_style())
            .wrap(Wrap { trim: false }),
        inner,
    );
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

    #[test]
    fn first_run_workspace_step_prefills_suggestion() {
        let screen = OnboardingScreen::new(shared_defaults());
        let (name, path) = screen.draft.workspace.clone().expect("suggestion expected");
        assert!(path.ends_with("kimun-notes"));
        assert_eq!(name, "kimun-notes");
    }

    #[test]
    fn first_run_enter_on_valid_workspace_advances() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_defaults());
        screen.handle_input(&key_event(KeyCode::Enter), &tx);
        assert_eq!(screen.step, OnbStep::NerdFonts);
    }

    #[test]
    fn first_run_right_blocked_without_workspace_draft() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_defaults());
        screen.draft.workspace = None;
        screen.handle_input(&key_event(KeyCode::Right), &tx);
        assert_eq!(screen.step, OnbStep::Workspace, "cannot advance without a workspace");
        assert!(screen.flash.is_some());
    }

    #[test]
    fn rerun_workspace_step_is_informational_and_lists_workspaces() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        assert!(screen.draft.workspace.is_none());
        screen.handle_input(&key_event(KeyCode::Enter), &tx);
        assert_eq!(screen.step, OnbStep::NerdFonts);

        let backend = ratatui::backend::TestBackend::new(100, 32);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        screen.step = OnbStep::Workspace;
        terminal.draw(|f| screen.render(f)).unwrap();
        let flat: String = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("notes"), "workspace list should show the entry name");
        assert!(flat.contains("Preferences"), "should point at Preferences for management");
    }

    #[test]
    fn name_edit_mode_validates_and_lowercases() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_defaults());
        screen.handle_input(&key_event(KeyCode::Char('e')), &tx);
        assert!(screen.name_editing);
        screen.handle_input(&key_event(KeyCode::Char('X')), &tx);
        screen.handle_input(&key_event(KeyCode::Enter), &tx);
        assert!(!screen.name_editing);
        let (name, _) = screen.draft.workspace.clone().unwrap();
        assert_eq!(name, "kimun-notesx");
    }

    #[test]
    fn name_edit_rejects_invalid_name_and_stays_editing() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_defaults());
        screen.handle_input(&key_event(KeyCode::Char('e')), &tx);
        assert!(screen.name_editing);
        // "?" is invalid on at least one major filesystem.
        screen.handle_input(&key_event(KeyCode::Char('?')), &tx);
        screen.handle_input(&key_event(KeyCode::Enter), &tx);
        assert!(screen.name_editing, "invalid name must keep edit mode open");
        assert!(screen.flash.is_some(), "invalid name must flash");
        let (name, _) = screen.draft.workspace.clone().unwrap();
        assert_eq!(name, "kimun-notes", "draft name unchanged on invalid input");
    }

    #[test]
    fn nerd_fonts_toggle_updates_draft_and_preview_icons() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        screen.step = OnbStep::NerdFonts;
        assert!(!screen.draft.use_nerd_fonts);
        screen.handle_input(&key_event(KeyCode::Down), &tx); // select "nerd fonts"
        assert!(screen.draft.use_nerd_fonts);
        assert!(!screen.icons.info.is_ascii(), "preview icons follow draft");
        screen.handle_input(&key_event(KeyCode::Up), &tx);
        assert!(!screen.draft.use_nerd_fonts);
        assert!(screen.icons.info.is_ascii());
    }

    #[test]
    fn nerd_fonts_step_renders_both_sample_rows() {
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        screen.step = OnbStep::NerdFonts;
        let backend = ratatui::backend::TestBackend::new(100, 32);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| screen.render(f)).unwrap();
        let flat: String = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        assert!(flat.contains("ASCII"), "ascii row labeled");
        assert!(flat.contains("Nerd Fonts"), "nerd row labeled");
    }

    #[test]
    fn browser_confirm_updates_draft_and_suggested_name() {
        let tmp = std::env::temp_dir().join(format!("kimun_onb_browse_{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("My-Vault")).unwrap();
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_defaults());
        screen.overlay = OnbOverlay::Browser(FileBrowserState::load(tmp.join("My-Vault")));
        screen.handle_input(&key_event(KeyCode::Char('c')), &tx);
        let (name, path) = screen.draft.workspace.clone().unwrap();
        assert_eq!(path, tmp.join("My-Vault"));
        assert_eq!(name, "my-vault");
        assert!(matches!(screen.overlay, OnbOverlay::None));
        std::fs::remove_dir_all(&tmp).ok();
    }
}
