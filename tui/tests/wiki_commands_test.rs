// tui/tests/wiki_commands_test.rs
//
// `kimun wiki` through the real binary, so the tests see exit codes and what
// is printed.

use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

mod common;
use common::sys;

async fn write_config(config_path: &Path, workspace_dir: &Path) {
    let config_dir = config_path.parent().unwrap();
    let content = format!(
        r#"config_version = 3
cache_dir = "."
history_dir = "history"
[global]
current_workspace = "default"
theme = "Nord"

[workspaces.default]
path = '{}'
last_paths = []
created = "2026-01-01T00:00:00Z"
"#,
        workspace_dir.display()
    );
    std::fs::write(config_path, content).unwrap();
    let index = kimun_core::IndexFile::in_dir(&sys(config_dir), "default");
    let vault = kimun_core::NoteVault::new(
        kimun_core::VaultConfig::new(sys(workspace_dir)).with_index(index),
    )
    .await
    .unwrap();
    vault.validate_and_init().await.unwrap();
    vault.close().await;
}

struct Fixture {
    _config_dir: TempDir,
    workspace: TempDir,
    config_path: std::path::PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let config_dir = TempDir::new().unwrap();
        let config_path = config_dir.path().join("config.toml");
        let workspace = TempDir::new().unwrap();
        write_config(&config_path, workspace.path()).await;
        Self {
            _config_dir: config_dir,
            workspace,
            config_path,
        }
    }

    fn kimun(&self, args: &[&str]) -> Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_kimun"))
            .args(["--config", self.config_path.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap()
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.workspace.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[tokio::test]
async fn wiki_init_then_check_exits_zero_on_a_fresh_vault() {
    let f = Fixture::new().await;
    let init = f.kimun(&["wiki", "init"]);
    assert_eq!(init.status.code(), Some(0), "stderr: {}", stderr(&init));
    assert!(
        stdout(&init).contains("created"),
        "stdout: {}",
        stdout(&init)
    );
    let check = f.kimun(&["wiki", "check"]);
    assert_eq!(check.status.code(), Some(0), "stdout: {}", stdout(&check));
    assert!(
        stdout(&check).contains("clean"),
        "stdout: {}",
        stdout(&check)
    );
    // A second init keeps everything.
    let again = f.kimun(&["wiki", "init"]);
    assert_eq!(again.status.code(), Some(0));
    assert!(stdout(&again).contains("kept"));
    assert!(!stdout(&again).contains("created"));
}

#[tokio::test]
async fn wiki_check_exits_one_when_a_problem_exists_and_prints_the_report_first() {
    let f = Fixture::new().await;
    f.kimun(&["wiki", "init"]);
    f.write("wiki/page.md", "# Page\n\n[[no such note]]\n");
    let check = f.kimun(&["wiki", "check"]);
    assert_eq!(check.status.code(), Some(1), "stderr: {}", stderr(&check));
    let out = stdout(&check);
    assert!(out.contains("problem  unresolved_link  "), "stdout: {out}");
    assert!(out.contains("1 note checked."), "stdout: {out}");
    assert!(stderr(&check).is_empty(), "stderr: {}", stderr(&check));
}

#[tokio::test]
async fn wiki_check_json_has_rule_names_in_snake_case() {
    let f = Fixture::new().await;
    f.kimun(&["wiki", "init"]);
    f.write("wiki/page.md", "# Page\n\n[[no such note]]\n");
    let check = f.kimun(&["wiki", "check", "--format", "json"]);
    assert_eq!(check.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_str(&stdout(&check)).unwrap();
    assert_eq!(json["declared"], true);
    let rules: Vec<&str> = json["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["rule"].as_str().unwrap())
        .collect();
    assert!(rules.contains(&"unresolved_link"), "rules: {rules:?}");
    let severity_of = |rule: &str| {
        json["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["rule"] == rule)
            .map(|x| x["severity"].as_str().unwrap().to_string())
    };
    assert_eq!(severity_of("unresolved_link").as_deref(), Some("problem"));
}

#[tokio::test]
async fn wiki_check_json_marks_an_unstamped_source_as_info() {
    let f = Fixture::new().await;
    f.kimun(&["wiki", "init"]);
    f.write("sources/a.md", "# Source\n\nraw text\n");
    let check = f.kimun(&["wiki", "check", "--format", "json"]);
    assert_eq!(check.status.code(), Some(0), "stdout: {}", stdout(&check));
    let json: serde_json::Value = serde_json::from_str(&stdout(&check)).unwrap();
    let unstamped = json["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["rule"] == "unstamped")
        .expect("an unstamped finding");
    assert_eq!(unstamped["severity"], "info");
}

#[tokio::test]
async fn wiki_catalogue_prints_the_more_line_when_capped() {
    let f = Fixture::new().await;
    f.kimun(&["wiki", "init"]);
    for n in ["a", "b", "c"] {
        f.write(
            &format!("wiki/{n}.md"),
            &format!("---\nsummary: Page {n}\n---\n# Page {n}\n"),
        );
    }
    let out = f.kimun(&["wiki", "catalogue", "--limit", "1"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Page a"), "stdout: {text}");
    assert!(!text.contains("Page b"), "stdout: {text}");
    assert!(text.trim_end().ends_with("2 more pages"), "stdout: {text}");

    let json = f.kimun(&["wiki", "catalogue", "--format", "json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert_eq!(v["total"], 3);
    assert_eq!(v["entries"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn wiki_stamp_without_a_declaration_exits_two_with_the_message() {
    let f = Fixture::new().await;
    let out = f.kimun(&["wiki", "stamp"]);
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("Error: "), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).contains("no wiki declaration"),
        "stderr: {}",
        stderr(&out)
    );
    let cat = f.kimun(&["wiki", "catalogue"]);
    assert_eq!(cat.status.code(), Some(2));
}

#[tokio::test]
async fn wiki_check_with_only_info_findings_exits_zero() {
    let f = Fixture::new().await;
    f.kimun(&["wiki", "init"]);
    f.write("sources/a.md", "# Source\n\nraw text\n");
    let check = f.kimun(&["wiki", "check"]);
    assert_eq!(check.status.code(), Some(0), "stdout: {}", stdout(&check));
    assert!(
        stdout(&check).contains("info  unstamped  "),
        "stdout: {}",
        stdout(&check)
    );

    let stamp = f.kimun(&["wiki", "stamp"]);
    assert_eq!(stamp.status.code(), Some(0), "stderr: {}", stderr(&stamp));
    assert!(
        stdout(&stamp).contains("/sources/a"),
        "stdout: {}",
        stdout(&stamp)
    );
    let again = f.kimun(&["wiki", "stamp"]);
    assert!(stdout(&again).contains("Nothing to stamp."));
}

#[tokio::test]
async fn wiki_check_without_a_declaration_runs_the_generic_rules() {
    let f = Fixture::new().await;
    f.write("n.md", "# N\n\n[[nothing here]]\n");
    let check = f.kimun(&["wiki", "check"]);
    assert_eq!(check.status.code(), Some(1), "stderr: {}", stderr(&check));
    assert!(stdout(&check).contains("unresolved_link"));
}

#[tokio::test]
async fn wiki_stamp_lists_a_skipped_source_and_exits_one() {
    let f = Fixture::new().await;
    f.kimun(&["wiki", "init"]);
    f.write("sources/a.md", "# A\n\ntext\n");
    f.write("sources/b.md", "---\nkey: [unclosed\n---\nbody\n");
    let stamp = f.kimun(&["wiki", "stamp"]);
    assert_eq!(stamp.status.code(), Some(1), "stderr: {}", stderr(&stamp));
    let out = stdout(&stamp);
    assert!(out.contains("/sources/a\n"), "stdout: {out}");
    assert!(out.contains("skipped  /sources/b  "), "stdout: {out}");
    assert!(stderr(&stamp).is_empty(), "stderr: {}", stderr(&stamp));
}
