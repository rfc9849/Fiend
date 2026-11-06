#!/bin/sh
# Build the OpenWrt packages for opkg (OpenWrt 24.10 and older).
#
#   tools/package/openwrt.sh [ARCH...]
#
# ARCH is an OpenWrt package architecture, for example aarch64_cortex-a53,
# x86_64 or arm_cortex-a7_neon-vfpv4. See it on the router with
# `opkg print-architecture`. Without ARCH: aarch64_cortex-a53.
#
# Output: target/openwrt/fiend_<version>-r1_<arch>.ipk and
#         target/openwrt/luci-app-fiend_<version>-r1_all.ipk
#
# Needs cargo-zigbuild, or a C toolchain for each Rust target.
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SOURCE=$ROOT/packaging/openwrt
OUT=$ROOT/target/openwrt
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)-r1
export COPYFILE_DISABLE=1

if tar --version 2>/dev/null | grep -q GNU; then
	TAR_OWNER="--owner=0 --group=0 --numeric-owner --sort=name"
else
	TAR_OWNER="--uid 0 --gid 0 --no-xattrs"
fi

rust_target() {
	case "$1" in
	aarch64_*) echo aarch64-unknown-linux-musl ;;
	x86_64) echo x86_64-unknown-linux-musl ;;
	arm_cortex-a*_neon* | arm_cortex-a*_vfp*) echo armv7-unknown-linux-musleabihf ;;
	*)
		echo "unsupported architecture: $1" >&2
		exit 1
		;;
	esac
}

ipk() {
	name=$1 arch=$2 stage=$3
	size=$(du -sk "$stage/data" | cut -f1)
	sed -i.bak "s/^Installed-Size:.*/Installed-Size: $((size * 1024))/" "$stage/control/control"
	rm -f "$stage/control/control.bak"
	printf '2.0\n' > "$stage/debian-binary"
	# shellcheck disable=SC2086
	(cd "$stage/data" && tar $TAR_OWNER -czf ../data.tar.gz .)
	# shellcheck disable=SC2086
	(cd "$stage/control" && tar $TAR_OWNER -czf ../control.tar.gz .)
	file=$OUT/${name}_${VERSION}_$arch.ipk
	# shellcheck disable=SC2086
	(cd "$stage" && tar $TAR_OWNER -czf "$file" ./debian-binary ./data.tar.gz ./control.tar.gz)
	echo "$file"
}

control() {
	name=$1 arch=$2 depends=$3 section=$4 description=$5
	cat <<EOF
Package: $name
Version: $VERSION
Depends: $depends
Source: https://github.com/rfc9849/Fiend
License: PolyForm-Noncommercial-1.0.0
Section: $section
Architecture: $arch
Installed-Size: 0
Description: $description
EOF
}

build_fiend() {
	arch=$1
	target=$(rust_target "$arch")
	if command -v cargo-zigbuild >/dev/null 2>&1; then
		build=zigbuild
	else
		build=build
	fi
	rustup target add "$target" >/dev/null 2>&1 || true
	(cd "$ROOT" && cargo "$build" --release --locked -p fiend-daemon -p fiend-cli --target "$target")
	stage=$OUT/stage/fiend-$arch
	rm -rf "$stage"
	mkdir -p "$stage/data/usr/bin" "$stage/control"
	cp -R "$SOURCE/fiend/files/." "$stage/data/"
	cp "$ROOT/target/$target/release/fiend-daemon" "$ROOT/target/$target/release/fiend" "$stage/data/usr/bin/"
	chmod 0755 "$stage/data/usr/bin/fiend-daemon" "$stage/data/usr/bin/fiend" "$stage/data/etc/init.d/fiend"
	control fiend "$arch" "libc, kmod-nft-tproxy, kmod-nft-socket, firewall4, dnsmasq" net \
		"Fiend stealth tunnel for routers. It sends the sites in your lists through the tunnel or through DPI bypass, and the other traffic stays on the fast path of the kernel." \
		> "$stage/control/control"
	cp "$SOURCE/fiend/conffiles" "$SOURCE/fiend/postinst" "$SOURCE/fiend/prerm" "$SOURCE/fiend/postrm" "$stage/control/"
	chmod 0755 "$stage/control/postinst" "$stage/control/prerm" "$stage/control/postrm"
	ipk fiend "$arch" "$stage"
}

build_luci() {
	stage=$OUT/stage/luci-app-fiend
	rm -rf "$stage"
	mkdir -p "$stage/data" "$stage/control"
	cp -R "$SOURCE/luci-app-fiend/files/." "$stage/data/"
	control luci-app-fiend all "libc, fiend, luci-base, rpcd-mod-ucode" luci \
		"Web interface for Fiend" > "$stage/control/control"
	cp "$SOURCE/luci-app-fiend/postinst" "$SOURCE/luci-app-fiend/postrm" "$stage/control/"
	chmod 0755 "$stage/control/postinst" "$stage/control/postrm"
	ipk luci-app-fiend all "$stage"
}

mkdir -p "$OUT"
[ $# -gt 0 ] || set -- aarch64_cortex-a53
for arch in "$@"; do
	build_fiend "$arch"
done
build_luci
rm -rf "$OUT/stage"
