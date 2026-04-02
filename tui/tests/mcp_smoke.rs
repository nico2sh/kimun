// tui/tests/mcp_smoke.rs
//
// End-to-end smoke test: spawns the real `kimun` binary, sends MCP JSON-RPC
// messages over stdin, and asserts that `tools/list` returns all 8 expected
// tool names.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

/// Locate the `kimun` binary relative to the test binary's directory.
fn kimun_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    eprintln!("test binary: {:?}", p);
    p.pop(); // remove test binary name
    if p.ends_with("deps") {
        p.pop();
    }
    let bin = p.join("kimun");
    eprintln!("kimun binary path: {:?}", bin);
    bin
}

/// Write a minimal config file that points the workspace at `workspace`.
fn write_config(dir: &std::path::Path, workspace: &std::path::Path) -> std::path::PathBuf {
    let config_path = dir.join("kimun_config.toml");
    std::fs::write(
        &config_path,
        format!(
            "workspace_dir = {:?}\n",
            workspace.to_string_lossy().as_ref()
        ),
    )
    .unwrap();
    config_path
}

const INITIALIZE_MSG: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-test","version":"0.0.1"}}}"#;
const INITIALIZED_NOTIF: &str =
    r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
const TOOLS_LIST_MSG: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;

#[test]
fn mcp_smoke_tools_list() {
    // Build the binary first so the path returned by kimun_bin() exists.
    let build_status = Command::new("cargo")
        .args(["build", "--package", "kimun-notes"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("failed to run cargo build");
    assert!(build_status.success(), "cargo build failed");

    let config_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    let config_path = write_config(config_dir.path(), workspace_dir.path());

    let bin = kimun_bin();
    assert!(
        bin.exists(),
        "kimun binary not found at {:?}",
        bin
    );

    let mut child = Command::new(&bin)
        .args(["--config", config_path.to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {:?}: {}", bin, e));

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Send MCP handshake then tools/list.
    writeln!(stdin, "{}", INITIALIZE_MSG).unwrap();
    writeln!(stdin, "{}", INITIALIZED_NOTIF).unwrap();
    writeln!(stdin, "{}", TOOLS_LIST_MSG).unwrap();
    // Drop stdin so the child sees EOF after the messages.
    drop(stdin);

    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut combined = String::new();

    for line in reader.lines() {
        if std::time::Instant::now() > deadline {
            panic!(
                "timed out waiting for tools/list response (id=2).\nReceived so far:\n{}",
                combined
            );
        }
        match line {
            Ok(l) => {
                eprintln!("stdout: {}", l);
                combined.push_str(&l);
                combined.push('\n');
                // The tools/list response carries id=2.
                if combined.contains(r#""id":2"#) {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = child.wait();

    assert!(
        combined.contains(r#""id":2"#),
        "never received a response with id=2.\nReceived:\n{}",
        combined
    );

    let expected_tools = [
        "create_note",
        "append_note",
        "show_note",
        "search_notes",
        "list_notes",
        "journal",
        "get_backlinks",
        "get_chunks",
    ];
    for tool in &expected_tools {
        assert!(
            combined.contains(tool),
            "tool '{}' not found in tools/list response:\n{}",
            tool,
            combined
        );
    }
}
