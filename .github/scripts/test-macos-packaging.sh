#!/usr/bin/env bash
# Tests for the macOS packaging scripts, on any machine: the bundle's structure and
# Info.plist, and the signing script's sequence of Apple commands in dry-run mode
# (the real tools exist only on macOS, with an Apple account).
# Run: bash .github/scripts/test-macos-packaging.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*"; exit 1; }
ok() { echo "ok: $*"; }

# --- the bundle -------------------------------------------------------------------
printf '#!/bin/sh\n' >"$work/kimun-gui"
bash .github/scripts/macos-bundle.sh "$work/kimun-gui" 1.2.3 "$work/out" >/dev/null
app="$work/out/Kimun.app"
[ -x "$app/Contents/MacOS/kimun-gui" ] || fail "the executable is missing or not executable"
[ -f "$app/Contents/Resources/kimun.icns" ] || fail "the icon is missing"
[ -f "$app/Contents/PkgInfo" ] || fail "PkgInfo"
python3 - "$app/Contents/Info.plist" <<'P' || fail "Info.plist"
import plistlib, sys
p = plistlib.load(open(sys.argv[1], "rb"))
assert p["CFBundleExecutable"] == "kimun-gui"
assert p["CFBundleIconFile"] == "kimun" and p["CFBundleIconName"] == "kimun"
assert p["CFBundleIdentifier"] == "dev.2co.kimun"
assert p["CFBundleShortVersionString"] == "1.2.3" and p["CFBundleVersion"] == "1.2.3"
assert p["CFBundlePackageType"] == "APPL" and p["NSHighResolutionCapable"] is True
assert "@" not in repr(p), "an unfilled placeholder"
P
ok "the bundle: executable, icon, a valid Info.plist"

# The Icon Composer file: its json parses and every image it names is there.
python3 - assets/packaging/macos/kimun.icon <<'P' || fail "kimun.icon"
import json, os, sys
d = sys.argv[1]
icon = json.load(open(os.path.join(d, "icon.json")))
names = [l["image-name"] for g in icon["groups"] for l in g["layers"]]
assert names, "no layers"
for n in names:
    assert os.path.isfile(os.path.join(d, "Assets", n)), f"missing Assets/{n}"
P
ok "kimun.icon is well formed"

# The committed Assets.car (the Tahoe icon) must be the one built from today's kimun.icon, and
# the bundle must carry it. Missing: said, not failed (make-icon.sh makes it, on a Mac); the
# release itself refuses a bundle without it (REQUIRE_ICON_CAR).
pkg=assets/packaging/macos
if [ -f "$pkg/Assets.car" ]; then
  [ "$(cat "$pkg/Assets.car.source-sha256" 2>/dev/null)" = "$(bash "$pkg/make-icon.sh" hash)" ] \
    || fail "kimun.icon changed since Assets.car was built: run assets/packaging/macos/make-icon.sh on a Mac and commit the result"
  cmp -s "$pkg/Assets.car" "$app/Contents/Resources/Assets.car" || fail "the bundle's Assets.car is not the committed one"
  ok "Assets.car matches kimun.icon and is in the bundle"
else
  echo "note: no committed Assets.car yet (run $pkg/make-icon.sh on a Mac)"
fi
if [ ! -f "$pkg/Assets.car" ]; then
  # No car: a release bundle must be refused.
  REQUIRE_ICON_CAR=1 bash .github/scripts/macos-bundle.sh "$work/kimun-gui" 1.2.3 "$work/req" >/dev/null 2>&1 \
    && fail "a release bundle without Assets.car was accepted"
  ok "a release bundle needs Assets.car"
fi

bash .github/scripts/macos-bundle.sh "$work/kimun-gui" 1.2.3-beta.4 "$work/pre" >/dev/null
python3 - "$work/pre/Kimun.app/Contents/Info.plist" <<'P' || fail "pre-release version"
import plistlib, sys
p = plistlib.load(open(sys.argv[1], "rb"))
assert p["CFBundleShortVersionString"] == "1.2.3-beta.4"
assert p["CFBundleVersion"] == "1.2.3", "the build number is three integers"
P
ok "a pre-release keeps its suffix in the short version only"
if bash .github/scripts/macos-bundle.sh "$work/kimun-gui" dryrun "$work/bad" >/dev/null 2>&1; then fail "a non-numeric version made a bundle"; fi
if bash .github/scripts/macos-bundle.sh "$work/missing" 1.2.3 "$work/bad" >/dev/null 2>&1; then fail "a missing binary made a bundle"; fi
ok "a bad version or a missing binary is refused"

# --- the signing sequence (dry run) -----------------------------------------------
export SIGN_DRY_RUN=1
APPLE_CERT_P12="$(printf 'cert' | base64)"
APPLE_NOTARY_KEY="$(printf 'key' | base64)"
export APPLE_CERT_P12 APPLE_NOTARY_KEY
export APPLE_CERT_PASSWORD="s3cret-pass" APPLE_SIGNING_IDENTITY="Developer ID Application: Test (ABCDE12345)"
export APPLE_NOTARY_KEY_ID="KEYID123" APPLE_NOTARY_ISSUER_ID="issuer-uuid"
log="$(bash .github/scripts/macos-sign.sh "$app" "$work/dmg/out.dmg")" || fail "the dry run failed: $log"

in_order() { # every pattern appears, each after the one before
  local n=0 pat
  for pat in "$@"; do
    local m
    m="$(echo "$log" | grep -n -m1 -- "$pat" | cut -d: -f1 || true)"
    [ -n "$m" ] || fail "missing from the sequence: $pat"
    [ "$m" -ge "$n" ] || fail "out of order: $pat"
    n="$m"
  done
}
in_order "security create-keychain" "security import" "set-key-partition-list" \
  "codesign --force --options runtime --timestamp" "codesign --verify --strict" \
  "notarytool submit .*app.zip" "stapler staple .*Kimun.app" \
  "hdiutil create" "codesign --force --timestamp .*out.dmg" \
  "notarytool submit .*out.dmg" "stapler staple .*out.dmg" "spctl --assess"
[ "$(echo "$log" | grep -c "notarytool submit")" = 2 ] || fail "the app and the dmg are both notarized"
echo "$log" | grep -q -- "--sign Developer ID Application: Test (ABCDE12345)" || fail "the signing identity is not passed"
ok "signing: keychain, sign app, notarize + staple, dmg, sign, notarize + staple, assess"

echo "$log" | grep -q "s3cret-pass" && fail "the certificate password was printed"
echo "$log" | grep -q "\*\*\*" || fail "nothing was masked where the password goes"
ok "secrets are masked in the log"

for var in APPLE_CERT_P12 APPLE_CERT_PASSWORD APPLE_NOTARY_KEY APPLE_NOTARY_KEY_ID APPLE_NOTARY_ISSUER_ID; do
  out="$(env -u "$var" bash .github/scripts/macos-sign.sh "$app" "$work/dmg/x.dmg" 2>&1)" && fail "ran without $var"
  echo "$out" | grep -q "$var is not set" || fail "$var not named: $out"
done
ok "each missing secret is named and stops the run"

# The identity is optional: without the secret, the .p12's own identity signs.
log2="$(env -u APPLE_SIGNING_IDENTITY bash .github/scripts/macos-sign.sh "$app" "$work/dmg/z.dmg")" || fail "the dry run needs the identity secret: $log2"
echo "$log2" | grep -q -- "--sign <the Developer ID Application identity in the keychain>" || fail "no identity taken from the keychain"
ok "the signing identity secret is optional"

# --check: the early step of the workflow. Secrets, certificate, Apple's API key; no app.
chk="$(bash .github/scripts/macos-sign.sh --check)" || fail "--check failed in the dry run: $chk"
echo "$chk" | grep -q "security import" || fail "--check did not import the certificate"
echo "$chk" | grep -q "notarytool history" || fail "--check did not ask Apple about the key"
echo "$chk" | grep -qE "codesign --|notarytool submit|hdiutil" && fail "--check went on to sign or notarize"
out="$(env -u APPLE_NOTARY_KEY_ID bash .github/scripts/macos-sign.sh --check 2>&1)" && fail "--check ran without a secret"
echo "$out" | grep -q "APPLE_NOTARY_KEY_ID is not set" || fail "--check did not name the missing secret: $out"
ok "--check: secrets, certificate and API key only"
if bash .github/scripts/macos-sign.sh "$work/nope.app" "$work/dmg/y.dmg" >/dev/null 2>&1; then fail "signed a missing bundle"; fi
ok "a missing bundle is refused"

echo "macOS packaging scripts behave"
