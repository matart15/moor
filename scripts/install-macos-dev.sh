#!/bin/bash
# Install only the locally built Moor Dev bundle. Intended for trusted source on the Mac host.
set -euo pipefail
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo 'Moor Dev installer requires Apple Silicon macOS.' >&2; exit 2; }
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
SOURCE="$ROOT/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Moor Dev.app"
DEST="$HOME/Applications/Moor Dev.app"
EXPECTED_ID='com.specdest.moor-dev'
[[ -d "$SOURCE" && ! -L "$SOURCE" ]] || { echo 'Built Moor Dev.app missing.' >&2; exit 1; }
[[ -f "$SOURCE/Contents/MacOS/moor" ]] || { echo 'Moor Dev executable missing.' >&2; exit 1; }
actual_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$SOURCE/Contents/Info.plist")"
[[ "$actual_id" == "$EXPECTED_ID" ]] || { echo 'Unexpected bundle ID; refusing installation.' >&2; exit 1; }
# Verify existing signatures (including ad-hoc) if present; do not treat unsigned code as notarized.
if /usr/bin/codesign -dv "$SOURCE" >/dev/null 2>&1; then
  /usr/bin/codesign --verify --deep --strict "$SOURCE" || { echo 'Code signature validation failed.' >&2; exit 1; }
fi
mkdir -p "$HOME/Applications"
STAGE="$(mktemp -d "$HOME/Applications/.moor-dev-install.XXXXXX")"
BACKUP=""
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT
/usr/bin/ditto "$SOURCE" "$STAGE/Moor Dev.app"
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$STAGE/Moor Dev.app/Contents/Info.plist")" == "$EXPECTED_ID" ]] || exit 1
# Local, source-built development bundle only. Never disable Gatekeeper globally.
/usr/bin/xattr -dr com.apple.quarantine "$STAGE/Moor Dev.app" 2>/dev/null || true
if [[ -d "$DEST" ]]; then
  /usr/bin/osascript -e 'tell application id "com.specdest.moor-dev" to quit' >/dev/null 2>&1 || true
  for i in {1..20}; do
    if ! /usr/bin/pgrep -f '/Moor Dev.app/Contents/MacOS/moor' >/dev/null 2>&1; then break; fi
    sleep 1
  done
  if /usr/bin/pgrep -f '/Moor Dev.app/Contents/MacOS/moor' >/dev/null 2>&1; then
    echo 'Moor Dev is still running; refusing to replace.' >&2; exit 1
  fi
  BACKUP="$STAGE/previous.app"
  mv "$DEST" "$BACKUP"
fi
if ! mv "$STAGE/Moor Dev.app" "$DEST"; then
  if [[ -n "$BACKUP" ]]; then mv "$BACKUP" "$DEST"; fi
  echo 'Installation failed; previous app restored.' >&2; exit 1
fi
if ! /usr/bin/open "$DEST"; then
  rm -rf "$DEST"
  if [[ -n "$BACKUP" ]]; then mv "$BACKUP" "$DEST"; fi
  echo 'Launch failed; previous app restored.' >&2; exit 1
fi
health_ok=false
for i in {1..20}; do
  if /usr/bin/curl --silent --fail --max-time 2 'http://127.0.0.1:9323/api/health' | /usr/bin/grep -q '"ok"'; then
    health_ok=true; break
  fi
  sleep 1
done
if [[ "$health_ok" != true ]]; then
  /usr/bin/osascript -e 'tell application id "com.specdest.moor-dev" to quit' >/dev/null 2>&1 || true
  for i in {1..20}; do
    if ! /usr/bin/pgrep -f '/Moor Dev.app/Contents/MacOS/moor' >/dev/null 2>&1; then break; fi
    sleep 1
  done
  if /usr/bin/pgrep -f '/Moor Dev.app/Contents/MacOS/moor' >/dev/null 2>&1; then
    echo 'Health check failed and new app is running. Backup preserved for manual recovery.' >&2
    if [[ -n "$BACKUP" ]]; then mv "$BACKUP" "$HOME/Applications/Moor Dev.previous.app"; fi
    exit 1
  fi
  rm -rf "$DEST"
  if [[ -n "$BACKUP" ]]; then mv "$BACKUP" "$DEST"; /usr/bin/open "$DEST" || true; fi
  echo 'Moor Dev health check failed; prior application restored.' >&2
  exit 1
fi
printf 'Installed and verified Moor Dev: %s\n' "$DEST"
