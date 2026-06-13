# Onboarding (Guided Setup) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A guided, dialog-styled onboarding flow that walks new users through the essential config (workspace → nerd fonts → theme → editor backend → summary), shown automatically when no workspace is configured and rerunnable anytime from the command palette as "guided setup".

**Architecture:** One new `ScreenKind::Onboarding` screen (`OnboardingScreen`) hosting an internal **step** state machine. The screen renders as a centered dialog box floating over an empty backdrop — it must feel like something running *for* the app, not part of it. Choices are staged in a local draft and committed atomically on Finish (`AppEvent::OnboardingFinished` → same handling as `PreferencesSaved`); Esc discards. Theme and nerd-font choices live-preview the dialog itself. On rerun (workspace already exists) the workspace step is informational only — it lists all workspaces and never mutates them.

**Tech Stack:** Rust, ratatui, tokio, existing kimun TUI infrastructure (`AppScreen` trait, `AppEvent`/`ScreenEvent` bus, leader tree → command palette, `SharedSettings`).

**Glossary (CONTEXT.md):** *Onboarding*, *Step*, *Workspace*, *Vault* — already added. Steps are not screens.

**Decisions locked during grilling (do not relitigate):**
1. First-run trigger = no workspace configured (NOT settings-file-missing). No `onboarding_completed` flag.
2. Single screen + internal steps. Steps: Workspace → NerdFonts → Theme → Backend → Summary.
3. Draft + commit-at-finish. Live preview for theme/fonts inside the dialog only.
4. Esc on first run = quit-confirm dialog → `AppEvent::Quit`. Esc on rerun with dirty draft = discard-confirm → `ScreenEvent::Start`. Clean rerun Esc → `ScreenEvent::Start` directly.
5. Workspace step first run: quick default `~/kimun-notes` accepted with plain Enter; `b` opens directory browser (starts at home); browser gets a create-directory action (`n`). Name prefilled from dir basename, lowercased, validated with `validate_filename`. Directory `create_dir_all` happens at Finish (allowed in TUI: workspace root is a configuration-level OS path per CLAUDE.md exception).
6. Workspace step on rerun: informational — lists all workspaces, marks current, hint "Manage workspaces in Preferences". No mutation ever.
7. Nerd fonts: side-by-side glyph rows (`Icons::new(true)` vs `Icons::new(false)`), user self-diagnoses. No auto-detection. Default off.
8. Theme: full `theme_list()`, live preview.
9. Backend: textarea (default) / vim / nvim with one-line descriptions. Probe PATH for nvim at screen construction; missing → nvim option disabled with hint. No `nvim_path` input here. No preview.
10. Keys: ↑/↓ select within step, Enter confirm/advance, ←/→ and Tab/BackTab step back/forward (←/→ go to the text input while the name field is in edit mode), Esc cancel. No skip key. Header shows "2 / 5" + step dots.
11. Palette/leader: `LeaderAction::AppOnboarding`, id `app.onboarding`, label "guided setup", key `o` in the `+vault` group.
12. `main.rs` no-vault fallthrough (`OpenPath` with no vault) routes to `OpenOnboarding` instead of `OpenPreferences`.

---

## File structure

| File | Action | Responsibility |
|---|---|---|
| `tui/src/components/dir_browser.rs` | Create | `FileBrowserState` moved out of `preferences.rs`, plus new `create_dir` method. Pure navigation state, no rendering. |
| `tui/src/app_screen/preferences.rs` | Modify | Import `FileBrowserState` from new module; delete local copy. No behavior change. |
| `tui/src/components/mod.rs` | Modify | `pub mod dir_browser;` |
| `tui/src/components/events.rs` | Modify | `ScreenEvent::OpenOnboarding`, `AppEvent::OnboardingFinished`. |
| `tui/src/keys/leader.rs` | Modify | `LeaderAction::AppOnboarding` (+ `id`, `ALL` 44→45, `default_label`, tree leaf `v o`). |
| `tui/src/app_screen/editor.rs` | Modify | Execute arm: `AppOnboarding` → `OpenScreen(OpenOnboarding)`. |
| `tui/src/app_screen/onboarding.rs` | Create | `OnboardingScreen`: step state machine, draft, dialog rendering, confirm overlays, finish commit. |
| `tui/src/app_screen/mod.rs` | Modify | `pub mod onboarding;`, `ScreenKind::Onboarding`. |
| `tui/src/main.rs` | Modify | `switch_screen` arm; no-vault fallthrough → onboarding; `OnboardingFinished` arm. |
| `tui/src/settings/mod.rs` | Modify | `pub fn default_workspace_suggestion()` (home-dir helper is `pub(super)`). |
| `docs/content/getting-started/configuration.md` | Modify | User-facing doc: guided setup section. |

Test runs: app_screen tests live in the **bin** target — `cargo test --bins` from `tui/`, or `cargo test --workspace` from root. `cargo test --lib` will NOT run them.

---

### Task 1: Extract `FileBrowserState` into `tui/src/components/dir_browser.rs`

**Files:**
- Create: `tui/src/components/dir_browser.rs`
- Modify: `tui/src/components/mod.rs` (add module)
- Modify: `tui/src/app_screen/preferences.rs:35-107` (delete struct + impl, import instead)

- [ ] **Step 1: Create the new module by moving the code verbatim**

Cut `FileBrowserState` (struct + entire impl block, currently `preferences.rs:35-107`) into the new file. Result:

```rust
//! Directory-only browser state shared by the Preferences screen and the
//! Onboarding screen. Pure navigation state — each host renders it and
//! routes keys itself.

use std::path::PathBuf;

use ratatui::widgets::ListState;

pub struct FileBrowserState {
    pub current_path: PathBuf,
    pub entries: Vec<PathBuf>,
    pub list_state: ListState,
    pub has_parent: bool,
    last_jump_char: Option<char>,
}

impl FileBrowserState {
    // load / navigate_into / go_up / jump_to_char — moved verbatim,
    // byte-for-byte, from preferences.rs:44-106.
}
```

In `tui/src/components/mod.rs` add `pub mod dir_browser;` next to the other module decls.

In `preferences.rs` replace the deleted block with:

```rust
use crate::components::dir_browser::FileBrowserState;
```

(`pub use` it as well if anything else imported it from `app_screen::preferences` — grep first: `grep -rn "preferences::FileBrowserState" tui/src/`.)

- [ ] **Step 2: Verify nothing broke**

Run: `cd tui && cargo test --bins`
Expected: all existing tests PASS (this is a pure move).

- [ ] **Step 3: Write the failing test for `create_dir`**

In `dir_browser.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_dir_creates_enters_and_lists_in_parent() {
        let tmp = std::env::temp_dir().join(format!("kimun_dirbrowser_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut fb = FileBrowserState::load(tmp.clone());

        let created = fb.create_dir("my-notes").unwrap();
        assert_eq!(created, tmp.join("my-notes"));
        assert!(created.is_dir());
        // Browser entered the new directory.
        assert_eq!(fb.current_path, created);

        // Going back up, the new dir is listed.
        fb.go_up();
        assert!(fb.entries.iter().any(|e| e == &created));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_dir_rejects_empty_and_reports_io_errors() {
        let tmp = std::env::temp_dir().join(format!("kimun_dirbrowser_e_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut fb = FileBrowserState::load(tmp.clone());
        assert!(fb.create_dir("").is_err());
        assert!(fb.create_dir("   ").is_err());
        std::fs::remove_dir_all(&tmp).ok();
    }
}
```

Run: `cd tui && cargo test --bins dir_browser`
Expected: FAIL — `create_dir` not found.

- [ ] **Step 4: Implement `create_dir`**

```rust
    /// Create `name` as a subdirectory of `current_path` and navigate into it.
    /// Returns the created path. The directory is created immediately (the
    /// browser must be able to enter it) — the only place onboarding touches
    /// the filesystem before Finish.
    pub fn create_dir(&mut self, name: &str) -> Result<PathBuf, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("directory name is empty".to_string());
        }
        let target = self.current_path.join(name);
        std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        self.navigate_into(target.clone());
        Ok(target)
    }
```

- [ ] **Step 5: Run tests, then commit**

Run: `cd tui && cargo test --bins`
Expected: PASS.

```bash
git add tui/src/components/dir_browser.rs tui/src/components/mod.rs tui/src/app_screen/preferences.rs
git commit -m "refactor: extract FileBrowserState into components::dir_browser, add create_dir"
```

---

### Task 2: Events and settings helper

**Files:**
- Modify: `tui/src/components/events.rs` (~line 253 `ScreenEvent`, ~line 100 `AppEvent`)
- Modify: `tui/src/settings/mod.rs` (impl AppSettings)

- [ ] **Step 1: Add the two event variants**

In `ScreenEvent` (events.rs:253):

```rust
    /// Open the guided-setup (onboarding) screen.
    OpenOnboarding,
```

In `AppEvent`, right after `PreferencesSaved` (events.rs:100):

```rust
    /// Sent by OnboardingScreen when the user confirms Finish on the summary
    /// step. The shared settings already contain the committed draft; main.rs
    /// rebuilds the vault and navigates to Start (same as PreferencesSaved).
    OnboardingFinished,
```

- [ ] **Step 2: Add `default_workspace_suggestion` to `AppSettings`** (test first)

`get_home_dir` is `pub(super)` in `settings/config_dir.rs` — the screen cannot call it, so settings exposes the suggestion. In `settings/mod.rs` tests module:

```rust
    #[test]
    fn default_workspace_suggestion_is_under_home() {
        let suggestion = AppSettings::default_workspace_suggestion();
        if let Some(p) = suggestion {
            assert!(p.ends_with("kimun-notes"));
            assert!(p.is_absolute());
        }
        // None is acceptable only when the platform has no home dir.
    }
```

Run: `cd tui && cargo test --lib default_workspace_suggestion`
Expected: FAIL — method not found.

Implementation, in `impl AppSettings`:

```rust
    /// Suggested directory for a first workspace (`~/kimun-notes`). `None`
    /// when the home directory cannot be determined.
    pub fn default_workspace_suggestion() -> Option<PathBuf> {
        config_dir::get_home_dir().ok().map(|h| h.join("kimun-notes"))
    }
```

Run: `cd tui && cargo test --lib default_workspace_suggestion`
Expected: PASS.

- [ ] **Step 3: Build + commit**

Run: `cd tui && cargo build`
Expected: compiles (new enum variants are additive; no match is exhaustive over `ScreenEvent` without wildcard — if the compiler flags a non-exhaustive match in main.rs, add the arm as a `todo!()`-free no-op `ScreenEvent::OpenOnboarding => {}` placeholder ONLY if needed for this commit; Task 6 fills it properly).

```bash
git add tui/src/components/events.rs tui/src/settings/mod.rs
git commit -m "feat: OpenOnboarding/OnboardingFinished events, default workspace suggestion"
```

---

### Task 3: Leader action + palette entry

**Files:**
- Modify: `tui/src/keys/leader.rs` (enum ~line 68, `id()` ~line 120, `ALL` ~line 125, `default_label()` ~line 228, tree `+vault` group ~line 356)
- Modify: `tui/src/app_screen/editor.rs:1044` area (execute arm)

- [ ] **Step 1: Write the failing round-trip test**

In `leader.rs` tests (next to `note_save_and_app_quit_round_trip_from_id`, ~line 848):

```rust
    #[test]
    fn app_onboarding_round_trip_from_id() {
        assert_eq!(
            LeaderAction::from_id("app.onboarding"),
            Some(LeaderAction::AppOnboarding)
        );
        assert_eq!(LeaderAction::AppOnboarding.id(), "app.onboarding");
    }
```

Run: `cd tui && cargo test --bins app_onboarding`
Expected: FAIL — variant does not exist.

- [ ] **Step 2: Add the variant everywhere**

Enum (after `AppQuit`):

```rust
    /// Open the guided-setup (onboarding) flow.
    AppOnboarding,
```

`id()`: `LeaderAction::AppOnboarding => "app.onboarding",`

`ALL`: append `LeaderAction::AppOnboarding,` and bump the array length `[LeaderAction; 44]` → `[LeaderAction; 45]`. (A doc-test/guard test compares `ALL` against `from_id` coverage — it will catch a miss.)

`default_label()`: `LeaderAction::AppOnboarding => "guided setup",`

Tree — in the `+vault` group children (`leader.rs:359`), after the `('p', …)` preferences leaf:

```rust
                        ('o', leaf("guided setup", A::AppOnboarding)),
```

- [ ] **Step 3: Wire the editor execute arm**

In `editor.rs`, next to the `VaultPreferences` arm (~line 1044):

```rust
            LeaderAction::AppOnboarding => {
                tx.send(AppEvent::OpenScreen(ScreenEvent::OpenOnboarding))
                    .ok();
            }
```

- [ ] **Step 4: Run tests + commit**

Run: `cd tui && cargo test --bins`
Expected: PASS (including the leader guard tests and command-palette flatten tests — the palette picks the new leaf up automatically from the tree).

```bash
git add tui/src/keys/leader.rs tui/src/app_screen/editor.rs
git commit -m "feat: app.onboarding leader action (guided setup) under +vault, palette-visible"
```

---

### Task 4: `OnboardingScreen` skeleton — steps, dialog frame, navigation

**Files:**
- Create: `tui/src/app_screen/onboarding.rs`
- Modify: `tui/src/app_screen/mod.rs` (`pub mod onboarding;`, `ScreenKind::Onboarding`)

- [ ] **Step 1: Module + kind plumbing**

`app_screen/mod.rs`: add `pub mod onboarding;` and extend the kind enum:

```rust
pub enum ScreenKind {
    Start,
    Browse,
    Editor,
    Preferences,
    Onboarding,
}
```

- [ ] **Step 2: Write the failing skeleton tests**

In `onboarding.rs` (bottom):

```rust
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
        wc.add_workspace("notes".to_string(), std::env::temp_dir().join("kimun_onb_ws"))
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
        // Left at the first step stays put.
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
```

Run: `cd tui && cargo test --bins onboarding`
Expected: FAIL — module/struct missing.

- [ ] **Step 3: Implement the skeleton**

```rust
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnbStep {
    Workspace,
    NerdFonts,
    Theme,
    Backend,
    Summary,
}

impl OnbStep {
    const ORDER: [OnbStep; 5] = [
        OnbStep::Workspace,
        OnbStep::NerdFonts,
        OnbStep::Theme,
        OnbStep::Backend,
        OnbStep::Summary,
    ];

    fn index(self) -> usize {
        Self::ORDER.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Option<OnbStep> {
        Self::ORDER.get(self.index() + 1).copied()
    }

    fn prev(self) -> Option<OnbStep> {
        self.index().checked_sub(1).map(|i| Self::ORDER[i])
    }
}

/// Staged choices — applied to shared settings only on Finish.
struct Draft {
    /// `Some((name, path))` only on first run; rerun never mutates workspaces.
    workspace: Option<(String, std::path::PathBuf)>,
    use_nerd_fonts: bool,
    theme_name: String,
    editor_backend: EditorBackendSetting,
}

/// Modal sub-states layered over the current step.
enum OnbOverlay {
    None,
    /// Directory browser for the workspace step (`b`).
    Browser(FileBrowserState),
    /// "New directory" name prompt inside the browser (`n`).
    NewDir(FileBrowserState, SingleLineInput),
    /// First-run Esc: no workspace yet, confirm quitting the app.
    ConfirmQuit,
    /// Rerun Esc with a dirty draft: confirm discarding changes.
    ConfirmDiscard,
}

pub struct OnboardingScreen {
    settings: SharedSettings,
    /// Preview theme — follows the draft selection live.
    theme: Theme,
    /// Preview icons — follow the draft nerd-fonts toggle live.
    icons: Icons,
    pub(crate) step: OnbStep,
    pub(crate) first_run: bool,
    draft: Draft,
    themes: Vec<Theme>,
    theme_idx: usize,
    backend_idx: usize, // 0 textarea, 1 vim, 2 nvim
    nvim_available: bool,
    /// Workspace-step name field; in edit mode arrow keys go to the input.
    name_input: SingleLineInput,
    name_editing: bool,
    overlay: OnbOverlay,
    flash: Option<String>,
}

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
                AppSettings::default_workspace_suggestion().map(|p| {
                    let name = suggest_name(&p);
                    (name, p)
                })
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

/// Derive a workspace name from a directory: basename, lowercased. Falls back
/// to "notes" when the basename is empty or invalid on some filesystem.
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
/// No auto-detection trickery beyond executable existence.
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
```

`AppScreen` impl (navigation core for this task — step handlers land in Tasks 5–8; until then unfinished steps just pass through):

```rust
#[async_trait(?Send)]
impl AppScreen for OnboardingScreen {
    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Onboarding
    }

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
            // (handled by workspace_step_key), not cancel the whole flow.
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
}

impl OnboardingScreen {
    fn go_next(&mut self) {
        if let Some(next) = self.step.next() {
            self.step = next;
            self.name_editing = false;
        }
    }

    fn go_prev(&mut self) {
        if let Some(prev) = self.step.prev() {
            self.step = prev;
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

    // Filled in by Tasks 5-8; default passthrough so the skeleton compiles.
    fn handle_step_key(&mut self, _key: &KeyEvent, _tx: &AppTx) {}
    fn handle_overlay_key(&mut self, _key: &KeyEvent, _tx: &AppTx) -> bool {
        false
    }
}
```

Dialog rendering (the "running *for* the app" look — blank backdrop, centered floating box):

```rust
impl OnboardingScreen {
    fn render_dialog(&mut self, f: &mut Frame) {
        // Backdrop: a flat, empty surface in the preview theme. Nothing of
        // the app shows through — the dialog is the only thing on screen.
        f.render_widget(
            Block::default().style(self.theme.base_style()),
            f.area(),
        );

        let area = crate::components::centered_rect(62, 75, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title(" Kimün Setup ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.accent.to_ratatui()))
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
        self.render_overlay(f, area);
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
            Paragraph::new(hints).style(
                Style::default().fg(self.theme.fg_secondary.to_ratatui()),
            ),
            area,
        );
    }

    // Filled in by Tasks 5-8.
    fn render_workspace_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_nerd_fonts_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_theme_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_backend_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_summary_step(&mut self, _f: &mut Frame, _area: Rect) {}
    fn render_overlay(&mut self, _f: &mut Frame, _dialog_area: Rect) {}
}
```

Note on theme field types: copy the exact accessor style used in `preferences.rs:907` (`theme.accent.to_ratatui()`, `theme.base_style()`, `theme.fg_secondary.to_ratatui()`) — if a name differs, follow `preferences.rs`, not this plan.

- [ ] **Step 4: Run tests + commit**

Run: `cd tui && cargo test --bins onboarding`
Expected: the four skeleton tests PASS.

```bash
git add tui/src/app_screen/onboarding.rs tui/src/app_screen/mod.rs
git commit -m "feat: OnboardingScreen skeleton — dialog frame, step state machine, navigation"
```

---

### Task 5: Workspace step (first run + rerun)

**Files:**
- Modify: `tui/src/app_screen/onboarding.rs`

- [ ] **Step 1: Write the failing tests**

```rust
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
        screen.draft.workspace = None; // simulate: no home dir, nothing chosen
        screen.handle_input(&key_event(KeyCode::Right), &tx);
        assert_eq!(screen.step, OnbStep::Workspace, "cannot advance without a workspace");
        assert!(screen.flash.is_some());
    }

    #[test]
    fn rerun_workspace_step_is_informational_and_lists_workspaces() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        assert!(screen.draft.workspace.is_none());
        // Enter passes through to the next step; nothing is editable.
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
        // Append "X" — committed names are lowercased.
        screen.handle_input(&key_event(KeyCode::Char('X')), &tx);
        screen.handle_input(&key_event(KeyCode::Enter), &tx); // commit name edit
        assert!(!screen.name_editing);
        let (name, _) = screen.draft.workspace.clone().unwrap();
        assert_eq!(name, "kimun-notesx");
    }

    #[test]
    fn browser_confirm_updates_draft_and_suggested_name() {
        let tmp = std::env::temp_dir().join(format!("kimun_onb_browse_{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("My-Vault")).unwrap();
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_defaults());
        screen.overlay = OnbOverlay::Browser(FileBrowserState::load(tmp.join("My-Vault")));
        // 'c' confirms the browser's current directory.
        screen.handle_input(&key_event(KeyCode::Char('c')), &tx);
        let (name, path) = screen.draft.workspace.clone().unwrap();
        assert_eq!(path, tmp.join("My-Vault"));
        assert_eq!(name, "my-vault");
        assert!(matches!(screen.overlay, OnbOverlay::None));
        std::fs::remove_dir_all(&tmp).ok();
    }
```

Run: `cd tui && cargo test --bins onboarding`
Expected: new tests FAIL (handlers are still no-ops).

- [ ] **Step 2: Implement step input handling**

Replace the `handle_step_key` stub. Workspace-step portion:

```rust
    fn handle_step_key(&mut self, key: &KeyEvent, tx: &AppTx) {
        match self.step {
            OnbStep::Workspace => self.workspace_step_key(key),
            OnbStep::NerdFonts => self.nerd_fonts_step_key(key),   // Task 6
            OnbStep::Theme => self.theme_step_key(key),            // Task 7
            OnbStep::Backend => self.backend_step_key(key),        // Task 7
            OnbStep::Summary => self.summary_step_key(key, tx),    // Task 8
        }
    }

    fn workspace_step_key(&mut self, key: &KeyEvent) {
        if !self.first_run {
            // Informational: Enter just advances.
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
                    self.name_input.handle_key(key);
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
```

Note: `Enter` reaches `workspace_step_key` because `handle_input` only intercepts Esc/arrows/Tab — it falls through to `handle_step_key`. The `first_run_right_blocked_without_workspace_draft` test needs one adjustment in `handle_input`: route `Right`/`Tab` through a guard:

```rust
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
```

- [ ] **Step 3: Implement the browser/new-dir/confirm overlay input**

Replace the `handle_overlay_key` stub (returns `true` when an overlay consumed the key). Browser keys mirror `preferences.rs:348-392` exactly (↑/↓ move, ←/Enter-on-`../` up, →/Enter enter, `c`/Ctrl+Enter confirm, Esc close, a–z jump), plus `n` for new-directory:

```rust
    fn handle_overlay_key(&mut self, key: &KeyEvent, tx: &AppTx) -> bool {
        use ratatui::crossterm::event::KeyModifiers;
        match std::mem::replace(&mut self.overlay, OnbOverlay::None) {
            OnbOverlay::None => false,
            OnbOverlay::Browser(mut fb) => {
                let offset = if fb.has_parent { 1 } else { 0 };
                let total = fb.entries.len() + offset;
                match key.code {
                    KeyCode::Esc => {} // overlay already cleared
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
                        input.handle_key(key);
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
                    KeyCode::Esc => {} // back to the wizard
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
        // overlay already cleared by the take() in handle_overlay_key
    }
```

- [ ] **Step 4: Implement workspace-step + overlay rendering**

```rust
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
            Paragraph::new(desc)
                .style(self.theme.base_style())
                .wrap(Wrap { trim: true }),
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
                    Paragraph::new(format!("  Name:       {name}"))
                        .style(self.theme.base_style()),
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
            f.render_widget(
                List::new(items).style(self.theme.base_style()),
                rows[1],
            );
        }
    }
```

Overlay rendering (browser dialog mirrors `preferences.rs:901-952`, sized inside the screen area; confirm boxes mirror `preferences.rs:955-1012`):

```rust
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
                // NewDir prompt floats over the browser.
                if let OnbOverlay::NewDir(_, input) = &mut self.overlay {
                    let prompt = crate::components::fixed_centered_rect(40, 3, f.area());
                    f.render_widget(Clear, prompt);
                    let pblock = Block::default()
                        .title(" New Directory ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(self.theme.accent.to_ratatui()))
                        .style(self.theme.base_style());
                    let pinner = pblock.inner(prompt);
                    f.render_widget(pblock, prompt);
                    input.render(f, pinner, self.theme.base_style(), 0, true);
                }
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
    }
```

With the free helper at module level:

```rust
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
```

Borrow-checker note for `render_overlay`: matching `&mut self.overlay` while also calling `self.theme` accessors inside is fine (disjoint fields), but the nested `if let OnbOverlay::NewDir(_, input) = &mut self.overlay` inside the outer match arm is NOT — restructure into two sequential matches if the compiler objects (first render browser from a borrowed `fb`, then a second `if let` for the prompt after the first borrow ends).

- [ ] **Step 5: Run tests + commit**

Run: `cd tui && cargo test --bins onboarding`
Expected: all Task 5 tests PASS.

```bash
git add tui/src/app_screen/onboarding.rs
git commit -m "feat: onboarding workspace step — suggestion, browser with mkdir, rerun list"
```

---

### Task 6: Nerd-fonts step

**Files:**
- Modify: `tui/src/app_screen/onboarding.rs`

- [ ] **Step 1: Failing tests**

```rust
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
        assert!(flat.contains("󰈙"), "nerd glyph row present");
        assert!(flat.contains("[-]"), "ascii fallback row present");
    }
```

Run: `cd tui && cargo test --bins onboarding`
Expected: FAIL.

- [ ] **Step 2: Implement**

```rust
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
             top sample row below shows icons (not boxes or question marks),\n\
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
```

- [ ] **Step 3: Run tests + commit**

Run: `cd tui && cargo test --bins onboarding`
Expected: PASS.

```bash
git add tui/src/app_screen/onboarding.rs
git commit -m "feat: onboarding nerd-fonts step with glyph self-test and live icon preview"
```

---

### Task 7: Theme step + backend step

**Files:**
- Modify: `tui/src/app_screen/onboarding.rs`

- [ ] **Step 1: Failing tests**

```rust
    #[test]
    fn theme_selection_updates_draft_and_live_preview() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        screen.step = OnbStep::Theme;
        assert!(screen.themes.len() >= 2, "need at least two builtin themes");
        let before = screen.draft.theme_name.clone();
        screen.handle_input(&key_event(KeyCode::Down), &tx);
        assert_ne!(screen.draft.theme_name, before);
        assert_eq!(screen.theme.name, screen.draft.theme_name, "dialog restyles live");
    }

    #[test]
    fn backend_selection_skips_unavailable_nvim() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        screen.step = OnbStep::Backend;
        screen.nvim_available = false;
        screen.backend_idx = 1; // vim
        screen.handle_input(&key_event(KeyCode::Down), &tx);
        assert_eq!(
            screen.draft.editor_backend,
            EditorBackendSetting::Vim,
            "selection must not land on disabled nvim"
        );
        screen.nvim_available = true;
        screen.handle_input(&key_event(KeyCode::Down), &tx);
        assert_eq!(screen.draft.editor_backend, EditorBackendSetting::Nvim);
    }
```

Run: `cd tui && cargo test --bins onboarding`
Expected: FAIL.

- [ ] **Step 2: Implement**

```rust
    fn theme_step_key(&mut self, key: &KeyEvent) {
        match key.code {
            KeyCode::Up if self.theme_idx > 0 => {
                self.theme_idx -= 1;
                self.apply_theme_preview();
            }
            KeyCode::Down if self.theme_idx + 1 < self.themes.len() => {
                self.theme_idx += 1;
                self.apply_theme_preview();
            }
            KeyCode::Enter => self.go_next(),
            _ => {}
        }
    }

    fn apply_theme_preview(&mut self) {
        if let Some(t) = self.themes.get(self.theme_idx) {
            self.draft.theme_name = t.name.clone();
            self.theme = t.clone().adapt_to_terminal();
        }
    }

    fn render_theme_step(&mut self, f: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);
        f.render_widget(
            Paragraph::new(
                "The color theme for the whole app. The dialog previews your\n\
                 selection live. Custom themes: ~/.config/kimun/themes/*.toml",
            )
            .style(self.theme.base_style())
            .wrap(Wrap { trim: true }),
            rows[0],
        );
        let items: Vec<ListItem> = self
            .themes
            .iter()
            .map(|t| ListItem::new(format!("  {}", t.name)))
            .collect();
        let mut state = ratatui::widgets::ListState::default();
        state.select(Some(self.theme_idx));
        let list = List::new(items)
            .style(self.theme.base_style())
            .highlight_symbol("▶ ")
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));
        f.render_stateful_widget(list, rows[1], &mut state);
    }

    fn backend_step_key(&mut self, key: &KeyEvent) {
        match key.code {
            KeyCode::Up => self.move_backend(-1),
            KeyCode::Down => self.move_backend(1),
            KeyCode::Enter => self.go_next(),
            _ => {}
        }
    }

    fn move_backend(&mut self, delta: isize) {
        let len = BACKENDS.len() as isize;
        let mut idx = self.backend_idx as isize;
        loop {
            idx += delta;
            if idx < 0 || idx >= len {
                return; // stay where we are at the edges
            }
            let (backend, _, _) = BACKENDS[idx as usize];
            if backend == EditorBackendSetting::Nvim && !self.nvim_available {
                continue; // hop over the disabled entry
            }
            self.backend_idx = idx as usize;
            self.draft.editor_backend = backend;
            return;
        }
    }

    fn render_backend_step(&mut self, f: &mut Frame, area: Rect) {
        let mut lines = vec![
            "Which engine drives the note editor. One config axis, three".to_string(),
            "values — changeable anytime in Preferences.".to_string(),
            String::new(),
        ];
        for (i, (backend, name, desc)) in BACKENDS.iter().enumerate() {
            let mark = if i == self.backend_idx { "▶" } else { " " };
            let disabled =
                *backend == EditorBackendSetting::Nvim && !self.nvim_available;
            if disabled {
                lines.push(format!(
                    "{mark} {name}  (nvim not found — install it or set its path in Preferences)"
                ));
            } else {
                lines.push(format!("{mark} {name}  —  {desc}"));
            }
        }
        f.render_widget(
            Paragraph::new(lines.join("\n"))
                .style(self.theme.base_style())
                .wrap(Wrap { trim: false }),
            area,
        );
    }
```

- [ ] **Step 3: Run tests + commit**

Run: `cd tui && cargo test --bins onboarding`
Expected: PASS.

```bash
git add tui/src/app_screen/onboarding.rs
git commit -m "feat: onboarding theme step (live preview) and editor-backend step (nvim probe)"
```

---

### Task 8: Summary step + Finish commit + cancel flows

**Files:**
- Modify: `tui/src/app_screen/onboarding.rs`

- [ ] **Step 1: Failing tests**

```rust
    #[tokio::test]
    async fn finish_commits_draft_creates_dir_and_emits_finished() {
        let tmp = std::env::temp_dir().join(format!("kimun_onb_fin_{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        let (tx, mut rx) = unbounded_channel();
        let settings = shared_defaults();
        let mut screen = OnboardingScreen::new(settings.clone());
        screen.draft.workspace = Some(("myws".to_string(), tmp.clone()));
        screen.draft.use_nerd_fonts = true;
        screen.draft.editor_backend = EditorBackendSetting::Vim;
        screen.step = OnbStep::Summary;

        screen.handle_input(&key_event(KeyCode::Enter), &tx);

        assert!(tmp.is_dir(), "workspace directory created at finish");
        let s = settings.read().unwrap();
        assert!(s.use_nerd_fonts);
        assert_eq!(s.editor_backend, EditorBackendSetting::Vim);
        assert_eq!(s.current_workspace_name().as_deref(), Some("myws"));
        assert_eq!(s.theme, screen.draft.theme_name);
        drop(s);
        let mut got_finished = false;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, AppEvent::OnboardingFinished) {
                got_finished = true;
            }
        }
        assert!(got_finished);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[tokio::test]
    async fn rerun_finish_never_touches_workspaces() {
        let (tx, _rx) = unbounded_channel();
        let settings = shared_with_workspace();
        let names_before: Vec<String> = settings
            .read()
            .unwrap()
            .workspace_config
            .as_ref()
            .unwrap()
            .workspaces
            .keys()
            .cloned()
            .collect();
        let mut screen = OnboardingScreen::new(settings.clone());
        screen.draft.use_nerd_fonts = true;
        screen.step = OnbStep::Summary;
        screen.handle_input(&key_event(KeyCode::Enter), &tx);
        let names_after: Vec<String> = settings
            .read()
            .unwrap()
            .workspace_config
            .as_ref()
            .unwrap()
            .workspaces
            .keys()
            .cloned()
            .collect();
        assert_eq!(names_before, names_after);
    }

    #[test]
    fn esc_first_run_opens_quit_confirm_then_quits() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_defaults());
        screen.handle_input(&key_event(KeyCode::Esc), &tx);
        assert!(matches!(screen.overlay, OnbOverlay::ConfirmQuit));
        screen.handle_input(&key_event(KeyCode::Enter), &tx);
        let mut got_quit = false;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, AppEvent::Quit) {
                got_quit = true;
            }
        }
        assert!(got_quit);
    }

    #[test]
    fn esc_rerun_clean_goes_straight_to_start() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = OnboardingScreen::new(shared_with_workspace());
        screen.handle_input(&key_event(KeyCode::Esc), &tx);
        let mut got_start = false;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, AppEvent::OpenScreen(ScreenEvent::Start)) {
                got_start = true;
            }
        }
        assert!(got_start, "clean rerun Esc leaves without confirmation");
    }

    #[test]
    fn esc_rerun_dirty_asks_discard_and_settings_stay_untouched() {
        let (tx, mut rx) = unbounded_channel();
        let settings = shared_with_workspace();
        let mut screen = OnboardingScreen::new(settings.clone());
        screen.set_nerd_fonts(true); // dirty the draft
        screen.handle_input(&key_event(KeyCode::Esc), &tx);
        assert!(matches!(screen.overlay, OnbOverlay::ConfirmDiscard));
        screen.handle_input(&key_event(KeyCode::Enter), &tx);
        assert!(!settings.read().unwrap().use_nerd_fonts, "draft discarded");
        let mut got_start = false;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, AppEvent::OpenScreen(ScreenEvent::Start)) {
                got_start = true;
            }
        }
        assert!(got_start);
    }
```

Run: `cd tui && cargo test --bins onboarding`
Expected: FAIL (`summary_step_key` is a stub; Esc flows partially in place from Task 4/5 — quit/discard confirms already work; the finish tests fail).

Caution for `finish_commits_draft…`: `save_to_disk` writes the real config file. The test must not clobber the developer's config — set a scratch config path on the settings first:

```rust
        settings.write().unwrap().config_file =
            Some(std::env::temp_dir().join(format!("kimun_onb_cfg_{}.toml", std::process::id())));
```

(`config_file: Option<PathBuf>` is the runtime-only override, `settings/mod.rs:172`; `save_to_disk` honors it via `get_config_file_path`.) Add this line to BOTH finish tests right after creating the settings.

- [ ] **Step 2: Implement summary + finish**

```rust
    fn summary_step_key(&mut self, key: &KeyEvent, tx: &AppTx) {
        if key.code == KeyCode::Enter {
            self.finish(tx);
        }
    }

    /// Commit the draft: create + register the workspace (first run only),
    /// apply fonts/theme/backend, persist, and hand off to main.rs.
    fn finish(&mut self, tx: &AppTx) {
        let mut s = self.settings.write().unwrap();
        if self.first_run {
            let Some((name, path)) = self.draft.workspace.clone() else {
                drop(s);
                self.flash = Some("no workspace configured".to_string());
                self.step = OnbStep::Workspace;
                return;
            };
            if let Err(e) = std::fs::create_dir_all(&path) {
                drop(s);
                self.flash = Some(format!("cannot create {}: {e}", path.display()));
                self.step = OnbStep::Workspace;
                return;
            }
            let wc = s
                .workspace_config
                .get_or_insert_with(crate::settings::workspace_config::WorkspaceConfig::new_empty);
            if let Err(e) = wc.add_workspace(name, path) {
                drop(s);
                self.flash = Some(e.to_string());
                self.step = OnbStep::Workspace;
                return;
            }
            s.config_version = crate::settings::config_migration::CURRENT_CONFIG_VERSION;
        }
        s.use_nerd_fonts = self.draft.use_nerd_fonts;
        s.editor_backend = self.draft.editor_backend;
        s.set_theme(self.draft.theme_name.clone());
        if let Err(e) = s.save_to_disk() {
            tracing::error!("failed to save settings after onboarding: {e}");
        }
        drop(s);
        tx.send(AppEvent::OnboardingFinished).ok();
    }

    fn render_summary_step(&mut self, f: &mut Frame, area: Rect) {
        let s = self.settings.read().unwrap();
        let workspace_line = match (&self.draft.workspace, self.first_run) {
            (Some((name, path)), _) => format!("{name}  —  {}", path.display()),
            (None, false) => {
                let n = s.current_workspace_name().unwrap_or_default();
                format!("{n}  (unchanged)")
            }
            (None, true) => "NOT CONFIGURED — go back to step 1".to_string(),
        };
        drop(s);
        let (_, backend_name, _) = BACKENDS[self.backend_idx];
        let text = format!(
            "Review your choices. Enter applies them all at once;\n\
             everything stays adjustable in Preferences.\n\n\
             Workspace:       {workspace_line}\n\
             Nerd fonts:      {}\n\
             Theme:           {}\n\
             Editor backend:  {backend_name}\n\n\
             [ Press Enter to finish ]",
            if self.draft.use_nerd_fonts { "on" } else { "off" },
            self.draft.theme_name,
        );
        f.render_widget(
            Paragraph::new(text)
                .style(self.theme.base_style())
                .wrap(Wrap { trim: false }),
            area,
        );
    }
```

Check visibility: `config_migration::CURRENT_CONFIG_VERSION` is `pub` (config_migration.rs:13); `WorkspaceConfig::new_empty` is used by `preferences.rs:297` so it is reachable — mirror that import path.

- [ ] **Step 3: Run tests + commit**

Run: `cd tui && cargo test --bins onboarding`
Expected: PASS.

```bash
git add tui/src/app_screen/onboarding.rs
git commit -m "feat: onboarding summary step, atomic finish commit, quit/discard flows"
```

---

### Task 9: Wire into main.rs — screen switch, first-run routing, finish handling

**Files:**
- Modify: `tui/src/main.rs` (`switch_screen` ~line 270, `OpenPath` fallthrough ~line 460, `PreferencesSaved` arm ~line 478)

- [ ] **Step 1: Add the switch arm**

In `switch_screen` (main.rs:270), after the preferences arms:

```rust
        ScreenEvent::OpenOnboarding => Box::new(
            crate::app_screen::onboarding::OnboardingScreen::new(app.settings.clone()),
        ),
```

(Add `use crate::app_screen::onboarding::OnboardingScreen;` to the imports at main.rs:51 and use the short name, matching the existing style.)

- [ ] **Step 2: Reroute the no-vault fallthrough**

main.rs:459-461 currently reads:

```rust
                } else {
                    tx.send(AppEvent::OpenScreen(ScreenEvent::OpenPreferences))
                        .ok();
                }
```

Change to:

```rust
                } else {
                    // No vault → the app is unconfigured. Route to the guided
                    // setup, not Preferences (decision: onboarding replaces the
                    // preferences fallthrough as the no-workspace path).
                    tx.send(AppEvent::OpenScreen(ScreenEvent::OpenOnboarding))
                        .ok();
                }
```

- [ ] **Step 3: Handle the finish event**

Extend the existing arm (main.rs:478):

```rust
        AppEvent::PreferencesSaved | AppEvent::OnboardingFinished => {
            app.vault = rebuild_vault(&app.settings).await;
            tx.send(AppEvent::OpenScreen(ScreenEvent::Start)).ok();
        }
```

- [ ] **Step 4: Full test run + clippy**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets`
Expected: PASS, no new warnings.

- [ ] **Step 5: Manual smoke test (first run)**

Debug builds use the `kimun_debug` config dir (`settings/mod.rs:56`). Back it up, wipe it, run:

```bash
mv ~/.config/kimun_debug ~/.config/kimun_debug.bak 2>/dev/null
cd tui && cargo run
```

Expected walk-through: setup dialog appears centered over a blank backdrop → workspace step suggests `~/kimun-notes` → Enter through nerd fonts (toggle, watch icons flip) → theme (watch dialog restyle) → backend (nvim greyed if not installed) → summary → Enter → indexing overlay → Browse screen. Then quit, rerun `cargo run` — app must boot straight to the vault, no wizard. Open palette (Ctrl+P), type "guided" → entry present, opens wizard with workspace step listing the workspace. Esc exits cleanly. Restore config:

```bash
rm -rf ~/.config/kimun_debug && mv ~/.config/kimun_debug.bak ~/.config/kimun_debug 2>/dev/null
```

- [ ] **Step 6: Commit**

```bash
git add tui/src/main.rs
git commit -m "feat: route first run (no workspace) to onboarding; handle OnboardingFinished"
```

---

### Task 10: User-facing docs

**Files:**
- Modify: `docs/content/getting-started/configuration.md`

- [ ] **Step 1: Document the guided setup**

Read the page first and match its tone/format. Add a short section near the top:

```markdown
## Guided setup

On the very first launch — before any workspace exists — Kimün opens a guided
setup dialog that walks you through the essentials, one step at a time:

1. **Workspace** — the directory where your notes live (suggested:
   `~/kimun-notes`; press `b` to browse, `n` inside the browser to create a
   new directory).
2. **Nerd fonts** — two sample rows tell you instantly whether your terminal
   font has the icons.
3. **Theme** — the dialog previews each theme live as you move through the list.
4. **Editor backend** — textarea (default), built-in vim emulation, or your
   own embedded Neovim.

Nothing is applied until you confirm the summary step; `Esc` discards.
You can rerun the guided setup anytime from the command palette
(`Ctrl+P` → “guided setup”) or with the leader sequence `v o`.
```

- [ ] **Step 2: Commit**

```bash
git add docs/content/getting-started/configuration.md
git commit -m "docs: guided setup walkthrough in getting-started/configuration"
```

---

## Self-review checklist (run after writing code, before claiming done)

- [ ] Every grilling decision (1–12 in the header) maps to implemented behavior.
- [ ] `cargo test --workspace` green; `cargo clippy --workspace --all-targets` clean.
- [ ] No `.md`/path-separator hardcoding crept in; the only filesystem writes outside `nfs` are workspace-dir creation (config-level OS path — allowed) and settings/save (pre-existing pattern).
- [ ] Manual smoke test (Task 9 Step 5) performed on a real terminal, both first-run and rerun paths.
