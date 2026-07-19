//! Server admin web UI: a small server-rendered dashboard (maud templates plus
//! a sprinkle of inline JS for job polling). Everything is embedded in the
//! binary — no build step, no external assets, no node.
//!
//! Scope (P5): view the running configuration, browse collections, watch jobs,
//! and run a test query. Config edits are **persisted to the TOML file and take
//! effect on restart** — the embedder, vector store, and LLM client are all
//! built at startup (changing the embedder even changes the vector width), so
//! the running instance is never mutated live. The page says as much, and the
//! Restart button triggers that restart in-process (adr/0028): the binary's
//! serving loop drains, re-reads the file, and rebinds.
//!
//! Auth reuses the server's bearer token: a login form exchanges the token for
//! an `HttpOnly` session cookie holding that same shared secret. With no token
//! configured the UI is open (matching the API's localhost-dev posture).

mod assets;
mod config;
mod login;
mod pages;
mod shell;

use std::sync::Arc;

use axum::{
    Router, middleware,
    routing::{get, post},
};

use crate::auth::session::web_auth;
use crate::server_state::AppState;
use assets::{font_asset, image_asset};
use config::{config_page, config_submit, restart_submit};
use login::{login_page, login_submit, logout};
use pages::{
    collections_page, dashboard, jobs_fragment, jobs_page, logs_page, query_page, query_submit,
};

/// Web-UI routes. Returned without state applied (main calls `.with_state`); the
/// auth middleware captures its own state clone.
pub fn routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let protected = Router::new()
        .route("/", get(dashboard))
        .route("/config", get(config_page).post(config_submit))
        .route("/restart", post(restart_submit))
        .route("/collections", get(collections_page))
        .route("/jobs", get(jobs_page))
        .route("/jobs/fragment", get(jobs_fragment))
        .route("/logs", get(logs_page))
        .route("/query", get(query_page).post(query_submit))
        .route("/logout", get(logout))
        .route_layer(middleware::from_fn_with_state(state, web_auth))
        // Outermost on the protected routes: cross-origin mutating requests
        // are rejected before auth even runs (see `csrf_guard`).
        .route_layer(middleware::from_fn(crate::auth::session::csrf_guard));

    Router::new()
        .merge(protected)
        .route("/login", get(login_page).post(login_submit))
        .route("/assets/fonts/{file}", get(font_asset))
        .route("/assets/img/{file}", get(image_asset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KimunRag;
    use crate::auth::session::session_value;
    use crate::config::RagConfig;
    use crate::test_support::{FakeEmbedder, FakeLlm, FakeVectorStore};
    use axum::body::{Body, to_bytes};
    use axum::http::header::{COOKIE, HOST, ORIGIN, SET_COOKIE};
    use axum::http::{Request, StatusCode, header::CONTENT_TYPE};
    use axum::response::IntoResponse;
    use tower::ServiceExt;

    fn state(token: Option<&str>, config_path: Option<std::path::PathBuf>) -> Arc<AppState> {
        let config_toml = format!(
            r#"
[server]
[vector_db]
type = "qdrant"
[llm]
provider = "gemini"
[reranker]
{}
"#,
            token
                .map(|t| format!("[auth]\ntoken = \"{t}\""))
                .unwrap_or_default()
        );
        let config: RagConfig = toml::from_str(&config_toml).unwrap();
        let rag = KimunRag::new(
            Arc::new(FakeVectorStore::default()),
            Arc::new(FakeEmbedder),
            Some(Arc::new(FakeLlm)),
        );
        let mut st = AppState::new(Some(rag), config);
        if let Some(p) = config_path {
            st = st.with_config_path(p);
        }
        Arc::new(st)
    }

    fn app(state: Arc<AppState>) -> Router {
        Router::new().merge(routes(state.clone())).with_state(state)
    }

    async fn body_text(resp: axum::response::Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn restart_without_a_wired_loop_is_an_honest_failure() {
        // Tests (and any embedding without the binary's loop) have no restart
        // channel: the endpoint must say so, not pretend to restart.
        let resp = app(state(None, None))
            .oneshot(Request::post("/restart").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("not available"));
    }

    #[tokio::test]
    async fn restart_signals_the_loop_and_blocks_cross_origin() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let config: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n").unwrap();
        let rag = KimunRag::new(
            Arc::new(FakeVectorStore::default()),
            Arc::new(FakeEmbedder),
            None,
        );
        let st = Arc::new(AppState::new(Some(rag), config).with_restart(tx));

        // Cross-origin POST is rejected before anything is signalled.
        let resp = app(st.clone())
            .oneshot(
                Request::post("/restart")
                    .header(ORIGIN, "http://evil.example")
                    .header(HOST, "localhost:7573")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(rx.try_recv().is_err(), "no signal on a rejected request");

        // Same-origin POST signals the serving loop and says what happens next.
        let resp = app(st)
            .oneshot(Request::post("/restart").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("Restarting"));
        assert!(rx.try_recv().is_ok(), "the loop got the restart signal");
    }

    #[tokio::test]
    async fn dashboard_renders_when_open() {
        let app = app(state(None, None));
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("Dashboard"));
    }

    #[tokio::test]
    async fn collections_page_lists_store_collections() {
        let app = app(state(None, None));
        let resp = app
            .oneshot(Request::get("/collections").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(body_text(resp).await.contains("vault-1"));
    }

    #[tokio::test]
    async fn protected_route_redirects_to_login_without_cookie() {
        let app = app(state(Some("secret"), None));
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers()["location"], "/login");
    }

    #[tokio::test]
    async fn valid_session_cookie_grants_access() {
        let app = app(state(Some("secret"), None));
        let cookie = format!("kimun_session={}", session_value("secret"));
        let resp = app
            .oneshot(
                Request::get("/")
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn raw_token_cookie_is_rejected() {
        // The cookie must carry the hash, not the token itself.
        let app = app(state(Some("secret"), None));
        let resp = app
            .oneshot(
                Request::get("/")
                    .header(COOKIE, "kimun_session=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn login_sets_cookie_on_correct_token() {
        let app = app(state(Some("secret"), None));
        let resp = app
            .oneshot(
                Request::post("/login")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("token=secret"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let cookie = resp.headers()[SET_COOKIE].to_str().unwrap();
        assert!(cookie.contains(&format!("kimun_session={}", session_value("secret"))));
        assert!(!cookie.contains("kimun_session=secret;")); // not the raw token
        assert!(cookie.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn login_survives_token_with_special_chars() {
        // A token containing ';' and a space must not break the cookie or lock
        // out the admin (regression: raw-token cookie).
        let token = "a b;c";
        let app = app(state(Some(token), None));
        let resp = app
            .clone()
            .oneshot(
                Request::post("/login")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    // URL-encoded form value for "a b;c".
                    .body(Body::from("token=a%20b%3Bc"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let set = resp.headers()[SET_COOKIE].to_str().unwrap().to_string();
        // The session cookie value round-trips: use it to reach a protected page.
        let jar = set.split(';').next().unwrap().to_string();
        let resp2 = app
            .oneshot(
                Request::get("/")
                    .header(COOKIE, jar)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn config_submit_persists_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let app = app(state(None, Some(path.clone())));
        let form = "host=127.0.0.1&port=7573&provider=claude&model=my-model&api_key=&embedder_provider=none&fastembed_model=&embedder_url=&embedder_model=&embedder_api_key=&vector_db=qdrant&sqlite_path=&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("Saved"));

        let saved = RagConfig::from_file(path).unwrap();
        let llm = saved.llm.as_ref().expect("llm saved");
        assert_eq!(llm.provider(), "claude");
        assert_eq!(llm.model(), "my-model");
    }

    /// Builds an AppState from a full config TOML, with a writable config path.
    fn state_from(config_toml: &str, path: std::path::PathBuf) -> Arc<AppState> {
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        let rag = KimunRag::new(
            Arc::new(FakeVectorStore::default()),
            Arc::new(FakeEmbedder),
            Some(Arc::new(FakeLlm)),
        );
        Arc::new(AppState::new(Some(rag), config).with_config_path(path))
    }

    /// An unconfigured server: no embedder in config, no pipeline (adr/0024).
    fn unconfigured_state() -> Arc<AppState> {
        let config: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\n").unwrap();
        assert!(config.embedder.is_none());
        Arc::new(AppState::new(None, config))
    }

    #[tokio::test]
    async fn unconfigured_dashboard_points_to_config() {
        let app = app(unconfigured_state());
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_text(resp).await;
        assert!(
            html.contains("unconfigured"),
            "dashboard must flag the state"
        );
        assert!(html.contains("/config"), "and link to the config page");
    }

    #[tokio::test]
    async fn degraded_dashboard_shows_startup_error() {
        let app = app(degraded_state());
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_text(resp).await;
        assert!(
            html.contains("Startup failed"),
            "dashboard must flag the degraded state"
        );
        assert!(
            html.contains("model download failed: connection refused"),
            "and show the cause"
        );
        assert!(html.contains("/logs"), "and link to the logs page");
    }

    #[tokio::test]
    async fn logs_page_shows_buffered_entries() {
        let state = unconfigured_state();
        state.log_buffer.push(crate::logbuffer::LogEntry {
            time: std::time::SystemTime::now(),
            level: tracing::Level::ERROR,
            target: "kimun_server".into(),
            message: "embedding model download failed".into(),
        });
        let app = app(state);
        let resp = app
            .oneshot(Request::get("/logs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_text(resp).await;
        assert!(html.contains("embedding model download failed"));
        assert!(html.contains("ERROR"));
    }

    #[tokio::test]
    async fn logs_page_empty_state() {
        let app = app(unconfigured_state());
        let resp = app
            .oneshot(Request::get("/logs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            body_text(resp)
                .await
                .contains("No warnings or errors since startup")
        );
    }

    #[tokio::test]
    async fn unconfigured_collections_page_shows_banner_not_error() {
        let app = app(unconfigured_state());
        let resp = app
            .oneshot(Request::get("/collections").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("unconfigured"));
    }

    #[tokio::test]
    async fn query_submit_shows_results_and_time() {
        let app = app(state(None, None));
        let resp = app
            .oneshot(
                Request::post("/query")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("vault_id=vault-1&query=hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_text(resp).await;
        assert!(html.contains("/notes/a.md"), "fake store's hit rendered");
        assert!(html.contains(" ms"), "query time shown");
        assert!(
            html.contains(r#"id="results""#),
            "results container present so the page script can clear it on the next submit"
        );
    }

    #[tokio::test]
    async fn unconfigured_query_page_disables_search() {
        let app = app(unconfigured_state());
        let resp = app
            .oneshot(Request::get("/query").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(resp).await;
        assert!(html.contains("unconfigured"));
        assert!(
            !html.contains(r#"<button type="submit">Search"#),
            "no live search form"
        );
    }

    #[tokio::test]
    async fn provider_switch_with_blank_key_does_not_carry_old_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let app = app(state_from(
            r#"
[server]
[vector_db]
type = "qdrant"
[llm]
provider = "gemini"
api_key = "gemini-key"
[reranker]
"#,
            path.clone(),
        ));
        // Switch to openai, leave key blank → must NOT reuse the gemini key.
        let form = "host=127.0.0.1&port=7573&provider=openai&model=gpt-4o-mini&api_key=&embedder_provider=none&fastembed_model=&embedder_url=&embedder_model=&embedder_api_key=&vector_db=qdrant&sqlite_path=&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let saved = RagConfig::from_file(path).unwrap();
        let llm = saved.llm.as_ref().expect("llm saved");
        assert_eq!(llm.provider(), "openai");
        assert_eq!(
            llm.api_key(),
            None,
            "old provider's key must not carry over"
        );
    }

    #[tokio::test]
    async fn invalid_port_flashes_error_without_400_or_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let app = app(state_from(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[llm]\nprovider = \"gemini\"\n[reranker]\n",
            path.clone(),
        ));
        let form = "host=127.0.0.1&port=99999999&provider=gemini&model=m&api_key=&embedder_provider=none&fastembed_model=&embedder_url=&embedder_model=&embedder_api_key=&vector_db=qdrant&sqlite_path=&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK); // friendly page, not a bare 400
        assert!(body_text(resp).await.contains("must be whole numbers"));
        assert!(
            !path.exists(),
            "invalid input must not write the config file"
        );
    }

    #[tokio::test]
    async fn saving_provider_none_clears_llm_to_semantic_only() {
        // Selecting "none" disables Q&A: llm is cleared to None, not written as a
        // keyless provider that would fail the boot key gate (adr/0022).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let app = app(state_from(
            r#"
[server]
[vector_db]
type = "qdrant"
[llm]
provider = "gemini"
api_key = "gemini-key"
[reranker]
"#,
            path.clone(),
        ));
        let form = "host=127.0.0.1&port=7573&provider=none&model=&api_key=&embedder_provider=none&fastembed_model=&embedder_url=&embedder_model=&embedder_api_key=&vector_db=qdrant&sqlite_path=&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let saved = RagConfig::from_file(path).unwrap();
        assert!(saved.llm.is_none(), "provider=none must clear the LLM");
    }

    #[tokio::test]
    async fn semantic_only_config_page_defaults_provider_to_none() {
        // With no [llm], the provider select must pre-select the none sentinel.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let app = app(state_from(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n",
            path,
        ));
        let resp = app
            .oneshot(Request::get("/config").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(resp).await;
        assert!(
            html.contains(r#"<option value="none" data-groups="" selected"#),
            "none must be the selected provider on a semantic-only server"
        );
    }

    #[tokio::test]
    async fn config_form_saves_embedder_and_vector_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let app = app(state_from(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n",
            path.clone(),
        ));
        let form = "host=127.0.0.1&port=7573&provider=none&model=&api_key=&embedder_provider=fastembed&fastembed_model=Xenova%2Fbge-small-en-v1.5&embedder_url=&embedder_model=&embedder_api_key=&vector_db=sqlite&sqlite_path=%2Fdata%2Fsqlite&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("Saved"));

        let saved = RagConfig::from_file(path).unwrap();
        match saved.embedder {
            Some(crate::config::EmbedderConfig::FastEmbed { model }) => {
                assert_eq!(model.as_deref(), Some("Xenova/bge-small-en-v1.5"))
            }
            other => panic!("expected fastembed, got {other:?}"),
        }
        assert!(matches!(
            saved.vector_db,
            crate::config::VectorDbConfig::Sqlite { .. }
        ));
    }

    /// An unconfigured server with a writable config path (for the form page).
    /// Embedder configured but its startup initialization failed → degraded.
    fn degraded_state() -> Arc<AppState> {
        let config: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"sqlite\"\n[embedder]\ntype = \"fastembed\"\n[reranker]\n",
        )
        .unwrap();
        assert!(config.embedder.is_some());
        Arc::new(
            AppState::new(None, config)
                .with_startup_error(Some("model download failed: connection refused".into())),
        )
    }

    fn unconfigured_state_with_path() -> Arc<AppState> {
        let config: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\n").unwrap();
        Arc::new(
            AppState::new(None, config).with_config_path(std::path::PathBuf::from("/dev/null")),
        )
    }

    #[tokio::test]
    async fn config_page_preselects_fastembed_model_saved_as_variant_name() {
        // A config may name the model by fastembed variant (`BGESmallENV15`);
        // the dropdown is keyed by model code. Without canonicalization no
        // option is selected, the browser submits the empty placeholder, and
        // any unrelated save fails with "Pick a fastembed model."
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let app = app(state_from(
            "[server]\n[vector_db]\ntype = \"sqlite\"\n[embedder]\ntype = \"fastembed\"\nmodel = \"BGESmallENV15\"\n[reranker]\n",
            path,
        ));
        let resp = app
            .oneshot(Request::get("/config").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(resp).await;
        assert!(
            html.contains(r#"<option value="Xenova/bge-small-en-v1.5" selected"#),
            "variant-name config must pre-select its model-code option"
        );
    }

    #[tokio::test]
    async fn unconfigured_config_page_preselects_embedder_none_and_warns() {
        let app = app(unconfigured_state_with_path());
        let resp = app
            .oneshot(Request::get("/config").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(resp).await;
        assert!(
            html.contains(r#"<select name="embedder_provider""#),
            "embedder select must render"
        );
        assert!(html.contains("invalidates"), "wipe warning must render");
    }

    #[tokio::test]
    async fn font_assets_are_served_publicly() {
        // Fonts must load on the login page, so the route sits outside auth.
        let app = app(state(Some("secret"), None));
        let resp = app
            .oneshot(
                Request::get("/assets/fonts/ahm-regular.woff2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[CONTENT_TYPE], "font/woff2");
    }

    #[tokio::test]
    async fn unknown_font_asset_is_404() {
        let app = app(state(None, None));
        let resp = app
            .oneshot(
                Request::get("/assets/fonts/evil.woff2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn logo_is_served_publicly() {
        // Nav brand + favicon; the login page needs it, so it sits outside auth.
        let app = app(state(Some("secret"), None));
        let resp = app
            .oneshot(
                Request::get("/assets/img/kimun.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[CONTENT_TYPE], "image/png");
    }

    #[tokio::test]
    async fn answer_handler_rejects_when_semantic_only() {
        // A semantic-only server (no [llm]) must reject /api/answer at submit
        // time with 503, not mint a job that can only fail (adr/0022).
        let config: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n").unwrap();
        assert!(config.llm.is_none());
        let rag = KimunRag::new(
            Arc::new(FakeVectorStore::default()),
            Arc::new(FakeEmbedder),
            None,
        );
        let state = Arc::new(AppState::new(Some(rag), config));

        let req = crate::handlers::AnswerRequest {
            vault_id: "vault-1".into(),
            query: "hello".into(),
            context_size: None,
            history: vec![],
        };
        let err = crate::handlers::answer_handler(axum::extract::State(state), axum::Json(req))
            .await
            .expect_err("semantic-only server must reject answering");
        assert!(matches!(err, crate::RagError::SemanticOnly));
        assert_eq!(
            err.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
