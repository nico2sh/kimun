use std::ops::Range;
use std::sync::Arc;

use kimun_core::NoteVault;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::host::AutocompleteHost;
use super::popup::{handle_key as popup_handle_key, PopupAction, PopupOutcome};
use super::state::{AutocompleteState, Suggestion, DEFAULT_MAX_VISIBLE_ROWS};
use super::trigger::{detect_trigger_with, TriggerKind, TriggerOptions};

/// Hard cap on suggestions fetched from core per query. The popup itself
/// only shows `max_visible_rows` at a time and scrolls inside the fetched
/// set, so a few dozen rows is plenty.
const DEFAULT_FETCH_LIMIT: usize = 50;

/// Whether wikilink triggers are honoured. The editor uses
/// `Both`; the search box uses `HashtagOnly` because the search syntax has
/// no `[[…]]` operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutocompleteMode {
    Both,
    HashtagOnly,
}

/// Owns the popup lifecycle and the (debounced via generation tokens)
/// query plumbing. The host calls `sync(host)` after every edit; the
/// controller decides whether to open, refresh, or close the popup.
pub struct AutocompleteController {
    state: Option<AutocompleteState>,
    vault: Arc<NoteVault>,
    mode: AutocompleteMode,
    /// Trigger-detection options passed to `detect_trigger_with` on every
    /// `sync`. The editor leaves header disambiguation on; the search box
    /// switches it off because its input has no Markdown headers.
    trigger_opts: TriggerOptions,
    /// Monotonic counter incremented on every fired query. Responses that
    /// arrive with a stale generation are discarded.
    generation: u64,
    result_tx: UnboundedSender<QueryResult>,
    result_rx: UnboundedReceiver<QueryResult>,
    fetch_limit: usize,
    max_visible_rows: usize,
}

#[derive(Debug)]
struct QueryResult {
    generation: u64,
    kind: TriggerKind,
    items: Vec<Suggestion>,
}

impl AutocompleteController {
    pub fn new(vault: Arc<NoteVault>, mode: AutocompleteMode) -> Self {
        let (result_tx, result_rx) = unbounded_channel();
        Self {
            state: None,
            vault,
            mode,
            trigger_opts: TriggerOptions::default(),
            generation: 0,
            result_tx,
            result_rx,
            fetch_limit: DEFAULT_FETCH_LIMIT,
            max_visible_rows: DEFAULT_MAX_VISIBLE_ROWS,
        }
    }

    /// Override the trigger-detection options. Used by the search-box
    /// controller to disable the column-0 header disambiguation rule
    /// (Markdown headers don't exist in a search input).
    pub fn with_trigger_opts(mut self, opts: TriggerOptions) -> Self {
        self.trigger_opts = opts;
        self
    }

    pub fn is_open(&self) -> bool {
        self.state.is_some()
    }

    pub fn state(&self) -> Option<&AutocompleteState> {
        self.state.as_ref()
    }

    pub fn state_mut(&mut self) -> Option<&mut AutocompleteState> {
        self.state.as_mut()
    }

    pub fn close(&mut self) {
        self.state = None;
    }

    /// Route a key event through the popup when one is open. Returns a
    /// `HandleKeyOutcome` so the host can decide whether to apply an
    /// accept, fall through to its own key handling, etc. The controller
    /// never mutates the host's buffer directly — on accept it returns an
    /// `AcceptAction` describing the replacement.
    pub fn handle_key<H: AutocompleteHost>(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        host: &H,
    ) -> HandleKeyOutcome {
        let Some(state) = self.state.as_mut() else {
            return HandleKeyOutcome::NotHandled;
        };
        let outcome = popup_handle_key(state, key);
        match outcome {
            PopupOutcome::Consumed(PopupAction::None) => HandleKeyOutcome::Consumed,
            PopupOutcome::Consumed(PopupAction::Accept) => {
                let action = self.compute_accept(host);
                self.close();
                match action {
                    Some(a) => HandleKeyOutcome::Accepted(a),
                    None => HandleKeyOutcome::Consumed,
                }
            }
            PopupOutcome::Consumed(PopupAction::Dismiss) => {
                self.close();
                HandleKeyOutcome::Dismissed
            }
            PopupOutcome::NotHandled => HandleKeyOutcome::NotHandled,
        }
    }

    /// Inspect the host's current buffer + cursor and reconcile the popup
    /// state. Call this after every edit. Cheap when nothing changes:
    /// recomputes the trigger and only re-fires the query when the query
    /// string actually changed.
    pub fn sync<H: AutocompleteHost>(&mut self, host: &H) {
        let text = host.buffer_text();
        let cursor = host.cursor_byte_offset();
        let trigger = detect_trigger_with(&text, cursor, self.trigger_opts);

        // Filter by mode before deciding anything else.
        let trigger = trigger.filter(|t| match (self.mode, t.kind) {
            (AutocompleteMode::Both, _) => true,
            (AutocompleteMode::HashtagOnly, TriggerKind::Hashtag) => true,
            (AutocompleteMode::HashtagOnly, TriggerKind::Wikilink) => false,
        });

        let Some(trigger) = trigger else {
            self.close();
            return;
        };

        let Some(anchor) = host.screen_anchor_for(trigger.anchor_col) else {
            self.close();
            return;
        };

        let query_changed;
        let kind_changed;
        match self.state.as_ref() {
            None => {
                kind_changed = true;
                query_changed = true;
            }
            Some(existing) => {
                kind_changed = existing.kind != trigger.kind;
                query_changed = kind_changed || existing.query != trigger.query;
            }
        }

        if self.state.is_none() || kind_changed {
            let mut st = AutocompleteState::new(trigger.kind, anchor);
            st.max_visible_rows = self.max_visible_rows;
            self.state = Some(st);
        }

        if let Some(state) = self.state.as_mut() {
            state.kind = trigger.kind;
            state.query = trigger.query.clone();
            state.replace_range = trigger.replace_range.clone();
            state.anchor = anchor;
        }

        if query_changed {
            self.fire_query(trigger.kind, trigger.query);
        }
    }

    /// Drain pending query responses and apply the latest one whose
    /// generation matches the controller's current generation. Older
    /// responses (stale) are discarded.
    pub fn poll_results(&mut self) {
        while let Ok(result) = self.result_rx.try_recv() {
            if result.generation != self.generation {
                continue;
            }
            let Some(state) = self.state.as_mut() else {
                continue;
            };
            if state.kind != result.kind {
                continue;
            }
            state.set_items(result.items);
        }
    }

    fn fire_query(&mut self, kind: TriggerKind, query: String) {
        self.generation = self.generation.wrapping_add(1);
        let req_gen = self.generation;
        let tx = self.result_tx.clone();
        let vault = self.vault.clone();
        let limit = self.fetch_limit;
        tokio::spawn(async move {
            let items: Vec<Suggestion> = match kind {
                TriggerKind::Wikilink => match vault.suggest_notes_by_prefix(&query, limit).await {
                    Ok(notes) => notes
                        .into_iter()
                        .map(|n| Suggestion {
                            display: n.name,
                            secondary: Some(n.path.to_string()),
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                },
                TriggerKind::Hashtag => match vault.suggest_tags_by_prefix(&query, limit).await {
                    Ok(tags) => tags
                        .into_iter()
                        .map(|t| Suggestion {
                            display: t.label,
                            secondary: Some(format!("{}×", t.usage_count)),
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                },
            };
            let _ = tx.send(QueryResult {
                generation: req_gen,
                kind,
                items,
            });
        });
    }

    fn compute_accept<H: AutocompleteHost>(&self, host: &H) -> Option<AcceptAction> {
        let state = self.state.as_ref()?;
        let suggestion = state.selected()?.clone();
        let kind = state.kind;
        let range = state.replace_range.clone();

        match kind {
            TriggerKind::Wikilink => {
                let buffer = host.buffer_text();
                let close_exists = buffer.as_bytes().get(range.end..range.end + 2)
                    == Some(b"]]");
                let new_text = if close_exists {
                    suggestion.display.clone()
                } else {
                    format!("{}]]", suggestion.display)
                };
                // Cursor lands after the closing `]]` whether we inserted
                // them or not. The contribution of the closing pair is
                // always 2 bytes from the start of `display`.
                let new_cursor_byte = range.start + suggestion.display.len() + 2;
                Some(AcceptAction {
                    range,
                    new_text,
                    new_cursor_byte,
                })
            }
            TriggerKind::Hashtag => {
                let new_cursor_byte = range.start + suggestion.display.len();
                Some(AcceptAction {
                    range,
                    new_text: suggestion.display,
                    new_cursor_byte,
                })
            }
        }
    }
}

/// What the controller decided when forwarded a key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleKeyOutcome {
    /// Popup was open and consumed the key as navigation.
    Consumed,
    /// Popup was open; user dismissed with Esc.
    Dismissed,
    /// Popup was open; user accepted. The host should apply the action.
    Accepted(AcceptAction),
    /// Popup was either closed or did not handle this key — host should
    /// process it as a normal key event, then call `sync()` afterward.
    NotHandled,
}

/// A buffer replacement the host needs to perform after an accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptAction {
    pub range: Range<usize>,
    pub new_text: String,
    pub new_cursor_byte: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::nfs::VaultPath;
    use kimun_core::{NoteVault, VaultConfig};
    use std::sync::Arc;
    use tempfile::TempDir;

    struct FakeHost {
        buffer: String,
        cursor: usize,
    }

    impl FakeHost {
        fn new(buffer: &str, cursor: usize) -> Self {
            Self {
                buffer: buffer.to_string(),
                cursor,
            }
        }

        fn apply(&mut self, action: &AcceptAction) {
            self.buffer.replace_range(action.range.clone(), &action.new_text);
            self.cursor = action.new_cursor_byte;
        }
    }

    impl AutocompleteHost for FakeHost {
        fn buffer_text(&self) -> String {
            self.buffer.clone()
        }
        fn cursor_byte_offset(&self) -> usize {
            self.cursor
        }
        fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
            Some((0, 0))
        }
    }

    async fn new_vault_with(notes: &[&str], tag_notes: &[(&str, &str)]) -> (TempDir, Arc<NoteVault>) {
        let tmp = TempDir::new().unwrap();
        let cfg = VaultConfig::new(tmp.path().to_path_buf());
        let vault = NoteVault::new(cfg).await.unwrap();
        vault.validate_and_init().await.unwrap();
        for name in notes {
            vault
                .create_note(&VaultPath::note_path_from(format!("/{name}.md")), "body")
                .await
                .unwrap();
        }
        for (path, body) in tag_notes {
            vault
                .create_note(&VaultPath::note_path_from(format!("/{path}.md")), *body)
                .await
                .unwrap();
        }
        (tmp, Arc::new(vault))
    }

    async fn drain_results(controller: &mut AutocompleteController) {
        // The spawned query task completes promptly on an in-memory DB;
        // yield once to let it run and post the result.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        controller.poll_results();
    }

    // ---- Lifecycle ----

    #[tokio::test]
    async fn no_trigger_keeps_popup_closed() {
        let (_tmp, vault) = new_vault_with(&[], &[]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        let host = FakeHost::new("plain text", 5);
        c.sync(&host);
        assert!(!c.is_open());
    }

    #[tokio::test]
    async fn wikilink_trigger_opens_popup_and_loads_results() {
        let (_tmp, vault) = new_vault_with(&["meeting", "music", "novel"], &[]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        let host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        assert!(c.is_open());
        drain_results(&mut c).await;
        let st = c.state().unwrap();
        assert_eq!(st.kind, TriggerKind::Wikilink);
        assert_eq!(st.query, "me");
        let names: Vec<&str> = st.items.iter().map(|s| s.display.as_str()).collect();
        assert!(names.contains(&"meeting"));
        assert!(!names.contains(&"novel"));
    }

    #[tokio::test]
    async fn hashtag_trigger_opens_popup_and_loads_results() {
        let (_tmp, vault) = new_vault_with(&[], &[("a", "x #projects"), ("b", "y #pro")]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        let host = FakeHost::new("about #pro", 10);
        c.sync(&host);
        drain_results(&mut c).await;
        let st = c.state().unwrap();
        assert_eq!(st.kind, TriggerKind::Hashtag);
        let labels: Vec<&str> = st.items.iter().map(|s| s.display.as_str()).collect();
        assert!(labels.contains(&"pro"));
        assert!(labels.contains(&"projects"));
    }

    #[tokio::test]
    async fn hashtag_only_mode_ignores_wikilinks() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::HashtagOnly);
        let host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        assert!(!c.is_open());
    }

    #[tokio::test]
    async fn losing_trigger_context_closes_popup() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        assert!(c.is_open());
        // User types a space — hashtag context for the typed query is now
        // broken; for wikilinks, the trigger stays alive but the query
        // gains a space. Here we simulate the cursor jumping outside.
        host.buffer = "see [[me\n".into();
        host.cursor = 9;
        c.sync(&host);
        assert!(!c.is_open());
    }

    // ---- Accept actions ----

    #[tokio::test]
    async fn accepting_wikilink_inserts_name_and_closes_brackets() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome =
            c.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "see [[meeting]]");
        assert_eq!(host.cursor, host.buffer.len());
        assert!(!c.is_open());
    }

    #[tokio::test]
    async fn accepting_wikilink_preserves_existing_closing_brackets() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("see [[me]]", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome =
            c.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "see [[meeting]]");
        assert_eq!(host.cursor, host.buffer.len());
    }

    #[tokio::test]
    async fn accepting_hashtag_inserts_label_no_trailing_space() {
        let (_tmp, vault) = new_vault_with(&[], &[("a", "x #projects")]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        let mut host = FakeHost::new("about #pro", 10);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome =
            c.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &host);
        let HandleKeyOutcome::Accepted(action) = outcome else {
            panic!("expected Accepted, got {:?}", outcome);
        };
        host.apply(&action);
        assert_eq!(host.buffer, "about #projects");
        assert_eq!(host.cursor, host.buffer.len());
    }

    #[tokio::test]
    async fn esc_dismisses_without_changing_buffer() {
        let (_tmp, vault) = new_vault_with(&["meeting"], &[]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        let host = FakeHost::new("see [[me", 8);
        c.sync(&host);
        drain_results(&mut c).await;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let outcome =
            c.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &host);
        assert_eq!(outcome, HandleKeyOutcome::Dismissed);
        assert_eq!(host.buffer, "see [[me");
        assert!(!c.is_open());
    }

    // ---- Generation / drop-stale ----

    #[tokio::test]
    async fn stale_results_are_dropped_on_query_change() {
        let (_tmp, vault) = new_vault_with(&["meeting", "memory"], &[]).await;
        let mut c = AutocompleteController::new(vault, AutocompleteMode::Both);
        // First query for `me` — fires generation 1.
        let host1 = FakeHost::new("see [[me", 8);
        c.sync(&host1);
        // Immediately change query to `mem` — fires generation 2 before
        // generation 1 has had a chance to respond.
        let host2 = FakeHost::new("see [[mem", 9);
        c.sync(&host2);
        drain_results(&mut c).await;
        let st = c.state().unwrap();
        // Only the `mem` results should be present — `meeting` doesn't
        // start with `mem` so the only match is `memory`.
        assert_eq!(st.query, "mem");
        let names: Vec<&str> = st.items.iter().map(|s| s.display.as_str()).collect();
        assert_eq!(names, vec!["memory"]);
    }
}
