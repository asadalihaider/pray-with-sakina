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
IDENTITY=$(security find-certificate -c "Sakina Local Signing" -Z \
  ~/Library/Keychains/login.keychain-db 2>/dev/null \
  | awk '/SHA-1 hash/{print $3}' | head -1)
if [ -z "$IDENTITY" ]; then
  echo "No 'Sakina Local Signing' certificate found. See README → Running it." >&2
  exit 1
fi

echo "==> quitting"
pkill -f "Sakina.app" 2>/dev/null || true
sleep 1

echo "==> building"
APPLE_SIGNING_IDENTITY="$IDENTITY" npm run tauri build

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
