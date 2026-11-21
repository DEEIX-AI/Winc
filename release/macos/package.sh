#!/usr/bin/env bash
# Build the app, sign and notarize it, package it as a dmg, and write the signed appcast. The dmg
# is both the installer and the in-app update payload.
set -euo pipefail
phase="${1:?a packaging phase is required}"
target="${2:?a Rust target triple is required}"
script_dir="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$script_dir/../.." && pwd)"
cd "$root"
case "$target" in
  aarch64-apple-darwin) arch=arm64; binary_arch=arm64 ;;
  x86_64-apple-darwin) arch=x64; binary_arch=x86_64 ;;
  *) echo "unsupported macOS target: $target" >&2; exit 1 ;;
esac
version="$(node release/common/version.mjs)"
bundle="$root/target/$target/release/bundle"
app="$bundle/macos/winc.app"
release_stem="Winc-$version-macos-$arch"
dmg="$bundle/dmg/$release_stem.dmg"
mode="${SIGNING_MODE:-adhoc}"
workdir=""
keychain=""
cleanup() {
  [[ -z "$keychain" ]] || security delete-keychain "$keychain" || true
  [[ -z "$workdir" ]] || rm -rf "$workdir"
}
trap cleanup EXIT
fail() { echo "package-macos: $*" >&2; exit 1; }
make_workdir() { workdir="$(mktemp -d "$bundle/.package-macos.XXXXXX")"; }

import_certificate() {
  make_workdir
  keychain="$workdir/signing.keychain-db"
  local password
  password="$(openssl rand -hex 24)"
  printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$workdir/certificate.p12"
  security create-keychain -p "$password" "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  security import "$workdir/certificate.p12" -k "$keychain" -P "${APPLE_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
}

sign_external() {
  # App and DMG use the exact same downloaded helper in this job.
  local helper="${RUNNER_TEMP:?}/signing-helper-$target.sh"
  if [[ ! -s "$helper" ]]; then
    curl -fsSL --connect-timeout 20 --max-time 120 --retry 2 -- "$SIGN_AND_NOTARIZE_SCRIPT_URL" > "$helper"
    [[ -s "$helper" ]] || fail "the signing helper is empty"
  fi
  API_BASE_URL="$SIGN_AND_NOTARIZE_BASE_URL" \
  API_TOKEN="$SIGN_AND_NOTARIZE_API_TOKEN" \
  RESULT_HELPER_URL="$SIGN_AND_NOTARIZE_HELPER_URL" \
    bash -c "$(cat "$helper")" -- "$1" "$2"
}

case "$phase" in
  app)
    cargo xtask app --target "$target" --no-build
    mkdir -p "$bundle/macos"
    mv "$bundle/winc.app" "$app"
    if [[ "$mode" == apple ]]; then
      import_certificate
      # Sign Mach-O leaves before their containing XPCs, frameworks and app.
      while IFS= read -r -d '' binary; do
        if file -b "$binary" | grep -q Mach-O; then
          codesign --force --options runtime --sign "$APPLE_SIGNING_IDENTITY" --keychain "$keychain" --timestamp "$binary"
        fi
      done < <(find "$app/Contents" -type f -print0)
      while IFS= read -r -d '' nested; do
        codesign --force --options runtime --sign "$APPLE_SIGNING_IDENTITY" --keychain "$keychain" --timestamp "$nested"
      done < <(find "$app/Contents" -depth -type d \( -name '*.xpc' -o -name '*.app' -o -name '*.framework' \) -print0)
      codesign --force --options runtime --sign "$APPLE_SIGNING_IDENTITY" --keychain "$keychain" --timestamp "$app"
      ditto -c -k --keepParent "$app" "$workdir/app.zip"
      xcrun notarytool submit "$workdir/app.zip" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
      xcrun stapler staple "$app"
    fi
    ;;
  sign-app)
    [[ "$mode" == external ]] || fail "sign-app requires the external signing provider"
    [[ -d "$app" ]] || fail "missing $app"
    make_workdir
    ditto "$app" "$workdir/$release_stem.app"
    mkdir -p "$workdir/signed"
    signed_app="$workdir/signed/winc.app"
    sign_external "$workdir/$release_stem.app" "$signed_app"
    [[ -d "$signed_app" ]] || fail "the signing helper did not produce an app"
    codesign --verify --deep --strict "$signed_app"
    team="$(codesign -dv "$signed_app" 2>&1 | sed -n 's/^TeamIdentifier=//p')"
    [[ "$team" =~ ^[A-Z0-9]{10}$ ]] || fail "the signing service returned an app without a Developer ID team"
    printf 'APPLE_TEAM_ID=%s\n' "$team" >> "${GITHUB_ENV:?}"
    rm -rf "$app"
    ditto "$signed_app" "$app"
    ;;
  dmg)
    [[ -d "$app" ]] || fail "missing $app"
    mkdir -p "$bundle/dmg"
    cargo xtask dmg --app "$app" --output "$dmg"
    ;;
  sign-dmg)
    [[ -s "$dmg" ]] || fail "missing $dmg"
    if [[ "$mode" == external ]]; then
      make_workdir
      cp "$dmg" "$workdir/$release_stem.dmg"
      sign_external "$workdir/$release_stem.dmg" "$workdir/signed.dmg"
      [[ -s "$workdir/signed.dmg" ]] || fail "the signing helper did not produce a DMG"
      mv -f "$workdir/signed.dmg" "$dmg"
    elif [[ "$mode" == apple ]]; then
      import_certificate
      codesign --force --sign "$APPLE_SIGNING_IDENTITY" --keychain "$keychain" --timestamp "$dmg"
      xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
      xcrun stapler staple "$dmg"
    else
      fail "sign-dmg requires an Apple signing provider"
    fi
    ;;
  appcast)
    [[ -s "$dmg" ]] || fail "missing $dmg"
    node release/common/sign-updates.mjs --platform "macos-$arch" --bundle "$bundle"
    # Cross-check the portable signer with Sparkle's own verifier before upload.
    make_workdir
    printf '%s' "$WINC_UPDATE_PRIVATE_KEY" > "$workdir/update-key"
    chmod 600 "$workdir/update-key"
    sparkle_tools="$(cargo xtask sparkle)"
    feed="$bundle/updates/appcast-macos-$binary_arch.xml"
    signature="$(sed -n 's/.*sparkle:edSignature="\([^"]*\)".*/\1/p' "$feed" | head -1)"
    "$sparkle_tools/sign_update" --ed-key-file "$workdir/update-key" --verify "$dmg" "$signature"
    "$sparkle_tools/sign_update" --ed-key-file "$workdir/update-key" --verify "$feed"
    ;;
  verify)
    bash "$script_dir/verify.sh" --arch "$binary_arch" "$dmg"
    ;;
  *) fail "unknown packaging phase: $phase" ;;
esac
