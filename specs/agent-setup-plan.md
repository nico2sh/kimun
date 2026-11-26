# Agent setup implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Kimün server is a visible soft dependency of the agent connector: the tools report its state with an exact next step, four small CLI pieces set it up, and the skills install with the standard tool or a plain fallback.

**Architecture:** A pure state and hint module in `tui/src/rag/state.rs` serves the MCP tools and the new `kimun server` command. Core gets one generic file-install function in `system`. New CLI modules live under `tui/src/cli/commands/`. Docs and guidance ship last, so that they describe only commands that exist.

**Tech Stack:** Rust (core and tui crates), clap, tokio, Python 3 for the CI check, Zola docs.

**Spec:** `specs/agent-setup.md`. Decision record: `adr/0066-the-agent-connector-reports-the-servers-state-and-never-changes-the-machine.md`.

## Global Constraints

- Follow `CLAUDE.md`. Writes outside the vault, and the home-folder rules for Windows and Unix, live in `kimun_core::system`. `check-host-fs.py` enforces it. The TUI crate can read files, but it must not call `rename`, `copy`, `remove_*`, `create_dir*`, `set_permissions` or a truncating `write`.
- Nothing starts, installs or restarts the server. Every hint tells the agent to tell the user and not to run the command.
- The owner commits. The plan has no commit steps. Each task ends when its tests pass.
- Writing in docs and text: plain English, short sentences, no semicolons, no em-dashes in prose. Docs describe only commands that exist.
- The MCP code stays in `tui/src/cli/commands/mcp/`. New commands get their own modules under `tui/src/cli/commands/`.
- Test commands: `cargo test -p kimun_core`, `cargo test -p kimun-notes --lib`. Before the last task run the seven CI gates from `.github/workflows/check.yml`. Use `rtk proxy cargo test --workspace > <file>` and read `test result` lines from the file, because a plain run is summarized.
- Module boundary gate: files under `tui/src/server_client` must not use `crate::` paths outside `server_client`.

## Review Focus

- A saved token must never follow a changed address. `kimun server use <new url>` without `--token` clears the old token when the address differs from the saved one, and keeps it when the address is the same. Test in Task 4.
- A skill folder that is a symlink is never written through, even with `--force`. A folder with different content needs `--force`. Identical content is "unchanged" and is not an error. Test in Task 1 and Task 6.
- `kimun mcp --print-config` must work with no workspace and no vault, print nothing but the config, and give a path that survives an upgrade: the bare name `kimun` when it resolves on `PATH` to the running binary, else the absolute path. Test in Task 5.
- The state is Stopped only for a local address whose port matches the service port in `server.toml`, with a marker showing an install here. A Docker container on another port, or a remote address, is Unreachable. Test in Task 2.
- No status call hangs. The probe has a 5 second limit, and neither the tools nor the CLI run any command. Test in Task 2 and Task 3.

---

### Task 1: Core file install

**Files:**
- Create: `core/src/system/folder_install.rs`
- Modify: `core/src/system/mod.rs` (add `mod folder_install;`, re-export, add the `Symlink` error variant)
- Test: in-file `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub enum FolderInstall { Created, Unchanged, Replaced(Vec<String>) }`
  - `pub fn install_files(dir: &SystemPath, files: &[(&str, &[u8])], force: bool) -> Result<FolderInstall, SystemError>`
  - New variant `SystemError::Symlink { path: String }`.
- Rules: a file name must be one plain path component, otherwise an `InvalidInput` error. `dir` is created with its parents when it is missing. If `dir` is a symlink, return `Symlink` in every case. If every file exists with identical bytes, return `Unchanged`. If a file differs and `force` is false, return the existing `AlreadyExists { path }`. With `force`, replace the differing files atomically (`replace_atomically`) and return their names in `Replaced`. Files in `dir` that are not in `files` stay untouched.

- [ ] **Step 1: Write failing tests.** `creates_the_folder_and_files`, `identical_content_is_unchanged`, `different_content_without_force_is_already_exists`, `force_replaces_only_the_differing_files_and_names_them`, `extra_files_stay`, `a_symlinked_folder_is_refused_even_with_force` (`#[cfg(unix)]`), `a_name_with_a_separator_or_dots_is_rejected`.
- [ ] **Step 2: Run** `cargo test -p kimun_core folder_install`. Expected: FAIL, items not defined.
- [ ] **Step 3: Implement** the function and the variant. Use `symlink_metadata` for the symlink check. Keep every direct fs call inside `system`.
- [ ] **Step 4: Run** `cargo test -p kimun_core`. Expected: PASS. Run `python3 .github/scripts/check-host-fs.py`. Expected: pass.

### Task 2: Server state and hints

**Files:**
- Create: `tui/src/rag/state.rs`
- Modify: `tui/src/rag/mod.rs` (declare and re-export), `tui/src/server_manage/mod.rs` and `tui/src/server_manage/service.rs` (raise `service_port`, the local-address and port helpers, and the unit and label constants to `pub(crate)` as needed)
- Test: in-file `#[cfg(test)]`

**Interfaces:**
- Consumes: `server_manage::marker::load(config_dir) -> Result<ServerInstall, MarkerProblem>` (`MarkerProblem::NoService` still proves an install), `server_manage::ServiceKind`, the existing local-address and port helpers, `ServerCapability::from_health`, `RagClient::new(url, token, vault_id)` and `RagClient::health()`.
- Produces:
  - `pub enum ServerState { None, Stopped, Unreachable, Unconfigured, SemanticOnly, Full }` with `Display`: `none`, `stopped`, `unreachable`, `unconfigured`, `semantic-only`, `full`.
  - `pub enum HostOs { Linux, Macos, Windows, Other }` with `HostOs::current()`.
  - `pub enum InstallEvidence { None, Service { kind: ServiceKind, install_dir: PathBuf }, NoService { install_dir: PathBuf } }` and `pub fn install_evidence(config_dir: &Path) -> InstallEvidence`.
  - `pub struct HintContext { pub os: HostOs, pub evidence: InstallEvidence, pub address_set: bool }`.
  - `pub fn hint(state: ServerState, ctx: &HintContext) -> Option<String>`: pure.
  - `pub fn classify(address: Option<&str>, probe: Probe, evidence: &InstallEvidence, service_port: u16) -> ServerState`: pure, with `pub enum Probe { Failed, Capability(ServerCapability) }`.
  - `pub struct ServerReport { pub state: ServerState, pub hint: Option<String> }`.
  - `pub async fn report_for(address: Option<&str>, token: Option<&str>, config_dir: &Path, os: HostOs, timeout: Duration) -> ServerReport` (needs no vault: the health call uses an empty vault id), and `pub async fn report(settings: &SharedSettings, timeout: Duration) -> ServerReport` that reads the address and token and the real config folder.
  - Constants: `INSTALL_LINE = "curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service"`, `DEFAULT_ADDRESS = "http://localhost:7573"`, `SERVER_DOCS_URL = "https://kimun.2co.dev/using-kimun/server/"`.
- Hint content (the exact words are the builder's, these parts are fixed):
  - None, no install evidence, Linux or macOS: the install line, then `kimun server use http://localhost:7573`, then "restart your agent app".
  - None, no install evidence, Windows or other: `SERVER_DOCS_URL` (Docker and WSL2), then `kimun server use ...`, then the restart.
  - None, with install evidence: `kimun server use http://localhost:7573` and the restart.
  - Stopped with systemd: `systemctl --user start kimun-server.service`. With launchd: `launchctl kickstart gui/$(id -u)/dev.2co.kimun-server`. With no service: the binary path `<install_dir>/kimun-server` to run in a terminal, or the installer again with `--service`.
  - Unreachable and Unconfigured keep the wording of today's hints.
  - Every hint says to tell the user and not to run the command.
- `classify` returns `Stopped` only when the probe failed, the address is local, evidence is not `None`, and the address port equals `service_port`. Any other failed probe is `Unreachable`. No address is `None`.

- [ ] **Step 1: Write failing tests.** A table test for `hint` over every state and evidence kind on Linux, macOS and Windows (assert the command text, "Tell the user" or the chosen words for it, and "not run"). `classify_stopped_needs_local_address_matching_port_and_evidence`, `classify_remote_address_is_unreachable`, `classify_docker_on_another_port_is_unreachable`, `classify_no_address_is_none`, `install_evidence_reads_a_marker_with_and_without_a_service`, and `report_for_times_out_as_unreachable` (a listener that never answers, a 50 ms timeout).
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib rag::state`. Expected: FAIL.
- [ ] **Step 3: Implement** the module. Do not run any command in this module and do not change the old `info.rs` yet.
- [ ] **Step 4: Run** the same command. Expected: PASS. Run the module boundary grep from the Global Constraints. Expected: no output.

### Task 3: MCP tools use the new state

**Files:**
- Modify: `tui/src/cli/commands/mcp/info.rs`, `tui/src/cli/commands/mcp/semantic.rs`, `tui/src/cli/commands/mcp/mod.rs`
- Test: the existing test modules in those files

**Interfaces:**
- Consumes: Task 2 `ServerState`, `ServerReport`, `report`, `report_for`.
- Produces: `vault_info` prints `semantic search: <state>` and, when there is a hint, one more line with the hint. `semantic_search` returns its tool error with the hint text for the states None, Stopped, Unreachable and Unconfigured. The 5 second limit stays (`PROBE_TIMEOUT`). The old `SemanticState`, `semantic_state` and `probe_state` are removed.

- [ ] **Step 1: Update and add failing tests.** Existing state tests move to the new names. Add `vault_info_reports_stopped_with_the_start_command` (a marker in a temp config folder, a local address, a closed port), `vault_info_none_names_the_install_line_without_a_marker`, `vault_info_none_names_server_use_with_a_marker`, `semantic_search_error_for_stopped_names_the_command`, and `hints_say_not_to_run_the_command`. Tests inject the config folder and the OS through `report_for`, and never touch the real home folder.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib mcp::`. Expected: FAIL.
- [ ] **Step 3: Replace** the old state code with the Task 2 calls, and keep the wording of the existing tool descriptions.
- [ ] **Step 4: Run** `cargo test -p kimun-notes --lib mcp::`. Expected: PASS.

### Task 4: `kimun server use` and `status`

**Files:**
- Create: `tui/src/cli/commands/server.rs`
- Modify: `tui/src/cli/commands/mod.rs`, `tui/src/cli/mod.rs` (the `CliCommand::Server` variant and its dispatch, next to `Workspace`)
- Test: in-file `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 2 `report_for`, `HostOs::current()`, `settings::config_dir()`, `AppSettings` with `workspace_config.global.kimun_server_url` and `kimun_server_token`, and `settings.save_to_disk()`.
- Produces:
  - `pub enum ServerSubcommand { Use { url: Option<String>, token: Option<String>, off: bool }, Status }` with clap: `use <URL> [--token T]`, `use --off` (conflicts with `URL` and `--token`), `status`.
  - `pub async fn run(sub: ServerSubcommand, settings: &mut AppSettings) -> Result<()>`.
  - `pub fn normalize_url(input: &str) -> Result<String, String>`: accepts only `http://` or `https://`, trims whitespace, removes trailing slashes, rejects anything else with a message that names the accepted form.
- Behavior: `use <url>` saves the normalized address even when the server does not answer, applies the token rule from the Review Focus, saves to disk, then prints the state and hint, and a last line "Restart your agent app so the MCP server picks up the change." `use --off` clears the address and the token and says so. `status` prints the state and hint and changes nothing. The token is never printed. With no workspace configuration, `use` returns a clear error like the workspace commands do.

- [ ] **Step 1: Write failing tests.** `normalize_url_accepts_http_and_https_and_trims_slashes`, `normalize_url_rejects_other_schemes_and_empty`, `use_saves_the_address_when_the_server_is_down`, `use_with_a_new_address_and_no_token_clears_the_old_token`, `use_with_the_same_address_keeps_the_token`, `use_off_clears_address_and_token`, `status_changes_nothing`, `the_token_is_never_printed`.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib cli::commands::server`. Expected: FAIL.
- [ ] **Step 3: Implement.** Persist through the same settings writer that `workspace use` uses.
- [ ] **Step 4: Run** the same command and `cargo test -p kimun-notes --lib cli`. Expected: PASS.

### Task 5: `kimun mcp --print-config`

**Files:**
- Create: `tui/src/cli/commands/mcp/config_print.rs`
- Modify: `tui/src/cli/mod.rs` (`Mcp { #[arg(long)] print_config: bool }` and its dispatch), `tui/src/cli/commands/mcp/mod.rs` (declare the module)
- Test: in-file `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub fn command_name(current_exe: &Path, on_path: Option<PathBuf>) -> String`: returns `kimun` when `on_path` resolves to the same file as `current_exe`, else the absolute `current_exe` path.
  - `pub fn render(command: &str) -> String`: the JSON block `{"mcpServers": {"kimun": {"command": <command>, "args": ["mcp"]}}}`, pretty printed, and a line `claude mcp add kimun -- <command> mcp`.
- Behavior: `kimun mcp --print-config` prints only the rendered text to stdout, opens no vault, and needs no workspace. `kimun mcp` without the option runs the stdio server exactly as before. Quote a command with spaces in the `claude mcp add` line.

- [ ] **Step 1: Write failing tests.** `command_name_prefers_the_bare_name_when_path_resolves_to_the_binary`, `command_name_uses_the_absolute_path_otherwise`, `render_is_valid_json_with_the_command_and_args`, `render_quotes_a_command_with_spaces`, and a CLI parse test that `mcp` and `mcp --print-config` both parse.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib config_print`. Expected: FAIL.
- [ ] **Step 3: Implement.** Find the binary on `PATH` by scanning the `PATH` entries for an executable named `kimun` and comparing canonical paths.
- [ ] **Step 4: Run** `cargo test -p kimun-notes --lib`. Expected: PASS. The existing `mcp_smoke` integration test must still pass: `cargo test -p kimun-notes --test mcp_smoke`.

### Task 6: `kimun skill install`

**Files:**
- Create: `tui/src/cli/commands/skill.rs`, `tui/src/cli/commands/skill/kimun-cli.md` (a copy of `skills/kimun-cli/SKILL.md`), `tui/src/cli/commands/skill/kimun-ask.frontmatter.md` (the frontmatter of `skills/kimun-ask/SKILL.md`)
- Modify: `tui/src/cli/commands/mod.rs`, `tui/src/cli/mod.rs` (the `CliCommand::Skill` variant), `.github/scripts/check-agent-guidance.py` and its test file
- Test: in-file `#[cfg(test)]`, the Python test

**Interfaces:**
- Consumes: Task 1 `install_files`, `FolderInstall`, `kimun_core::system::home()`, the existing `GUIDANCE` text.
- Produces:
  - `pub enum SkillSubcommand { Install { only: Option<SkillName>, dir: Option<PathBuf>, force: bool } }` with `SkillName { Ask, Cli }` (`--only ask|cli`).
  - `pub fn embedded_skills() -> [EmbeddedSkill; 2]` with `EmbeddedSkill { name: &'static str, files: Vec<(&'static str, Vec<u8>)> }`. The `kimun-ask` file is its frontmatter followed by the guidance text, so it equals `skills/kimun-ask/SKILL.md`. The `kimun-cli` file equals `skills/kimun-cli/SKILL.md`.
  - `pub fn known_targets(home: &SystemPath) -> Vec<(&'static str, SystemPath)>`: Claude Code `~/.claude/skills` and Codex `~/.agents/skills`. The Codex path is unconfirmed, so check it against the Codex docs before you fix it.
- Behavior: without `--dir`, install into each known target whose parent folder exists (`~/.claude`, `~/.agents`), and print the targets it skipped. With `--dir PATH`, install into `PATH/<skill>/` only. Print one line per folder: written, unchanged, replaced (with the file names), kept because it differs (name `--force`), or refused because it is a symlink managed elsewhere. Exit non-zero if any folder was refused. The CI check also fails when `skills/kimun-cli/SKILL.md` differs from the embedded copy, or when `skills/kimun-ask/SKILL.md` differs from frontmatter plus guidance.

- [ ] **Step 1: Write failing tests.** `embedded_ask_skill_equals_the_repository_file`, `embedded_cli_skill_equals_the_repository_file`, `install_into_dir_writes_both_skills`, `only_ask_writes_one_skill`, `a_differing_folder_is_kept_without_force`, `force_replaces_it_and_names_the_files`, `a_symlinked_folder_is_refused` (`#[cfg(unix)]`), `known_targets_use_the_home_folder_on_windows_and_unix` (inject the home), `missing_target_parents_are_skipped_with_a_message`. The Python tests cover the two new comparisons.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib skill` and the Python test. Expected: FAIL.
- [ ] **Step 3: Implement.** Do all writes through Task 1. Do not write a marker or a record of installs.
- [ ] **Step 4: Run** the same commands and `python3 -I .github/scripts/check-agent-guidance.py`. Expected: PASS.

### Task 7: Guidance for the server

**Files:**
- Modify: `tui/src/cli/commands/mcp/guidance.md`, `skills/kimun-ask/SKILL.md` (regenerated from the guidance and its frontmatter), and the embedded copy rules from Task 6 if the frontmatter changes
- Test: the Rust and Python checks from earlier tasks

**Interfaces:**
- Produces: the full method gets two additions. The short block does not change.
  - The server rule: once per session, in one sentence, only when the agent tried `semantic_search` or the question is about meaning, the agent says what the tool reported and gives the command from the hint. It never delays or changes the answer. For exact words, tags, links and date questions it says nothing. It never runs the command.
  - The older-binary line: if `vault_info` or `semantic_search` does not exist, this Kimün build is older than the skill. Tell the user to update Kimün, and use `search_notes` with the query syntax meanwhile.

- [ ] **Step 1: Write a failing test.** `full_guidance_has_the_server_rule_and_the_older_binary_line`, and `short_block_does_not_mention_the_server_rule` in `mcp/mod.rs`.
- [ ] **Step 2: Run** `cargo test -p kimun-notes --lib mcp::`. Expected: FAIL.
- [ ] **Step 3: Edit** `guidance.md` and regenerate `SKILL.md`.
- [ ] **Step 4: Run** `cargo test -p kimun-notes --lib mcp::` and `python3 -I .github/scripts/check-agent-guidance.py`. Expected: PASS.

### Task 8: Documentation

**Files:**
- Modify: `docs/content/using-kimun/ai.md`, `ai-mcp-server.md`, `ai-skills.md`, `server.md`, `cli.md`, and the README paragraph about skills (the `cp -r skills/...` lines)

**Interfaces:**
- Produces:
  - `ai.md` is the start page. It opens with a table: works without a server (`search_notes` with its date filters, `get_chunks`, `show_note`, the links and properties tools, the writing tools, `vault_info`) and needs a server with an embedder and no LLM key (`semantic_search`). Then the quick start in three steps: the MCP server (`claude mcp add kimun -- kimun mcp`, and `kimun mcp --print-config` for other apps), the skills (`npx skills add nico2sh/kimun`, and `kimun skill install` without Node), and the optional server (the install line, `kimun server use http://localhost:7573`, `kimun server status`, and the restart of the agent app).
  - `ai-mcp-server.md` lists the tools and marks which need a server. It describes `vault_info` and the states, including Stopped.
  - `ai-skills.md` covers both skills, the standard tool with `npx skills update`, and the fallback with its refusal rules and `--force`.
  - `server.md` adds the CLI way to set and clear the address (`kimun server use`, `--off`, `status`) next to Preferences, and says that an agent app must restart after a change.
  - `cli.md` documents `kimun server`, `kimun skill install` and `kimun mcp --print-config`.

- [ ] **Step 1: Check each command** named in the docs against `kimun --help` and the subcommand help of the built binary. A command that does not exist is a defect.
- [ ] **Step 2: Write the pages** in plain English: short sentences, no semicolons or em-dashes, one word for one meaning, imperative mood for steps.
- [ ] **Step 3: Check the docs build** with the project's docs tool if it is installed (look at `docs/config.toml`). If it is not installed, say so in the report.
- [ ] **Step 4: Check links** between the pages, for example with a search for each `@/using-kimun/` link target.

### Task 9: Final gates and owner checks

**Files:** none.

- [ ] **Step 1: Touch the sources, then run the seven CI gates**: fmt (apply, then check, and check again last), the module boundary grep, `check-host-fs.py`, `check-agent-guidance.py`, clippy with `-D warnings`, the bench-profile check, docs with `RUSTDOCFLAGS="-D warnings"`, and the workspace tests. Expected: all pass.
- [ ] **Step 2: Hand three checks to the owner.** First, in an empty scratch folder, run `npx skills add /home/nico/Development/kimun --list` to see that the Skills CLI finds `kimun-ask` and `kimun-cli` (it runs downloaded code, so it is the owner's call). Second, run `kimun server status`, `kimun mcp --print-config` and `kimun skill install --dir <temp folder>` with the new build. Third, confirm the Codex skills folder against the Codex docs.
