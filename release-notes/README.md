# Release notes

One hand-written file per app version: `release-notes/<version>.md` (the version of the `kimun-notes` crate, which is the app's).
The release page shows the file as it is. The changelog generated from commit messages stays in `tui/CHANGELOG.md` as the
commit-level history; this is the part written for the people who use the app.

**How it goes**
1. release-plz opens (or updates) the release PR. A step in `release-plz.yml` adds an empty `release-notes/<version>.md`
   to the PR branch: just a comment saying what to write. Your edits are kept when release-plz rewrites the branch.
2. Write the notes in that file, on the PR branch. Plain words for users: what is new, what changed, what to know when upgrading.
   Delete the comment.
3. The `release-notes` check on the PR stays red while the file is missing or still only the template. Make it a required check
   in the branch protection so the release cannot be merged without notes.
4. On the release tag, `build.yml` sets the file as the release body. If it is somehow missing or empty there, it warns and the
   release keeps the generated changelog as its body.

A release PR that does not release the app (a server-only bump, where the app's version already has a tag) needs no notes.
`release.sh` (the manual path) creates the same empty file on a version bump.

Tools: `.github/scripts/release_notes.py` (`scaffold`, `check`, `ready`, `unreleased`, `version`, `path`), tested by
`.github/scripts/test_release_notes.py`. The server has its own releases and no notes file yet.
