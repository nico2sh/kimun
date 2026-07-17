pub mod citations;
pub mod save;

use kimun_core::nfs::VaultPath;
use kimun_server_client::dto::ChunkResult;

/// How many trailing `Done` turns feed conversation history sent to the server.
const HISTORY_WINDOW: usize = 5;

/// A single retrieved chunk backing an answer (CONTEXT.md: **Source**).
#[derive(Debug)]
pub struct AskSource {
    pub path: VaultPath,
    pub heading: String,
    pub score: f64,
    pub text: String,
}

impl From<ChunkResult> for AskSource {
    fn from(c: ChunkResult) -> Self {
        Self {
            path: VaultPath::new(&c.path),
            heading: c.title,
            score: c.similarity_score,
            text: c.content,
        }
    }
}

/// A turn's lifecycle. `Streaming` is reserved for a future streaming feature
/// and is never constructed in v1.
#[allow(dead_code)]
pub enum TurnStatus {
    Thinking,
    Streaming,
    Done,
    Error(String),
}

/// One question/answer exchange (CONTEXT.md: **Turn**). Always knows its own sources.
pub struct Turn {
    pub id: u64,
    pub question: String,
    pub answer: String,
    pub sources: Vec<AskSource>,
    pub status: TurnStatus,
}

/// The running ask conversation (CONTEXT.md: **Thread**): an ordered list of
/// turns plus which one is selected for viewing.
#[derive(Default)]
pub struct Thread {
    turns: Vec<Turn>,
    next_id: u64,
    selected: usize,
}

impl Thread {
    /// Append a new `Thinking` turn for `question`, select it, and return its id.
    pub fn ask(&mut self, question: String) -> u64 {
        let id = self.bump();
        self.turns.push(Turn {
            id,
            question,
            answer: String::new(),
            sources: vec![],
            status: TurnStatus::Thinking,
        });
        self.selected = self.turns.len() - 1;
        id
    }

    /// Resolve a `Thinking` turn into `Done`. Returns `false` (no-op) for an
    /// unknown id or a turn that isn't currently `Thinking` (stale completion).
    pub fn complete(&mut self, id: u64, answer: String, sources: Vec<AskSource>) -> bool {
        let Some(turn) = self.thinking_turn_mut(id) else {
            return false;
        };
        turn.answer = answer;
        turn.sources = sources;
        turn.status = TurnStatus::Done;
        true
    }

    /// Resolve a `Thinking` turn into `Error`. Same stale-completion rules as `complete`.
    pub fn fail(&mut self, id: u64, error: String) -> bool {
        let Some(turn) = self.thinking_turn_mut(id) else {
            return false;
        };
        turn.status = TurnStatus::Error(error);
        true
    }

    /// Rewind a `Done`/`Error` turn back to `Thinking`, keeping its sources, and
    /// return its question so the caller can re-issue the request.
    pub fn regenerate(&mut self, id: u64) -> Option<String> {
        let turn = self.turns.iter_mut().find(|t| t.id == id)?;
        if matches!(turn.status, TurnStatus::Thinking | TurnStatus::Streaming) {
            return None;
        }
        turn.status = TurnStatus::Thinking;
        Some(turn.question.clone())
    }

    /// The last `HISTORY_WINDOW` `Done` turns before the newest in-flight
    /// (`Thinking`/`Streaming`) turn, as `(question, answer)` pairs with
    /// citation markers stripped.
    pub fn history(&self) -> Vec<(String, String)> {
        let boundary = self
            .turns
            .iter()
            .rposition(|t| matches!(t.status, TurnStatus::Thinking | TurnStatus::Streaming))
            .unwrap_or(self.turns.len());
        let mut done: Vec<_> = self.turns[..boundary]
            .iter()
            .filter(|t| matches!(t.status, TurnStatus::Done))
            .rev()
            .take(HISTORY_WINDOW)
            .collect();
        done.reverse();
        done.into_iter()
            .map(|t| (t.question.clone(), citations::strip(&t.answer)))
            .collect()
    }

    /// The currently selected turn, if any.
    pub fn selected(&self) -> Option<&Turn> {
        self.turns.get(self.selected)
    }

    /// Move the selection to the previous (older) turn, if any.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the selection to the next (newer) turn, if any.
    pub fn select_next(&mut self) {
        if self.selected + 1 < self.turns.len() {
            self.selected += 1;
        }
    }

    /// Select the most recent turn.
    pub fn select_last(&mut self) {
        self.selected = self.turns.len().saturating_sub(1);
    }

    /// Drop all turns, resetting the thread.
    pub fn clear(&mut self) {
        self.turns.clear();
        self.selected = 0;
    }

    /// All turns, oldest first.
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// Whether the thread has no turns.
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    fn bump(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn thinking_turn_mut(&mut self, id: u64) -> Option<&mut Turn> {
        self.turns
            .iter_mut()
            .find(|t| t.id == id && matches!(t.status, TurnStatus::Thinking))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(thread: &mut Thread, q: &str, a: &str) {
        let id = thread.ask(q.to_string());
        assert!(thread.complete(id, a.to_string(), vec![]));
    }

    #[test]
    fn ask_appends_a_thinking_turn_and_selects_it() {
        let mut t = Thread::default();
        let id = t.ask("q?".into());
        assert_eq!(t.turns().len(), 1);
        assert!(matches!(t.selected().unwrap().status, TurnStatus::Thinking));
        assert_eq!(t.selected().unwrap().id, id);
    }

    #[test]
    fn history_takes_last_five_done_turns_and_strips_citations() {
        let mut t = Thread::default();
        for i in 0..7 {
            done(&mut t, &format!("q{i}"), &format!("a{i} [1]"));
        }
        t.ask("new".into()); // the in-flight turn history is built for
        let h = t.history();
        assert_eq!(h.len(), 5);
        assert_eq!(h[0].0, "q2");
        assert_eq!(h[4].1, "a6"); // "[1]" stripped
    }

    #[test]
    fn stale_completion_is_dropped() {
        let mut t = Thread::default();
        let id = t.ask("q".into());
        t.clear();
        assert!(!t.complete(id, "late".into(), vec![]));
        assert!(t.is_empty());
    }

    #[test]
    fn regenerate_rewinds_a_done_turn_keeping_sources() {
        let mut t = Thread::default();
        let id = t.ask("q".into());
        let src = AskSource {
            path: kimun_core::nfs::VaultPath::new("a.md"),
            heading: "h".into(),
            score: 0.9,
            text: "body".into(),
        };
        t.complete(id, "a".into(), vec![src]);
        assert_eq!(t.regenerate(id).as_deref(), Some("q"));
        let turn = t.selected().unwrap();
        assert!(matches!(turn.status, TurnStatus::Thinking));
        assert_eq!(turn.sources.len(), 1, "regenerate reuses the same sources");
    }
}
