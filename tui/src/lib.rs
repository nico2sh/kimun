//! kimun-notes as a library: the TUI, the CLI and the MCP server.
//!
//! `src/main.rs` is a shim over [`app::main`]. Everything — the app loop and
//! the screens included — lives here, so integration tests under `tests/` can
//! drive any screen, or the loop itself, without a terminal.
pub mod app;
pub mod app_screen;
pub mod ask;
pub mod cli;
pub mod components;
pub mod keys;
pub mod rag;
// Self-contained modules with no dependency on the rest of kimün. Each was its
// own workspace crate and is kept extractable: nothing inside them may name
// `crate::` outside their own subtree (enforced in .github/workflows/check.yml).
pub mod ropetext;
pub mod server_client;
pub mod settings;
pub mod update;
pub mod util;

#[cfg(test)]
mod test_support;
