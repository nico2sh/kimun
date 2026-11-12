#!/usr/bin/env bash
# Sign, notarize and package the desktop app's macOS bundle.
#
#   macos-sign.sh APP_DIR OUT_DMG
#   macos-sign.sh --check
#
# --check does only the first part: it proves the secrets are all there, the certificate
# imports, and Apple accepts the API key. The release workflow runs it before the
# build, so a wrong secret fails in a minute and not after the Rust build.
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
#   APPLE_SIGNING_IDENTITY   optional. Pins the identity, e.g. "Developer ID Application:
#                            Your Name (TEAMID)". Without it the first Developer ID
#                            Application identity in the .p12 is used.
#   APPLE_NOTARY_KEY         an App Store Connect API key, the .p8 file, base64
#   APPLE_NOTARY_KEY_ID      its key id
#   APPLE_NOTARY_ISSUER_ID   the issuer id
#
# NOTARY_WAIT_MINUTES (default 40) is how long to wait for Apple's answer to one
# submission. The upload is bounded too (NOTARY_UPLOAD_MINUTES, default 15), and so is
# each codesign call. Without bounds a stuck call holds the runner for six hours and the
# log stays empty: every step prints a "==>" line first, so the log names the last one.
#
# SIGN_DRY_RUN=1 prints each Apple command (secrets masked) instead of running it:
# the sequence is tested that way on any machine.
set -euo pipefail

check=0
if [ "${1:-}" = "--check" ]; then
  check=1
else
  app="${1:?usage: macos-sign.sh APP_DIR OUT_DMG | --check}"
  out="${2:?usage: macos-sign.sh APP_DIR OUT_DMG | --check}"
fi
dry="${SIGN_DRY_RUN:-0}"

for var in APPLE_CERT_P12 APPLE_CERT_PASSWORD APPLE_NOTARY_KEY APPLE_NOTARY_KEY_ID APPLE_NOTARY_ISSUER_ID; do
  [ -n "${!var:-}" ] || { echo "::error::$var is not set" >&2; exit 1; }
done
[ "$check" = 1 ] || [ -d "$app" ] || { echo "::error::no app bundle at $app" >&2; exit 1; }

work="$(mktemp -d)"
keychain="$work/signing.keychain-db"
# No `tr </dev/urandom | head`: on the macOS runner `tr` never saw the broken pipe and
# ran forever, so the script hung before its first command with an empty log.
keychain_password="$(openssl rand -hex 16)"
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
say() { printf '==> %s\n' "$*"; }

# with_timeout SECONDS COMMAND...: run COMMAND, kill it after SECONDS (macOS has no
# `timeout`). Returns the command's status, or 124 when it was killed.
with_timeout() {
  local secs="$1" pid watcher rc=0 flag
  shift
  flag="$(mktemp)"
  "$@" &
  pid=$!
  # Flag a timeout only if the command is still running: killing this sleep when the
  # command ends early must not look like a timeout.
  ( sleep "$secs"; if kill -0 "$pid" 2>/dev/null; then echo 1 >"$flag"; kill "$pid" 2>/dev/null; fi ) >/dev/null 2>&1 &
  watcher=$!
  wait "$pid" || rc=$?
  pkill -P "$watcher" 2>/dev/null || true
  kill "$watcher" 2>/dev/null || true
  wait "$watcher" 2>/dev/null || true # reaps it, so bash prints no "Terminated" line
  if [ -s "$flag" ]; then
    rc=124
    echo "::error::timed out after ${secs}s: ${*:1:3}" >&2
  fi
  rm -f "$flag"
  return "$rc"
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
say "importing the certificate into a throw-away keychain"
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

# The identity to sign with: the pinned one, or the first Developer ID Application
# identity that the .p12 brought into the keychain.
identity="${APPLE_SIGNING_IDENTITY:-}"
if [ -z "$identity" ]; then
  if [ "$dry" = 1 ]; then
    identity="<the Developer ID Application identity in the keychain>"
  else
    identity="$(security find-identity -v -p codesigning "$keychain" | awk '/"Developer ID Application:/ {print $2; exit}')"
    [ -n "$identity" ] || { echo "::error::no Developer ID Application identity in the certificate" >&2; exit 1; }
  fi
fi

if [ "$check" = 1 ]; then
  say "asking Apple if the API key works"
  if [ "$dry" = 1 ]; then
    echo "+ xcrun notarytool history --key <key> --key-id $APPLE_NOTARY_KEY_ID --issuer $APPLE_NOTARY_ISSUER_ID"
  else
    with_timeout 120 xcrun notarytool history --key "$work/notary.p8" --key-id "$APPLE_NOTARY_KEY_ID" \
      --issuer "$APPLE_NOTARY_ISSUER_ID" --output-format json >/dev/null \
      || { echo "::error::Apple did not accept the notarization API key (check APPLE_NOTARY_KEY, _KEY_ID and _ISSUER_ID)" >&2; exit 1; }
  fi
  say "the Apple secrets work"
  exit 0
fi

# Submit FILE to Apple and wait; anything but "Accepted" prints the log and fails.
# Upload, then poll, so the submission id is in the log as soon as the upload ends: if
# Apple is slow, ask Apple about that id from any Mac (`xcrun notarytool info ID ...`).
notarize() {
  local file="$1" result="$work/notarize.json" id status deadline
  local up_s=$(( ${NOTARY_UPLOAD_MINUTES:-15} * 60 )) wait_m="${NOTARY_WAIT_MINUTES:-40}"
  if [ "$dry" = 1 ]; then
    echo "+ xcrun notarytool submit $file --key <key> --key-id $APPLE_NOTARY_KEY_ID --issuer $APPLE_NOTARY_ISSUER_ID --verbose"
    echo "+ xcrun notarytool info <submission id> (every 30 s, up to ${wait_m} minutes)"
    return 0
  fi
  say "uploading $(basename "$file") to Apple"
  # notarytool exits non-zero on a failed submit; under pipefail that would kill the
  # script before the message below. Judge by the JSON.
  with_timeout "$up_s" xcrun notarytool submit "$file" --key "$work/notary.p8" --key-id "$APPLE_NOTARY_KEY_ID" \
    --issuer "$APPLE_NOTARY_ISSUER_ID" --verbose --output-format json | tee "$result" || true
  id="$(jq -r '.id // empty' "$result" 2>/dev/null || true)"
  [ -n "$id" ] || { echo "::error::the upload of $file gave no submission id" >&2; exit 1; }
  say "submission id: $id (polling Apple for up to ${wait_m} minutes)"
  # Polling `notarytool info`, not `notarytool wait`: wait printed nothing here and the
  # script read that as a failure. A poll logs every status and bounds the time.
  deadline=$(( $(date +%s) + wait_m * 60 ))
  while :; do
    with_timeout 90 xcrun notarytool info "$id" --key "$work/notary.p8" --key-id "$APPLE_NOTARY_KEY_ID" \
      --issuer "$APPLE_NOTARY_ISSUER_ID" --output-format json >"$result" 2>"$work/info.err" || true
    status="$(jq -r '.status // "no result"' "$result" 2>/dev/null || echo "no result")"
    say "status: $status"
    [ "$status" != "no result" ] || cat "$work/info.err" >&2
    case "$status" in Accepted | Invalid | Rejected) break ;; esac
    if [ "$(date +%s)" -ge "$deadline" ]; then
      echo "::error::notarization of $file is still '$status' after ${wait_m} minutes (submission $id)" >&2
      exit 1
    fi
    sleep "${NOTARY_POLL_SECONDS:-30}"
  done
  if [ "$status" != "Accepted" ]; then
    echo "::error::notarization of $file ended '$status' (submission $id)" >&2
    xcrun notarytool log "$id" --key "$work/notary.p8" --key-id "$APPLE_NOTARY_KEY_ID" --issuer "$APPLE_NOTARY_ISSUER_ID" >&2 || true
    exit 1
  fi
  say "accepted: $(basename "$file")"
}

# --- the app --------------------------------------------------------------------
say "signing the app"
run with_timeout 300 codesign --force --options runtime --timestamp --verbose --keychain "$keychain" --sign "$identity" "$app"
say "checking the app's signature"
run codesign --verify --strict --verbose=2 "$app"
say "zipping the app"
run ditto -c -k --keepParent "$app" "$work/app.zip"
notarize "$work/app.zip"
say "stapling the app"
run xcrun stapler staple "$app"
run xcrun stapler validate "$app"

# --- the .dmg -------------------------------------------------------------------
stage="$work/dmg"
mkdir -p "$stage"
run cp -R "$app" "$stage/"
run ln -s /Applications "$stage/Applications"
mkdir -p "$(dirname "$out")"
say "building the .dmg"
run hdiutil create -volname "Kimün" -srcfolder "$stage" -ov -format UDZO "$out"
say "signing the .dmg"
run with_timeout 300 codesign --force --timestamp --verbose --keychain "$keychain" --sign "$identity" "$out"
notarize "$out"
say "stapling the .dmg"
run xcrun stapler staple "$out"
run xcrun stapler validate "$out"
run spctl --assess --type open --context context:primary-signature --verbose=2 "$out"

echo "signed, notarized and stapled: $out"
