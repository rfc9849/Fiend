#!/bin/sh
set -eu
BIN=${FIEND_BIN:-/opt/fiend/bin}
CONFIG=${FIEND_CONFIG:-/etc/fiend/node.toml}
exec "$BIN/fiend-core" node "$CONFIG"
