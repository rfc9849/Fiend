#!/bin/sh
set -eu
BIN=${FIEND_BIN:-/opt/fiend/bin}
CONFIG=${FIEND_CONFIG:-/etc/fiend/node.toml}
runuser -u "${FIEND_NODE_USER:-fiend}" -- "$BIN/fiend-core.prev" check "$CONFIG"
mv -f "$BIN/fiend-core" "$BIN/fiend-core.failed"
mv "$BIN/fiend-core.prev" "$BIN/fiend-core"
