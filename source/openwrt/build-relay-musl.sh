#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 OpenXPlay.org
# SPDX-License-Identifier: AGPL-3.0-only
#
# Cross-compile the CarPlay RECEIVER (direlay-linux / openxplay-relay) into a
# static musl binary. This is the process that talks to the iPhone (iAP2/MFi),
# then supervises the Stream output process (oxp-relay with --cast).
#
#   TARGET=aarch64-unknown-linux-musl bash build-relay-musl.sh
#   TARGET=x86_64-unknown-linux-musl  bash build-relay-musl.sh
#
# Produces: openwrt/diplay-cast/src/openxplay-receiver

set -euo pipefail

TARGET="${TARGET:-aarch64-unknown-linux-musl}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
RELAY_DIR="$REPO/relay/rust"
OUT_DIR="$HERE/diplay-cast/src"

echo "==> Target  : $TARGET"
echo "==> Relay   : $RELAY_DIR"
echo "==> Output  : $OUT_DIR"
mkdir -p "$OUT_DIR"

rustup target add "$TARGET"
rustup component add llvm-tools

MUSL_VERSION=1.2.5
MUSL_SHA256=a9a118bbe84d8764da0ea0d28b3ab3fae8477fc7e4085d90102b8596fc7c75e4
case "$TARGET" in
  aarch64-*) ARCH=aarch64 ;;
  x86_64-*)  ARCH=x86_64 ;;
  *) echo "Unsupported target: $TARGET" >&2; exit 1 ;;
esac

SYSROOT="$RELAY_DIR/target/musl-sysroot-$ARCH"
if [ ! -f "$SYSROOT/include/stddef.h" ]; then
  echo "==> Preparing musl headers ($ARCH)"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  curl -sSfL -o "$tmp/musl.tar.gz" "https://musl.libc.org/releases/musl-$MUSL_VERSION.tar.gz"
  echo "$MUSL_SHA256  $tmp/musl.tar.gz" | sha256sum -c - >/dev/null
  tar xzf "$tmp/musl.tar.gz" -C "$tmp"
  make -s -C "$tmp/musl-$MUSL_VERSION" ARCH="$ARCH" prefix="$SYSROOT" install-headers
fi

AR=$(ls "$(rustc --print sysroot)"/lib/rustlib/*/bin/llvm-ar 2>/dev/null | head -n 1 || true)
[ -x "$AR" ] || { echo "llvm-ar not found" >&2; exit 1; }

ENVKEY="${TARGET//-/_}"
export CC_${ENVKEY}=clang
export CFLAGS_${ENVKEY}="--target=$TARGET --sysroot=$SYSROOT -isystem $SYSROOT/include"
export AR_${ENVKEY}="$AR"

echo "==> Building receiver ($TARGET, static)"
cd "$RELAY_DIR"
# mdns-sd is vendored+patched; rely on crates.io for the rest (CI has network).
cargo build --release --target "$TARGET" -p openxplay-relay --bin openxplay-relay

BIN="$RELAY_DIR/target/$TARGET/release/openxplay-relay"
[ -x "$BIN" ] || { echo "Expected binary not found: $BIN" >&2; exit 1; }
cp -f "$BIN" "$OUT_DIR/openxplay-receiver"
echo "==> Receiver ready:"
ls -lh "$OUT_DIR/openxplay-receiver"
