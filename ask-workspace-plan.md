# Ask Workspace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single-shot `RagAnswerOverlay` with the **Ask workspace**: a rail entry that swaps the drawer to a per-turn Sources view and the editor area to a conversation Thread with docked composer, citations, source reader, and save-answer-as-note.

**Architecture:** Panel content inside the editor screen, not a screen or overlay (adr/0030). Server gains an optional `history` field on `/api/answer` and a citation-numbered prompt; the client crate forwards history; the TUI grows an `ask` domain module (thread state, citation logic, note export) plus two components (`ThreadPanel` in the editor area, `SourcesPanel` in the drawer). Design decisions are recorded in CONTEXT.md (Ask section), adr/0030, and the `ask-handoff-deviations` memory — the HTML prototype in `ask-rag-handoff/` is **not** the spec of record where they disagree.

**Tech Stack:** Rust workspace — `kimun_core` (core), `kimun-notes` (TUI, ratatui), `kimun_server` (axum), `kimun_server_client` (reqwest).

## Global Constraints

- All path manipulation in core; TUI never touches `/` or `.md` literals; vault paths are `VaultPath` (project CLAUDE.md).
- All citation-marker (`[n]`) logic lives in ONE module: `tui/src/ask/citations.rs`. No other module may parse, strip, or rewrite `[n]` markers.
- Lean structs, small methods, reusable components — follow the existing component idiom (`Component` trait, `panel_block`, `SingleLineInput`).
- No streaming v1; `TurnStatus::Streaming` variant exists but is never constructed yet.
- No extractive fallback: LLM/network failure = `TurnStatus::Error` + regenerate.
- History: last **5** `Done` turns, citation markers stripped, optional wire field (backward compatible).
- Context size: TUI always passes `None`; server context cut decides.
- Prompt keeps notes-first + common-knowledge supplement; citations mandatory for context-derived claims.
- Rail entry ASK visible only when `RagStatus::llm_available()`; capability loss disables the composer, never evicts the thread.
- Per-task commits on the `ask-rag` branch (conventional prefix); Nico reviews/squashes/merges himself. End every task by reporting test results (green/red with output).
- Test commands: `cargo test -p kimun_server`, `cargo test -p kimun_server_client`, `cargo test -p kimun_core`, and for the TUI **`cargo test -p kimun-notes --bins`** (`--lib` silently skips the `app_screen` tests).

## File Structure

```
server/src/llmclients/mod.rs        # prompt rewrite + history messages     (modify)
server/src/lib.rs                   # KimunRag::answer history param        (modify)
server/src/handlers.rs              # AnswerRequest.history                 (modify)
client/src/dto.rs                   # HistoryTurn, QueryRequest.history     (modify)
client/src/lib.rs                   # RagClient::ask history param          (modify)
core/src/nfs/filename.rs            # note_name_from_title()                (modify)
tui/src/ask/mod.rs                  # Thread, Turn, TurnStatus, AskSource   (create)
tui/src/ask/citations.rs            # ALL [n] logic: scan/strip/link        (create)
tui/src/ask/save.rs                 # saved-answer content + suggested path (create)
tui/src/ask/locate.rs               # section_range() for reader highlight  (create)
tui/src/components/ask_thread.rs    # ThreadPanel (editor-area content)     (create)
tui/src/components/ask_sources.rs   # SourcesPanel (drawer view, 2 faces)   (create)
tui/src/components/events.rs        # AppEvent::Ask(AskData)                (modify)
tui/src/app_screen/panel_set.rs     # EditorAreaContent 3rd arm             (modify)
tui/src/components/drawer.rs        # DrawerView::Ask + host field          (modify)
tui/src/components/activity_rail.rs # ASK item + ask_visible gating         (modify)
tui/src/app_screen/editor.rs        # wiring, gating, leader, repurposed key(modify)
tui/src/components/rag_answer.rs    # DELETE (overlay superseded)           (delete)
tui/src/rag/mod.rs                  # drop RagAnswer/RagSource structs      (modify)
docs/                               # user docs page for Ask                (modify)
```

---

### Task 1: Server — citation-numbered prompt

**Files:**
- Modify: `server/src/llmclients/mod.rs` (`build_prompt` at :195, its test at :354)

**Interfaces:**
- Consumes: `FlattenedChunk { doc_path, doc_hash, title, text, date }` (`server/src/document.rs:25`)
- Produces: `build_prompt(question, context)` emitting numbered `[i]` chunk frames; the source order in `Answer.sources` already matches `context` order (`lib.rs:422-425`), so `[n]` ↔ `sources[n-1]` holds end to end with no further change.

- [ ] **Step 1: Write the failing test** (replace/extend `prompt_frames_each_chunk_and_carries_the_question`)

```rust
#[test]
fn prompt_numbers_chunks_and_mandates_citations() {
    let context = vec![
        (0.91, chunk("intro", "alpha text")),
        (0.72, chunk("setup", "beta text")),
    ];
    let p = build_prompt("how do I start?", &context);
    // numbered frames, prompt order = sources order
    let i1 = p.find("[1]").expect("first chunk numbered");
    let i2 = p.find("[2]").expect("second chunk numbered");
    assert!(i1 < i2);
    assert!(p.contains("alpha text") && p.contains("beta text"));
    // citation contract in the instructions
    assert!(p.contains("cite"), "prompt must instruct citing");
    assert!(p.contains("[n]"), "prompt must name the [n] form");
    assert!(p.contains("how do I start?"));
}
```

(reuse the file's existing `chunk(title, text)` test helper; add one if the current test builds chunks inline)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun_server prompt_numbers -- --nocapture`
Expected: FAIL (no `[1]` in current output)

- [ ] **Step 3: Rewrite `build_prompt`**

```rust
/// The one RAG prompt, shared by every provider: chunks are numbered `[i]` in
/// sources order, citations are mandatory for note-derived claims, and the
/// answer may supplement with common knowledge — uncited text IS the signal
/// that a claim is general knowledge, so the two never blur.
fn build_prompt(question: &str, context: &[(f64, FlattenedChunk)]) -> String {
    let mut context_string = String::new();
    for (i, (_, chunk)) in context.iter().enumerate() {
        let mut title = chunk.title.clone();
        let mut date_line = String::new();
        if let Some(date) = chunk.get_date_string() {
            date_line = format!("Date: {date}\n");
            title = title
                .trim()
                .strip_prefix(&date)
                .map(|t| t.trim().to_string())
                .unwrap_or(title);
        }
        context_string.push_str(&format!(
            "[{}] {} — \"{}\"\n{}{}\n\n",
            i + 1,
            chunk.doc_path,
            title.trim(),
            date_line,
            chunk.text
        ));
    }

    format!(
        r#"You are an intelligent assistant with access to a personal knowledge base.
Answer the user's question using the numbered context below first; base the answer primarily on it when it is relevant.
Every claim drawn from the context MUST carry an inline citation in the form [n], where n is the number of the supporting context entry. A sentence may carry several citations.
You may supplement with accurate, widely accepted common knowledge when the context falls short — never cite [n] for such claims; leaving them uncited is how they are marked as general knowledge.
Preserve any [[wikilinks]] and #tags that appear in the context verbatim when you quote or reference them.
If neither the context nor common knowledge suffices, respond with: 'I don't have enough information to answer.'

Context:
---------------------
{context_string}---------------------

Question: {question}"#
    )
}
```

- [ ] **Step 4: Run the module's tests**

Run: `cargo test -p kimun_server llmclients`
Expected: PASS (update any older prompt assertion that greps for the removed `--- Document:` framing)

- [ ] **Step 5: Report** — test output, note that sources order == numbering order is guaranteed by `lib.rs:422-425`.

---

### Task 2: Server — conversation history through the LLM client

**Files:**
- Modify: `server/src/llmclients/mod.rs`

**Interfaces:**
- Produces: `LLMClient::ask(&self, question: &str, history: &[(String, String)], context: &[(f64, FlattenedChunk)])` — history is (question, answer) pairs, oldest first. Pure builder `chat_messages(history: &[(String, String)], prompt: String) -> Vec<ChatMessage>` (unit-testable without network). `GeminiContent` gains `role: String` (`"user"` / `"model"`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn history_folds_into_alternating_messages_before_the_prompt() {
    let history = vec![
        ("q1".to_string(), "a1".to_string()),
        ("q2".to_string(), "a2".to_string()),
    ];
    let msgs = chat_messages(&history, "PROMPT".to_string());
    let shape: Vec<(&str, &str)> = msgs
        .iter()
        .map(|m| (m.role.as_str(), m.content.as_str()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("user", "q1"),
            ("assistant", "a1"),
            ("user", "q2"),
            ("assistant", "a2"),
            ("user", "PROMPT"),
        ]
    );
}

#[test]
fn empty_history_is_a_single_prompt_message() {
    let msgs = chat_messages(&[], "PROMPT".to_string());
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, "user");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p kimun_server history_folds -- --nocapture`
Expected: FAIL — `chat_messages` not defined

- [ ] **Step 3: Implement**

```rust
/// History pairs + the final RAG prompt as one chat transcript. Shared by the
/// OpenAI-compat and Anthropic wires; Gemini maps the same list to `contents`
/// with the "model" role name.
fn chat_messages(history: &[(String, String)], prompt: String) -> Vec<ChatMessage> {
    let mut msgs = Vec::with_capacity(history.len() * 2 + 1);
    for (q, a) in history {
        msgs.push(ChatMessage { role: "user".into(), content: q.clone() });
        msgs.push(ChatMessage { role: "assistant".into(), content: a.clone() });
    }
    msgs.push(ChatMessage { role: "user".into(), content: prompt });
    msgs
}
```

Then thread the parameter through:
- trait: `async fn ask(&self, question: &str, history: &[(String, String)], context: &[(f64, FlattenedChunk)]) -> anyhow::Result<String>;`
- `ChatClient::ask`: `let messages = chat_messages(history, build_prompt(question, context));` — use `messages` in both `ChatRequest` arms (replacing the inline `vec![ChatMessage {…}]`).
- Gemini arm: `GeminiContent` gains `role: String`; map `messages` → contents with `assistant` → `"model"`:

```rust
Wire::Gemini => {
    let contents: Vec<GeminiContent> = messages
        .into_iter()
        .map(|m| GeminiContent {
            role: if m.role == "assistant" { "model".into() } else { "user".into() },
            parts: vec![GeminiPart { text: m.content }],
        })
        .collect();
    // …existing request build with `contents`
}
```

- Fix every other `LLMClient::ask` call/impl site (`server/src/lib.rs:421` becomes `llm.ask(question, history, &context)` in Task 3; any test doubles implement the new signature with `&[]`).

- [ ] **Step 4: Run crate tests**

Run: `cargo test -p kimun_server`
Expected: PASS (compile errors at call sites are the todo list — fix each by passing `&[]` until Task 3 wires real history)

- [ ] **Step 5: Report.**

---

### Task 3: Server — `history` on `/api/answer`

**Files:**
- Modify: `server/src/handlers.rs` (`AnswerRequest` :72, `answer_handler` :255)
- Modify: `server/src/lib.rs` (`KimunRag::answer` :410)

**Interfaces:**
- Consumes: `chat_messages`-shaped history from Task 2.
- Produces: wire contract `POST /api/answer` body `{ vault_id, query, context_size?, history?: [{question, answer}] }` — absent/empty history = today's behavior (backward compatible). `KimunRag::answer(&self, collection, question: &str, history: &[(String, String)], top_k)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn answer_request_parses_without_history() {
    let r: AnswerRequest =
        serde_json::from_str(r#"{"vault_id":"v1","query":"q"}"#).unwrap();
    assert!(r.history.is_empty());
}

#[test]
fn answer_request_parses_history_pairs() {
    let r: AnswerRequest = serde_json::from_str(
        r#"{"vault_id":"v1","query":"q","history":[{"question":"q1","answer":"a1"}]}"#,
    )
    .unwrap();
    assert_eq!(r.history.len(), 1);
    assert_eq!(r.history[0].question, "q1");
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p kimun_server answer_request_parses` → FAIL (no field)

- [ ] **Step 3: Implement**

```rust
/// One prior Q&A exchange sent as conversation history. The client strips
/// citation markers before sending; the server passes pairs through verbatim.
#[derive(Debug, Deserialize)]
pub struct HistoryTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub vault_id: String,
    pub query: String,
    #[serde(default)]
    pub context_size: Option<ContextSize>,
    #[serde(default)]
    pub history: Vec<HistoryTurn>,
}
```

In `answer_handler`, before the spawn: `let history: Vec<(String, String)> = request.history.into_iter().map(|t| (t.question, t.answer)).collect;` (destructure — `request` is moved into the task; capture `history` alongside `request.query`). Call `rag.answer(&collection, &request.query, &history, top_k)`.

In `lib.rs`:

```rust
pub async fn answer(
    &self,
    collection: &CollectionKey,
    question: &str,
    history: &[(String, String)],
    top_k: usize,
) -> Result<Answer, RagError> {
    let llm = self.llm_client.clone().ok_or(RagError::SemanticOnly)?;
    let raw = self.retrieve(collection, question).await?;   // retrieval sees ONLY the question
    let mut context = self.rank(question, raw).await;
    let cut = self.cut_len(&context, top_k);
    context.truncate(cut);
    let text = llm.ask(question, history, &context).await?;
    Ok(Answer { text, sources: context })
}
```

Update existing `answer` tests (e.g. `answer_keeps_chunk_level_context_cut_by_normalized_score`, lib.rs:1149) to pass `&[]`.

- [ ] **Step 4: Run** — `cargo test -p kimun_server` → PASS
- [ ] **Step 5: Report.**

---

### Task 4: Client crate — history on `ask()`

**Files:**
- Modify: `client/src/dto.rs`, `client/src/lib.rs` (`ask` :258, `search` :235)

**Interfaces:**
- Produces: `RagClient::ask(&self, query: &str, history: &[(String, String)], context_size: Option<ContextSize>) -> Result<AnswerResult, RagError>`. `dto::HistoryTurn { question, answer }` (Serialize). `QueryRequest.history: Vec<HistoryTurn>` skipped when empty (a pre-history server never sees the field).

- [ ] **Step 1: Failing tests** (in `dto.rs` tests)

```rust
#[test]
fn query_request_omits_empty_history() {
    let req = QueryRequest {
        vault_id: "v".into(),
        query: "q".into(),
        context_size: None,
        history: vec![],
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("history"), "empty history must not hit the wire: {json}");
}

#[test]
fn query_request_serializes_history_pairs() {
    let req = QueryRequest {
        vault_id: "v".into(),
        query: "q".into(),
        context_size: None,
        history: vec![HistoryTurn { question: "q1".into(), answer: "a1".into() }],
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""history":[{"question":"q1","answer":"a1"}]"#));
}
```

- [ ] **Step 2: Verify failure** — `cargo test -p kimun_server_client query_request` → FAIL
- [ ] **Step 3: Implement**

```rust
/// One prior Q&A pair sent as conversation history on `/api/answer`.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryTurn {
    pub question: String,
    pub answer: String,
}
```

Add to `QueryRequest`: `#[serde(skip_serializing_if = "Vec::is_empty")] pub history: Vec<HistoryTurn>,`.
`ask()` gains `history: &[(String, String)]`, builds `history: history.iter().map(|(q, a)| HistoryTurn { question: q.clone(), answer: a.clone() }).collect()`. `search()` fills `history: vec![]`. Fix the one TUI call site (`tui/src/components/rag_answer.rs:76-99`) with `&[]` for now — it dies in Task 12.

- [ ] **Step 4: Run** — `cargo test -p kimun_server_client && cargo check --workspace` → PASS
- [ ] **Step 5: Report.**

---

### Task 5: TUI — `ask::citations`, the one home of `[n]` logic

**Files:**
- Create: `tui/src/ask/citations.rs`, `tui/src/ask/mod.rs` (module shell), register `mod ask;` in `tui/src/main.rs`/`lib.rs` alongside `mod rag;`

**Interfaces:**
- Produces (all pure, no I/O):
  - `pub struct CitationSpan { pub range: std::ops::Range<usize>, pub index: usize }` — byte range of a `[n]` marker, `index` 1-based.
  - `pub fn scan(text: &str) -> Vec<CitationSpan>` — every `[digits]` marker, in order.
  - `pub fn strip(text: &str) -> String` — text with all markers removed (used by copy `y` and by `Thread::history`).
  - `pub fn link_sources(text: &str, source_names: &[String]) -> String` — in-range `[n]` → `[[name]]`; out-of-range markers left untouched (used by save-as-note).
- Consumers: `ask_thread.rs` (render + copy), `ask::mod` (history), `ask::save`. **No other module touches `[n]`.**

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn scan_finds_markers_with_ranges_and_indices() {
    let t = "Alpha [1] beta [12].";
    let spans = scan(t);
    assert_eq!(spans.len(), 2);
    assert_eq!(&t[spans[0].range.clone()], "[1]");
    assert_eq!(spans[0].index, 1);
    assert_eq!(spans[1].index, 12);
}

#[test]
fn scan_ignores_non_numeric_brackets() {
    assert!(scan("a [[wikilink]] and [tag] and [1a]").is_empty());
}

#[test]
fn strip_removes_markers_and_tidies_double_spaces() {
    assert_eq!(strip("Fact [1] stands. Next [2]."), "Fact stands. Next.");
}

#[test]
fn link_sources_rewrites_in_range_and_keeps_out_of_range() {
    let names = vec!["alpha".to_string()];
    assert_eq!(
        link_sources("See [1] not [7].", &names),
        "See [[alpha]] not [7]."
    );
}
```

- [ ] **Step 2: Verify failure** — `cargo test -p kimun-notes --bins citations` → FAIL (module missing)
- [ ] **Step 3: Implement** — hand scanner, no regex dep:

```rust
//! The ONE home of citation-marker (`[n]`) logic (CONTEXT.md: **Citation**).
//! Scanning, stripping (copy, history), and wikilink conversion (saved
//! answers) all live here; no other module may parse `[n]`.

pub struct CitationSpan {
    pub range: std::ops::Range<usize>,
    pub index: usize,
}

pub fn scan(text: &str) -> Vec<CitationSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            // at least one digit, closed by ']', not part of '[[…'
            if j > i + 1 && j < bytes.len() && bytes[j] == b']' {
                let index: usize = text[i + 1..j].parse().unwrap_or(0);
                if index > 0 {
                    spans.push(CitationSpan { range: start..j + 1, index });
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    spans
}

pub fn strip(text: &str) -> String {
    rewrite(text, |_| String::new())
}

pub fn link_sources(text: &str, source_names: &[String]) -> String {
    rewrite(text, |span| match source_names.get(span.index - 1) {
        Some(name) => format!("[[{name}]]"),
        None => text[span.range.clone()].to_string(),
    })
}

/// Shared splice loop: replace each scanned span via `f`, then collapse the
/// " ." / "  " droppings a removed marker leaves behind.
fn rewrite(text: &str, f: impl Fn(&CitationSpan) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for span in scan(text) {
        out.push_str(&text[last..span.range.start]);
        out.push_str(&f(&span));
        last = span.range.end;
    }
    out.push_str(&text[last..]);
    out.replace("  ", " ").replace(" .", ".").replace(" ,", ",")
}
```

(`mod.rs` for now: `pub mod citations;`)

- [ ] **Step 4: Run** — `cargo test -p kimun-notes --bins citations` → PASS
- [ ] **Step 5: Report.**

---

### Task 6: TUI — ask domain: `Thread`, `Turn`, `AskSource`

**Files:**
- Modify: `tui/src/ask/mod.rs`

**Interfaces:**
- Consumes: `citations::strip`; `kimun_server_client::dto::ChunkResult`; `kimun_core::nfs::VaultPath`.
- Produces:
  - `pub struct AskSource { pub path: VaultPath, pub heading: String, pub score: f64, pub text: String }` + `impl From<ChunkResult> for AskSource` (`path: VaultPath::new(&c.path)`, `heading: c.title`, `score: c.similarity_score`, `text: c.content`).
  - `pub enum TurnStatus { Thinking, Streaming, Done, Error(String) }` (Streaming reserved, never built v1).
  - `pub struct Turn { pub id: u64, pub question: String, pub answer: String, pub sources: Vec<AskSource>, pub status: TurnStatus }`
  - `pub struct Thread` with: `ask(&mut self, question: String) -> u64` · `complete(&mut self, id, answer: String, sources: Vec<AskSource>) -> bool` · `fail(&mut self, id, error: String) -> bool` · `regenerate(&mut self, id) -> Option<String>` (Done/Error turn → back to Thinking, returns the question; sources kept) · `history(&self) -> Vec<(String, String)>` (last `HISTORY_WINDOW = 5` Done turns *before the newest Thinking turn*, answers passed through `citations::strip`) · `selected(&self) -> Option<&Turn>` · `select_prev/select_next` · `select_last` · `clear` · `turns(&self) -> &[Turn]` · `is_empty`.
  - Stale safety: `complete`/`fail` return `false` (no-op) for an unknown id or a turn not in `Thinking` — a late answer after `clear()`/regenerate must never resurrect.

- [ ] **Step 1: Failing tests** (same file `#[cfg(test)]`)

```rust
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
```

- [ ] **Step 2: Verify failure** — `cargo test -p kimun-notes --bins ask::` → FAIL
- [ ] **Step 3: Implement** — `Thread { turns: Vec<Turn>, next_id: u64, selected: usize }`, `#[derive(Default)]`; `ask` pushes `Turn { id: self.bump(), status: TurnStatus::Thinking, answer: String::new(), sources: vec![], question }` and selects it; `history` iterates `turns` up to the last non-Done, filters `Done`, takes the trailing 5, maps `(question.clone(), citations::strip(&answer))`. Keep every method under ~10 lines.
- [ ] **Step 4: Run** — `cargo test -p kimun-notes --bins ask::` → PASS
- [ ] **Step 5: Report.**

---

### Task 7: Saved answer — core name helper + `ask::save`

**Files:**
- Modify: `core/src/nfs/filename.rs` (+ its tests)
- Create: `tui/src/ask/save.rs`

**Interfaces:**
- Core produces: `pub fn note_name_from_title(title: &str) -> String` — lowercase, disallowed chars (via existing `is_disallowed_char`) and whitespace → `-`, runs collapsed, trimmed of `-`, truncated to 60 chars on a char boundary, fallback `"answer"` when empty. Pure name, **no extension, no separators** — path assembly stays in `VaultPath`.
- TUI produces (`ask::save`):
  - `pub fn suggested_path(question: &str) -> VaultPath` = `VaultPath::new("ask").append(&VaultPath::new(note_name_from_title(question)))` (extension is applied by the existing create-note flow, same as every other new note).
  - `pub fn note_content(turn: &Turn) -> String` — `# {question}`, blank line, `citations::link_sources(answer, names)` where `names[i] = turn.sources[i].path.get_clean_name()`, then a `## Sources` footer listing each distinct `[[name]]`.

- [ ] **Step 1: Failing tests**

Core (`filename.rs`):

```rust
#[test]
fn note_name_from_title_slugs_and_survives_garbage() {
    assert_eq!(note_name_from_title("How do I Ship v2?"), "how-do-i-ship-v2");
    assert_eq!(note_name_from_title("///???"), "answer");
    assert!(note_name_from_title(&"x".repeat(200)).len() <= 60);
}
```

TUI (`save.rs`):

```rust
#[test]
fn note_content_links_citations_and_lists_sources() {
    let turn = turn_with(
        "Why kimün?",
        "Because notes [1]. And general knowledge.",
        vec![source("projects/kimun.md", "intro")],
    );
    let body = note_content(&turn);
    assert!(body.starts_with("# Why kimün?\n"));
    assert!(body.contains("Because notes [[kimun]]."));
    assert!(body.contains("## Sources"));
    assert!(body.contains("- [[kimun]]"));
}
```

(local helpers `turn_with`/`source` construct the Task 6 structs)

- [ ] **Step 2: Verify failure** — `cargo test -p kimun_core note_name_from_title` and `cargo test -p kimun-notes --bins save` → FAIL
- [ ] **Step 3: Implement** both functions (core first; keep `note_name_from_title` a single fold over chars + truncate; reuse `is_disallowed_char`).
- [ ] **Step 4: Run both** — PASS
- [ ] **Step 5: Report.**

---

### Task 8: TUI — `AskData` events + editor-area third arm

**Files:**
- Modify: `tui/src/components/events.rs`
- Modify: `tui/src/app_screen/panel_set.rs` (:208-:360 attachment plumbing)
- Create: `tui/src/components/ask_thread.rs` (state-only skeleton this task; render/input in Task 9)

**Interfaces:**
- Produces (`events.rs`):

```rust
/// Async data addressed to the Ask workspace. Its own family — Ask is a
/// panel, and `OverlayData` is routed only to the OverlayHost (adr/0030).
#[derive(Debug)]
pub enum AskData {
    /// A completed (or failed) answer for the turn with this id. Stale ids
    /// (cleared thread, superseded regenerate) are dropped by `Thread`.
    AnswerReady {
        turn_id: u64,
        result: Result<(String, Vec<crate::ask::AskSource>), String>,
    },
    /// The note text the source reader asked for. `None` = load failed.
    ReaderNote {
        path: VaultPath,
        text: Option<String>,
    },
}
```

  plus variant `AppEvent::Ask(AskData)`.
- Produces (`panel_set.rs`): `enum EditorAreaContent { Note, Attachment(AttachmentView), Ask(ThreadPanel) }` replacing `attachment: Option<AttachmentView>`. Public API preserved: `editor()` returns `Some` only for `Note`; `show_attachment`/`clear_attachment`/`is_showing_attachment` keep signatures. New: `show_ask(&mut self, panel: ThreadPanel)` (focuses the editor panel, same as `show_attachment`), `take_ask(&mut self) -> Option<ThreadPanel>` (back to `Note` — the screen stashes the panel so the thread survives view switches), `ask_mut(&mut self) -> Option<&mut ThreadPanel>`, `is_showing_ask(&self) -> bool`.
- `ThreadPanel` skeleton: `pub struct ThreadPanel { thread: Thread, composer: SingleLineInput, capability: bool, focus: ThreadFocus }`, `enum ThreadFocus { Composer, Turns }`, `pub fn new(bindings: KeyBindings, theme_seed: …) -> Self` matching however `AttachmentView` is constructed, `pub fn set_capability(&mut self, on: bool)`, `pub fn thread(&self) -> &Thread`, `pub fn thread_mut(&mut self) -> &mut Thread`. Render draws a placeholder line; input is a no-op until Task 9.

- [ ] **Step 1: Failing tests** (extend the existing `panel_set` test block — `focus_cycle_wraps_over_visible_panels` :756 shows the harness idiom)

```rust
#[test]
fn ask_content_hides_the_editor_and_take_restores_it() {
    let mut ps = test_panel_set(); // whatever helper the existing tests use
    assert!(ps.editor().is_some());
    ps.show_ask(test_thread_panel());
    assert!(ps.editor().is_none());
    assert!(ps.is_showing_ask());
    let panel = ps.take_ask().expect("panel comes back to the caller");
    let _ = panel; // thread state survives with it
    assert!(ps.editor().is_some());
}

#[test]
fn showing_an_attachment_replaces_ask() {
    let mut ps = test_panel_set();
    ps.show_ask(test_thread_panel());
    ps.show_attachment(test_attachment_view());
    assert!(!ps.is_showing_ask());
    assert!(ps.is_showing_attachment());
}
```

- [ ] **Step 2: Verify failure** — `cargo test -p kimun-notes --bins panel_set` → FAIL
- [ ] **Step 3: Implement** — mechanical: every `self.attachment.is_some()` → `matches!(self.content, EditorAreaContent::Attachment(_))` etc.; render/input dispatch gets an `Ask` arm mirroring the attachment arm. Keep the ADR-0017 comment updated to name three arms.
- [ ] **Step 4: Run** — `cargo test -p kimun-notes --bins` → PASS (whole bin — this refactor touches routing)
- [ ] **Step 5: Report.**

---

### Task 9: TUI — `ThreadPanel`: render, input, ask task

**Files:**
- Modify: `tui/src/components/ask_thread.rs`

**Interfaces:**
- Consumes: `Thread`/`TurnStatus` (Task 6), `citations::{scan, strip}` (Task 5), `ask::save` (Task 7), `RagClient::ask(query, history, None)` (Task 4), `AppTx` + `AskData` (Task 8), `SingleLineInput`, `panel_block`.
- Produces:
  - `pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx, client: Option<&Arc<RagClient>>) -> EventState` — match the signature style of `AttachmentView`'s input handler; adapt to whatever `Component` shape Task 8 compiled against.
  - `pub fn handle_data(&mut self, data: AskData)` — `AnswerReady` → `thread.complete/fail` (stale-safe by Task 6).
  - `pub fn selection_changed(&self) -> Option<u64>` — consumed by the editor screen to sync the Sources drawer (set on any j/k/submit, cleared on read; lean: `take_selection_dirty() -> Option<u64>`).
  - Submit path (Enter in composer, non-empty, `capability == true`):

```rust
fn submit(&mut self, tx: &AppTx, client: &Arc<RagClient>) {
    let question = self.composer.take_text();
    let history = self.thread.history();
    let turn_id = self.thread.ask(question.clone());
    let (tx, client) = (tx.clone(), client.clone());
    tokio::spawn(async move {
        let result = client
            .ask(&question, &history, None)
            .await
            .map(|a| (a.answer, a.sources.into_iter().map(AskSource::from).collect()))
            .map_err(|e| e.to_string());
        let _ = tx.send(AppEvent::Ask(AskData::AnswerReady { turn_id, result }));
    });
}
```

  ⚠ order: `history()` is read **before** `ask()` pushes the new turn — Task 6's `history` excludes the in-flight turn either way; keeping the read first makes that not load-bearing.
  - Keys — composer focused: `Enter` submit · `Esc` → `ThreadFocus::Turns`. Turns focused: `j`/`k` select (mark selection dirty) · `i` or `/` → composer · `y` copy `citations::strip(&turn.answer)` to the OS clipboard via the app's existing clipboard seam (grep for the Ctrl-c path in the editor; reuse, don't add a dependency) · `e` → send the existing create-note-dialog open event prefilled with `save::suggested_path(&turn.question)` + `save::note_content(turn)` (reuse `dialogs/create_note_dialog.rs`; extend its constructor with prefilled content if it only takes a path — that dialog already owns validation and the create call) · `r` → if `regenerate(id)` returns the question, respawn `submit`'s task with that question, **reusing the turn's kept sources on completion** — i.e. the spawned closure calls `client.ask` and on `Ok` passes only the answer text through, sources unchanged: send `AnswerReady { turn_id, result: Ok((answer, existing_sources)) }` (clone them before the spawn).
  - Render (lean, one method per concern): a scrolling list of turns — question line styled bold, answer as wrapped `Paragraph` lines where `citations::scan` spans get the accent style (superscript effect = colored `[n]`), `Thinking` → spinner-line, `Error(msg)` → error style + `r hint`; composer docked at the bottom inside `panel_block` (disabled style + "server unavailable" title when `capability == false`); selected turn carries the selection background. Mouse: click a citation span → same effect as selecting its source (mark selection dirty + remember `citation_target: Option<usize>` the screen reads to focus that source row); click a turn selects it.

- [ ] **Step 1: Failing tests** (state-level, no terminal)

```rust
#[test]
fn enter_submits_only_with_capability() {
    let mut p = test_panel(); // capability=false, composer focused, text "q"
    let sent = p_handle_enter(&mut p); // helper drives handle_input with a stub tx
    assert!(p.thread().is_empty(), "no capability → no turn");
    p.set_capability(true);
    // …enter again → one Thinking turn exists
}

#[test]
fn answer_ready_completes_matching_turn_only() {
    let mut p = test_panel_online();
    let id = p.thread_mut().ask("q".into());
    p.handle_data(AskData::AnswerReady { turn_id: 999, result: Ok(("x".into(), vec![])) });
    assert!(matches!(p.thread().selected().unwrap().status, TurnStatus::Thinking));
    p.handle_data(AskData::AnswerReady { turn_id: id, result: Ok(("a".into(), vec![])) });
    assert!(matches!(p.thread().selected().unwrap().status, TurnStatus::Done));
}
```

(The submit test needs no real client: factor `submit` so the test can call the pre-spawn half — e.g. `fn begin_turn(&mut self) -> Option<(String, Vec<(String,String)>, u64)>` returning `None` without capability; the spawn wrapper stays 5 lines and untested.)

- [ ] **Step 2: Verify failure** — `cargo test -p kimun-notes --bins ask_thread` → FAIL
- [ ] **Step 3: Implement** render/input/data per the interface block. Keep `render` split: `render_turns`, `render_turn`, `render_composer`.
- [ ] **Step 4: Run** — `cargo test -p kimun-notes --bins` → PASS
- [ ] **Step 5: Report.**

---

### Task 10: TUI — `SourcesPanel` drawer view with reader face

**Files:**
- Create: `tui/src/ask/locate.rs`
- Create: `tui/src/components/ask_sources.rs`

**Interfaces:**
- `locate` produces: `pub fn section_range(note_text: &str, heading: &str, chunk_text: &str) -> Option<std::ops::Range<usize>>` — resolution order: exact `chunk_text` substring match; else the `ContentChunk` (via `kimun_core::note::NoteDetails::chunks_and_links_of`) whose breadcrumb's innermost segment equals `heading` (case-insensitive) → range of its text within `note_text` (substring search of the chunk text); else `None` (reader shows the note from the top, unhighlighted). Pure function.
- `SourcesPanel` produces:
  - `pub fn set_turn(&mut self, turn_id: u64, sources: Vec<AskSource>)` — repopulates the list, resets to the list face; same `turn_id` = no-op (keeps cursor).
  - `pub fn open_reader(&mut self, source_index: usize, tx: &AppTx, vault: &NoteVault)` — spawns the note load (`vault` read via the same async call the attachment/preview paths use), flips to `Face::Reader { source_index, text: None }`.
  - `pub fn handle_data(&mut self, data: AskData)` — `ReaderNote { path, text }` accepted only if the reader face is waiting on that path (stale-drop, same discipline as everywhere).
  - Keys — list face: `j`/`k` move · `Enter`/`l` `open_reader(cursor)` · `o` emit `AppEvent::open(path)` (leaves Ask) · `y` yank the path string via the clipboard seam. Reader face: `j`/`k` scroll · `h`/`Esc` back to list · `o` as above.
  - Render — list face: one row per source — `{rank} {heading}` line + dimmed `{path} {score bar}` line, selected row highlighted; empty state label "no sources — ask something". Reader face: note text `Paragraph` with the `section_range` lines styled highlight (accent left bar via a `▌` prefix on highlighted lines), scrolled to the first highlighted line on load.
- Consumed by Task 11: `DrawerHost` field + `ask_sources_mut()`.

- [ ] **Step 1: Failing tests** (`locate` first — it's the risky logic)

```rust
#[test]
fn section_range_prefers_exact_chunk_text() {
    let note = "# a\nalpha body\n# b\nbeta body\n";
    let r = section_range(note, "b", "beta body").unwrap();
    assert_eq!(&note[r], "beta body");
}

#[test]
fn section_range_falls_back_to_heading_match() {
    let note = "# intro\nreal text here\n";
    // chunk text was normalized server-side and no longer matches verbatim
    let r = section_range(note, "INTRO", "normalized text").unwrap();
    assert!(note[r].contains("real text here"));
}

#[test]
fn section_range_gives_up_gracefully() {
    assert!(section_range("# x\nbody\n", "missing", "nope").is_none());
}
```

Panel state tests: `set_turn` same-id keeps cursor; `ReaderNote` for the wrong path is dropped.

- [ ] **Step 2: Verify failure** — `cargo test -p kimun-notes --bins locate` → FAIL
- [ ] **Step 3: Implement** `locate.rs`, then the panel (mirror `SemanticPanel`'s shape — `tui/src/components/semantic_search.rs` — it is the closest existing drawer view; the reader face borrows the scroll idiom from `preview_pane.rs`, do not import the whole `PreviewPane`).
- [ ] **Step 4: Run** — `cargo test -p kimun-notes --bins` → PASS
- [ ] **Step 5: Report.**

---

### Task 11: Integration — rail entry, drawer view, editor wiring, keys

**Files:**
- Modify: `tui/src/components/drawer.rs` (enum :22, labels :34, host :52)
- Modify: `tui/src/components/activity_rail.rs` (ITEMS :25, `new` :69, glyph :37)
- Modify: `tui/src/app_screen/editor.rs` (`open_rag_answer` :1431 region, RagStatus handling, leader tree :1732 region)
- Modify: `tui/src/keys/action_shortcuts.rs` (:52 label only)

**Interfaces:**
- `DrawerView::Ask`, label `"ASK"`, glyph `"?"` (ASCII-safe, matches the SEM precedent :42). Rail `ITEMS` grows to 8 with `("ASK", DrawerView::Ask)` after SEM; `ActivityRail::new(bindings, icons, semantic_visible, ask_visible)` filters it like SEM (:74-77).
- `DrawerHost` gains `ask_sources: SourcesPanel` + `ask_sources_mut()`; `is_text_input` unchanged (list view); dispatch arms mirror the SEM arms.
- Editor screen owns `ask_stash: Option<ThreadPanel>` and the wiring rules:
  1. Drawer switches **to** `Ask` → `panel_set.show_ask(self.ask_stash.take().unwrap_or_else(new_panel))`; sync `SourcesPanel` from the thread's selected turn.
  2. Drawer switches **away** from `Ask` → `self.ask_stash = panel_set.take_ask()` (thread survives; CONTEXT.md: Thread lifetime).
  3. `AppEvent::Ask(data)` → route `AnswerReady` to the live ThreadPanel **or the stash** (an answer may land while the user browses FILES), `ReaderNote` to `drawer.ask_sources_mut()`. After `AnswerReady`, if the completed turn is the selected one, refresh the SourcesPanel.
  4. `AppEvent::RagStatus(s)` (existing handler): when `s.llm_available()` changed, rebuild the rail with the new `ask_visible` (the SEM rebuild-on-config-change pattern) and call `set_capability` on the live panel and the stash. Active Ask view stays put when capability drops (adr/0030).
  5. `ActionShortcuts::OpenRagAnswer` (label → `"Ask"`, serialized name kept for config compat): now performs rule 1 — the flash-message gates at :1435-1453 collapse to a single `llm_available` check (hidden rail entry ⇒ shortcut is the only path in).
  6. Leader tree: `a` subtree — `a` focus composer (entering Ask if needed) · `n` `thread_mut().clear()` + sources reset · `y`/`e`/`r` forward to the ThreadPanel's turn actions · `s` open top source of the selected turn in the reader. Follow the existing `LeaderAction` enum + which-key registration pattern (:1732 shows the arm idiom).
  7. Turn-selection sync each frame: `if let Some(turn_id) = panel.take_selection_dirty() { drawer.ask_sources_mut().set_turn(turn_id, sources.clone()) }`, plus citation clicks focusing the matching source row.
- Footer: drawer label ASK ships via `DrawerView::label`; the editor-area content label for Ask mirrors the attachment-view labeling so the footer reads sensibly (find where the attachment label is produced and add the Ask arm).

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn rail_hides_ask_without_llm() {
    let rail = ActivityRail::new(test_bindings(), test_icons(), true, false);
    assert!(rail_views(&rail).contains(&DrawerView::Semantic));
    assert!(!rail_views(&rail).contains(&DrawerView::Ask));
}

#[test]
fn rail_shows_ask_with_llm() {
    let rail = ActivityRail::new(test_bindings(), test_icons(), true, true);
    assert!(rail_views(&rail).contains(&DrawerView::Ask));
}
```

Plus an editor-screen test in the `app_screen` idiom: switching drawer view Ask→Files→Ask preserves the thread (ask, stash, restore, assert `turns().len()` unchanged) — build it with the existing screen test harness used by the panel-set tests.

- [ ] **Step 2: Verify failure** — `cargo test -p kimun-notes --bins rail_` → FAIL
- [ ] **Step 3: Implement** in dependency order: enum+label → rail → drawer host arms → editor wiring 1-7 → shortcut relabel → leader subtree.
- [ ] **Step 4: Run** — `cargo test -p kimun-notes --bins` → PASS, then a manual smoke: `cargo run -p kimun-notes` against a configured server — ask, follow-up, j/k, reader, `e`, `r`, kill the server mid-thread (composer disables, thread stays), restart (recovers).
- [ ] **Step 5: Report** (including the smoke-test observations).

---

### Task 12: Delete the overlay + docs

**Files:**
- Delete: `tui/src/components/rag_answer.rs`
- Modify: `tui/src/components/overlay.rs` (:17 `OverlayKind::RagAnswer`, :21/:32 label arms), `tui/src/components/events.rs` (:244 `OverlayData::RagAnswerReady`), `tui/src/rag/mod.rs` (:41-54 `RagAnswer`/`RagSource` structs + doc comment :12), `tui/src/app_screen/editor.rs` (`OverlayOpen::RagAnswer` :822 and any residual arms)
- Modify: `docs/` — the user-facing page describing Ask (find the existing RAG/ask user doc; rewrite for the workspace: entering via rail/shortcut, follow-ups, sources + reader keys, `y`/`e`/`r`, the offline behavior). End-user voice only — no implementation detail (project CLAUDE.md).

**Interfaces:**
- Consumes: everything above. Produces: a compiling workspace with zero references to the overlay path.

- [ ] **Step 1: Delete + fix compile** — remove the file, chase `cargo check --workspace` errors; each error site is either dead (delete) or was migrated in Task 11 (verify, then delete the old arm).
- [ ] **Step 2: Grep for stragglers**

Run: `grep -rn "RagAnswer\|rag_answer" tui/src client/src server/src`
Expected: zero hits (the `rag/` module keeps only `RagStatus` + client/sync helpers)

- [ ] **Step 3: Full suite**

Run: `cargo test --workspace` and `cargo test -p kimun-notes --bins`
Expected: PASS

- [ ] **Step 4: Docs** — update the Ask user page; check `docs/` nav/index references the renamed surface.
- [ ] **Step 5: Report** — final green summary across all four crates.

---

## Self-Review Notes

- **Spec coverage** against the grill decisions: surface + gating (T11), degradation (T9 capability + T11 rule 4), no streaming (constraint; `TurnStatus::Streaming` reserved in T6), history (T2-T4, T6), citations + prompt policy (T1, T5), sources + reader with heading fallback (T10), copy/save/regenerate incl. wikilink conversion (T7, T9), context size `None` (T9 submit), thread lifetime (T6 `clear` + T11 stash), keyboard model (T9/T10/T11), overlay deletion (T12). Out of scope v1 (multi-vault ask, saved threads, streaming) — untouched, per handoff.
- **Type consistency**: history is `&[(String, String)]` at every layer boundary (server trait, `KimunRag::answer`, client `ask`, `Thread::history`); `AskSource` conversion defined once (T6) and used by T9's spawn; `turn_id: u64` everywhere; `HistoryTurn` exists twice by design (server deserialize, client serialize — the dto.rs header mandates independence).
- **Known judgment calls left to the implementer** (bounded, non-blocking): exact constructor shapes matched to `AttachmentView`/`SemanticPanel` idioms; clipboard seam reuse; create-note-dialog prefill extension; footer label site. Each names its reference file.
