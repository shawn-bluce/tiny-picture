#!/usr/bin/env bash
# Builds "Tiny Picture.app" and a zip of it for GitHub releases.
#
# Output (in target/bundle/):
#   Tiny Picture.app
#   Tiny-Picture-<version>-arm64.zip
#
# The app is ad-hoc signed (no Apple Developer account), so it is not notarized.
#
# Versioning: CFBundleShortVersionString is the version from Cargo.toml.
# CFBundleVersion is the build number: $BUILD_NUMBER if set, otherwise the
# number of commits on the current branch (monotonic on master).
set -euo pipefail

cd "$(dirname "$0")/.."

APP_NAME="Tiny Picture"
BUNDLE_ID="com.shawn-bluce.tiny-picture"
EXECUTABLE="tiny-picture"
# Matches the minimum version rustc writes into the binary; older systems are not blocked.
MIN_MACOS="11.0"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
BUILD_NUMBER="${BUILD_NUMBER:-$(git rev-list --count HEAD 2>/dev/null || echo 0)}"

OUT_DIR="target/bundle"
APP="$OUT_DIR/$APP_NAME.app"
ZIP="$OUT_DIR/${APP_NAME// /-}-$VERSION-arm64.zip"

echo "==> Building $APP_NAME $VERSION (build $BUILD_NUMBER, release)"
cargo build --release

echo "==> Assembling $APP"
rm -rf "$APP" "$ZIP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/release/$EXECUTABLE" "$APP/Contents/MacOS/$EXECUTABLE"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$EXECUTABLE</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$BUILD_NUMBER</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.graphics-design</string>
    <key>LSMinimumSystemVersion</key>
    <string>$MIN_MACOS</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF
plutil -lint -s "$APP/Contents/Info.plist"

echo "==> Signing (ad hoc)"
codesign --force --sign - "$APP"
codesign --verify --strict "$APP"

echo "==> Zipping"
# ditto keeps the bundle structure, permissions and signature intact.
ditto -c -k --keepParent "$APP" "$ZIP"

echo "==> Done"
echo "    $APP"
echo "    $ZIP"
