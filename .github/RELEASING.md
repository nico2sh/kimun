# Releasing

How a release goes, how to rehearse one without publishing, and how to back one out. The pieces: `release-plz` (versions, changelog,
tags), `.github/workflows/build.yml` (assets), `release-notes/` (the hand-written notes), `install.sh` and the in-app updater (what
users run). What is and is not verified is in `plans/m11/task-m11f*-report.md`; the follow-ups are in `plans/m11/follow-ups.md`.

## A normal release

1. Conventional commits land on `main`. release-plz keeps one **release PR** open, bumping versions and `tui/CHANGELOG.md`.
2. A step in `release-plz.yml` adds an empty `release-notes/<version>.md` to that PR's branch. **Write the notes there**, in plain words
   for users (see `release-notes/README.md`). The `release-notes` check stays red until you have; make it a required check.
3. Merge the PR. release-plz tags `kimun-notes-vX.Y.Z` (and `kimun_core-…` / `kimun_server-…` when those changed) and creates the
   GitHub release, as a **draft** for the app and the server (see "A draft until its assets exist" below). Nobody can install it yet.
4. The app tag starts `build.yml`: both apps on five platforms, the macOS app signed and notarized when the Apple secrets exist
   (`assets/packaging/macos/SIGNING.md`), the assets assembled by `release-dist.sh` and **verified** by `verify-release-assets.sh`
   before anything is attached; your notes become the release body; then the draft is **published**, and the Homebrew formula is
   pushed (stable only).
5. Look at the release page (the release stays a draft, visible only to you, until step 4 succeeds): the 20 assets (+2 `.dmg` when signed) and `checksums-sha256.txt`, the body is your notes, the app's release
   wears "Latest". Then `curl -fsSL https://kimun.2co.dev/install.sh | sh` and `... | sh -s -- --gui` on a clean machine.

## A draft until its assets exist

release-plz creates the app's and the server's releases as drafts (`git_release_draft` in `release-plz.toml`). The tag exists at once, so
the build starts; the build's last step attaches the files and publishes (`draft: false`). Until then the release is invisible to
`install.sh`, `install.ps1`, the updater and anyone not signed in as a maintainer, so there is no window in which the newest release has
nothing to download. `kimun_core` has no assets and is published at once.

If a build fails, the release stays a draft: fix the cause and re-run the workflow from the tag (`gh run rerun <id>`), or delete the draft
and tag (`gh release delete <tag> --cleanup-tag`) and re-tag. The previous stable release is what everyone gets meanwhile.

## The release manifest

`https://kimun.2co.dev/latest.json` names the newest stable app release and server release, every file of each with its URL and sha256
(`.github/scripts/release_manifest.py`, built into the site by `docs.yml`). `install.sh`, `install-server.sh` and `install.ps1` read it
first, so they neither page through the GitHub API (newest 100 releases across all three tag series; 60 calls an hour per address) nor
download the checksums file. When it cannot be read (unreachable, a schema they do not know, no entry for the series) they print a
one-line warning and use the API as before. The site rebuilds when `release-plz`, `release` or `release-server` finish, so it follows a
release only once the draft has been published with its files. After a release, check `curl -fsS https://kimun.2co.dev/latest.json`
names the new version; if not, look at the `Publish latest.json` step of the last `docs` run (it cannot fail the deploy, only leave the file out).

## Signing

Every release carries `checksums-sha256.txt.minisig`: a [minisign](https://jedisct1.github.io/minisign/) signature of
`checksums-sha256.txt`, which lists the sha256 of every other file, so one signature covers the release. `build.yml` and
`build-server.yml` sign it (`.github/scripts/sign-release.sh`) before anything is published. A real release without the key
**fails at that step**; a manual dry run only warns. The manifest carries `checksums_url` and `signature_url` for each series.

The **installers** (`install.sh`, `install-server.sh`, `install.ps1`) check it when `minisign` is installed. The in-app updater does
**not** check it yet: it still trusts the sha256 in `latest.json`. Each installer pins the public key in `SIGNING_PUBKEY`; while that is
empty, signing is not live and nothing is checked. With a key pinned and minisign installed, a bad signature refuses the install, and so
does a missing one on the latest release (a version pinned with `KIMUN_VERSION` only warns, since it may predate signing). Without
minisign they install on the sha256 and warn. `test-install-server.sh` checks the shell installers pin the same key as `release-signing.pub`.

One-time setup, on your own machine (never in CI):

1. `minisign -G -p release-signing.pub -s kimun-release.key` (pick a password).
2. Commit `release-signing.pub` at the repository root. `sign-release.sh` checks every new signature against it and fails the release if
   `MINISIGN_SECRET_KEY` is not its key.
3. Repository secrets: `MINISIGN_SECRET_KEY` = the contents of `kimun-release.key`, `MINISIGN_PASSWORD` = its password.
4. Keep `kimun-release.key` and the password somewhere safe and offline. Losing the key means shipping a new public key in a new release.

To turn checking on, in the release PR of the **first signed release**, paste line 2 of `release-signing.pub` into `SIGNING_PUBKEY` in all
three installers. Until that release is published, `latest` is unsigned and the installers would refuse it, so merge the PR and publish
the release together.

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

## Rehearsing a pipeline change on a pre-release

The first desktop release went out as stable 0.24.8 without a pre-release first: the manual dry run (above) had proven the assets, and a
real install against 0.24.8 covers the download path. Use a pre-release when a change to the release pipeline itself needs a real run
before anyone can be offered it (the first signed macOS build is the next such change).

A hyphenated version is a GitHub pre-release: it never wears "Latest", Homebrew skips it, `install.sh`, `install.ps1` and the updater ignore it. So:

1. `bash .github/scripts/release_notes.py scaffold 0.25.0-beta.1`, write the notes, commit them.
2. Tag it outside release-plz: `git tag kimun-notes-v0.25.0-beta.1 && git push origin kimun-notes-v0.25.0-beta.1`. `build.yml` runs as for a
   real release and creates the pre-release.
3. Install it for real, over the network: `KIMUN_VERSION=0.25.0-beta.1 sh -c "$(curl -fsSL https://kimun.2co.dev/install.sh)" -- --gui`
   (and without `--gui`; on Windows `$env:KIMUN_VERSION = "0.25.0-beta.1"` before `irm https://kimun.2co.dev/install.ps1 | iex`). Walk the checklist (`plans/m11/release-checklist.md`) on each platform you have.
4. Clean up if you like: `gh release delete kimun-notes-v0.25.0-beta.1 --cleanup-tag`.

## Testing self-update for real

Pre-releases are never offered, so an in-place update needs two stable releases: install release N with the installer, publish N+1, then
in the app: the toast, the dialog, `Install update`, "Updated to N+1. Restart Kimün to use it.", restart, the new version. Until then the
scripted provider (`KIMUN_GUI_FAKE_UPDATE`, the `fake-update` feature) and the update tests are the coverage; they exercise the dialog and
the flow, not the download from GitHub.

## Backing out

- A bad release (published, or a draft that must not go out): `gh release delete <tag> --cleanup-tag`. The installer and the updater read the newest *stable* release, so they fall
  back to the one before at once. Revert the Homebrew tap's commit if the formula was pushed.
- Only the desktop builds are bad: set `DESKTOP_BINARY_PUBLISHED = false` in `tui/src/logic/update.rs` and ship a patch; the desktop app
  then offers the releases page instead of installing.
