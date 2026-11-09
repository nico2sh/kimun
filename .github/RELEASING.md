# Releasing

How a release goes, how to rehearse one without publishing, and how to back one out. The pieces: `release-plz` (versions, changelog,
tags), `.github/workflows/build.yml` (assets), `release-notes/` (the hand-written notes), `install.sh` and the in-app updater (what
users run). What is and is not verified is in `plans/m11/task-m11f*-report.md`; the follow-ups are in `plans/m11/follow-ups.md`.

## A normal release

1. Conventional commits land on `main`. release-plz keeps one **release PR** open, bumping versions and `tui/CHANGELOG.md`.
2. A step in `release-plz.yml` adds an empty `release-notes/<version>.md` to that PR's branch. **Write the notes there**, in plain words
   for users (see `release-notes/README.md`). The `release-notes` check stays red until you have; make it a required check.
3. Merge the PR. release-plz tags `kimun-notes-vX.Y.Z` (and `kimun_core-…` / `kimun_server-…` when those changed) and creates the
   GitHub release.
4. The app tag starts `build.yml`: both apps on five platforms, the macOS app signed and notarized when the Apple secrets exist
   (`assets/packaging/macos/SIGNING.md`), the assets assembled by `release-dist.sh` and **verified** by `verify-release-assets.sh`
   before anything is attached; your notes become the release body; the Homebrew formula is pushed (stable only).
5. Look at the release page: the 20 assets (+2 `.dmg` when signed) and `checksums-sha256.txt`, the body is your notes, the app's release
   wears "Latest". Then `curl -fsSL https://kimun.2co.dev/install.sh | sh` and `... | sh -s -- --gui` on a clean machine.

## Rehearse without publishing

- **On this machine, this platform:** `bash .github/scripts/rehearse-release.sh`. Builds both apps in release mode, packages them with
  the release scripts, verifies, installs both with `install.sh` into a scratch home, starts the installed apps. Nothing leaves the
  machine. (`SKIP_BUILD=1` reuses `target/release`.)
- **On GitHub, all platforms, nothing published:** Actions → *release* → *Run workflow* with a version such as `0.0.0-dryrun`, or
  `gh workflow run build.yml -f version=0.0.0-dryrun`. The workflow file must be on the default branch to be dispatchable. It builds
  everything, runs the verifier, and keeps the assets as the `release-assets` artifact; no release is touched, no formula pushed.
  With the Apple secrets set it signs and notarizes for real, which is the way to test signing.
  ```sh
  gh run watch                                   # pick the run
  gh run download <run-id> -n release-assets -D /tmp/dry
  EXPECT_DMG=0 bash .github/scripts/verify-release-assets.sh /tmp/dry 0.0.0-dryrun   # EXPECT_DMG=1 when signing is on
  HOME=$(mktemp -d) KIMUN_VERSION=0.0.0-dryrun KIMUN_DOWNLOAD_BASE=file:///tmp/dry sh install.sh --gui
  ```

## The first desktop release: a pre-release first

A hyphenated version is a GitHub pre-release: it never wears "Latest", Homebrew skips it, `install.sh` and the updater ignore it. So:

1. `bash .github/scripts/release_notes.py scaffold 0.25.0-beta.1`, write the notes, commit them.
2. Tag it outside release-plz: `git tag kimun-notes-v0.25.0-beta.1 && git push origin kimun-notes-v0.25.0-beta.1`. `build.yml` runs as for a
   real release and creates the pre-release.
3. Install it for real, over the network: `KIMUN_VERSION=0.25.0-beta.1 sh -c "$(curl -fsSL https://kimun.2co.dev/install.sh)" -- --gui`
   (and without `--gui`). Walk the checklist (`plans/m11/release-checklist.md`) on each platform you have.
4. Clean up if you like: `gh release delete kimun-notes-v0.25.0-beta.1 --cleanup-tag`.

## Testing self-update for real

Pre-releases are never offered, so an in-place update needs two stable releases: install release N with the installer, publish N+1, then
in the app: the toast, the dialog, `Install update`, "Updated to N+1. Restart Kimün to use it.", restart, the new version. Until then the
scripted provider (`KIMUN_GUI_FAKE_UPDATE`, the `fake-update` feature) and the update tests are the coverage; they exercise the dialog and
the flow, not the download from GitHub.

## Backing out

- A bad release: `gh release delete <tag> --cleanup-tag`. The installer and the updater read the newest *stable* release, so they fall
  back to the one before at once. Revert the Homebrew tap's commit if the formula was pushed.
- Only the desktop builds are bad: set `DESKTOP_BINARY_PUBLISHED = false` in `tui/src/logic/update.rs` and ship a patch; the desktop app
  then offers the releases page instead of installing.
