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
# Use a persistent, locally created signing identity when available. Ad-hoc
# signatures change after rebuilds and can cause repeated macOS Keychain prompts.
# Never create, export, or change trust for a signing key automatically.
LOCAL_SIGNING_IDENTITY='Moor Dev Local Signing'
SIGNING_IDENTITY='-'
IDENTITIES="$(/usr/bin/security find-identity -v -p codesigning 2>/dev/null || true)"
if [[ "$IDENTITIES" == *"\"$LOCAL_SIGNING_IDENTITY\""* ]]; then
  SIGNING_IDENTITY="$LOCAL_SIGNING_IDENTITY"
  echo 'Using persistent Moor Dev local signing identity.'
else
  echo 'No Moor Dev Local Signing identity found; using ad-hoc signing.' >&2
  echo 'Ad-hoc signing can trigger repeated Keychain prompts after each rebuild.' >&2
  echo 'See docs/base-connector.md for one-time local signing setup.' >&2
fi
mkdir -p "$HOME/Applications"
STAGE="$(mktemp -d "$HOME/Applications/.moor-dev-install.XXXXXX")"
BACKUP=""
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT
/usr/bin/ditto "$SOURCE" "$STAGE/Moor Dev.app"
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$STAGE/Moor Dev.app/Contents/Info.plist")" == "$EXPECTED_ID" ]] || exit 1
/usr/bin/codesign --force --deep --sign "$SIGNING_IDENTITY" "$STAGE/Moor Dev.app" || { echo 'Moor Dev signing failed.' >&2; exit 1; }
/usr/bin/codesign --verify --deep --strict "$STAGE/Moor Dev.app" || { echo 'Staged app signature verification failed.' >&2; exit 1; }
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
if ! /bin/launchctl asuser "$(/usr/bin/id -u)" /usr/bin/open -n "$DEST"; then
  rm -rf "$DEST"
  if [[ -n "$BACKUP" ]]; then mv "$BACKUP" "$DEST"; fi
  echo 'Launch failed; previous app restored.' >&2; exit 1
fi
health_ok=false
DATA_DIR="$HOME/Library/Application Support/com.specdest.moor-dev"
# The entire /api/* surface requires the owner-only X-Moor-Token header.
# Read the rotating token from disk inside Python, never put it in command arguments or logs.
health_ready() {
  /usr/bin/python3 - "$DATA_DIR" <<'PYHEALTH'
import json, pathlib, stat, sys, urllib.request
folder = pathlib.Path(sys.argv[1])
try:
    secret = folder / 'management-token'
    mode = secret.stat()
    if stat.S_IMODE(mode.st_mode) != 0o600:
        sys.exit(1)
    token = secret.read_text().strip()
    port = int((folder / 'port').read_text().strip())
    if not (1024 <= port <= 65535 and len(token) == 64):
        sys.exit(1)
    request = urllib.request.Request(f'http://127.0.0.1:{port}/api/health',
        headers={'X-Moor-Token': token})
    with urllib.request.urlopen(request, timeout=2) as response:
        sys.exit(0 if json.load(response).get('status') == 'ok' else 1)
except (OSError, ValueError, KeyError, TimeoutError, json.JSONDecodeError):
    sys.exit(1)
PYHEALTH
}
for i in {1..30}; do
  if health_ready; then health_ok=true; break; fi
  sleep 1
done
if [[ "$health_ok" != true ]]; then
  echo 'Startup health failed; checking launch diagnostics.' >&2
  /usr/bin/pgrep -fl 'Moor Dev.app/Contents/MacOS/moor' >&2 || true
  echo 'Authenticated local health check did not pass.' >&2
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
