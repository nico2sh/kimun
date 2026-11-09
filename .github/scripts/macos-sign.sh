#!/usr/bin/env bash
# Sign, notarize and package the desktop app's macOS bundle.
#
#   macos-sign.sh APP_DIR OUT_DMG
#
# What it does, in order: import the Developer ID certificate into a throw-away
# keychain; sign the app with the hardened runtime; notarize the app and staple the
# ticket; wrap it in a .dmg (with an Applications shortcut); sign the .dmg; notarize
# it and staple that too. Any failure stops it, and a rejected notarization prints
# Apple's log.
#
# Needs macOS (codesign, security, notarytool, stapler, hdiutil, ditto) and jq, and these
# in the environment (GitHub secrets in CI; see assets/packaging/macos/SIGNING.md):
#   APPLE_CERT_P12           the Developer ID Application certificate and key, .p12, base64
#   APPLE_CERT_PASSWORD      its password
#   APPLE_SIGNING_IDENTITY   e.g. "Developer ID Application: Your Name (TEAMID)"
#   APPLE_NOTARY_KEY         an App Store Connect API key, the .p8 file, base64
#   APPLE_NOTARY_KEY_ID      its key id
#   APPLE_NOTARY_ISSUER_ID   the issuer id
#
# SIGN_DRY_RUN=1 prints each Apple command (secrets masked) instead of running it:
# the sequence is tested that way on any machine.
set -euo pipefail

app="${1:?usage: macos-sign.sh APP_DIR OUT_DMG}"
out="${2:?usage: macos-sign.sh APP_DIR OUT_DMG}"
dry="${SIGN_DRY_RUN:-0}"

for var in APPLE_CERT_P12 APPLE_CERT_PASSWORD APPLE_SIGNING_IDENTITY APPLE_NOTARY_KEY APPLE_NOTARY_KEY_ID APPLE_NOTARY_ISSUER_ID; do
  [ -n "${!var:-}" ] || { echo "::error::$var is not set" >&2; exit 1; }
done
[ -d "$app" ] || { echo "::error::no app bundle at $app" >&2; exit 1; }

work="$(mktemp -d)"
keychain="$work/signing.keychain-db"
keychain_password="$(LC_ALL=C tr -dc 'a-f0-9' </dev/urandom | head -c 32 || true)"
cleanup() {
  [ "$dry" = 1 ] || security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

mask() { # stdin -> stdout, secrets replaced
  local line
  while IFS= read -r line; do
    line="${line//${APPLE_CERT_PASSWORD}/***}"
    line="${line//${keychain_password}/***}"
    printf '%s\n' "$line"
  done
}
run() {
  if [ "$dry" = 1 ]; then
    echo "+ $*" | mask
  else
    "$@"
  fi
}

# --- the certificate, in its own keychain ---------------------------------------
printf '%s' "$APPLE_CERT_P12" | base64 --decode >"$work/cert.p12"
printf '%s' "$APPLE_NOTARY_KEY" | base64 --decode >"$work/notary.p8"
run security create-keychain -p "$keychain_password" "$keychain"
run security set-keychain-settings -lut 21600 "$keychain"
run security unlock-keychain -p "$keychain_password" "$keychain"
run security import "$work/cert.p12" -k "$keychain" -P "$APPLE_CERT_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security
run security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$keychain"
if [ "$dry" = 1 ]; then
  echo "+ security list-keychains -d user -s $keychain <the existing ones>"
else
  # shellcheck disable=SC2046
  security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
fi

# Submit FILE to Apple and wait; anything but "Accepted" prints the log and fails.
notarize() {
  local file="$1" result="$work/notarize.json" id status
  if [ "$dry" = 1 ]; then
    echo "+ xcrun notarytool submit $file --key <key> --key-id $APPLE_NOTARY_KEY_ID --issuer $APPLE_NOTARY_ISSUER_ID --wait"
    return 0
  fi
  # notarytool exits non-zero on an Invalid/Rejected result; under pipefail
  # that would kill the script before the log below. Judge by the JSON status.
  xcrun notarytool submit "$file" --key "$work/notary.p8" --key-id "$APPLE_NOTARY_KEY_ID" \
    --issuer "$APPLE_NOTARY_ISSUER_ID" --wait --output-format json | tee "$result" || true
  id="$(jq -r '.id // empty' "$result" 2>/dev/null || true)"
  status="$(jq -r '.status // "no result"' "$result" 2>/dev/null || echo "no result")"
  if [ "$status" != "Accepted" ]; then
    [ -n "$id" ] || { echo "::error::notarization of $file failed before a submission id was issued ('$status')" >&2; exit 1; }
    echo "::error::notarization of $file ended '$status'" >&2
    xcrun notarytool log "$id" --key "$work/notary.p8" --key-id "$APPLE_NOTARY_KEY_ID" --issuer "$APPLE_NOTARY_ISSUER_ID" >&2 || true
    exit 1
  fi
}

# --- the app --------------------------------------------------------------------
run codesign --force --options runtime --timestamp --keychain "$keychain" --sign "$APPLE_SIGNING_IDENTITY" "$app"
run codesign --verify --strict --verbose=2 "$app"
run ditto -c -k --keepParent "$app" "$work/app.zip"
notarize "$work/app.zip"
run xcrun stapler staple "$app"
run xcrun stapler validate "$app"

# --- the .dmg -------------------------------------------------------------------
stage="$work/dmg"
mkdir -p "$stage"
run cp -R "$app" "$stage/"
run ln -s /Applications "$stage/Applications"
mkdir -p "$(dirname "$out")"
run hdiutil create -volname "Kimün" -srcfolder "$stage" -ov -format UDZO "$out"
run codesign --force --timestamp --keychain "$keychain" --sign "$APPLE_SIGNING_IDENTITY" "$out"
notarize "$out"
run xcrun stapler staple "$out"
run xcrun stapler validate "$out"
run spctl --assess --type open --context context:primary-signature --verbose=2 "$out"

echo "signed, notarized and stapled: $out"
