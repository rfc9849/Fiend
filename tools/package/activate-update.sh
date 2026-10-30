#!/bin/sh
set -eu
umask 077
BIN=${FIEND_BIN:-/opt/fiend/bin}
CONFIG=${FIEND_CONFIG:-/etc/fiend/node.toml}
STATE=${FIEND_STATE:-/var/lib/fiend/state.json}
UPDATES=${FIEND_UPDATES:-/var/lib/fiend/updates}
USER=${FIEND_NODE_USER:-fiend}
MARKER=$UPDATES/core.pending.sha256
[ -f "$MARKER" ] || exit 0
reject() {
    printf '%s\n' 'fiend: rejected the pending binary update' >&2
    rm -f "$BIN/fiend-core.new"
    mv -f "$MARKER" "$MARKER.rejected"
    exit 0
}
EXPECTED=$(runuser -u "$USER" -- dd "if=$MARKER" bs=65 count=1 2>/dev/null) || reject
[ "${#EXPECTED}" = 64 ] || reject
case "$EXPECTED" in *[!0-9a-f]*) reject ;; esac
runuser -u "$USER" -- dd "if=$UPDATES/core.pending" bs=1048576 count=65 > "$BIN/fiend-core.new" 2>/dev/null || reject
SIZE=$(wc -c < "$BIN/fiend-core.new" | awk '{print $1}')
[ "$SIZE" -le 67108864 ] || reject
ACTUAL=$(sha256sum "$BIN/fiend-core.new" | awk '{print $1}')
[ "$ACTUAL" = "$EXPECTED" ] || reject
chmod 0755 "$BIN/fiend-core.new"
runuser -u "$USER" -- "$BIN/fiend-core.new" check "$CONFIG" "$STATE" || reject
if [ -f "$BIN/fiend-core" ]; then
    cp -p "$BIN/fiend-core" "$BIN/fiend-core.prev.new"
    mv -f "$BIN/fiend-core.prev.new" "$BIN/fiend-core.prev"
fi
mv -f "$BIN/fiend-core.new" "$BIN/fiend-core"
rm -f "$UPDATES/core.pending" "$MARKER"
