#!/bin/sh
# Build the KernelSU and Magisk module with the root helper.
#
#   tools/package/android-module.sh
#
# Output: target/module/fiend-module-<version>.zip
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SOURCE=$ROOT/packaging/android-module
OUT=$ROOT/target/module
STAGE=$OUT/stage
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
CODE=$(echo "$VERSION" | awk -F. '{ print $1 * 1000000 + $2 * 1000 + $3 }')

cd "$ROOT"
cargo ndk -t arm64-v8a -t x86_64 -P 26 build --release --locked -p fiend-root
rm -rf "$STAGE"
mkdir -p "$STAGE/bin/arm64-v8a" "$STAGE/bin/x86_64"
cp target/aarch64-linux-android/release/fiend-root "$STAGE/bin/arm64-v8a/"
cp target/x86_64-linux-android/release/fiend-root "$STAGE/bin/x86_64/"
cp -R "$SOURCE/META-INF" "$SOURCE/customize.sh" "$SOURCE/uninstall.sh" "$STAGE/"
sed -e "s/@VERSION@/$VERSION/" -e "s/@CODE@/$CODE/" "$SOURCE/module.prop.in" > "$STAGE/module.prop"
chmod 0755 "$STAGE/META-INF/com/google/android/update-binary" "$STAGE/uninstall.sh"
ZIP=$OUT/fiend-module-$VERSION.zip
rm -f "$ZIP"
(cd "$STAGE" && zip -qr -X "$ZIP" .)
echo "$ZIP"
