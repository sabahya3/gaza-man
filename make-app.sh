#!/usr/bin/env bash
# Builds Gaza Man and wraps it in a macOS "Gaza Man.app" bundle you can double-click.
#
#   ./make-app.sh             build for this Mac
#   ./make-app.sh universal   one app for Apple Silicon and Intel Macs (needs rustup targets
#                             aarch64-apple-darwin and x86_64-apple-darwin)
set -euo pipefail
cd "$(dirname "$0")"

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

if [ "${1:-}" = "universal" ]; then
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    mkdir -p target/universal
    lipo -create -output target/universal/gaza-man \
        target/aarch64-apple-darwin/release/gaza-man \
        target/x86_64-apple-darwin/release/gaza-man
    BIN=target/universal/gaza-man
else
    cargo build --release
    BIN=target/release/gaza-man
fi

APP="Gaza Man.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/gaza-man"
cp assets/GazaMan.icns "$APP/Contents/Resources/GazaMan.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Gaza Man</string>
  <key>CFBundleDisplayName</key><string>Gaza Man</string>
  <key>CFBundleIdentifier</key><string>com.gazaman.app</string>
  <key>CFBundleExecutable</key><string>gaza-man</string>
  <key>CFBundleIconFile</key><string>GazaMan</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Tell Finder and the Dock to pick up the new icon.
touch "$APP"

echo "Done: $(pwd)/$APP"
