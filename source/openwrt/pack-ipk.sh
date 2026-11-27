#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 OpenXPlay.org
# SPDX-License-Identifier: AGPL-3.0-only
#
# Build an OpenWrt ipk from the prebuilt static binary + package files, without
# needing the full OpenWrt SDK. An ipk is just an ar archive of:
#   debian-binary, control.tar.gz, data.tar.gz
#
#   TARGET=aarch64-unknown-linux-musl bash pack-ipk.sh
#   TARGET=x86_64-unknown-linux-musl  bash pack-ipk.sh
#
# Produces: dist/diplay-cast_0.1.3-1_<opkg-arch>.ipk

set -euo pipefail

TARGET="${TARGET:-aarch64-unknown-linux-musl}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PKG_DIR="$HERE/diplay-cast"
BIN="$PKG_DIR/src/openxplay-relay"
DIST="$HERE/dist"

[ -x "$BIN" ] || { echo "Binary not found: $BIN — run build-musl.sh first" >&2; exit 1; }

# Map Rust target triple -> OpenWrt/opkg architecture string.
case "$TARGET" in
  aarch64-*) OPKG_ARCH=aarch64_cortex-a53 ;;
  x86_64-*)  OPKG_ARCH=x86_64 ;;
  *) echo "Unsupported target: $TARGET" >&2; exit 1 ;;
esac

VER=0.1.3
REL=1
IPK="diplay-cast_${VER}-${REL}_${OPKG_ARCH}.ipk"

echo "==> Packing $IPK"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/control" "$WORK/data" "$DIST"

# --- control.tar.gz ---
cat > "$WORK/control/control" <<EOF
Package: diplay-cast
Version: ${VER}-${REL}
Depends: libc, hostapd, wpa-supplicant
Section: multimedia
Category: Multimedia
Architecture: ${OPKG_ARCH}
Maintainer: OpenXPlay contributors
Description: Headless CarPlay receiver with DLNA cast output
 Headless CarPlay receiver based on OpenXPlay Linux relay, with a DLNA/UPnP
 output that streams MPEG-TS to a TV or head unit with only a DLNA receiver.
 MFi identity files must be supplied by the user and are NOT included.
EOF

cat > "$WORK/control/conffiles" <<'EOF'
/etc/config/diplay
EOF
( cd "$WORK/control" && tar --numeric-owner --owner=0 --group=0 -czf "$WORK/control.tar.gz" ./control ./conffiles )

# --- data.tar.gz (installed filesystem tree) ---
DATA="$WORK/data"
mkdir -p "$DATA/usr/bin" "$DATA/etc/init.d" "$DATA/etc/config" \
         "$DATA/etc/diplay/identity" "$DATA/usr/share/rpcd/acl.d" \
         "$DATA/usr/share/luci/menu.d" \
         "$DATA/www/luci-static/resources/view/diplay"

RECEIVER="$PKG_DIR/src/openxplay-receiver"
[ -x "$RECEIVER" ] || { echo "Receiver binary not found: $RECEIVER — run build-relay-musl.sh first" >&2; exit 1; }
WRAPPER="$PKG_DIR/files/usr/bin/oxp-relay-wrapper"

install -m 0755 "$BIN" "$DATA/usr/bin/oxp-relay"
install -m 0755 "$RECEIVER" "$DATA/usr/bin/openxplay-receiver"
install -m 0755 "$WRAPPER" "$DATA/usr/bin/oxp-relay-wrapper"
install -m 0755 "$PKG_DIR/files/etc/init.d/diplay-cast" "$DATA/etc/init.d/diplay-cast"
install -m 0644 "$PKG_DIR/files/etc/config/diplay" "$DATA/etc/config/diplay"
install -m 0644 "$PKG_DIR/files/usr/share/rpcd/acl.d/luci-app-diplay.json" "$DATA/usr/share/rpcd/acl.d/luci-app-diplay.json"
install -m 0644 "$PKG_DIR/files/usr/share/luci/menu.d/luci-app-diplay.json" "$DATA/usr/share/luci/menu.d/luci-app-diplay.json"
install -m 0644 "$PKG_DIR/files/www/luci-static/resources/view/diplay/cast.js" "$DATA/www/luci-static/resources/view/diplay/cast.js"
chmod 0700 "$DATA/etc/diplay/identity"

( cd "$DATA" && tar --numeric-owner --owner=0 --group=0 -czf "$WORK/data.tar.gz" . )

# --- assemble ar archive ---
printf '2.0\n' > "$WORK/debian-binary"
( cd "$WORK" && ar rc "$DIST/$IPK" debian-binary control.tar.gz data.tar.gz )

echo "==> Done: $DIST/$IPK"
ls -lh "$DIST/$IPK"
