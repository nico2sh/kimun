// tui/src/cli/commands/mcp/config_print.rs
//
// `kimun mcp --print-config`: the text a person pastes into an agent app so
// the app can start `kimun mcp`. Reads the environment, writes nothing.
// Standard output gets only the JSON block. The other text goes to standard error.

use std::path::{Path, PathBuf};

use kimun_core::system::SystemPath;

/// The command an agent app should run: the bare name when `kimun` on the
/// `PATH` is this very binary, otherwise the absolute path of this binary.
pub fn command_name(current_exe: &Path, on_path: Option<PathBuf>) -> String {
    let same = on_path
        .and_then(|p| SystemPath::canonical(p).ok())
        .zip(SystemPath::canonical(current_exe).ok())
        .is_some_and(|(a, b)| a == b);
    if same {
        "kimun".to_string()
    } else {
        current_exe.to_string_lossy().into_owned()
    }
}

/// Finds an executable file named `kimun` in the `PATH` entries.
pub fn find_on_path() -> Option<PathBuf> {
    let name = format!("kimun{}", std::env::consts::EXE_SUFFIX);
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(&name))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The sentence before the JSON block. It goes to standard error.
const INTRO: &str = "Add this to the MCP settings of your agent app:";

/// The JSON block that goes to standard output. Nothing else goes there, so the text can be piped
/// or pasted into a settings file as it is.
pub fn render_json(command: &str) -> String {
    let json = serde_json::json!({
        "mcpServers": { "kimun": { "command": command, "args": ["mcp"] } }
    });
    serde_json::to_string_pretty(&json).expect("a JSON value always serializes")
}

/// The `claude mcp add` line and its heading. They go to standard error.
pub fn render_notes(command: &str) -> String {
    let shown = if command.contains(char::is_whitespace) {
        format!("\"{command}\"")
    } else {
        command.to_string()
    };
    format!("For Claude Code, run:\nclaude mcp add kimun -- {shown} mcp")
}

/// Prints the JSON block for this binary to standard output, and the sentences around it to
/// standard error.
pub fn run() -> color_eyre::eyre::Result<()> {
    use std::io::Write;
    let exe = std::env::current_exe()?;
    let command = command_name(&exe, find_on_path());
    eprintln!("{INTRO}");
    println!("{}", render_json(&command));
    std::io::stdout().flush()?;
    eprintln!("\n{}", render_notes(&command));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::CliCommand;
    use clap::Parser;

    #[derive(Parser)]
    struct Wrap {
        #[command(subcommand)]
        cmd: CliCommand,
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"x").unwrap();
        p.canonicalize().unwrap()
    }

    #[test]
    fn command_name_prefers_the_bare_name_when_path_resolves_to_the_binary() {
        let dir = tempfile::TempDir::new().unwrap();
        let exe = touch(dir.path(), "kimun");
        assert_eq!(command_name(&exe, Some(exe.clone())), "kimun");
    }

    #[cfg(unix)]
    #[test]
    fn command_name_follows_a_symlink_to_the_binary() {
        let dir = tempfile::TempDir::new().unwrap();
        let exe = touch(dir.path(), "real");
        let link = dir.path().join("kimun");
        std::os::unix::fs::symlink(&exe, &link).unwrap();
        assert_eq!(command_name(&exe, Some(link)), "kimun");
    }

    #[test]
    fn command_name_uses_the_absolute_path_otherwise() {
        let dir = tempfile::TempDir::new().unwrap();
        let exe = touch(dir.path(), "kimun");
        let other = touch(dir.path(), "other");
        assert_eq!(command_name(&exe, None), exe.to_string_lossy());
        assert_eq!(command_name(&exe, Some(other)), exe.to_string_lossy());
        let missing = dir.path().join("missing");
        assert_eq!(command_name(&exe, Some(missing)), exe.to_string_lossy());
    }

    #[test]
    fn the_json_text_parses_as_json_as_a_whole() {
        let text = render_json("/opt/my \"tools\"/kimun");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v["mcpServers"]["kimun"]["command"],
            "/opt/my \"tools\"/kimun"
        );
        assert_eq!(v["mcpServers"]["kimun"]["args"], serde_json::json!(["mcp"]));
        assert!(!text.contains("claude mcp add"), "{text}");
    }

    #[test]
    fn the_notes_hold_the_claude_line() {
        let notes = render_notes("kimun");
        assert!(notes.contains("For Claude Code, run:"), "{notes}");
        assert!(
            notes.contains("claude mcp add kimun -- kimun mcp"),
            "{notes}"
        );
    }

    #[test]
    fn the_notes_quote_a_command_with_spaces() {
        let notes = render_notes("/opt/my tools/kimun");
        assert!(
            notes.contains("claude mcp add kimun -- \"/opt/my tools/kimun\" mcp"),
            "{notes}"
        );
    }

    #[test]
    fn mcp_parses_with_and_without_print_config() {
        let plain = Wrap::try_parse_from(["kimun", "mcp"]).unwrap();
        assert!(matches!(
            plain.cmd,
            CliCommand::Mcp {
                print_config: false
            }
        ));
        let print = Wrap::try_parse_from(["kimun", "mcp", "--print-config"]).unwrap();
        assert!(matches!(print.cmd, CliCommand::Mcp { print_config: true }));
    }
}
