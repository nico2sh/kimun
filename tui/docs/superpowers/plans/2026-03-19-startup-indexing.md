# Startup Indexing Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run `vault.init_and_validate()` once on app startup when a vault is configured, showing an auto-dismissing progress dialog.

**Architecture:** Extract `IndexingProgressState`/`spawn_running`/`fixed_centered_rect` from `SettingsScreen` into a shared `components/indexing.rs`. `StartScreen` gains an optional overlay and a vault reference (passed only on first launch, not on settings re-entries) so it can show a throbber and defer `OpenPath` until indexing completes.

**Tech Stack:** Rust 2021, ratatui, tokio, throbber-widgets-tui, kimun_core (`NoteVault`, `init_and_validate`)

**Spec:** `docs/superpowers/specs/2026-03-19-startup-indexing-design.md`

---

## Chunk 1: Shared indexing module

### Task 1: Create `components/indexing.rs`

**Files:**
- Create: `src/components/indexing.rs`
- Modify: `src/components/mod.rs`

- [ ] **Step 1: Write failing test for `IndexingProgressState::Drop` in the new file**

Create `src/components/indexing.rs` with just the test (no implementation yet):

```rust
use std::time::Duration;
use ratatui::layout::Rect;
use crate::components::events::{AppEvent, AppTx};

pub enum IndexingProgressState {
    Running {
        work: tokio::task::JoinHandle<()>,
        ticker: tokio::task::JoinHandle<()>,
    },
    Done(Duration),
    Failed(String),
}

pub fn spawn_running(_work: tokio::task::JoinHandle<()>, _tx: &AppTx) -> IndexingProgressState {
    unimplemented!()
}

pub fn fixed_centered_rect(_width: u16, _height: u16, _r: Rect) -> Rect {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

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
```

Add `pub mod indexing;` to `src/components/mod.rs`.

- [ ] **Step 2: Run test to verify it fails**

```bash
cd /Users/nhormazabal/development/personal/kimun/tui
cargo test drop_aborts_running_tasks 2>&1
```

Expected: compile error — `Drop` not implemented, `unimplemented!()` panics, or test fails.

- [ ] **Step 3: Implement `IndexingProgressState`, `spawn_running`, `fixed_centered_rect`**

Replace the stub implementations in `src/components/indexing.rs`:

```rust
use std::time::Duration;

use ratatui::layout::Rect;

use crate::components::events::{AppEvent, AppTx};

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
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test drop_aborts_running_tasks 2>&1
```

Expected: `test components::indexing::tests::drop_aborts_running_tasks ... ok`

- [ ] **Step 5: Commit**

```bash
git add src/components/indexing.rs src/components/mod.rs
git commit -m "feat: add components::indexing shared module"
```

---

### Task 2: Update `settings.rs` to import from shared module

**Files:**
- Modify: `src/app_screen/settings.rs`

`settings.rs` currently defines `IndexingProgressState`, `spawn_running`, and `fixed_centered_rect`. After this task they are imported from `components::indexing` instead.

- [ ] **Step 1: Update imports and remove local definitions**

At the top of `src/app_screen/settings.rs`, add:
```rust
use crate::components::indexing::{IndexingProgressState, fixed_centered_rect, spawn_running};
```

Then remove the three items that are now in the shared module:
- The `IndexingProgressState` enum definition (lines ~82–89)
- The `impl Drop for IndexingProgressState` block (lines ~91–98)
- The `spawn_running` function (lines ~100–111)
- The `fixed_centered_rect` function (lines ~753–762)

`centered_rect` (percentage-based, only used by the file browser overlay) stays in `settings.rs` — do not remove it.

- [ ] **Step 2: Verify the existing settings tests still pass**

```bash
cargo test -p tui 2>&1 | grep -E "settings|indexing|FAILED|error"
```

Expected: all settings tests pass, no compile errors.

- [ ] **Step 3: Commit**

```bash
git add src/app_screen/settings.rs
git commit -m "refactor: settings.rs uses components::indexing"
```

---

## Chunk 2: StartScreen with startup indexing

### Task 3: Rewrite `start.rs` with indexing overlay

**Files:**
- Modify: `src/app_screen/start.rs`

- [ ] **Step 1: Write failing tests**

Add a `#[cfg(test)]` block at the bottom of `src/app_screen/start.rs`. The tests exercise the three new behaviours: `on_enter` branching, `handle_app_message(IndexingDone)`, and `handle_input` blocking. Write all tests first, then implement.

The tests need a real `NoteVault` for the `vault = Some` case. Use the same temp-dir pattern as `browse.rs`.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc::unbounded_channel;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use kimun_core::NoteVault;
    use crate::components::events::AppEvent;
    use crate::components::indexing::IndexingProgressState;
    use crate::settings::AppSettings;

    async fn make_vault() -> Arc<NoteVault> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!("kimun_start_test_{nonce}"));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(NoteVault::new(&dir).await.unwrap())
    }

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    // --- on_enter: no vault ---

    #[tokio::test]
    async fn on_enter_no_vault_sends_open_path() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.on_enter(&tx).await;
        let msg = rx.try_recv().expect("OpenPath should be sent immediately");
        assert!(matches!(msg, AppEvent::OpenPath(_)));
        assert!(screen.overlay.is_none(), "no overlay when vault is None");
    }

    // --- on_enter: with vault ---

    #[tokio::test]
    async fn on_enter_with_vault_sets_overlay_and_defers_open_path() {
        let vault = make_vault().await;
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), Some(vault));
        screen.on_enter(&tx).await;
        // Overlay should be Running immediately after on_enter
        assert!(
            matches!(screen.overlay, Some(IndexingProgressState::Running { .. })),
            "overlay should be Running"
        );
        // No OpenPath yet — deferred until IndexingDone.
        // Drain the channel (a Redraw from the ticker may arrive) and assert
        // no OpenPath was sent.
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !events.iter().any(|e| matches!(e, AppEvent::OpenPath(_))),
            "OpenPath must not be sent before IndexingDone"
        );
    }

    // --- handle_app_message: IndexingDone Ok ---

    #[tokio::test]
    async fn indexing_done_ok_clears_overlay_and_sends_open_path() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        // Manually place a running overlay to simulate mid-index state
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(AppEvent::IndexingDone(Ok(Duration::from_secs(1))), &tx)
            .await;
        assert!(screen.overlay.is_none(), "overlay must be cleared");
        let msg = rx.try_recv().expect("OpenPath should be sent");
        assert!(matches!(msg, AppEvent::OpenPath(_)));
    }

    // --- handle_app_message: IndexingDone Err ---

    #[tokio::test]
    async fn indexing_done_err_clears_overlay_and_sends_open_path() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(
                AppEvent::IndexingDone(Err("disk error".to_string())),
                &tx,
            )
            .await;
        assert!(screen.overlay.is_none(), "overlay must be cleared even on error");
        let msg = rx.try_recv().expect("OpenPath should be sent even on error");
        assert!(matches!(msg, AppEvent::OpenPath(_)));
    }

    // --- handle_input: blocked while Running ---

    #[test]
    fn handle_input_consumed_while_running() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: rt.spawn(async {}),
            ticker: rt.spawn(async {}),
        });
        let result = screen.handle_input(&key(KeyCode::Enter), &tx);
        assert!(
            matches!(result, EventState::Consumed),
            "input must be blocked while indexing"
        );
        assert!(rx.try_recv().is_err(), "no event sent while blocked");
    }

    // --- handle_input: not consumed when idle ---

    #[test]
    fn handle_input_not_consumed_when_idle() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        let result = screen.handle_input(&key(KeyCode::Enter), &tx);
        assert!(matches!(result, EventState::NotConsumed));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p tui start 2>&1 | tail -20
```

Expected: compile errors — `StartScreen::new` still has old signature, `overlay` field doesn't exist.

- [ ] **Step 3: Rewrite `src/app_screen/start.rs`**

Replace the entire file:

```rust
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear};
use throbber_widgets_tui::{Throbber, ThrobberState};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::indexing::{IndexingProgressState, fixed_centered_rect, spawn_running};
use crate::settings::AppSettings;

pub struct StartScreen {
    settings: AppSettings,
    vault: Option<Arc<NoteVault>>,
    overlay: Option<IndexingProgressState>,
    throbber_state: ThrobberState,
}

impl StartScreen {
    /// `vault` should be `Some` only on initial app startup.
    /// Pass `None` when re-creating `StartScreen` after returning from settings,
    /// to avoid triggering a gratuitous reindex.
    pub fn new(settings: AppSettings, vault: Option<Arc<NoteVault>>) -> Self {
        Self {
            settings,
            vault,
            overlay: None,
            throbber_state: ThrobberState::default(),
        }
    }
}

#[async_trait]
impl AppScreen for StartScreen {
    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Start
    }

    async fn on_enter(&mut self, tx: &AppTx) {
        if let Some(vault) = self.vault.clone() {
            let tx2 = tx.clone();
            let handle = tokio::spawn(async move {
                let result = vault
                    .init_and_validate()
                    .await
                    .map_err(|e| e.to_string())
                    .map(|r| r.duration);
                tx2.send(AppEvent::IndexingDone(result)).ok();
            });
            self.overlay = Some(spawn_running(handle, tx));
        } else {
            let path = self
                .settings
                .last_paths
                .last()
                .map_or_else(VaultPath::root, |p| p.to_owned());
            tx.send(AppEvent::OpenPath(path)).ok();
        }
    }

    fn handle_input(&mut self, _event: &InputEvent, _tx: &AppTx) -> EventState {
        if matches!(self.overlay, Some(IndexingProgressState::Running { .. })) {
            EventState::Consumed
        } else {
            EventState::NotConsumed
        }
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) -> Option<AppEvent> {
        if let AppEvent::IndexingDone(_) = &msg {
            self.overlay = None;
            let path = self
                .settings
                .last_paths
                .last()
                .map_or_else(VaultPath::root, |p| p.to_owned());
            tx.send(AppEvent::OpenPath(path)).ok();
            return None;
        }
        Some(msg)
    }

    fn render(&mut self, f: &mut Frame) {
        let block = Block::default().title("Start app").borders(Borders::ALL);
        f.render_widget(block, f.area());

        if matches!(self.overlay, Some(IndexingProgressState::Running { .. })) {
            let theme_accent = {
                let t = self.settings.get_theme();
                (t.accent.to_ratatui(), t.fg.to_ratatui(), t.bg.to_ratatui(), t.base_style())
            };
            let area = fixed_centered_rect(44, 5, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title("Indexing")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme_accent.0))
                .style(theme_accent.3);
            let inner = block.inner(area);
            f.render_widget(block, area);
            self.throbber_state.calc_next();
            let throbber = Throbber::default()
                .label("  Initializing vault\u{2026}")
                .style(Style::default().fg(theme_accent.1).bg(theme_accent.2));
            f.render_stateful_widget(throbber, inner, &mut self.throbber_state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use kimun_core::NoteVault;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    use crate::components::events::AppEvent;
    use crate::components::indexing::IndexingProgressState;
    use crate::settings::AppSettings;

    async fn make_vault() -> Arc<NoteVault> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!("kimun_start_test_{nonce}"));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(NoteVault::new(&dir).await.unwrap())
    }

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[tokio::test]
    async fn on_enter_no_vault_sends_open_path() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.on_enter(&tx).await;
        let msg = rx.try_recv().expect("OpenPath should be sent immediately");
        assert!(matches!(msg, AppEvent::OpenPath(_)));
        assert!(screen.overlay.is_none(), "no overlay when vault is None");
    }

    #[tokio::test]
    async fn on_enter_with_vault_sets_overlay_and_defers_open_path() {
        let vault = make_vault().await;
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), Some(vault));
        screen.on_enter(&tx).await;
        assert!(
            matches!(screen.overlay, Some(IndexingProgressState::Running { .. })),
            "overlay should be Running"
        );
        assert!(
            rx.try_recv().is_err(),
            "OpenPath must not be sent before IndexingDone"
        );
    }

    #[tokio::test]
    async fn indexing_done_ok_clears_overlay_and_sends_open_path() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(AppEvent::IndexingDone(Ok(Duration::from_secs(1))), &tx)
            .await;
        assert!(screen.overlay.is_none(), "overlay must be cleared");
        let msg = rx.try_recv().expect("OpenPath should be sent");
        assert!(matches!(msg, AppEvent::OpenPath(_)));
    }

    #[tokio::test]
    async fn indexing_done_err_clears_overlay_and_sends_open_path() {
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(
                AppEvent::IndexingDone(Err("disk error".to_string())),
                &tx,
            )
            .await;
        assert!(screen.overlay.is_none(), "overlay must be cleared even on error");
        let msg = rx.try_recv().expect("OpenPath should be sent even on error");
        assert!(matches!(msg, AppEvent::OpenPath(_)));
    }

    #[test]
    fn handle_input_consumed_while_running() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, mut rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: rt.spawn(async {}),
            ticker: rt.spawn(async {}),
        });
        let result = screen.handle_input(&key(KeyCode::Enter), &tx);
        assert!(matches!(result, EventState::Consumed));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn handle_input_not_consumed_when_idle() {
        let (tx, _rx) = unbounded_channel();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        let result = screen.handle_input(&key(KeyCode::Enter), &tx);
        assert!(matches!(result, EventState::NotConsumed));
    }
}
```

- [ ] **Step 4: Run the new tests**

```bash
cargo test -p tui start:: 2>&1
```

Expected: all 6 `start::` tests pass.

- [ ] **Step 5: Run the full test suite to catch regressions**

```bash
cargo test -p tui 2>&1 | tail -5
```

Expected: all tests pass (the suite now fails to compile because `app.rs` and `main.rs` still call `StartScreen::new` with the old one-argument signature — compile errors are expected here if those files haven't been updated yet).

If there are compile errors only in `app.rs` / `main.rs` due to the signature change, that is expected — proceed to Task 4.

- [ ] **Step 6: Commit**

```bash
git add src/app_screen/start.rs
git commit -m "feat: StartScreen startup indexing overlay"
```

---

## Chunk 3: Wire vault through App and switch_screen

### Task 4: Update `app.rs` and `main.rs`

**Files:**
- Modify: `src/app.rs` (pass vault to `StartScreen::new`)
- Modify: `src/main.rs` (`switch_screen` passes `None` for re-entries)

- [ ] **Step 1: Update `src/app.rs`**

Change line ~35 in `App::new` from:
```rust
current_screen: Some(Box::new(StartScreen::new(settings.clone()))),
```
to:
```rust
current_screen: Some(Box::new(StartScreen::new(settings.clone(), vault.clone()))),
```

- [ ] **Step 2: Update `src/main.rs` — `switch_screen`**

In `switch_screen`, change the `ScreenEvent::Start` arm from:
```rust
ScreenEvent::Start => Box::new(StartScreen::new(app.settings.clone())),
```
to:
```rust
ScreenEvent::Start => Box::new(StartScreen::new(app.settings.clone(), None)),
```

- [ ] **Step 3: Run full test suite**

```bash
cargo test -p tui 2>&1 | tail -10
```

Expected: all tests pass, no compile errors.

- [ ] **Step 4: Verify it builds cleanly**

```bash
cargo build -p tui 2>&1
```

Expected: no errors or warnings about unused imports. If there are dead-code warnings in `settings.rs` about `centered_rect` that's fine — it is still used by the file browser overlay.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat: wire vault into StartScreen for startup indexing"
```

---

### Task 5: Final verification

- [ ] **Step 1: Run complete test suite one last time**

```bash
cargo test -p tui 2>&1
```

Expected output ends with: `test result: ok. N passed; 0 failed`

The new tests added in this feature are:
- `components::indexing::tests::drop_aborts_running_tasks`
- `app_screen::start::tests::on_enter_no_vault_sends_open_path`
- `app_screen::start::tests::on_enter_with_vault_sets_overlay_and_defers_open_path`
- `app_screen::start::tests::indexing_done_ok_clears_overlay_and_sends_open_path`
- `app_screen::start::tests::indexing_done_err_clears_overlay_and_sends_open_path`
- `app_screen::start::tests::handle_input_consumed_while_running`
- `app_screen::start::tests::handle_input_not_consumed_when_idle`

- [ ] **Step 2: Final commit if any loose changes remain**

```bash
git status
# If clean, nothing to do. If there are unstaged changes, stage and commit them.
```
