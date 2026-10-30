#!/bin/bash
set -euo pipefail
export COPYFILE_DISABLE=1
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ARCH=${1:-all}
case "$ARCH" in
    all) ARCHES=(amd64 arm64) ;;
    amd64|arm64) ARCHES=("$ARCH") ;;
    *) echo 'Usage: server-bundle.sh [all|amd64|arm64]' >&2; exit 2 ;;
esac
VERSION=$(awk -F '"' '/^version = / {print $2; exit}' "$ROOT/fiend-core/Cargo.toml")
OUT=$ROOT/target/server-bundle
CACHE=$ROOT/target/server-cache/wireproxy-1.1.3
mkdir -p "$OUT" "$CACHE"
STAGE=$(mktemp -d "$OUT/.stage.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
(
    cd "$ROOT/fiend-panel/web"
    npm ci --no-audit --no-fund
    npm run check
    npm run build
)
TARGETS=()
for arch in "${ARCHES[@]}"; do
    case "$arch" in
        amd64) target=x86_64-unknown-linux-musl ;;
        arm64) target=aarch64-unknown-linux-musl ;;
    esac
    TARGETS+=(--target "$target")
done
(
    cd "$ROOT"
    cargo zigbuild --locked --release -p fiend-core -p fiend-panel "${TARGETS[@]}"
)
for arch in "${ARCHES[@]}"; do
    case "$arch" in
        amd64) target=x86_64-unknown-linux-musl ;;
        arm64) target=aarch64-unknown-linux-musl ;;
    esac
    asset=wireproxy_linux_$arch.tar.gz
    expected=$(awk -v file="$asset" '$2 == file {print $1}' "$ROOT/tools/package/wireproxy.sha256")
    if [ ! -f "$CACHE/$asset" ]; then
        curl --fail --location --proto '=https' --tlsv1.2 --retry 3 \
            "https://github.com/windtf/wireproxy/releases/download/v1.1.3/$asset" -o "$CACHE/$asset.part"
        mv "$CACHE/$asset.part" "$CACHE/$asset"
    fi
    actual=$(shasum -a 256 "$CACHE/$asset" | awk '{print $1}')
    [ "$actual" = "$expected" ] || { echo "wireproxy checksum failed: $asset" >&2; exit 1; }
    dir=$STAGE/$arch
    mkdir -p "$dir/systemd"
    cp "$ROOT/target/$target/release/fiend-core" "$ROOT/target/$target/release/fiend-panel" "$dir/"
    tar -xzf "$CACHE/$asset" -C "$dir" wireproxy
    cp "$ROOT/tools/package/install.sh" "$ROOT/tools/package/rollback.sh" "$dir/"
    cp "$ROOT/tools/package/systemd/"*.service "$dir/systemd/"
    chmod 755 "$dir/fiend-core" "$dir/fiend-panel" "$dir/wireproxy" "$dir/install.sh" "$dir/rollback.sh"
    (
        cd "$dir"
        shasum -a 256 fiend-core fiend-panel wireproxy install.sh rollback.sh systemd/*.service > SHA256SUMS
        if [ "$(uname -s)" = Darwin ]; then
            tar --no-xattrs --no-mac-metadata -czf "$OUT/fiend-$VERSION-linux-$arch.tar.gz.part" .
        else
            tar -czf "$OUT/fiend-$VERSION-linux-$arch.tar.gz.part" .
        fi
    )
    mv "$OUT/fiend-$VERSION-linux-$arch.tar.gz.part" "$OUT/fiend-$VERSION-linux-$arch.tar.gz"
    echo "Built $OUT/fiend-$VERSION-linux-$arch.tar.gz"
done
