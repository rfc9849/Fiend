#!/bin/sh
# Build Fiend.app with an ad-hoc signature, and a DMG when hdiutil is present.
#
#   tools/package/macos-bundle.sh            release build
#   FIEND_PROFILE=debug tools/package/macos-bundle.sh
#
# Output: target/bundle/Fiend.app and target/bundle/Fiend.dmg
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
PROFILE=${FIEND_PROFILE:-release}
OUT=$ROOT/target/bundle
APP=$OUT/Fiend.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/fiend-desktop/Cargo.toml" | head -1)

cd "$ROOT"
if [ "$PROFILE" = release ]; then
    cargo build --release -p fiend-desktop -p fiend-daemon -p fiend-cli
    BIN=$ROOT/target/release
else
    cargo build -p fiend-desktop -p fiend-daemon -p fiend-cli
    BIN=$ROOT/target/debug
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/fiend-desktop" "$BIN/fiend-daemon" "$BIN/fiend" "$APP/Contents/MacOS/"
cp "$ROOT/fiend-desktop/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Fiend</string>
    <key>CFBundleDisplayName</key><string>Fiend</string>
    <key>CFBundleIdentifier</key><string>dev.fiend.desktop</string>
    <key>CFBundleExecutable</key><string>fiend-desktop</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key><string>Fiend share link</string>
            <key>CFBundleURLSchemes</key>
            <array><string>fiend</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"
echo "Built $APP"

if command -v hdiutil >/dev/null 2>&1; then
    rm -f "$OUT/Fiend.dmg"
    STAGE=$(mktemp -d)
    cp -R "$APP" "$STAGE/"
    ln -s /Applications "$STAGE/Applications"
    hdiutil create -quiet -volname Fiend -srcfolder "$STAGE" -ov -format UDZO "$OUT/Fiend.dmg"
    rm -rf "$STAGE"
    echo "Built $OUT/Fiend.dmg"
fi
