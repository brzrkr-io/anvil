#!/usr/bin/env bash
# Build an optimized Anvil.app and install it for daily use.
# Usage: ./tools/install-app.sh [dest-dir]   (default: /Applications)
# After the first install, right-click the Dock icon > Options > Keep in Dock.
# Re-run anytime to update in place — the Dock entry persists (path is stable).
set -euo pipefail
cd "$(dirname "$0")/.."

DEST="${1:-/Applications}"
APP="$DEST/Anvil.app"

.zig/zig build bundle -Doptimize=ReleaseFast

rm -rf "$APP"
mkdir -p "$DEST"
cp -R zig-out/Anvil.app "$APP"
touch "$APP"

# Nudge Launch Services so Finder/Dock pick up the icon and bundle metadata.
LSREG="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
[ -x "$LSREG" ] && "$LSREG" -f "$APP" >/dev/null 2>&1 || true

echo "Installed $APP"
echo "Launch: open \"$APP\"  (then Dock > right-click > Options > Keep in Dock)"
