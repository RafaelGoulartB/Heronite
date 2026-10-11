#!/usr/bin/env bash
# Packs the release binaries into dist/Heronite-<version>-x86_64.AppImage.
# Only the two binaries go in: the libraries they link (libudev, libxcb,
# libxkbcommon) and the ones loaded at run time (Vulkan, Wayland, fontconfig)
# come from the system, like any desktop app. The Epic login window also uses
# the system's WebKitGTK 4.1; without it the app offers the browser login.
#   usage: scripts/appimage.sh [version]
#   env:   APPIMAGETOOL=path to appimagetool (default: on PATH)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
VERSION=${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}
TOOL=${APPIMAGETOOL:-appimagetool}
BIN=target/release

[[ -x $BIN/heronite-rs && -x $BIN/heronite-epic-login ]] || cargo build --release -p heronite -p heronite-epic-login

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APPDIR=$WORK/Heronite.AppDir
ID=com.heronite.rs

install -Dm755 $BIN/heronite-rs "$APPDIR/usr/bin/heronite-rs"
install -Dm755 $BIN/heronite-epic-login "$APPDIR/usr/bin/heronite-epic-login"
install -Dm644 packaging/linux/$ID.desktop "$APPDIR/usr/share/applications/$ID.desktop"
install -Dm644 packaging/linux/$ID.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/$ID.png"
install -Dm644 LICENSE "$APPDIR/usr/share/doc/heronite/LICENSE"
install -Dm644 THIRD-PARTY-NOTICES.md "$APPDIR/usr/share/doc/heronite/THIRD-PARTY-NOTICES.md"
cp -r licenses "$APPDIR/usr/share/doc/heronite/licenses"

ln -s usr/share/applications/$ID.desktop "$APPDIR/$ID.desktop"
ln -s usr/share/icons/hicolor/256x256/apps/$ID.png "$APPDIR/$ID.png"
ln -s $ID.png "$APPDIR/.DirIcon"
cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/heronite-rs" "$@"
APPRUN
chmod 755 "$APPDIR/AppRun"

mkdir -p dist
OUT="dist/Heronite-$VERSION-x86_64.AppImage"
ARCH=x86_64 "$TOOL" --no-appstream "$APPDIR" "$OUT"
echo "$OUT"
