#!/usr/bin/env bash
# Regenerate assets/AppIcon.icns and assets/app-icon.png from assets/AppIcon.png.
# Run after changing the source art. Requires macOS (swift, iconutil).
set -euo pipefail
cd "$(dirname "$0")/.."

SRC="assets/AppIcon.png"
[ -f "$SRC" ] || { echo "missing $SRC" >&2; exit 1; }

TMP="$(mktemp -d)"
SET="$TMP/AppIcon.iconset"
swift tools/gen-iconset.swift "$SRC" "$SET"
iconutil -c icns -o assets/AppIcon.icns "$SET"
cp "$SET/app-icon-512.png" assets/app-icon.png
rm -rf "$TMP"
echo "wrote assets/AppIcon.icns and assets/app-icon.png"
