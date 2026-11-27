#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 OpenXPlay.org
# SPDX-License-Identifier: AGPL-3.0-only
#
# Cross-compile the OpenXPlay relay (with the DLNA cast output baked in) into a
# static musl binary suitable for OpenWrt. Works on Linux hosts and GitHub
# Actions ubuntu-latest. Both aarch64 and x86_64 soft-route targets supported.
#
#   TARGET=aarch64-unknown-linux-musl bash build-musl.sh
#   TARGET=x86_64-unknown-linux-musl  bash build-musl.sh
#
# Produces: openwrt/diplay-cast/src/openxplay-relay

set -euo pipefail

TARGET="${TARGET:-aarch64-unknown-linux-musl}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
RUST_DIR="$REPO/rust"
OUT_DIR="$HERE/diplay-cast/src"

echo "==> Target : $TARGET"
echo "==> Rust   : $RUST_DIR"
echo "==> Output : $OUT_DIR"
mkdir -p "$OUT_DIR"

rustup target add "$TARGET"
rustup component add llvm-tools

# ring (crypto dep) compiles C code and needs musl headers + an ELF archiver.
# Prepare a musl sysroot (headers only) for the target architecture.
MUSL_VERSION=1.2.5
MUSL_SHA256=a9a118bbe84d8764da0ea0d28b3ab3fae8477fc7e4085d90102b8596fc7c75e4
case "$TARGET" in
  aarch64-*) ARCH=aarch64 ;;
  x86_64-*)  ARCH=x86_64 ;;
  *) echo "Unsupported target: $TARGET" >&2; exit 1 ;;
esac

SYSROOT="$RUST_DIR/target/musl-sysroot-$ARCH"
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
[ -x "$AR" ] || { echo "llvm-ar not found; run rustup component add llvm-tools" >&2; exit 1; }

# Environment variable names use the target triple with '-' replaced by '_'.
ENVKEY="${TARGET//-/_}"
export CC_${ENVKEY}=clang
export CFLAGS_${ENVKEY}="--target=$TARGET --sysroot=$SYSROOT -isystem $SYSROOT/include"
export AR_${ENVKEY}="$AR"

echo "==> Building $TARGET (release, static)"
cd "$RUST_DIR"
cargo build --release --target "$TARGET" -p oxp-bin

BIN="$RUST_DIR/target/$TARGET/release/oxp-relay"
[ -x "$BIN" ] || { echo "Expected binary not found: $BIN" >&2; exit 1; }
cp -f "$BIN" "$OUT_DIR/openxplay-relay"
# oxp-replay is the local H.264 file producer used for DLNA-only testing
# (path C): it feeds an Annex-B file into oxp-relay over a unix socket,
# bypassing the CarPlay receiver entirely (no WiFi / AP needed).
REPLAY="$RUST_DIR/target/$TARGET/release/oxp-replay"
[ -x "$REPLAY" ] || { echo "Expected replay binary not found: $REPLAY" >&2; exit 1; }
cp -f "$REPLAY" "$OUT_DIR/oxp-replay"
echo "==> Binaries ready:"
ls -lh "$OUT_DIR/openxplay-relay" "$OUT_DIR/oxp-replay"
