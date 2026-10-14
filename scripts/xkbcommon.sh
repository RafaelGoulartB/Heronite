#!/usr/bin/env bash
# Builds libxkbcommon and libxkbcommon-x11 for the AppImage and prints the
# folder that holds them. Ubuntu 22.04's 1.4.0 is too old to bundle: it does
# not know the keysyms that newer systems' Compose files use, skips those
# lines, and gives up on a whole file after ten of them (no dead keys, so no
# accents). A recent release reads older data too. It is built on the oldest
# supported Ubuntu, like the app, so it runs on glibc 2.35 and newer, and it
# reads the system's keyboard data (/usr/share/X11/xkb, /usr/share/X11/locale).
#   usage: scripts/xkbcommon.sh   (cached in target/xkbcommon-<version>)
#   needs: curl, a C compiler, meson >= 1.4, ninja, bison, pkg-config and
#          the libxcb and libxcb-xkb headers
set -euo pipefail

VERSION=1.13.2
SHA256=acc4d5f7c3cbba5f9f8d08d8bdbeede84ecede46792f47929aa9321873385528
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT=$ROOT/target/xkbcommon-$VERSION

if [[ ! -f $OUT/libxkbcommon.so.0 || ! -f $OUT/libxkbcommon-x11.so.0 ]]; then
  WORK=$(mktemp -d)
  trap 'rm -rf "$WORK"' EXIT
  curl -fsSL -o "$WORK/src.tar.gz" \
    "https://github.com/xkbcommon/libxkbcommon/archive/refs/tags/xkbcommon-$VERSION.tar.gz"
  echo "$SHA256  $WORK/src.tar.gz" | sha256sum -c - >&2
  tar -xzf "$WORK/src.tar.gz" -C "$WORK"
  meson setup "$WORK/build" "$WORK/libxkbcommon-xkbcommon-$VERSION" \
    --prefix=/usr --libdir=lib --buildtype=release \
    -Dxkb-config-root=/usr/share/X11/xkb -Dx-locale-root=/usr/share/X11/locale \
    -Denable-x11=true -Denable-wayland=false -Denable-xkbregistry=false \
    -Denable-tools=false -Denable-docs=false -Denable-bash-completion=false >&2
  meson compile -C "$WORK/build" >&2
  mkdir -p "$OUT"
  # The sonames, as the app links them (not the versioned file names).
  cp -L "$WORK/build/libxkbcommon.so.0" "$WORK/build/libxkbcommon-x11.so.0" "$OUT/"
fi
echo "$OUT"
