#!/bin/zsh
# Construit target/Deezer.app (double-clic ou : open target/Deezer.app).
#   ./bundle.sh              build pour cette machine
#   UNIVERSAL=1 ./bundle.sh  binaire universel arm64 + x86_64 (releases) + zip dans dist/
set -e
cd "$(dirname "$0")"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/Deezer.app
if [[ -n "$UNIVERSAL" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  BIN=target/Deezer-universal
  lipo -create -output "$BIN" target/aarch64-apple-darwin/release/deezer-rs target/x86_64-apple-darwin/release/deezer-rs
else
  cargo build --release
  BIN=target/release/deezer-rs
fi
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/Deezer"
mkdir -p "$APP/Contents/Resources"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Deezer</string>
<key>CFBundleDisplayName</key><string>Deezer</string>
<key>CFBundleIdentifier</key><string>local.deezer-rs</string>
<key>CFBundleExecutable</key><string>Deezer</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundleIconName</key><string>AppIcon</string>
<key>CFBundleVersion</key><string>$VERSION</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>11.0</string>
</dict></plist>
PL
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
if [[ -n "$UNIVERSAL" ]]; then
  mkdir -p dist
  rm -f dist/Deezer-macos-universal.zip
  ditto -c -k --keepParent "$APP" dist/Deezer-macos-universal.zip
  echo "OK -> dist/Deezer-macos-universal.zip ($VERSION)"
fi
echo "OK -> $APP ($VERSION)"
