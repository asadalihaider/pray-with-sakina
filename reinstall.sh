#!/bin/bash
# A full reinstall, the way a new user would get one.
#
# Testing an onboarding change against an already-onboarded install tests
# nothing, and replacing a running .app leaves the old binary in memory. So
# this always quits, removes, rebuilds, replaces and starts clean.
#
#   ./reinstall.sh          keep existing records and settings
#   ./reinstall.sh --fresh  wipe them, so onboarding runs from the top
set -euo pipefail

cd "$(dirname "$0")"
APP="/Applications/Sakina.app"
DATA="$HOME/Library/Application Support/dev.asadalihaider.sakina"

# Signing is not optional: UNUserNotificationCenter refuses an ad-hoc
# bundle outright, and the hardened runtime needs the entitlements that
# only a signed build carries. See README.
IDENTITY=$(security find-certificate -c "Sakina Signing" -Z \
  ~/Library/Keychains/login.keychain-db 2>/dev/null \
  | awk '/SHA-1 hash/{print $3}' | head -1)
if [ -z "$IDENTITY" ]; then
  echo "No 'Sakina Signing' certificate found. See README → Building it." >&2
  exit 1
fi

echo "==> quitting"
pkill -f "Sakina.app" 2>/dev/null || true
sleep 1

# Releases carry an updater signature, so the bundler insists on a key even
# for a throwaway local build. A contributor has no business holding the real
# one, so make them a local key once and use that: nothing will ever check an
# update signed with it.
UPDATER_KEY="release/updater.key"
if [ ! -f "$UPDATER_KEY" ]; then
  echo "==> generating a local updater key (release/updater.key)"
  mkdir -p release
  openssl rand -base64 18 | tr -d '\n' > release/updater-password.txt
  npm run tauri -- signer generate \
    -w "$UPDATER_KEY" -p "$(cat release/updater-password.txt)" >/dev/null
fi

echo "==> building"
APPLE_SIGNING_IDENTITY="$IDENTITY" \
TAURI_SIGNING_PRIVATE_KEY="$(cat "$UPDATER_KEY")" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat release/updater-password.txt)" \
  npm run tauri build

echo "==> replacing $APP"
rm -rf "$APP"
cp -R src-tauri/target/release/bundle/macos/Sakina.app /Applications/

if [ "${1:-}" = "--fresh" ]; then
  echo "==> wiping records and settings"
  rm -rf "$DATA"
  # The launch agent is rewritten on first run, and holds an absolute path.
  rm -f "$HOME/Library/LaunchAgents/Sakina.plist"
fi

echo "==> starting"
open "$APP"
sleep 4
pgrep -f "Sakina.app" >/dev/null && echo "running" || { echo "did not start" >&2; exit 1; }
codesign -dvv "$APP" 2>&1 | grep -E "^Authority|^Identifier" || true
