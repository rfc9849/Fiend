#!/bin/sh
# Install Fiend on macOS from a release archive. curl does not add the
# quarantine flag, so Gatekeeper does not block the ad-hoc signed app.
#
#   curl -fsSL <archive-url> -o /tmp/fiend.tar.gz && sh install-macos.sh /tmp/fiend.tar.gz
#   sh install-macos.sh <archive-url-or-file>
#
# The archive holds Fiend.app. Use `tar -C target/bundle -czf fiend.tar.gz Fiend.app`.
set -eu

SOURCE=${1:?Give the archive path or URL.}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

case "$SOURCE" in
    http://*|https://*) curl -fsSL "$SOURCE" -o "$WORK/fiend.tar.gz"; ARCHIVE=$WORK/fiend.tar.gz ;;
    *) ARCHIVE=$SOURCE ;;
esac

tar -xzf "$ARCHIVE" -C "$WORK"
[ -d "$WORK/Fiend.app" ] || { echo "The archive has no Fiend.app."; exit 1; }

rm -rf /Applications/Fiend.app
cp -R "$WORK/Fiend.app" /Applications/Fiend.app
xattr -dr com.apple.quarantine /Applications/Fiend.app 2>/dev/null || true

echo "Fiend is in /Applications."
echo "Open it and choose Install service, or run:"
echo "  sudo /Applications/Fiend.app/Contents/MacOS/fiend-daemon install"
echo "CLI: /Applications/Fiend.app/Contents/MacOS/fiend"
