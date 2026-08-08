// tui/tests/mcp_smoke.rs
//
// End-to-end smoke test: spawns the real `kimun` binary, sends MCP JSON-RPC
// messages over stdin, and asserts that `tools/list` returns all 11 expected
// tool names and that `prompts/list` returns all 6 expected prompt names.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

/// The `kimun` binary under test.
///
/// Cargo sets `CARGO_BIN_EXE_<name>` for integration tests and guarantees the
/// binary is built before the test runs, so there is nothing to build here and
/// nothing to locate: no walking up from `current_exe`, and no `EXE_SUFFIX`
/// handling, since the path Cargo hands over already names `kimun.exe` on
/// Windows.
///
/// These two tests used to shell out to `cargo build` in their own bodies.
/// That cost ~110s each on a Windows CI runner and starved every test running
/// beside them — which is how a `workspace rename` elsewhere in the suite came
/// to lose a 9-second race against a file lock, and how both of these timed
/// out against their own 15s deadline.
fn kimun_bin() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_kimun"))
}

/// A config at the current version that points the workspace at `workspace`.
///
/// `cache_dir`/`history_dir` are left at their defaults, which resolve against
/// the config file's own directory — so this keeps the index and history in
/// `dir` rather than in the real installation.
fn write_config(dir: &std::path::Path, workspace: &std::path::Path) -> std::path::PathBuf {
    let config_path = dir.join("config.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"config_version = 6

[global]
current_workspace = "default"

[workspaces.default]
path = {:?}
created = "2024-01-15T10:30:00Z"
"#,
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
    let config_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    let config_path = write_config(config_dir.path(), workspace_dir.path());

    let bin = kimun_bin();
    assert!(bin.exists(), "kimun binary not found at {:?}", bin);

    let mut child = Command::new(bin)
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
        "get_outlinks",
        "rename_note",
        "move_note",
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

const PROMPTS_LIST_MSG: &str = r#"{"jsonrpc":"2.0","id":3,"method":"prompts/list","params":{}}"#;

#[test]
fn mcp_smoke_prompts_list() {
    let config_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    let config_path = write_config(config_dir.path(), workspace_dir.path());

    let mut child = Command::new(kimun_bin())
        .args(["--config", config_path.to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn kimun mcp");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    writeln!(stdin, "{}", INITIALIZE_MSG).unwrap();
    writeln!(stdin, "{}", INITIALIZED_NOTIF).unwrap();
    writeln!(stdin, "{}", PROMPTS_LIST_MSG).unwrap();
    drop(stdin);

    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut combined = String::new();
    for line in reader.lines() {
        if std::time::Instant::now() > deadline {
            panic!("timed out waiting for prompts/list response");
        }
        match line {
            Ok(l) => {
                combined.push_str(&l);
                combined.push('\n');
                if combined.contains(r#""id":3"#) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.wait();

    let expected_prompts = [
        "daily_review",
        "find_connections",
        "research_note",
        "brainstorm",
        "weekly_review",
        "link_suggestions",
    ];
    for prompt in &expected_prompts {
        assert!(
            combined.contains(prompt),
            "prompt '{}' not found in prompts/list response:\n{}",
            prompt,
            combined
        );
    }
}
