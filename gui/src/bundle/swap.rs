//! Replacing the installed `Kimun.app` with the one inside a release `.dmg`
//! (ADR-0056). The steps mirror `install.sh`:
//!
//! 1. mount the image read-only, copy the app next to the installed one (same
//!    volume, so the final moves are renames), unmount;
//! 2. check the staged copy ([`BundleSignature::trusts`]) before anything
//!    moves;
//! 3. move the old app aside, move the new one in, and put the old one back if
//!    the second move fails.
//!
//! Every program call goes through a [`CommandRunner`], so the tests drive the
//! whole sequence on any platform against fake apps in a temp folder.

// Only the macOS installer calls this; the unit tests build it without the app.
#![cfg_attr(test, allow(dead_code))]

use std::cell::Cell;
use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use super::signature::{BundleSignature, CommandRunner, SignatureError};

/// The folder the new app is copied into, next to the installed app.
const STAGING_DIR: &str = ".Kimun.new";
/// Where the old app waits while the new one moves in.
const BACKUP_DIR: &str = ".Kimun.app.old";

/// Why the app was not replaced. In every case the installed app is as it was.
#[derive(Debug)]
pub(crate) enum SwapError {
    /// A program could not run, or refused.
    Tool(String),
    /// The image holds no app.
    NoApp,
    /// The signature checks refused the new app.
    Signature(SignatureError),
    /// A file operation failed.
    Io(String),
}

impl fmt::Display for SwapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tool(e) | Self::Io(e) => write!(f, "{e}"),
            Self::NoApp => write!(f, "the downloaded image holds no Kimun.app"),
            Self::Signature(e) => write!(f, "{e}"),
        }
    }
}

type RenameFn = Box<dyn Fn(&Path, &Path) -> io::Result<()>>;

/// The swap, for one installed app.
pub(crate) struct BundleSwap<'a, R> {
    runner: &'a R,
    installed: PathBuf,
    rename: RenameFn,
    /// An image could not be unmounted: nothing under the scratch folder may be
    /// deleted, or the delete would reach into the live volume.
    mount_left: Cell<bool>,
}

impl<'a, R: CommandRunner> BundleSwap<'a, R> {
    pub(crate) fn new(runner: &'a R, installed: PathBuf) -> Self {
        Self {
            runner,
            installed,
            rename: Box::new(|from, to| std::fs::rename(from, to)),
            mount_left: Cell::new(false),
        }
    }

    /// Replace the installed app with the image whose verified bytes are
    /// `image`: write them to a fresh folder inside `scratch_root`, install, and
    /// remove that folder on every path.
    pub(crate) fn install_image(&self, image: &[u8], scratch_root: &Path) -> Result<(), SwapError> {
        let work = scratch_root.join(format!(
            "kimun-update-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&work).map_err(io_error("could not prepare the update"))?;
        let dmg = work.join("kimun.dmg");
        let result = std::fs::write(&dmg, image)
            .map_err(io_error("could not save the downloaded update"))
            .and_then(|()| self.install_dmg(&dmg, &work));
        if self.mount_left.get() {
            tracing::warn!(
                "the update image is still mounted under {}; leaving the folder alone",
                work.display()
            );
        } else {
            let _ = std::fs::remove_dir_all(&work);
        }
        result
    }

    /// Replace the installed app with the one in `dmg`. `work` is an empty
    /// scratch folder the caller owns (the image is mounted inside it). The
    /// staging folder is removed on every path.
    pub(crate) fn install_dmg(&self, dmg: &Path, work: &Path) -> Result<(), SwapError> {
        let parent = self
            .installed
            .parent()
            .ok_or_else(|| SwapError::Io("the installed app has no folder".into()))?;
        let staging = parent.join(STAGING_DIR);
        let result = self.stage_check_swap(dmg, work, parent, &staging);
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    fn stage_check_swap(
        &self,
        dmg: &Path,
        work: &Path,
        parent: &Path,
        staging: &Path,
    ) -> Result<(), SwapError> {
        let staged = self.stage(dmg, work, staging)?;
        BundleSignature::new(self.runner)
            .trusts(&staged, &self.installed)
            .map_err(SwapError::Signature)?;
        self.swap_in(&staged, parent)
    }

    /// Mount `dmg` inside `work`, copy its app into `staging`, unmount. The
    /// image is unmounted whether or not the copy worked.
    fn stage(&self, dmg: &Path, work: &Path, staging: &Path) -> Result<PathBuf, SwapError> {
        let mount = work.join("mnt");
        std::fs::create_dir_all(&mount).map_err(io_error("could not prepare the update"))?;
        let attached = self
            .runner
            .run(
                "hdiutil",
                &[
                    OsStr::new("attach"),
                    OsStr::new("-nobrowse"),
                    OsStr::new("-readonly"),
                    OsStr::new("-mountpoint"),
                    mount.as_os_str(),
                    dmg.as_os_str(),
                ],
            )
            .map_err(SwapError::Tool)?;
        if !attached.success {
            return Err(SwapError::Tool(first_line(
                &attached.stderr,
                "could not open the update image",
            )));
        }
        let copied = self.copy_app(&mount, staging);
        self.detach(&mount);
        copied
    }

    fn copy_app(&self, mount: &Path, staging: &Path) -> Result<PathBuf, SwapError> {
        let source = mount.join(APP_NAME);
        if !source.is_dir() {
            return Err(SwapError::NoApp);
        }
        let _ = std::fs::remove_dir_all(staging);
        std::fs::create_dir_all(staging).map_err(io_error(
            "could not copy the app next to the installed one (is its folder writable?)",
        ))?;
        let staged = staging.join(APP_NAME);
        // `ditto` keeps the signature's extended attributes and links intact.
        let copied = self
            .runner
            .run("ditto", &[source.as_os_str(), staged.as_os_str()])
            .map_err(SwapError::Tool)?;
        if copied.success {
            Ok(staged)
        } else {
            Err(SwapError::Tool(first_line(
                &copied.stderr,
                "could not copy the new app",
            )))
        }
    }

    /// Unmount, forcing it when the plain unmount is refused. A failure never
    /// decides the update, but it is remembered ([`Self::mount_left`]).
    fn detach(&self, mount: &Path) {
        let worked =
            |out: &Result<super::signature::Output, String>| matches!(out, Ok(o) if o.success);
        let plain = self
            .runner
            .run("hdiutil", &[OsStr::new("detach"), mount.as_os_str()]);
        if worked(&plain) {
            return;
        }
        let forced = self.runner.run(
            "hdiutil",
            &[
                OsStr::new("detach"),
                OsStr::new("-force"),
                mount.as_os_str(),
            ],
        );
        if !worked(&forced) {
            self.mount_left.set(true);
        }
    }

    /// Old app aside, new app in; the old app back if the second move fails.
    fn swap_in(&self, staged: &Path, parent: &Path) -> Result<(), SwapError> {
        let backup = parent.join(BACKUP_DIR);
        // The running app is at `installed` (the signature check read it), so a
        // backup here is a leftover of an earlier update whose cleanup failed:
        // the app it replaced is not the one needed for recovery. Say so in the
        // log instead of deleting it silently.
        if backup.exists() {
            tracing::warn!(
                "removing the old app left by an earlier update at {}",
                backup.display()
            );
        }
        let _ = std::fs::remove_dir_all(&backup);
        (self.rename)(&self.installed, &backup).map_err(io_error(
            "could not move the old app aside (is its folder writable?)",
        ))?;
        if let Err(e) = (self.rename)(staged, &self.installed) {
            return match (self.rename)(&backup, &self.installed) {
                Ok(()) => Err(SwapError::Io(format!(
                    "could not move the new app in ({e}); the old app is back"
                ))),
                Err(restore) => Err(SwapError::Io(format!(
                    "could not move the new app in ({e}) and could not put the old app back ({restore}); it is at {}",
                    backup.display()
                ))),
            };
        }
        let _ = std::fs::remove_dir_all(&backup);
        Ok(())
    }
}

/// The bundle's directory name inside the release image.
const APP_NAME: &str = "Kimun.app";

fn first_line(text: &str, fallback: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn io_error(context: &'static str) -> impl Fn(io::Error) -> SwapError {
    move |e| SwapError::Io(format!("{context}: {e}"))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;

    use super::*;
    use crate::bundle::signature::Output;

    const TEAM: &str = "ABCDE12345";

    /// Writes a fake app: a `VERSION` file, and markers the fake `codesign`
    /// reads (`TEAM`, `INVALID`).
    fn fake_app(dir: &Path, version: &str, team: Option<&str>, invalid: bool) {
        fs::create_dir_all(dir.join("Contents")).unwrap();
        fs::write(dir.join("VERSION"), version).unwrap();
        if let Some(team) = team {
            fs::write(dir.join("TEAM"), team).unwrap();
        }
        if invalid {
            fs::write(dir.join("INVALID"), "").unwrap();
        }
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// A fake `hdiutil`, `ditto` and `codesign` over real temp folders.
    struct Host {
        /// What mounting the image puts at `<mountpoint>/Kimun.app`.
        image: Option<(String, Option<String>, bool)>,
        attach_fails: bool,
        /// Both the plain and the forced `hdiutil detach` fail.
        detach_fails: bool,
        log: RefCell<Vec<String>>,
    }

    impl Host {
        fn new(version: &str, team: Option<&str>, invalid: bool) -> Self {
            Self {
                image: Some((version.into(), team.map(Into::into), invalid)),
                attach_fails: false,
                detach_fails: false,
                log: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self, program: &str) -> usize {
            self.log
                .borrow()
                .iter()
                .filter(|l| l.starts_with(program))
                .count()
        }
    }

    fn ok(stderr: &str) -> Result<Output, String> {
        Ok(Output {
            success: true,
            stderr: stderr.to_string(),
        })
    }

    impl CommandRunner for Host {
        fn run(&self, program: &str, args: &[&OsStr]) -> Result<Output, String> {
            let args: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            self.log
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            match (program, args.first().map(String::as_str)) {
                ("hdiutil", Some("attach")) => {
                    if self.attach_fails {
                        return Ok(Output {
                            success: false,
                            stderr: "hdiutil: attach failed".into(),
                        });
                    }
                    let at = args.iter().position(|a| a == "-mountpoint").unwrap();
                    let mount = Path::new(&args[at + 1]);
                    if let Some((version, team, invalid)) = &self.image {
                        fake_app(&mount.join("Kimun.app"), version, team.as_deref(), *invalid);
                    }
                    ok("")
                }
                ("hdiutil", Some("detach")) if self.detach_fails => Ok(Output {
                    success: false,
                    stderr: "hdiutil: couldn't unmount: resource busy".into(),
                }),
                ("hdiutil", Some("detach")) => ok(""),
                ("ditto", _) => {
                    copy_dir(Path::new(&args[0]), Path::new(&args[1]));
                    ok("")
                }
                ("codesign", Some("--verify")) => {
                    let app = Path::new(args.last().unwrap());
                    let invalid = app.join("INVALID").exists();
                    Ok(Output {
                        success: !invalid,
                        stderr: if invalid {
                            "a sealed resource is missing or invalid".into()
                        } else {
                            String::new()
                        },
                    })
                }
                ("codesign", Some("-dv")) => {
                    let app = Path::new(args.last().unwrap());
                    let team = fs::read_to_string(app.join("TEAM"))
                        .unwrap_or_else(|_| "not set".to_string());
                    ok(&format!("Identifier=io.kimun.app\nTeamIdentifier={team}\n"))
                }
                other => panic!("unexpected call {other:?}"),
            }
        }
    }

    /// `<root>/apps/Kimun.app` installed as version `old`, a scratch folder,
    /// and a (never read) dmg path.
    struct Setup {
        _root: tempfile::TempDir,
        apps: PathBuf,
        installed: PathBuf,
        work: PathBuf,
        dmg: PathBuf,
    }

    fn setup() -> Setup {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        let installed = apps.join("Kimun.app");
        fake_app(&installed, "old", Some(TEAM), false);
        let work = root.path().join("work");
        fs::create_dir_all(&work).unwrap();
        Setup {
            apps,
            installed,
            work,
            dmg: root.path().join("kimun.dmg"),
            _root: root,
        }
    }

    fn version(app: &Path) -> String {
        fs::read_to_string(app.join("VERSION")).unwrap()
    }

    fn assert_clean(s: &Setup) {
        let names: Vec<_> = fs::read_dir(&s.apps)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Kimun.app"], "no staging or backup left behind");
    }

    #[test]
    fn a_trusted_app_replaces_the_installed_one() {
        let s = setup();
        let host = Host::new("new", Some(TEAM), false);
        BundleSwap::new(&host, s.installed.clone())
            .install_dmg(&s.dmg, &s.work)
            .unwrap();
        assert_eq!(version(&s.installed), "new");
        assert_clean(&s);
        assert_eq!(host.calls("hdiutil detach"), 1, "the image is unmounted");
    }

    #[test]
    fn the_image_is_mounted_read_only_without_a_finder_window() {
        let s = setup();
        let host = Host::new("new", Some(TEAM), false);
        BundleSwap::new(&host, s.installed.clone())
            .install_dmg(&s.dmg, &s.work)
            .unwrap();
        let attach = host
            .log
            .borrow()
            .iter()
            .find(|l| l.starts_with("hdiutil attach"))
            .cloned()
            .unwrap();
        assert!(attach.contains("-readonly") && attach.contains("-nobrowse"));
        assert!(attach.contains(&*s.work.to_string_lossy()), "inside work");
    }

    #[test]
    fn an_invalid_signature_leaves_the_installed_app_alone() {
        let s = setup();
        let host = Host::new("new", Some(TEAM), true);
        let err = BundleSwap::new(&host, s.installed.clone())
            .install_dmg(&s.dmg, &s.work)
            .unwrap_err();
        assert!(matches!(
            err,
            SwapError::Signature(SignatureError::Invalid(_))
        ));
        assert_eq!(version(&s.installed), "old");
        assert_clean(&s);
        assert_eq!(host.calls("hdiutil detach"), 1);
    }

    #[test]
    fn another_team_leaves_the_installed_app_alone() {
        let s = setup();
        let host = Host::new("new", Some("ZZZZZ99999"), false);
        let err = BundleSwap::new(&host, s.installed.clone())
            .install_dmg(&s.dmg, &s.work)
            .unwrap_err();
        assert!(matches!(
            err,
            SwapError::Signature(SignatureError::OtherTeam { .. })
        ));
        assert_eq!(version(&s.installed), "old");
        assert_clean(&s);
    }

    #[test]
    fn a_failed_second_move_puts_the_old_app_back() {
        let s = setup();
        let host = Host::new("new", Some(TEAM), false);
        let mut swap = BundleSwap::new(&host, s.installed.clone());
        let calls = Cell::new(0);
        swap.rename = Box::new(move |from, to| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err(io::Error::other("disk full"))
            } else {
                fs::rename(from, to)
            }
        });
        let err = swap.install_dmg(&s.dmg, &s.work).unwrap_err();
        assert!(matches!(err, SwapError::Io(m) if m.contains("disk full")));
        assert_eq!(version(&s.installed), "old", "the old app is back");
        assert_clean(&s);
    }

    #[test]
    fn a_failed_mount_touches_nothing() {
        let s = setup();
        let mut host = Host::new("new", Some(TEAM), false);
        host.attach_fails = true;
        let err = BundleSwap::new(&host, s.installed.clone())
            .install_dmg(&s.dmg, &s.work)
            .unwrap_err();
        assert!(matches!(err, SwapError::Tool(m) if m.contains("attach failed")));
        assert_eq!(version(&s.installed), "old");
        assert_clean(&s);
        assert_eq!(host.calls("hdiutil detach"), 0, "nothing was mounted");
        assert_eq!(host.calls("ditto"), 0);
    }

    #[test]
    fn an_image_without_the_app_is_refused() {
        let s = setup();
        let mut host = Host::new("new", Some(TEAM), false);
        host.image = None;
        let err = BundleSwap::new(&host, s.installed.clone())
            .install_dmg(&s.dmg, &s.work)
            .unwrap_err();
        assert!(matches!(err, SwapError::NoApp));
        assert_eq!(version(&s.installed), "old");
        assert_clean(&s);
        assert_eq!(host.calls("hdiutil detach"), 1);
    }

    #[test]
    fn downloaded_image_bytes_are_written_mounted_and_cleaned_up() {
        let s = setup();
        let scratch = s.work.join("scratch-root");
        fs::create_dir_all(&scratch).unwrap();
        let host = Host::new("new", Some(TEAM), false);
        BundleSwap::new(&host, s.installed.clone())
            .install_image(b"dmg bytes", &scratch)
            .unwrap();
        assert_eq!(version(&s.installed), "new");
        let attach = host
            .log
            .borrow()
            .iter()
            .find(|l| l.starts_with("hdiutil attach"))
            .cloned()
            .unwrap();
        assert!(attach.contains(&*scratch.to_string_lossy()));
        assert_eq!(
            fs::read_dir(&scratch).unwrap().count(),
            0,
            "scratch is gone"
        );
    }

    #[test]
    fn a_mount_that_will_not_unmount_is_never_deleted() {
        let s = setup();
        let scratch = s.work.join("scratch-root");
        fs::create_dir_all(&scratch).unwrap();
        let mut host = Host::new("new", Some(TEAM), false);
        host.detach_fails = true;
        // The update itself still lands: the copy was made before the unmount.
        BundleSwap::new(&host, s.installed.clone())
            .install_image(b"dmg bytes", &scratch)
            .unwrap();
        assert_eq!(version(&s.installed), "new");
        assert_eq!(host.calls("hdiutil detach"), 2, "plain, then forced");
        // Deleting a folder with a live volume under it would delete the
        // volume's files: it stays, mount point and all.
        let left: Vec<_> = fs::read_dir(&scratch).unwrap().collect();
        assert_eq!(left.len(), 1, "the scratch folder is kept");
        let work = left[0].as_ref().unwrap().path();
        assert!(work.join("mnt").join("Kimun.app").is_dir());
    }

    #[test]
    fn the_scratch_folder_goes_even_when_the_swap_is_refused() {
        let s = setup();
        let scratch = s.work.join("scratch-root");
        fs::create_dir_all(&scratch).unwrap();
        let host = Host::new("new", Some("ZZZZZ99999"), false);
        BundleSwap::new(&host, s.installed.clone())
            .install_image(b"dmg bytes", &scratch)
            .unwrap_err();
        assert_eq!(
            fs::read_dir(&scratch).unwrap().count(),
            0,
            "scratch is gone"
        );
        assert_eq!(version(&s.installed), "old");
    }

    #[test]
    fn a_backup_left_by_an_earlier_run_does_not_block_the_swap() {
        let s = setup();
        fake_app(&s.apps.join(BACKUP_DIR), "stale", Some(TEAM), false);
        let host = Host::new("new", Some(TEAM), false);
        BundleSwap::new(&host, s.installed.clone())
            .install_dmg(&s.dmg, &s.work)
            .unwrap();
        assert_eq!(version(&s.installed), "new");
        assert_clean(&s);
    }
}
