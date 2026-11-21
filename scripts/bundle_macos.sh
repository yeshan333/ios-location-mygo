#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

APP_NAME="iOS Location"
BUNDLE_ID="com.local.ios-location"
VERSION="0.2.0"
ARCH="$(uname -m)"
OUT_DIR="$ROOT_DIR/build/darwin-$ARCH"
APP_DIR="$OUT_DIR/$APP_NAME.app"
DMG_PATH="$OUT_DIR/ios-location-$VERSION-$ARCH.dmg"

echo "==> Building release binary (GPUI native Metal)..."
cargo build --release

echo "==> Generating AppIcon.icns from resources/icon.png..."
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
for sz in 16 32 128 256 512; do
  sips -z "$sz" "$sz" resources/icon.png --out "$ICONSET/icon_${sz}x${sz}.png" >/dev/null
  dbl=$((sz * 2))
  sips -z "$dbl" "$dbl" resources/icon.png --out "$ICONSET/icon_${sz}x${sz}@2x.png" >/dev/null
done
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
iconutil -c icns "$ICONSET" -o "$APP_DIR/Contents/Resources/AppIcon.icns"
cp resources/icon.png "$APP_DIR/Contents/Resources/icon.png"

cp target/release/ios-location "$APP_DIR/Contents/MacOS/$APP_NAME"
chmod +x "$APP_DIR/Contents/MacOS/$APP_NAME"

cat > "$APP_DIR/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>$APP_NAME</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIdentifier</key>
  <string>$BUNDLE_ID</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>$APP_NAME</string>
  <key>CFBundleDisplayName</key>
  <string>$APP_NAME</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$VERSION</string>
  <key>CFBundleVersion</key>
  <string>2</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

echo "==> Ad-hoc signing $APP_DIR..."
codesign --force --deep --sign - "$APP_DIR"

echo "==> Packaging $DMG_PATH..."
STAGING="$(mktemp -d)"
cp -R "$APP_DIR" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
rm -f "$DMG_PATH"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGING" -ov -format UDZO "$DMG_PATH" >/dev/null
rm -rf "$STAGING"

echo "==> Done:"
ls -lh "$OUT_DIR"
