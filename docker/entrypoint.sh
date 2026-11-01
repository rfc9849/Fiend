#!/bin/sh
set -eu
umask 077
case "${1:-}" in
    node)
        if [ ! -e /etc/fiend/node.toml ]; then
            fiend-core prepare /etc/fiend --port "${FIEND_NODE_PORT:-443}"
        fi
        if [ ! -e /etc/fiend/control.secret ]; then
            fiend-core generate-secret > /etc/fiend/control.secret.new
            mv /etc/fiend/control.secret.new /etc/fiend/control.secret
        fi
        fiend-core check /etc/fiend/node.toml
        exec fiend-core node /etc/fiend/node.toml
        ;;
    panel)
        if [ ! -e /etc/fiend/panel.toml ]; then
            fiend-panel init /etc/fiend/panel.toml
            echo 'The setup link works one time and for one hour.'
        fi
        fiend-panel check /etc/fiend/panel.toml
        exec fiend-panel serve /etc/fiend/panel.toml
        ;;
    *)
        exec "$@"
        ;;
esac
