// tui/src/cli/commands/skill.rs
//
// Skill CLI commands: `install` copies the Kimün agent skills into the skill
// folders of agent apps. It writes only the skill folders, through
// `kimun_core::system::install_files`, and keeps no record of what it did
// (ADR 0066).

use std::io::Write;
use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{Result, eyre};
use kimun_core::SystemPath;
use kimun_core::system::{self, FolderInstall, SystemError};

use super::mcp::GUIDANCE;
use crate::cli::UserError;

/// The bytes of `skills/kimun-cli/SKILL.md`. CI checks that they match.
const CLI_SKILL: &str = include_str!("skill/kimun-cli.md");
/// The bytes of `skills/kimun-wiki/SKILL.md`. CI checks that they match.
const WIKI_SKILL: &str = include_str!("skill/kimun-wiki.md");
/// The bytes of `skills/kimun-ask/SKILL.md` before the guidance text. CI checks
/// that this text followed by `guidance.md` matches the repository skill.
const ASK_FRONTMATTER: &str = include_str!("skill/kimun-ask.frontmatter.md");

#[derive(Subcommand, Debug)]
pub enum SkillSubcommand {
    /// Copy the Kimün skills into the skill folders of your agent apps
    Install {
        /// Install only this skill
        #[arg(long, value_enum)]
        only: Option<SkillName>,
        /// Install into PATH, one folder per skill, instead of the agent app folders
        #[arg(long, value_name = "PATH")]
        dir: Option<PathBuf>,
        /// Replace skill files that differ from this version
        #[arg(long)]
        force: bool,
    },
}

/// The skills that `--only` can choose.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillName {
    /// The kimun-ask skill
    Ask,
    /// The kimun-cli skill
    Cli,
    /// The kimun-wiki skill
    Wiki,
}

impl SkillName {
    fn folder(self) -> &'static str {
        match self {
            SkillName::Ask => "kimun-ask",
            SkillName::Cli => "kimun-cli",
            SkillName::Wiki => "kimun-wiki",
        }
    }
}

/// One skill as it is embedded in the binary: its folder name and its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedSkill {
    pub name: &'static str,
    pub files: Vec<(&'static str, Vec<u8>)>,
}

/// The skills this binary carries.
pub fn embedded_skills() -> [EmbeddedSkill; 3] {
    let ask = [ASK_FRONTMATTER.as_bytes(), GUIDANCE.as_bytes()].concat();
    [
        EmbeddedSkill {
            name: SkillName::Ask.folder(),
            files: vec![("SKILL.md", ask)],
        },
        EmbeddedSkill {
            name: SkillName::Cli.folder(),
            files: vec![("SKILL.md", CLI_SKILL.as_bytes().to_vec())],
        },
        EmbeddedSkill {
            name: SkillName::Wiki.folder(),
            files: vec![("SKILL.md", WIKI_SKILL.as_bytes().to_vec())],
        },
    ]
}

/// The skill folders of the agent apps that Kimün knows, under `home`.
pub fn known_targets(home: &SystemPath) -> Vec<(&'static str, SystemPath)> {
    vec![
        ("Claude Code", home.join(".claude").join("skills")),
        // The Codex docs name `$HOME/.agents/skills` as the user skills folder (developers.openai.com/codex/skills).
        ("Codex", home.join(".agents").join("skills")),
    ]
}

/// What `install` should do.
#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub only: Option<SkillName>,
    pub dir: Option<SystemPath>,
    pub force: bool,
}

/// Installs the skills and writes one line per folder to `out`.
///
/// `home` is the home folder, used only without `dir`. Returns `true` when
/// every skill folder was written or already up to date.
pub fn install(
    home: Option<&SystemPath>,
    out: &mut impl Write,
    opts: &InstallOptions,
) -> std::io::Result<bool> {
    let skills: Vec<EmbeddedSkill> = embedded_skills()
        .into_iter()
        .filter(|s| opts.only.is_none_or(|only| only.folder() == s.name))
        .collect();

    let targets: Vec<(Option<&str>, SystemPath)> = match (&opts.dir, home) {
        (Some(dir), _) => vec![(None, dir.clone())],
        (None, Some(home)) => {
            let mut found = Vec::new();
            for (app, target) in known_targets(home) {
                // The app folder (for example ~/.claude) shows the app is installed.
                let app_dir = target.parent().unwrap_or_else(|| target.clone());
                if app_dir.is_dir() {
                    found.push((Some(app), target));
                } else {
                    writeln!(
                        out,
                        "Skipped {app}: {} does not exist. Use --dir to install somewhere else.",
                        app_dir.as_path().display()
                    )?;
                }
            }
            found
        }
        (None, None) => {
            writeln!(
                out,
                "The home folder could not be found. Use --dir to choose where to install."
            )?;
            return Ok(false);
        }
    };
    if targets.is_empty() {
        writeln!(
            out,
            "No agent app folder was found, so nothing was installed. Use --dir to choose where to install."
        )?;
        return Ok(false);
    }

    let mut all_ok = true;
    for (app, target) in &targets {
        let prefix = app.map(|a| format!("{a}: ")).unwrap_or_default();
        for skill in &skills {
            let folder = target.join(skill.name);
            let shown = folder.as_path().display();
            let files: Vec<(&str, &[u8])> = skill
                .files
                .iter()
                .map(|(n, b)| (*n, b.as_slice()))
                .collect();
            match system::install_files(&folder, &files, opts.force) {
                Ok(FolderInstall::Created) => writeln!(out, "{prefix}{shown} written")?,
                Ok(FolderInstall::Unchanged) => writeln!(out, "{prefix}{shown} unchanged")?,
                Ok(FolderInstall::Replaced(names)) => {
                    writeln!(out, "{prefix}{shown} replaced {}", names.join(", "))?
                }
                Err(SystemError::AlreadyExists { .. }) => {
                    all_ok = false;
                    writeln!(
                        out,
                        "{prefix}{shown} kept because it differs from this version. Use --force to replace it."
                    )?
                }
                Err(SystemError::Symlink { .. }) => {
                    all_ok = false;
                    writeln!(
                        out,
                        "{prefix}{shown} refused because it is a symbolic link managed by another tool. Nothing was changed."
                    )?
                }
                Err(e) => {
                    all_ok = false;
                    writeln!(out, "{prefix}{shown} failed: {e}")?
                }
            }
        }
    }
    Ok(all_ok)
}

pub fn run(sub: SkillSubcommand) -> Result<()> {
    let SkillSubcommand::Install { only, dir, force } = sub;
    let dir = match dir {
        Some(d) => {
            let cwd = std::env::current_dir().map_err(|e| eyre!("{e}"))?;
            let cwd = SystemPath::try_absolute(cwd).map_err(|e| eyre!("{e}"))?;
            Some(SystemPath::resolve(d, &cwd))
        }
        None => None,
    };
    let home = system::home().ok();
    let opts = InstallOptions { only, dir, force };
    let ok = install(home.as_ref(), &mut std::io::stdout().lock(), &opts)?;
    if ok {
        Ok(())
    } else {
        Err(UserError("Some skills were not installed. See the lines above.".to_string()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn abs(p: impl AsRef<std::path::Path>) -> SystemPath {
        SystemPath::try_absolute(p).unwrap()
    }

    fn repo_file(rel: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(rel);
        std::fs::read(path).unwrap()
    }

    fn skill(name: &str) -> EmbeddedSkill {
        embedded_skills()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    fn opts(dir: &SystemPath, only: Option<SkillName>, force: bool) -> InstallOptions {
        InstallOptions {
            only,
            dir: Some(dir.clone()),
            force,
        }
    }

    fn run_install(home: Option<&SystemPath>, opts: &InstallOptions) -> (bool, String) {
        let mut out = Vec::new();
        let ok = install(home, &mut out, opts).unwrap();
        (ok, String::from_utf8(out).unwrap())
    }

    #[test]
    fn embedded_ask_skill_equals_the_repository_file() {
        let s = skill("kimun-ask");
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].0, "SKILL.md");
        assert_eq!(s.files[0].1, repo_file("skills/kimun-ask/SKILL.md"));
    }

    #[test]
    fn embedded_cli_skill_equals_the_repository_file() {
        let s = skill("kimun-cli");
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].0, "SKILL.md");
        assert_eq!(s.files[0].1, repo_file("skills/kimun-cli/SKILL.md"));
    }

    #[test]
    fn embedded_wiki_skill_equals_the_repository_file() {
        let s = skill("kimun-wiki");
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].0, "SKILL.md");
        assert_eq!(s.files[0].1, repo_file("skills/kimun-wiki/SKILL.md"));
    }

    #[test]
    fn install_into_dir_writes_every_skill() {
        let tmp = TempDir::new().unwrap();
        let dir = abs(tmp.path());
        let (ok, text) = run_install(None, &opts(&dir, None, false));
        assert!(ok, "{text}");
        for name in ["kimun-ask", "kimun-cli", "kimun-wiki"] {
            let written = std::fs::read(dir.join(name).join("SKILL.md")).unwrap();
            assert_eq!(written, skill(name).files[0].1);
        }
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(text.lines().all(|l| l.contains("written")), "{text}");
    }

    #[test]
    fn only_ask_writes_one_skill() {
        let tmp = TempDir::new().unwrap();
        let dir = abs(tmp.path());
        let (ok, text) = run_install(None, &opts(&dir, Some(SkillName::Ask), false));
        assert!(ok, "{text}");
        assert!(dir.join("kimun-ask").join("SKILL.md").exists());
        assert!(!dir.join("kimun-cli").exists());
        assert_eq!(text.lines().count(), 1, "{text}");
    }

    #[test]
    fn only_wiki_writes_one_skill() {
        let tmp = TempDir::new().unwrap();
        let dir = abs(tmp.path());
        let (ok, text) = run_install(None, &opts(&dir, Some(SkillName::Wiki), false));
        assert!(ok, "{text}");
        assert!(dir.join("kimun-wiki").join("SKILL.md").exists());
        assert!(!dir.join("kimun-ask").exists());
        assert!(!dir.join("kimun-cli").exists());
        assert_eq!(text.lines().count(), 1, "{text}");
    }

    #[test]
    fn a_differing_folder_is_kept_without_force() {
        let tmp = TempDir::new().unwrap();
        let dir = abs(tmp.path());
        system::install_files(&dir.join("kimun-ask"), &[("SKILL.md", b"mine")], false).unwrap();
        let (ok, text) = run_install(None, &opts(&dir, Some(SkillName::Ask), false));
        assert!(!ok);
        assert!(text.contains("kept"), "{text}");
        assert!(text.contains("--force"), "{text}");
        let kept = std::fs::read(dir.join("kimun-ask").join("SKILL.md")).unwrap();
        assert_eq!(kept, b"mine");
    }

    #[test]
    fn force_replaces_it_and_names_the_files() {
        let tmp = TempDir::new().unwrap();
        let dir = abs(tmp.path());
        system::install_files(&dir.join("kimun-ask"), &[("SKILL.md", b"mine")], false).unwrap();
        let (ok, text) = run_install(None, &opts(&dir, Some(SkillName::Ask), true));
        assert!(ok, "{text}");
        assert!(text.contains("replaced"), "{text}");
        assert!(text.contains("SKILL.md"), "{text}");
        let now = std::fs::read(dir.join("kimun-ask").join("SKILL.md")).unwrap();
        assert_eq!(now, skill("kimun-ask").files[0].1);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_folder_is_refused() {
        let tmp = TempDir::new().unwrap();
        let dir = abs(tmp.path());
        let real = dir.join("elsewhere");
        system::install_files(&real, &[("other.md", b"x")], false).unwrap();
        std::os::unix::fs::symlink(real.as_path(), dir.join("kimun-ask").as_path()).unwrap();
        let (ok, text) = run_install(None, &opts(&dir, Some(SkillName::Ask), true));
        assert!(!ok);
        assert!(text.contains("symbolic link"), "{text}");
        assert!(!real.join("SKILL.md").exists());
    }

    #[test]
    fn known_targets_use_the_home_folder() {
        let tmp = TempDir::new().unwrap();
        let home = abs(tmp.path());
        let targets = known_targets(&home);
        assert_eq!(
            targets,
            vec![
                ("Claude Code", home.join(".claude").join("skills")),
                ("Codex", home.join(".agents").join("skills")),
            ]
        );
    }

    #[test]
    fn missing_target_parents_are_skipped_with_a_message() {
        let tmp = TempDir::new().unwrap();
        let home = abs(tmp.path());
        system::ensure_dir(&home.join(".claude")).unwrap();
        let o = InstallOptions {
            only: None,
            dir: None,
            force: false,
        };
        let (ok, text) = run_install(Some(&home), &o);
        assert!(ok, "{text}");
        let claude = home.join(".claude").join("skills");
        assert!(claude.join("kimun-ask").join("SKILL.md").exists());
        assert!(claude.join("kimun-cli").join("SKILL.md").exists());
        assert!(claude.join("kimun-wiki").join("SKILL.md").exists());
        assert!(!home.join(".agents").exists());
        let skipped: Vec<&str> = text.lines().filter(|l| l.contains("Skipped")).collect();
        assert_eq!(skipped.len(), 1, "{text}");
        assert!(skipped[0].contains(".agents"), "{text}");
        assert!(skipped[0].contains("--dir"), "{text}");
    }

    #[test]
    fn no_target_at_all_returns_failure() {
        let tmp = TempDir::new().unwrap();
        let home = abs(tmp.path());
        let o = InstallOptions {
            only: None,
            dir: None,
            force: false,
        };
        let (ok, text) = run_install(Some(&home), &o);
        assert!(!ok);
        assert!(text.contains("--dir"), "{text}");
        assert!(!home.join(".claude").exists());
        assert!(!home.join(".agents").exists());
    }

    #[test]
    fn a_second_install_reports_unchanged() {
        let tmp = TempDir::new().unwrap();
        let dir = abs(tmp.path());
        run_install(None, &opts(&dir, None, false));
        let (ok, text) = run_install(None, &opts(&dir, None, false));
        assert!(ok, "{text}");
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(text.lines().all(|l| l.contains("unchanged")), "{text}");
    }

    mod parse {
        use crate::cli::CliCommand;
        use clap::Parser;

        use super::super::{SkillName, SkillSubcommand};

        #[derive(Parser)]
        struct Wrap {
            #[command(subcommand)]
            cmd: CliCommand,
        }

        fn parse(args: &[&str]) -> Result<SkillSubcommand, clap::Error> {
            let mut argv = vec!["kimun"];
            argv.extend_from_slice(args);
            match Wrap::try_parse_from(argv)?.cmd {
                CliCommand::Skill { subcommand } => Ok(subcommand),
                _ => panic!("not a skill command"),
            }
        }

        #[test]
        fn skill_install_parses() {
            let SkillSubcommand::Install { only, dir, force } =
                parse(&["skill", "install"]).unwrap();
            assert_eq!((only, dir, force), (None, None, false));

            let SkillSubcommand::Install { only, .. } =
                parse(&["skill", "install", "--only", "ask"]).unwrap();
            assert_eq!(only, Some(SkillName::Ask));

            let SkillSubcommand::Install { only, .. } =
                parse(&["skill", "install", "--only", "wiki"]).unwrap();
            assert_eq!(only, Some(SkillName::Wiki));

            let SkillSubcommand::Install { only, dir, force } =
                parse(&["skill", "install", "--dir", "X", "--force"]).unwrap();
            assert_eq!(only, None);
            assert_eq!(dir, Some(std::path::PathBuf::from("X")));
            assert!(force);
        }

        #[test]
        fn an_unknown_skill_name_is_rejected() {
            assert!(parse(&["skill", "install", "--only", "other"]).is_err());
        }
    }
}
