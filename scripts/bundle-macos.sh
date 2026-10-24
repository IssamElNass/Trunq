#!/usr/bin/env bash
# Builds Trunq.app (and optionally a .dmg) from source.
#   ./scripts/bundle-macos.sh          -> target/release/bundle/Trunq.app
#   ./scripts/bundle-macos.sh --dmg    -> also target/release/bundle/Trunq.dmg
#   ./scripts/bundle-macos.sh --install -> also copies Trunq.app to /Applications
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
OUT=target/release/bundle
APP="$OUT/Trunq.app"

# CI builds a universal binary first and passes it in via TRUNQ_BINARY.
BINARY=${TRUNQ_BINARY:-target/release/trunq}
if [ -z "${TRUNQ_BINARY:-}" ]; then
  echo "==> Building release binary (v$VERSION)…"
  cargo build --release
fi

echo "==> Creating $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/Trunq"

echo "==> Generating icon"
ICONSET=$(mktemp -d)/Trunq.iconset
mkdir -p "$ICONSET"
for size in 16 32 64 128 256 512; do
  sips -z $size $size assets/app_icon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double assets/app_icon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Trunq.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Trunq</string>
  <key>CFBundleDisplayName</key><string>Trunq</string>
  <key>CFBundleIdentifier</key><string>dev.trunq.Trunq</string>
  <key>CFBundleExecutable</key><string>Trunq</string>
  <key>CFBundleIconFile</key><string>Trunq</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeExtensions</key><array><string>sql</string></array>
      <key>CFBundleTypeName</key><string>SQL script</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# Ad-hoc signature so Gatekeeper / keychain recognise the app consistently.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
echo "==> Built $APP"

for arg in "$@"; do
  case "$arg" in
    --dmg)
      rm -f "$OUT/Trunq.dmg"
      hdiutil create -volname Trunq -srcfolder "$APP" -ov -format UDZO "$OUT/Trunq.dmg" >/dev/null
      echo "==> Built $OUT/Trunq.dmg"
      ;;
    --install)
      rm -rf /Applications/Trunq.app
      cp -R "$APP" /Applications/
      echo "==> Installed to /Applications/Trunq.app"
      ;;
  esac
done
