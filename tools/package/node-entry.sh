#!/bin/sh
set -eu
BIN=${FIEND_BIN:-/opt/fiend/bin}
CONFIG=${FIEND_CONFIG:-/etc/fiend/node.toml}
if "$BIN/fiend-core" --help | awk '$1 == "node" {found=1} END {exit !found}'; then
    exec "$BIN/fiend-core" node "$CONFIG"
fi
exec "$BIN/fiend-core" server "$CONFIG"
