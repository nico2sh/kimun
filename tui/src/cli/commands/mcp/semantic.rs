//! The `semantic_search` tool: a filtered search on the Kimun server, with the
//! hits rendered the way the other note tools render theirs.

use std::time::Duration;

use kimun_core::error::VaultError;
use kimun_core::note::BREADCRUMB_SEP;
use kimun_core::{DateSource, NoteVault};
use rmcp::model::{CallToolResult, Content};

use super::SemanticSearchParams;
pub(super) use super::info::PROBE_TIMEOUT;
use super::info::{SemanticState, probe_state};
use super::results::{RangeError, cap_chars, parse_date_key, present, resolve_range};
use crate::server_client::dto::{ChunkResult, SearchFilter};
use crate::server_client::{ContextSize, RagError};
use crate::settings::SharedSettings;

/// How long the tool waits for the server.
pub(super) const TIMEOUT: Duration = Duration::from_secs(20);
/// The refusal for a call with no query.
const RANGE_WITHOUT_QUERY: &str = "For a date range with no query, use search_notes, which lists every note in the range newest first.";

/// Hits returned when the caller gives no `limit`.
const DEFAULT_HITS: usize = 10;
/// The most hits a caller can ask for.
const MAX_HITS: usize = 40;
/// Longest chunk shown, in characters.
const CHUNK_CHARS: usize = 400;
/// What the server says when a filtered search runs on a Qdrant store.
const QDRANT_REFUSAL: &str = "filtered search is not supported on the qdrant store";

fn tool_error(msg: impl Into<String>) -> Result<CallToolResult, VaultError> {
    Ok(CallToolResult::error(vec![Content::text(msg.into())]))
}

/// The date key as the server stores it: `journal`, or the bare lowercase
/// property key. The server key space cannot tell a journal date from a date
/// property named journal, so `property:journal` also becomes `journal`.
fn server_date_key(key: &str) -> String {
    match parse_date_key(key) {
        DateSource::Journal => "journal".to_string(),
        DateSource::Property(k) => k.to_lowercase(),
    }
}

fn context_size(limit: usize) -> ContextSize {
    match limit {
        0..=10 => ContextSize::Small,
        11..=20 => ContextSize::Medium,
        _ => ContextSize::Large,
    }
}

/// The tool error text for a failed request.
fn failure_text(e: &RagError) -> String {
    match e {
        RagError::Status { status: 401 | 403, .. } => {
            "The server rejected the token. Set kimun_server_token in the kimun configuration to a valid token.".to_string()
        }
        RagError::Status { body, .. } if body.contains(QDRANT_REFUSAL) => {
            "The server stores its vectors in Qdrant, which cannot filter a search. Search again without from, to, date_key, folder or tags, or use search_notes for those filters.".to_string()
        }
        RagError::Http(_) => SemanticState::Unreachable
            .hint()
            .unwrap_or_default()
            .to_string(),
        RagError::Protocol(msg) if msg.contains("too old to filter") => msg.clone(),
        other => format!("Semantic search failed: {other}"),
    }
}

fn render_hit(hit: &ChunkResult, position: usize) -> String {
    // The title is the section path. Core joins its headings with a control
    // character, so show a visible separator to the agent.
    let title = hit.title.replace(BREADCRUMB_SEP, " › ");
    let mut out = format!("{} — {}", hit.path, title);
    if let Some(date) = hit.date.as_deref().filter(|d| !d.is_empty()) {
        out.push_str(&format!("\n  date: {date}"));
    }
    let rank = if hit.ordinal == 0 {
        position
    } else {
        hit.ordinal
    };
    out.push_str(&format!("\n  score: {:.2}", hit.similarity_score));
    out.push_str(&format!("\n  rank: {rank}"));
    let flat = hit.content.split_whitespace().collect::<Vec<_>>().join(" ");
    out.push_str(&format!("\n  chunk: {}", cap_chars(&flat, CHUNK_CHARS)));
    out
}

/// Runs the tool. `timeout` bounds the search request and `probe_timeout`
/// bounds the health probe before it.
pub(super) async fn run(
    vault: &NoteVault,
    settings: Option<&SharedSettings>,
    p: SemanticSearchParams,
    timeout: Duration,
    probe_timeout: Duration,
) -> Result<CallToolResult, VaultError> {
    let Some(query) = present(p.query.as_deref()) else {
        // The server ranks by similarity to the query, so it cannot list a
        // range. search_notes can (ruling R11).
        return tool_error(RANGE_WITHOUT_QUERY);
    };
    let range = match resolve_range(
        vault,
        p.from.as_deref(),
        p.to.as_deref(),
        p.date_key.as_deref(),
    )
    .await
    {
        Ok(r) => r,
        Err(RangeError::Tool(m)) => return tool_error(m),
        Err(RangeError::Vault(e)) => return Err(e),
    };
    let state = match settings {
        Some(s) => probe_state(s, vault, probe_timeout).await,
        None => SemanticState::None,
    };
    if let Some(hint) = state.hint() {
        return tool_error(hint);
    }
    let client = match settings {
        Some(s) => crate::rag::rag_client(s, vault).await,
        None => None,
    };
    let Some(client) = client else {
        return tool_error(SemanticState::None.hint().unwrap_or_default());
    };
    let limit = p
        .limit
        .map(|l| l as usize)
        .unwrap_or(DEFAULT_HITS)
        .clamp(1, MAX_HITS);
    let filter = SearchFilter {
        date_from: range.as_ref().map(|r| r.from),
        date_to: range.as_ref().map(|r| r.to),
        date_key: range.as_ref().map(|r| server_date_key(&r.key)),
        folder: present(p.folder.as_deref()).map(str::to_string),
        tags: p
            .tags
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
    };
    let sent = client.search_filtered(query, &filter, Some(context_size(limit)));
    let hits = match tokio::time::timeout(timeout, sent).await {
        Err(_) => {
            return tool_error(
                "The Kimun server did not answer in time. Try again, or check that the server is running and not overloaded.",
            );
        }
        Ok(Err(e)) => return tool_error(failure_text(&e)),
        Ok(Ok(hits)) => hits,
    };
    if hits.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(
            "No matching notes.",
        )]));
    }
    let text = hits
        .iter()
        .take(limit)
        .enumerate()
        .map(|(i, h)| render_hit(h, i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::mcp::KimunHandler;
    use crate::settings::AppSettings;
    use crate::settings::workspace_config::WorkspaceConfig;
    use kimun_core::VaultConfig;
    use kimun_core::nfs::VaultPath;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::RawContent;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex, RwLock};
    use tempfile::TempDir;

    const READY: &str =
        r#"{"status":"ok","embedder":"fake","llm_provider":null,"search_filters":true}"#;

    /// A one-thread HTTP server. `GET /health` returns `health`. Any other
    /// request waits `delay`, then returns `status` and `body`. Each request
    /// is logged as `"<METHOD> <path> <body>"`.
    fn server(
        health: &'static str,
        status: u16,
        body: &'static str,
        delay: Duration,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let log = log.clone();
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let n = stream.read(&mut chunk).unwrap_or(0);
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                            let len = head
                                .lines()
                                .find_map(|l| {
                                    l.to_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap())
                                })
                                .unwrap_or(0);
                            if rest.len() >= len || n == 0 {
                                break;
                            }
                        } else if n == 0 {
                            break;
                        }
                    }
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                    let mut first = head.lines().next().unwrap_or("").split(' ');
                    let method = first.next().unwrap_or("");
                    let path = first.next().unwrap_or("");
                    log.lock().unwrap().push(format!("{method} {path} {rest}"));
                    let (code, reply) = if path == "/health" {
                        (200, health)
                    } else {
                        std::thread::sleep(delay);
                        (status, body)
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        reply.len(),
                        reply
                    );
                });
            }
        });
        (url, seen)
    }

    fn settings_for(url: Option<&str>) -> SharedSettings {
        let mut config = WorkspaceConfig::new_empty();
        config.global.kimun_server_url = url.map(str::to_string);
        let mut settings = AppSettings::default();
        settings.workspace_config = Some(config);
        Arc::new(RwLock::new(settings))
    }

    async fn handler_for(dir: &TempDir, settings: Option<SharedSettings>) -> KimunHandler {
        let vault = NoteVault::new(VaultConfig::new(crate::test_support::sys(dir.path())))
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        let handler = KimunHandler::new(vault);
        match settings {
            Some(s) => handler.with_settings(s),
            None => handler,
        }
    }

    fn text_of(r: &CallToolResult) -> String {
        match &r.content[0].raw {
            RawContent::Text(t) => t.text.clone(),
            other => panic!("expected text, got {other:?}"),
        }
    }

    fn q(query: &str) -> SemanticSearchParams {
        SemanticSearchParams {
            query: Some(query.into()),
            ..Default::default()
        }
    }

    async fn call(handler: &KimunHandler, p: SemanticSearchParams) -> CallToolResult {
        handler.semantic_search(Parameters(p)).await.unwrap()
    }

    fn embeddings_body(log: &Arc<Mutex<Vec<String>>>) -> serde_json::Value {
        let log = log.lock().unwrap();
        let post = log
            .iter()
            .rfind(|l| l.starts_with("POST /api/embeddings"))
            .unwrap_or_else(|| panic!("no search was sent: {log:?}"));
        serde_json::from_str(post.strip_prefix("POST /api/embeddings ").unwrap()).unwrap()
    }

    /// Gives the vault a journal date and a `due` date property.
    async fn add_dated_notes(h: &KimunHandler) {
        h.vault
            .create_note(&VaultPath::new("/journal/2026-11-20.md"), "# day\n")
            .await
            .unwrap();
        h.vault
            .create_note(&VaultPath::new("task.md"), "---\ndue: 2026-11-25\n---\nx\n")
            .await
            .unwrap();
        h.vault.validate_and_init().await.unwrap();
    }

    const ONE_HIT: &str = r#"{"chunks":[{"path":"/work/plan.md","title":"Plan","date":"2026-11-20","content":"the chunk text","hash":"h","similarity_score":0.8312,"ordinal":1}]}"#;

    #[tokio::test]
    async fn semantic_hit_renders_fields() {
        let (url, _) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let r = call(&h, q("plans")).await;
        assert_eq!(r.is_error, Some(false));
        let text = text_of(&r);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "/work/plan.md — Plan");
        assert_eq!(lines[1], "  date: 2026-11-20");
        assert_eq!(lines[2], "  score: 0.83");
        assert_eq!(lines[3], "  rank: 1");
        assert_eq!(lines[4], "  chunk: the chunk text");
    }

    #[tokio::test]
    async fn semantic_hit_title_shows_the_section_path_with_a_visible_separator() {
        // The server stores the section path with the control character U+001F
        // between the headings. The agent must see a visible separator instead.
        let body: &'static str = r#"{"chunks":[{"path":"/journal/2026-11-22.md","title":"2026-11-22\u001fMorning\u001fCall","content":"text","hash":"h","similarity_score":0.69,"ordinal":1}]}"#;
        let (url, _) = server(READY, 200, body, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let text = text_of(&call(&h, q("calls")).await);
        let first = text.lines().next().unwrap();
        assert_eq!(
            first,
            "/journal/2026-11-22.md — 2026-11-22 › Morning › Call"
        );
        assert!(
            !text.contains('\u{1f}'),
            "no control character may reach the agent"
        );
    }

    #[tokio::test]
    async fn semantic_hit_without_date_omits_the_line_and_caps_the_chunk() {
        let long = "x".repeat(500);
        let body: &'static str = Box::leak(
            format!(
                r#"{{"chunks":[{{"path":"/a.md","title":"A","content":"{long}","hash":"h","similarity_score":0.5,"ordinal":0}}]}}"#
            )
            .into_boxed_str(),
        );
        let (url, _) = server(READY, 200, body, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let text = text_of(&call(&h, q("a")).await);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[1], "  score: 0.50");
        assert_eq!(lines[2], "  rank: 1", "rank 0 falls back to the position");
        assert_eq!(lines[3].chars().count(), "  chunk: ".len() + 400);
        assert!(lines[3].ends_with('…'));
    }

    #[tokio::test]
    async fn semantic_no_hits_says_so() {
        let (url, _) = server(READY, 200, r#"{"chunks":[]}"#, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let r = call(&h, q("nothing")).await;
        assert_eq!(r.is_error, Some(false));
        assert_eq!(text_of(&r), "No matching notes.");
    }

    #[tokio::test]
    async fn semantic_limit_truncates_and_picks_the_context_size() {
        let (url, seen) = server(
            READY,
            200,
            r#"{"chunks":[
            {"path":"/a.md","title":"A","content":"a","hash":"1","similarity_score":0.9,"ordinal":1},
            {"path":"/b.md","title":"B","content":"b","hash":"2","similarity_score":0.8,"ordinal":2}]}"#,
            Duration::ZERO,
        );
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let mut p = q("x");
        p.limit = Some(1);
        let text = text_of(&call(&h, p).await);
        assert!(text.contains("/a.md") && !text.contains("/b.md"), "{text}");
        assert_eq!(embeddings_body(&seen)["context_size"], "small");
        let mut p = q("x");
        p.limit = Some(15);
        call(&h, p).await;
        assert_eq!(embeddings_body(&seen)["context_size"], "medium");
        let mut p = q("x");
        p.limit = Some(99);
        call(&h, p).await;
        assert_eq!(embeddings_body(&seen)["context_size"], "large");
    }

    #[tokio::test]
    async fn semantic_error_no_server_configured() {
        let dir = TempDir::new().unwrap();
        for settings in [None, Some(settings_for(None))] {
            let h = handler_for(&dir, settings).await;
            let r = call(&h, q("x")).await;
            assert_eq!(r.is_error, Some(true));
            assert!(text_of(&r).contains("kimun_server_url"), "{}", text_of(&r));
        }
    }

    #[tokio::test]
    async fn semantic_error_server_unreachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let r = call(&h, q("x")).await;
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("does not answer"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn semantic_error_server_unconfigured() {
        let (url, seen) = server(
            r#"{"status":"ok","embedder":null,"llm_provider":null}"#,
            200,
            "{}",
            Duration::ZERO,
        );
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let r = call(&h, q("x")).await;
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("no embedder"), "{}", text_of(&r));
        assert!(seen.lock().unwrap().iter().all(|l| !l.starts_with("POST")));
    }

    #[tokio::test]
    async fn semantic_error_token_rejected() {
        for code in [401, 403] {
            let (url, _) = server(READY, code, r#"{"error":"no"}"#, Duration::ZERO);
            let dir = TempDir::new().unwrap();
            let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
            let r = call(&h, q("x")).await;
            assert_eq!(r.is_error, Some(true));
            assert!(
                text_of(&r).contains("kimun_server_token"),
                "{}",
                text_of(&r)
            );
        }
    }

    #[tokio::test]
    async fn semantic_error_filter_on_qdrant() {
        let (url, _) = server(
            READY,
            500,
            r#"{"error":"filtered search is not supported on the qdrant store"}"#,
            Duration::ZERO,
        );
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let mut p = q("x");
        p.folder = Some("work".into());
        let r = call(&h, p).await;
        assert_eq!(r.is_error, Some(true));
        let text = text_of(&r);
        assert!(text.contains("Qdrant"), "{text}");
        assert!(text.contains("without"), "{text}");
    }

    #[tokio::test]
    async fn semantic_other_server_error_keeps_the_body() {
        let (url, _) = server(READY, 500, "boom", Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let r = call(&h, q("x")).await;
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("boom"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn semantic_error_server_too_old_for_filters() {
        let (url, seen) = server(
            r#"{"status":"ok","embedder":"fake","llm_provider":null}"#,
            200,
            "{}",
            Duration::ZERO,
        );
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let mut p = q("x");
        p.tags = Some(vec!["a".into()]);
        let r = call(&h, p).await;
        assert_eq!(r.is_error, Some(true));
        assert!(
            text_of(&r).contains("This server is too old to filter a search. Update the server."),
            "{}",
            text_of(&r)
        );
        assert!(seen.lock().unwrap().iter().all(|l| !l.starts_with("POST")));
    }

    #[tokio::test]
    async fn semantic_timeout_returns_a_tool_error() {
        let (url, _) = server(READY, 200, ONE_HIT, Duration::from_millis(1500));
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let started = std::time::Instant::now();
        let r = run(
            &h.vault,
            h.settings.as_ref(),
            q("x"),
            Duration::from_millis(100),
            PROBE_TIMEOUT,
        )
        .await
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(1200));
        assert_eq!(r.is_error, Some(true));
        assert!(
            text_of(&r).contains("did not answer in time"),
            "{}",
            text_of(&r)
        );
    }

    #[tokio::test]
    async fn semantic_range_without_date_key_errors_listing_keys() {
        let (url, seen) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        h.vault
            .create_note(&VaultPath::new("a.md"), "---\ndue: 2026-11-25\n---\nx\n")
            .await
            .unwrap();
        h.vault.validate_and_init().await.unwrap();
        let mut p = q("x");
        p.from = Some("2026-11-01".into());
        let r = call(&h, p).await;
        assert_eq!(r.is_error, Some(true));
        let text = text_of(&r);
        assert!(text.contains("date_key is required"), "{text}");
        assert!(text.contains("due"), "{text}");
        assert!(seen.lock().unwrap().iter().all(|l| !l.starts_with("POST")));
    }

    #[tokio::test]
    async fn semantic_invalid_date_names_the_format() {
        let (url, _) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let mut p = q("x");
        p.from = Some("25/11/2026".into());
        p.date_key = Some("journal".into());
        let r = call(&h, p).await;
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("YYYY-MM-DD"));
    }

    #[tokio::test]
    async fn semantic_neither_query_nor_range_is_an_error() {
        let (url, _) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        for query in [None, Some("  ".to_string())] {
            let r = call(
                &h,
                SemanticSearchParams {
                    query,
                    ..Default::default()
                },
            )
            .await;
            assert_eq!(r.is_error, Some(true));
            assert!(text_of(&r).contains("date range"), "{}", text_of(&r));
        }
    }

    #[tokio::test]
    async fn semantic_filters_reach_the_server_body() {
        let (url, seen) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        add_dated_notes(&h).await;
        let p = SemanticSearchParams {
            query: Some("plans".into()),
            from: Some("2026-11-01".into()),
            to: Some("2026-11-30".into()),
            date_key: Some("Due".into()),
            folder: Some("work".into()),
            tags: Some(vec!["a".into(), "b".into()]),
            limit: None,
        };
        let r = call(&h, p).await;
        assert_eq!(r.is_error, Some(false), "{}", text_of(&r));
        let body = embeddings_body(&seen);
        assert_eq!(body["query"], "plans");
        assert_eq!(body["date_from"], "2026-11-01");
        assert_eq!(body["date_to"], "2026-11-30");
        assert_eq!(body["date_key"], "due");
        assert_eq!(body["folder"], "work");
        assert_eq!(body["tags"], serde_json::json!(["a", "b"]));
    }

    #[tokio::test]
    async fn semantic_date_keys_map_for_the_server() {
        let (url, seen) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        add_dated_notes(&h).await;
        for (given, sent) in [("journal", "journal"), ("property:Due", "due")] {
            let mut p = q("x");
            p.from = Some("2026-11-01".into());
            p.date_key = Some(given.into());
            let r = call(&h, p).await;
            assert_eq!(r.is_error, Some(false), "{}", text_of(&r));
            assert_eq!(embeddings_body(&seen)["date_key"], sent);
        }
    }

    #[tokio::test]
    async fn semantic_from_alone_fills_the_open_end() {
        let (url, seen) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        add_dated_notes(&h).await;
        let mut p = q("x");
        p.from = Some("2026-11-01".into());
        p.date_key = Some("journal".into());
        call(&h, p).await;
        let body = embeddings_body(&seen);
        assert_eq!(body["date_from"], "2026-11-01");
        assert_eq!(body["date_to"], "9999-12-31");
        let mut p = q("x");
        p.to = Some("2026-11-30".into());
        p.date_key = Some("journal".into());
        call(&h, p).await;
        let body = embeddings_body(&seen);
        assert_eq!(body["date_from"], "0001-01-01");
        assert_eq!(body["date_to"], "2026-11-30");
    }

    #[tokio::test]
    async fn semantic_range_without_query_is_refused() {
        let (url, seen) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        add_dated_notes(&h).await;
        for query in [None, Some(String::new()), Some("  ".to_string())] {
            let p = SemanticSearchParams {
                query,
                from: Some("2026-11-01".into()),
                to: Some("2026-11-30".into()),
                date_key: Some("journal".into()),
                ..Default::default()
            };
            let r = call(&h, p).await;
            assert_eq!(r.is_error, Some(true));
            assert_eq!(
                text_of(&r),
                "For a date range with no query, use search_notes, which lists every note in the range newest first."
            );
        }
        assert!(seen.lock().unwrap().is_empty(), "nothing is sent");
    }

    #[tokio::test]
    async fn semantic_unknown_date_key_errors_listing_keys() {
        let (url, seen) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        add_dated_notes(&h).await;
        let mut p = q("x");
        p.from = Some("2026-11-01".into());
        p.date_key = Some("Deadline".into());
        let r = call(&h, p).await;
        assert_eq!(r.is_error, Some(true));
        assert_eq!(
            text_of(&r),
            "Unknown date property 'deadline'. Date sources in this vault: due, journal"
        );
        assert!(seen.lock().unwrap().iter().all(|l| !l.starts_with("POST")));
    }

    #[tokio::test]
    async fn semantic_blank_arguments_count_as_absent() {
        let (url, seen) = server(READY, 200, ONE_HIT, Duration::ZERO);
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        add_dated_notes(&h).await;
        let p = SemanticSearchParams {
            query: Some("plans".into()),
            from: Some("".into()),
            to: Some(" ".into()),
            date_key: Some("".into()),
            folder: Some(" ".into()),
            tags: Some(vec!["".into()]),
            limit: None,
        };
        let r = call(&h, p).await;
        assert_eq!(r.is_error, Some(false), "{}", text_of(&r));
        let body = embeddings_body(&seen);
        for field in ["date_from", "date_to", "date_key", "folder", "tags"] {
            assert!(body[field].is_null(), "{field} sent: {body}");
        }
        // A blank key with a range is a missing key.
        let mut p = q("plans");
        p.from = Some("2026-11-01".into());
        p.date_key = Some("  ".into());
        let r = call(&h, p).await;
        assert_eq!(r.is_error, Some(true));
        let text = text_of(&r);
        assert!(text.contains("date_key is required"), "{text}");
        assert!(text.contains("due, journal"), "{text}");
    }

    #[tokio::test]
    async fn semantic_probe_that_never_answers_counts_as_unreachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        // Accept and hold every connection without a reply.
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                held.push(stream);
            }
        });
        let dir = TempDir::new().unwrap();
        let h = handler_for(&dir, Some(settings_for(Some(&url)))).await;
        let started = std::time::Instant::now();
        let r = run(
            &h.vault,
            h.settings.as_ref(),
            q("x"),
            Duration::from_secs(20),
            Duration::from_millis(50),
        )
        .await
        .unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(r.is_error, Some(true));
        assert!(text_of(&r).contains("does not answer"), "{}", text_of(&r));
    }
}
