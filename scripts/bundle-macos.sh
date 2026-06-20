#!/usr/bin/env bash
# Build a macOS .app bundle for animata with a proper Dock/Finder icon.
#
# The runtime light/dark Dock-icon swap lives in the binary (see crates/animata/src/mac_icon.rs);
# this bundle gives Finder/Launchpad (where the runtime swap doesn't reach) a static default icon —
# the LIGHT variant, per project choice. Re-run after changing the icon assets or the release binary.
#
# Usage: scripts/bundle-macos.sh   (output: target/release/bundle/animata.app)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ICONS="$ROOT/crates/animata/assets/icons/light"   # default bundle icon = light variant
OUT="$ROOT/target/release/bundle/animata.app"
APP_NAME="animata"
BUNDLE_ID="com.animata.viewer"

echo "› building release binary…"
cargo build --release -p animata --manifest-path "$ROOT/Cargo.toml"
BIN="$ROOT/target/release/$APP_NAME"
[ -x "$BIN" ] || { echo "release binary not found at $BIN"; exit 1; }

echo "› assembling $APP_NAME.app…"
rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
cp "$BIN" "$OUT/Contents/MacOS/$APP_NAME"

# Build animata.icns from the provided per-size PNGs (best quality — artist-made sizes, no rescale).
ICONSET="$(mktemp -d)/animata.iconset"
mkdir -p "$ICONSET"
cp "$ICONS/animata-light-16.png"   "$ICONSET/icon_16x16.png"
cp "$ICONS/animata-light-32.png"   "$ICONSET/icon_16x16@2x.png"
cp "$ICONS/animata-light-32.png"   "$ICONSET/icon_32x32.png"
cp "$ICONS/animata-light-64.png"   "$ICONSET/icon_32x32@2x.png"
cp "$ICONS/animata-light-128.png"  "$ICONSET/icon_128x128.png"
cp "$ICONS/animata-light-256.png"  "$ICONSET/icon_128x128@2x.png"
cp "$ICONS/animata-light-256.png"  "$ICONSET/icon_256x256.png"
cp "$ICONS/animata-light-512.png"  "$ICONSET/icon_256x256@2x.png"
cp "$ICONS/animata-light-512.png"  "$ICONSET/icon_512x512.png"
cp "$ICONS/animata-light-1024.png" "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$OUT/Contents/Resources/animata.icns"

cat > "$OUT/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>            <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>     <string>Animata</string>
    <key>CFBundleExecutable</key>      <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>      <string>$BUNDLE_ID</string>
    <key>CFBundleIconFile</key>        <string>animata</string>
    <key>CFBundlePackageType</key>     <string>APPL</string>
    <key>CFBundleShortVersionString</key> <string>0.2.0</string>
    <key>CFBundleVersion</key>         <string>0.2.0</string>
    <key>NSHighResolutionCapable</key> <true/>
    <key>LSMinimumSystemVersion</key>  <string>11.0</string>
</dict>
</plist>
PLIST

# Refresh the icon cache so Finder shows the new icon immediately.
touch "$OUT"
echo "✓ $OUT"
