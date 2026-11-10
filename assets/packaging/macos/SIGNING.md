# Signing the macOS desktop app

The release workflow builds `Kimun.app` for each macOS target on every release. With the six secrets below it also signs it,
notarizes it with Apple, staples the ticket, and publishes `kimun-gui-<version>-macos-<arch>.dmg` beside the other assets. Without them
it builds the bundle, prints a notice, and ships the raw binary unsigned as before: nothing breaks, there is just no `.dmg`.

The scripts are `.github/scripts/macos-bundle.sh` (assemble) and `.github/scripts/macos-sign.sh` (sign, notarize, staple, `.dmg`).
`test-macos-packaging.sh` tests the sequence in dry-run mode on any machine. None of it has run against Apple's real services yet.
The first run with real secrets is the real test. Do it through the manual dry run (below), not a release.

## One-time setup

1. **Apple Developer Program** membership (paid, per year), under the name that you want on the signed app.
2. **A Developer ID Application certificate.** In the Apple Developer portal: Certificates → `+` → *Developer ID Application*. Make a
   certificate signing request in Keychain Access (Certificate Assistant → Request a Certificate From a Certificate Authority). Save it to
   disk and upload it. Download the `.cer`, then double-click it to install it in your login keychain.
3. **Export it as a `.p12`:** Keychain Access → My Certificates → right-click the "Developer ID Application: …" entry (it must show
   its private key under it) → Export → `.p12`, with a password you choose. Then `base64 -i certificate.p12 | pbcopy`.
4. **The signing identity string:** `security find-identity -v -p codesigning` prints it. For example:
   `Developer ID Application: Your Name (ABCDE12345)`.
5. **A notarization API key.** App Store Connect → Users and Access → Integrations → Team Keys → generate a key (Developer access is
   enough). Download the `.p8` (Apple lets you do it once), note the **Key ID** and the **Issuer ID** shown on that page. Then
   `base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy`.
6. **GitHub → repository Settings → Secrets and variables → Actions**, add:

| Secret | Value |
|---|---|
| `APPLE_CERT_P12` | the `.p12`, base64 |
| `APPLE_CERT_PASSWORD` | its password |
| `APPLE_SIGNING_IDENTITY` | the identity string from step 4 |
| `APPLE_NOTARY_KEY` | the `.p8`, base64 |
| `APPLE_NOTARY_KEY_ID` | the key id |
| `APPLE_NOTARY_ISSUER_ID` | the issuer id |

If `APPLE_CERT_P12` is set, signing turns on. The workflow tests that one. You must then set the other secrets too. The script
names any secret that is missing.

## Trying it without a release

Actions → *release* → Run workflow is the manual dry run. Give it a version like `0.0.0-dryrun`. The macOS jobs sign and notarize for
real, notarization takes a few minutes per submission (two: the app, then the `.dmg`), and the `.dmg` files come out in the
`release-assets` artifact. Nothing is published.

Download a `.dmg` through a browser on a Mac that is not yours. The browser sets the quarantine flag. Then make sure that:

- the `.dmg` opens, and you can drag `Kimun.app` to Applications and start it with no "unidentified developer" or "damaged" warning.
- `spctl --assess --type execute --verbose=2 /Applications/Kimun.app` says `accepted` and `source=Notarized Developer ID`.
- `codesign --verify --deep --strict --verbose=2 /Applications/Kimun.app` is clean.
- the app starts, shows the window, and opens a workspace. A hardened-runtime app that needs an exception crashes at launch (see below).

## If something fails

- **Notarization rejected.** The script prints Apple's log (`notarytool log`). The log names the file and the reason, for example an unsigned nested
  binary, a missing timestamp, or a disallowed entitlement.
- **The signed app crashes at launch.** The hardened runtime blocks something that GPUI or a dependency does, and the crash report names it.
  Add an entitlements plist to `assets/packaging/macos/`. Add `--entitlements` to the app's `codesign` line in `macos-sign.sh`.
  Set only the exception that is needed: `com.apple.security.cs.allow-jit`, `…allow-unsigned-executable-memory`, or
  `…disable-library-validation`. None is set today because none is known to be needed.
- **`security import` or keychain errors** in CI: usually a wrong `.p12` password or a `.p12` that was exported without its private key.
- **Certificate lifetime.** A Developer ID Application certificate lasts five years. The API key lasts until you revoke it. To rotate, replace the secrets.
- `LSMinimumSystemVersion` in `Info.plist.in` is 11.0. This is a guess at what GPUI needs. Set it to the version that the oldest macOS you test on runs.

## What this does not do (yet)

When the release has a `.dmg`, `install.sh --gui` on macOS installs the signed bundle into `~/Applications`. Otherwise it installs the raw
binary. The script checks that the bundle seal is intact and that the signature chains to an Apple Developer ID Application
certificate. It does not check notarization. The app updates from a bundle by a new download (the `Bundle` install channel), never in
place. A Homebrew cask for the `.dmg` is not set up.
