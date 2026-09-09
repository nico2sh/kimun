//! The `kimun-server` binary is the in-process restart loop (adr 0028) and
//! nothing else: CLI, tracing init, then per iteration — load the config,
//! build and serve, drain on a restart or Ctrl-C. Everything between "config
//! loaded" and "listener bound" is `kimun_server::startup`, so it compiles
//! into the library and has tests.

use clap::Parser;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use kimun_server::{config::RagConfig, logbuffer::LogBuffer, server_state::AppState, startup};

#[derive(Parser)]
#[command(version, about = "Kimun RAG Server", long_about = None)]
struct Cli {
    /// Path to configuration file (default: ~/.config/kimun/server.toml)
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,

    /// Start with built-in defaults — embedded SQLite plus the local
    /// fastembed embedder (default model) — without reading a config file.
    #[arg(long, conflicts_with = "config")]
    default_config: bool,

    /// Host to bind to (overrides config)
    #[arg(long)]
    host: Option<String>,

    /// Port to bind to (overrides config)
    #[arg(short, long)]
    port: Option<u16>,
}

/// Why one `run_server` iteration ended: an operator asked for an in-process
/// restart (drain, reload the config file, rebind), or the process
/// is done (Ctrl-C).
enum Shutdown {
    Restart,
    Terminate,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing ONCE for the process lifetime. Besides stdout,
    // WARN/ERROR events are copied into an in-memory ring buffer the web UI
    // serves at /logs; it survives in-process restarts so the log page shows
    // what happened across them.
    let log_buffer = LogBuffer::new();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kimun_server=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(log_buffer.layer())
        .init();

    let cli = Cli::parse();

    // The restart loop: each iteration builds and serves the whole
    // server; a web-UI restart drains in-flight requests, then the next
    // iteration re-reads the config file and rebinds, so every setting —
    // including the bind address — applies without a supervisor.
    let mut first_run = true;
    loop {
        match run_server(&cli, first_run, log_buffer.clone()).await? {
            Shutdown::Restart => {
                tracing::info!("Restart requested — reloading configuration");
                first_run = false;
            }
            Shutdown::Terminate => return Ok(()),
        }
    }
}

async fn run_server(cli: &Cli, first_run: bool, log_buffer: LogBuffer) -> anyhow::Result<Shutdown> {
    // Load configuration (remembering the path so the web UI can persist edits).
    tracing::info!("Loading configuration...");
    let config_path = RagConfig::resolve_path(cli.config.clone());
    let config = if cli.default_config && first_run {
        // Explicit opt-in to local defaults; no file is read. A missing config
        // file is created with these defaults so later file-based starts (and
        // web-UI edits) have a real file; an existing file is left untouched.
        tracing::info!(
            "--default-config: using built-in defaults (SQLite + fastembed), not reading a config file"
        );
        RagConfig::ready_default_persisted(&config_path)?
    } else {
        // On an in-process restart the seeded file (plus any web edits) is the
        // source of truth, --default-config or not.
        RagConfig::load(cli.config.clone())?
    };
    let config = config.merge_with_cli(cli.host.clone(), cli.port);

    tracing::info!("Configuration loaded successfully");
    tracing::debug!("Server: {}:{}", config.server.host, config.server.port);

    // Config → pipeline. Never aborts: an unconfigured or degraded server
    // still serves the web UI so the config can be fixed there.
    let parts = startup::build(&config).await;

    // Application state. The restart channel is how the web UI's Restart
    // button reaches this loop iteration's graceful shutdown.
    let (restart_tx, mut restart_rx) = tokio::sync::mpsc::channel::<()>(1);
    let state = Arc::new(
        AppState::from_parts(parts, config.clone())
            .with_config_path(config_path)
            .with_log_buffer(log_buffer)
            .with_restart(restart_tx),
    );
    startup::spawn_job_sweep(&state);

    if state.config.auth.token.is_some() {
        tracing::info!("Bearer-token auth enabled on /api routes");
    } else if config.server.host != "127.0.0.1" && config.server.host != "localhost" {
        tracing::warn!(
            "No [auth] token set and bound to {} — the API is OPEN to the network",
            config.server.host
        );
    }

    let app = startup::router(state);

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("RAG server listening on {}", addr);
    tracing::info!("Health check available at http://{}/health", addr);

    // Serve until the web UI requests a restart or the process gets Ctrl-C;
    // either way axum drains in-flight requests before returning. The oneshot
    // smuggles the reason out of the shutdown future.
    let (reason_tx, reason_rx) = tokio::sync::oneshot::channel();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let reason = tokio::select! {
                _ = restart_rx.recv() => Shutdown::Restart,
                _ = tokio::signal::ctrl_c() => Shutdown::Terminate,
            };
            let _ = reason_tx.send(reason);
        })
        .await?;

    Ok(reason_rx.await.unwrap_or(Shutdown::Terminate))
}
