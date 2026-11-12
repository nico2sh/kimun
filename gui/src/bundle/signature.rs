//! Whether a staged `Kimun.app` may replace the running one.
//!
//! Two checks, both through `codesign`, both required (ADR-0056):
//!
//! 1. the staged app's seal is intact and its signature chains to Apple as a
//!    Developer ID Application certificate (the same requirement `install.sh`
//!    enforces: not ad-hoc, not self-signed);
//! 2. its Team ID equals the running app's, so a validly signed app from
//!    another developer is refused.
//!
//! A running app with no Team ID (a development or ad-hoc build) never
//! updates itself. Notarization is not checked, as in `install.sh`.

// Only the macOS installer calls this; the unit tests build it without the app.
#![cfg_attr(test, allow(dead_code))]

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

/// The requirement a Developer ID Application signature meets (`install.sh`
/// passes the same string).
const DEVELOPER_ID_REQUIREMENT: &str = "=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists";

/// What a finished command said.
#[derive(Debug, Clone)]
pub(crate) struct Output {
    pub(crate) success: bool,
    /// `codesign` reports its details on standard error.
    pub(crate) stderr: String,
}

/// Runs a program: the seam the tests replace with canned output.
pub(crate) trait CommandRunner {
    /// Run `program` with `args` to the end. `Err`: it could not be started.
    fn run(&self, program: &str, args: &[&OsStr]) -> Result<Output, String>;
}

impl<T: CommandRunner + ?Sized> CommandRunner for &T {
    fn run(&self, program: &str, args: &[&OsStr]) -> Result<Output, String> {
        (**self).run(program, args)
    }
}

/// The real runner.
#[cfg(any(target_os = "macos", test))]
pub(crate) struct SystemRunner;

#[cfg(any(target_os = "macos", test))]
impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&OsStr]) -> Result<Output, String> {
        let output = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("could not run {program}: {e}"))?;
        Ok(Output {
            success: output.status.success(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Why an app may not replace the running one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SignatureError {
    /// `codesign` could not be run.
    Tool(String),
    /// The staged app's signature is not a valid Developer ID signature.
    Invalid(String),
    /// The running app has no Team ID (a development or ad-hoc build).
    RunningUnsigned,
    /// The staged app has no Team ID.
    StagedUnsigned,
    /// The staged app is signed by another team.
    OtherTeam { staged: String, running: String },
}

impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tool(e) => write!(f, "{e}"),
            Self::Invalid(why) => write!(
                f,
                "the new app does not have a valid Developer ID signature ({why})"
            ),
            Self::RunningUnsigned => write!(
                f,
                "this copy is not signed with a Developer ID certificate, so it cannot check the new app"
            ),
            Self::StagedUnsigned => write!(f, "the new app has no signing team"),
            Self::OtherTeam { staged, running } => write!(
                f,
                "the new app is signed by another team ({staged}, this copy: {running})"
            ),
        }
    }
}

/// The signature checks, over a [`CommandRunner`].
pub(crate) struct BundleSignature<R> {
    runner: R,
}

impl<R: CommandRunner> BundleSignature<R> {
    pub(crate) fn new(runner: R) -> Self {
        Self { runner }
    }

    /// `staged` may replace `running`: it verifies, and both carry the same
    /// Team ID. The running app is read first: a build that cannot vouch for
    /// the new one fails before anything else runs.
    pub(crate) fn trusts(&self, staged: &Path, running: &Path) -> Result<(), SignatureError> {
        let running_team = self
            .team_id(running)?
            .ok_or(SignatureError::RunningUnsigned)?;
        self.verify(staged)?;
        let staged_team = self
            .team_id(staged)?
            .ok_or(SignatureError::StagedUnsigned)?;
        if staged_team == running_team {
            Ok(())
        } else {
            Err(SignatureError::OtherTeam {
                staged: staged_team,
                running: running_team,
            })
        }
    }

    /// The Developer ID signature of `app` is valid.
    pub(crate) fn verify(&self, app: &Path) -> Result<(), SignatureError> {
        let output = self
            .runner
            .run(
                "codesign",
                &[
                    OsStr::new("--verify"),
                    OsStr::new("--deep"),
                    OsStr::new("--strict"),
                    OsStr::new("-R"),
                    OsStr::new(DEVELOPER_ID_REQUIREMENT),
                    app.as_os_str(),
                ],
            )
            .map_err(SignatureError::Tool)?;
        if output.success {
            Ok(())
        } else {
            let why = output
                .stderr
                .lines()
                .next()
                .unwrap_or("codesign refused it");
            Err(SignatureError::Invalid(why.trim().to_string()))
        }
    }

    /// The Team ID `app` is signed with: `None` for an app with no signature
    /// or no team (ad-hoc, development).
    pub(crate) fn team_id(&self, app: &Path) -> Result<Option<String>, SignatureError> {
        let output = self
            .runner
            .run(
                "codesign",
                &[
                    OsStr::new("-dv"),
                    OsStr::new("--verbose=2"),
                    app.as_os_str(),
                ],
            )
            .map_err(SignatureError::Tool)?;
        if !output.success {
            return Ok(None);
        }
        Ok(output
            .stderr
            .lines()
            .find_map(|line| line.strip_prefix("TeamIdentifier="))
            .map(str::trim)
            .filter(|id| !id.is_empty() && *id != "not set")
            .map(str::to_string))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    const STAGED: &str = "/apps/.Kimun.new/Kimun.app";
    const RUNNING: &str = "/apps/Kimun.app";

    /// Canned `codesign`: per app path, whether verify passes and what
    /// `-dv` prints. Records every call.
    #[derive(Default)]
    struct Fake {
        verify_ok: HashMap<String, bool>,
        details: HashMap<String, String>,
        missing_tool: bool,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl Fake {
        fn app(mut self, path: &str, verify_ok: bool, team: Option<&str>) -> Self {
            self.verify_ok.insert(path.to_string(), verify_ok);
            let team_line = match team {
                Some(id) => format!("TeamIdentifier={id}"),
                None => "TeamIdentifier=not set".to_string(),
            };
            self.details.insert(
                path.to_string(),
                format!(
                    "Executable={path}/Contents/MacOS/kimun-gui\nIdentifier=io.kimun.app\n{team_line}\nSealed Resources version=2\n"
                ),
            );
            self
        }
    }

    impl CommandRunner for Fake {
        fn run(&self, program: &str, args: &[&OsStr]) -> Result<Output, String> {
            let args: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            self.calls.borrow_mut().push(args.clone());
            if self.missing_tool {
                return Err(format!("could not run {program}: not found"));
            }
            assert_eq!(program, "codesign");
            let app = args.last().cloned().unwrap();
            if args.iter().any(|a| a == "--verify") {
                let ok = self.verify_ok.get(&app).copied().unwrap_or(false);
                return Ok(Output {
                    success: ok,
                    stderr: if ok {
                        String::new()
                    } else {
                        format!("{app}: a sealed resource is missing or invalid")
                    },
                });
            }
            assert!(args.iter().any(|a| a == "-dv"), "unexpected call {args:?}");
            match self.details.get(&app) {
                Some(text) => Ok(Output {
                    success: true,
                    stderr: text.clone(),
                }),
                None => Ok(Output {
                    success: false,
                    stderr: format!("{app}: code object is not signed at all"),
                }),
            }
        }
    }

    fn check(fake: Fake) -> (Result<(), SignatureError>, Vec<Vec<String>>) {
        let signature = BundleSignature::new(fake);
        let result = signature.trusts(Path::new(STAGED), Path::new(RUNNING));
        let calls = signature.runner.calls.borrow().clone();
        (result, calls)
    }

    #[test]
    fn a_valid_app_from_the_same_team_is_trusted() {
        let (result, _) = check(Fake::default().app(STAGED, true, Some("ABCDE12345")).app(
            RUNNING,
            true,
            Some("ABCDE12345"),
        ));
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn verification_asks_for_the_developer_id_requirement() {
        let (_, calls) = check(Fake::default().app(STAGED, true, Some("ABCDE12345")).app(
            RUNNING,
            true,
            Some("ABCDE12345"),
        ));
        let verify = calls
            .iter()
            .find(|c| c.iter().any(|a| a == "--verify"))
            .expect("the staged app is verified");
        assert!(verify.iter().any(|a| a == "--deep"));
        assert!(verify.iter().any(|a| a == "--strict"));
        assert!(verify.iter().any(|a| a == "-R"));
        assert!(verify.iter().any(|a| a == DEVELOPER_ID_REQUIREMENT));
        assert_eq!(verify.last().map(String::as_str), Some(STAGED));
    }

    #[test]
    fn an_invalid_signature_is_refused() {
        let (result, _) = check(Fake::default().app(STAGED, false, Some("ABCDE12345")).app(
            RUNNING,
            true,
            Some("ABCDE12345"),
        ));
        match result {
            Err(SignatureError::Invalid(why)) => assert!(why.contains("sealed resource")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn another_team_is_refused() {
        let (result, _) = check(Fake::default().app(STAGED, true, Some("ZZZZZ99999")).app(
            RUNNING,
            true,
            Some("ABCDE12345"),
        ));
        assert_eq!(
            result,
            Err(SignatureError::OtherTeam {
                staged: "ZZZZZ99999".into(),
                running: "ABCDE12345".into(),
            })
        );
    }

    #[test]
    fn a_running_app_without_a_team_never_updates_and_nothing_else_runs() {
        let (result, calls) = check(
            Fake::default()
                .app(STAGED, true, Some("ABCDE12345"))
                .app(RUNNING, true, None),
        );
        assert_eq!(result, Err(SignatureError::RunningUnsigned));
        assert!(
            calls
                .iter()
                .all(|c| c.last().map(String::as_str) == Some(RUNNING)),
            "the staged app is never looked at: {calls:?}"
        );
    }

    #[test]
    fn a_staged_app_without_a_team_is_refused() {
        let (result, _) = check(Fake::default().app(STAGED, true, None).app(
            RUNNING,
            true,
            Some("ABCDE12345"),
        ));
        assert_eq!(result, Err(SignatureError::StagedUnsigned));
    }

    #[test]
    fn an_unsigned_running_app_is_refused_as_unsigned() {
        // `codesign -dv` fails on an app with no signature at all.
        let (result, _) = check(Fake::default().app(STAGED, true, Some("ABCDE12345")));
        assert_eq!(result, Err(SignatureError::RunningUnsigned));
    }

    #[test]
    fn a_missing_codesign_is_a_tool_error() {
        let fake = Fake {
            missing_tool: true,
            ..Fake::default()
        };
        let (result, _) = check(fake);
        assert!(matches!(result, Err(SignatureError::Tool(_))), "{result:?}");
    }

    #[test]
    fn team_id_is_read_from_the_details_and_ignores_other_lines() {
        let signature = BundleSignature::new(Fake::default().app(STAGED, true, Some("ABCDE12345")));
        assert_eq!(
            signature.team_id(Path::new(STAGED)),
            Ok(Some("ABCDE12345".to_string()))
        );
    }

    #[test]
    fn malformed_details_have_no_team() {
        let mut fake = Fake::default();
        fake.details
            .insert(STAGED.to_string(), "garbage\nTeamIdentifier=\n".to_string());
        let signature = BundleSignature::new(fake);
        assert_eq!(signature.team_id(Path::new(STAGED)), Ok(None));
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(
            SignatureError::OtherTeam {
                staged: "B".into(),
                running: "A".into()
            }
            .to_string(),
            "the new app is signed by another team (B, this copy: A)"
        );
        assert!(
            SignatureError::RunningUnsigned
                .to_string()
                .contains("not signed with a Developer ID")
        );
    }
}
