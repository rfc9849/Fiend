#!/bin/sh
set -eu
umask 077
fail() { printf '%s\n' "$*" >&2; exit 1; }
[ "$(uname -s)" = Linux ] || fail 'This installer needs Linux.'
[ "$(id -u)" = 0 ] || fail 'Run this installer as root.'
command -v systemctl >/dev/null || fail 'This installer needs systemd.'
MODE=${1:-}
[ "$#" -gt 0 ] && shift
case "$MODE" in panel|node|uninstall) ;; *) fail 'Usage: install.sh <panel|node|uninstall> [options]' ;; esac
BUNDLE=
IMPORT=
PORT=443
CONTROL=0
NO_START=0
PANEL_ONLY=0
KEEP_DATA=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --bundle|--import|--port)
            [ "$#" -ge 2 ] || fail "Missing value for $1"
            case "$1" in --bundle) BUNDLE=$2 ;; --import) IMPORT=$2 ;; --port) PORT=$2 ;; esac
            shift 2 ;;
        --control-secret-stdin) CONTROL=1; shift ;;
        --no-start) NO_START=1; shift ;;
        --panel-only) PANEL_ONLY=1; shift ;;
        --keep-data) KEEP_DATA=1; shift ;;
        *) fail "Unknown option: $1" ;;
    esac
done
if [ "$MODE" = uninstall ]; then
    [ -z "$BUNDLE$IMPORT" ] && [ "$CONTROL$PANEL_ONLY$NO_START" = 000 ] || fail 'Invalid uninstall options.'
    for service in fiend-panel fiend-node fiend-node-rollback; do
        systemctl disable --now "$service.service" 2>/dev/null || true
        rm -f "/etc/systemd/system/$service.service"
    done
    systemctl daemon-reload
    rm -rf /opt/fiend /run/fiend
    if [ "$KEEP_DATA" = 0 ]; then rm -rf /etc/fiend /var/lib/fiend; fi
    exit 0
fi
[ "$KEEP_DATA" = 0 ] || fail '--keep-data is only for uninstall.'
[ "$PANEL_ONLY" = 0 ] || [ "$MODE" = panel ] || fail '--panel-only needs panel mode.'
[ "$CONTROL" = 0 ] || [ "$MODE" = node ] || fail '--control-secret-stdin needs node mode.'
[ "$MODE" != node ] || [ "$CONTROL" = 1 ] || fail 'Node mode needs --control-secret-stdin.'
[ -z "$IMPORT" ] || [ "$PANEL_ONLY" = 0 ] || fail '--import cannot be used with --panel-only.'
[ -z "$IMPORT" ] || [ ! -e /etc/fiend/node.toml ] || fail 'A node config exists. Import would replace it.'
case "$PORT" in ''|*[!0-9]*) fail 'Invalid node port.' ;; esac
[ "$PORT" -gt 0 ] && [ "$PORT" -le 65535 ] || fail 'Invalid node port.'
TMP=
trap '[ -z "$TMP" ] || rm -rf "$TMP"' EXIT HUP INT TERM
SOURCE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ -n "$BUNDLE" ]; then
    TMP=$(mktemp -d)
    tar -tzf "$BUNDLE" | awk '/^\// || /(^|\/)\.\.(\/|$)/ {bad=1} END {exit bad}' || fail 'Invalid bundle paths.'
    tar --no-same-owner -xzf "$BUNDLE" -C "$TMP"
    SOURCE=$TMP
fi
(
    cd "$SOURCE"
    for file in fiend-core fiend-panel wireproxy install.sh rollback.sh activate-update.sh node-entry.sh systemd/fiend-node.service systemd/fiend-panel.service systemd/fiend-node-rollback.service; do
        [ -f "$file" ] && [ ! -L "$file" ] || fail "Missing bundle file: $file"
        awk -v name="$file" '$2 == name && length($1) == 64 {found++} END {exit found != 1}' SHA256SUMS || fail "Missing checksum: $file"
    done
    sha256sum --check --strict SHA256SUMS
)
"$SOURCE/fiend-core" --version
"$SOURCE/fiend-panel" --version
if [ -n "$IMPORT" ]; then "$SOURCE/fiend-core" check "$IMPORT"; NO_START=1; fi
if [ "$CONTROL" = 1 ]; then
    IFS= read -r CONTROL_SECRET || fail 'Send the control secret as one hex line with a newline.'
    [ "${#CONTROL_SECRET}" = 64 ] || fail 'The control secret needs 64 hex characters.'
    case "$CONTROL_SECRET" in *[!0-9a-fA-F]*) fail 'The control secret is not valid hex.' ;; esac
    if [ -e /etc/fiend/control.secret ]; then
        OLD_CONTROL=$(tr 'A-F' 'a-f' < /etc/fiend/control.secret)
        NEW_CONTROL=$(printf '%s' "$CONTROL_SECRET" | tr 'A-F' 'a-f')
        [ "$OLD_CONTROL" = "$NEW_CONTROL" ] || fail 'The stored control secret differs. No files changed.'
        unset OLD_CONTROL NEW_CONTROL
    fi
fi
if [ "$PANEL_ONLY" = 0 ] && [ "$NO_START" = 0 ] && [ ! -e /etc/fiend/node.toml ]; then
    command -v ss >/dev/null || fail 'This installer needs ss to check the node port.'
    [ -z "$(ss -H -ltn "sport = :$PORT")" ] || fail "Node port $PORT is in use."
fi
if ! getent passwd fiend >/dev/null; then
    useradd --system --user-group --home-dir /var/lib/fiend --shell /usr/sbin/nologin fiend
fi
install -d -o root -g root -m 0755 /opt/fiend /opt/fiend/bin
install -d -o root -g fiend -m 0750 /etc/fiend
install -d -o fiend -g fiend -m 0700 /var/lib/fiend /run/fiend
if [ "$PANEL_ONLY" = 0 ]; then
    if [ ! -e /etc/fiend/node.toml ]; then
        if [ -n "$IMPORT" ]; then
            "$SOURCE/fiend-core" prepare /etc/fiend --import "$IMPORT"
        else
            "$SOURCE/fiend-core" prepare /etc/fiend --port "$PORT"
        fi
        chown fiend:fiend /etc/fiend/node.toml
        chmod 0600 /etc/fiend/node.toml
    fi
    "$SOURCE/fiend-core" check /etc/fiend/node.toml
    if [ "$CONTROL" = 1 ] && [ ! -e /etc/fiend/control.secret ]; then
        printf '%s\n' "$CONTROL_SECRET" > /etc/fiend/control.secret
        chown fiend:fiend /etc/fiend/control.secret
        chmod 0600 /etc/fiend/control.secret
    fi
    if [ ! -e /etc/fiend/control.secret ]; then
        "$SOURCE/fiend-core" generate-secret > /etc/fiend/control.secret
        chown fiend:fiend /etc/fiend/control.secret
        chmod 0600 /etc/fiend/control.secret
    fi
    unset CONTROL_SECRET
    if [ -f /opt/fiend/bin/fiend-core ]; then
        if ! cmp -s "$SOURCE/fiend-core" /opt/fiend/bin/fiend-core; then
            cp -p /opt/fiend/bin/fiend-core /opt/fiend/bin/fiend-core.prev.new
            mv -f /opt/fiend/bin/fiend-core.prev.new /opt/fiend/bin/fiend-core.prev
        fi
    fi
    for binary in fiend-core wireproxy; do
        install -m 0755 "$SOURCE/$binary" "/opt/fiend/bin/$binary.new"
        mv -f "/opt/fiend/bin/$binary.new" "/opt/fiend/bin/$binary"
    done
    install -d -o fiend -g fiend -m 0700 /var/lib/fiend/updates
    install -m 0755 "$SOURCE/rollback.sh" /opt/fiend/bin/fiend-rollback
    install -m 0755 "$SOURCE/activate-update.sh" /opt/fiend/bin/fiend-activate-update
    install -m 0755 "$SOURCE/node-entry.sh" /opt/fiend/bin/fiend-node
    install -m 0644 "$SOURCE/systemd/fiend-node.service" "$SOURCE/systemd/fiend-node-rollback.service" /etc/systemd/system/
fi
if [ "$MODE" = panel ]; then
    install -m 0755 "$SOURCE/fiend-panel" /opt/fiend/bin/fiend-panel.new
    mv -f /opt/fiend/bin/fiend-panel.new /opt/fiend/bin/fiend-panel
    ACCESS=$(/opt/fiend/bin/fiend-panel init /etc/fiend/panel.toml ${IMPORT:+--import})
    chown fiend:fiend /etc/fiend/panel.toml
    chmod 0600 /etc/fiend/panel.toml
    install -d -o fiend -g fiend -m 0700 /var/lib/fiend/bundles /var/lib/fiend/bundles/inbox
    INBOX=/var/lib/fiend/bundles/inbox/install-$(date +%s).tar.gz
    if [ -n "$BUNDLE" ]; then
        install -o fiend -g fiend -m 0600 "$BUNDLE" "$INBOX"
    else
        tar -czf "$INBOX" -C "$SOURCE" .
        chown fiend:fiend "$INBOX"
        chmod 0600 "$INBOX"
    fi
    install -m 0644 "$SOURCE/systemd/fiend-panel.service" /etc/systemd/system/
fi
systemctl daemon-reload
if [ "$PANEL_ONLY" = 0 ] && [ "$NO_START" = 0 ]; then
    systemctl enable fiend-node.service
    systemctl restart fiend-node.service
    systemctl is-active --quiet fiend-node.service
else
    printf '%s\n' 'Node start skipped. The existing server has not been stopped.'
fi
if [ "$MODE" = panel ]; then
    systemctl enable fiend-panel.service
    systemctl restart fiend-panel.service
    systemctl is-active --quiet fiend-panel.service
fi
printf '%s\n' 'Server package installed.'
if [ "$MODE" = panel ]; then
    printf '%s\n' "$ACCESS" | sed "s/<host>/$(hostname)/"
    printf '%s\n' 'The setup link works one time and for one hour. Open the panel port in the firewall, or use an SSH tunnel or a reverse proxy.'
    printf '%s\n' 'New setup link: sudo /opt/fiend/bin/fiend-panel renew-setup /etc/fiend/panel.toml && sudo systemctl restart fiend-panel'
fi
