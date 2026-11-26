#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 OpenXPlay.org
# SPDX-License-Identifier: AGPL-3.0-only
#
# Build an OpenWrt 25.12+ .apk from the prebuilt static binaries using the
# official OpenWrt SDK. An apk is an ADB-signed archive (magic "ADBd"), so it
# cannot be assembled by hand the way an ipk (ar + tar.gz) can — the SDK's
# package build is required to produce a valid, installable apk.
#
# The SDK is discovered dynamically: names embed the gcc version and use the
# .tar.zst suffix, so hardcoding them breaks on every OpenWrt upgrade.
#
#   TARGET=aarch64-unknown-linux-musl bash build-apk.sh
#   TARGET=x86_64-unknown-linux-musl  bash build-apk.sh
#
# Produces: dist/diplay-cast-0.1.2-r1-<arch>.apk
# (apk files carry no arch segment in their name; we append one so the two
#  matrix artifacts don't collide when merged into a single Release.)

set -euo pipefail

TARGET="${TARGET:-aarch64-unknown-linux-musl}"
SDK_VERSION="${SDK_VERSION:-25.12.5}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
PKG_DIR="$HERE/diplay-cast"
DIST="$HERE/dist"

BIN="$PKG_DIR/src/openxplay-relay"
RECEIVER="$PKG_DIR/src/openxplay-receiver"
[ -x "$BIN" ] || { echo "relay binary not found: $BIN — run build-musl.sh first" >&2; exit 1; }
[ -x "$RECEIVER" ] || { echo "receiver binary not found: $RECEIVER — run build-relay-musl.sh first" >&2; exit 1; }

# Rust target triple -> OpenWrt SDK target + apk arch label.
case "$TARGET" in
  aarch64-*)
    SDK_SUBTARGET="armsr/armv8"
    SDK_ARCH="armsr-armv8"
    APK_ARCH="aarch64_cortex-a53"
    ;;
  x86_64-*)
    SDK_SUBTARGET="x86/64"
    SDK_ARCH="x86-64"
    APK_ARCH="x86_64"
    ;;
  *) echo "Unsupported target: $TARGET" >&2; exit 1 ;;
esac

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> Fetching OpenWrt SDK $SDK_VERSION ($SDK_ARCH)"
BASE="https://downloads.openwrt.org/releases/${SDK_VERSION}/targets/${SDK_SUBTARGET}"
NAME="$(curl -fsSL --retry 3 "$BASE/" \
  | grep -oE 'openwrt-sdk-[^"< ]*\.tar\.(zst|xz)' \
  | grep -- "-${SDK_ARCH}_" | head -n1 || true)"
[ -n "$NAME" ] || { echo "::error::SDK not found at $BASE/ (arch=$SDK_ARCH)" >&2; exit 1; }
echo "    SDK: $NAME"
curl -fsSL --retry 3 -o "$WORK/sdk.tar" "$BASE/$NAME"

mkdir -p "$WORK/sdk"
case "$NAME" in
  *.tar.zst) tar --zstd -xf "$WORK/sdk.tar" -C "$WORK/sdk" --strip-components=1 ;;
  *.tar.xz)  tar -xJf "$WORK/sdk.tar" -C "$WORK/sdk" --strip-components=1 ;;
  *) echo "::error::Unknown SDK archive format: $NAME" >&2; exit 1 ;;
esac
SDK="$WORK/sdk"

echo "==> Mounting diplay-cast into SDK package/"
rm -rf "$SDK/package/diplay-cast"
mkdir -p "$SDK/package/diplay-cast/src"
cp "$PKG_DIR/Makefile" "$SDK/package/diplay-cast/Makefile"
cp -r "$PKG_DIR/files" "$SDK/package/diplay-cast/files"
cp "$BIN" "$SDK/package/diplay-cast/src/openxplay-relay"
cp "$RECEIVER" "$SDK/package/diplay-cast/src/openxplay-receiver"

echo "==> Configuring SDK (.config)"
printf '%s\n' \
  'CONFIG_ALL=n' \
  'CONFIG_ALL_KMODS=n' \
  'CONFIG_PACKAGE_diplay-cast=y' > "$SDK/.config"
make -C "$SDK" defconfig >/dev/null

# Verify make actually recognized the package (commented-out still counts).
if grep -q "^CONFIG_PACKAGE_diplay-cast=" "$SDK/.config"; then
  echo "    recognized: $(grep '^CONFIG_PACKAGE_diplay-cast=' "$SDK/.config")"
elif grep -q "^# CONFIG_PACKAGE_diplay-cast is not set" "$SDK/.config"; then
  echo "    recognized but not selected — aborting"
  exit 1
else
  echo "::error::make did not recognize package/diplay-cast" >&2
  grep -i diplay "$SDK/.config" || true
  exit 1
fi

echo "==> Compiling apk"
set -o pipefail
make -C "$SDK" package/diplay-cast/compile -j"$(nproc)" V=s 2>&1 | tee "$WORK/build.log"

# NOTE: we deliberately do NOT fail on "has a dependency on ..." warnings.
# A dependency warning can be harmless build-time noise (the index scan
# reports it before feeds/install resolve it), and target-specific packages
# such as hostapd/wpa-supplicant may not be expanded in a generic SDK's
# .config even though they exist on the real device. A successful compile
# plus the .config recognition check above are the reliable signals.

APK="$(find "$SDK/bin" -type f -name 'diplay-cast-*.apk' | head -n1)"
[ -n "$APK" ] || { echo "::error::no apk produced under $SDK/bin" >&2; find "$SDK/bin" -type f | head -30; exit 1; }

mkdir -p "$DIST"
OUT="$DIST/diplay-cast-0.1.2-r1-${APK_ARCH}.apk"
cp -f "$APK" "$OUT"
echo "==> Done: $OUT"
ls -lh "$OUT"
