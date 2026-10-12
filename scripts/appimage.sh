#!/usr/bin/env bash
# Packs the release binaries into dist/Heronite-x86_64.AppImage (the name the
# in-app updater and AppImageUpdate look for in the latest release) and
# dist/Heronite-x86_64.AppImage.zsync. The name carries no version, so the
# README and every release link to the same file.
# Only the two binaries go in: the libraries they link (libudev, libxcb,
# libxkbcommon) and the ones loaded at run time (Vulkan, Wayland, fontconfig)
# come from the system, like any desktop app. The Epic login window also uses
# the system's WebKitGTK 4.1; without it the app offers the browser login.
# The AppImage carries update information, so AppImageUpdate-based tools
# (Gear Lever, AppImageLauncher) update it from the .zsync of the latest
# release, downloading only the changed blocks.
#   usage: scripts/appimage.sh
#   env:   APPIMAGETOOL=path to appimagetool (default: on PATH)
#          BIN=folder of the binaries (default: target/release)
#          UPDATE_INFO=update information to embed ("" = none)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
TOOL=${APPIMAGETOOL:-appimagetool}
[[ $TOOL == */* ]] && TOOL=$(realpath "$TOOL")
BIN=${BIN:-target/release}
ARCH=x86_64
NAME=Heronite-$ARCH.AppImage
UPDATE_INFO=${UPDATE_INFO-"gh-releases-zsync|RafaelGoulartB|Heronite|latest|$NAME.zsync"}

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
# Only this build in dist/: sign-release.sh signs every AppImage there.
rm -f dist/Heronite-*
# appimagetool writes the .zsync into its working folder, which its AppRun
# takes from OWD (unset when it runs extracted, as in CI: it would be $HOME).
(cd dist && OWD=$PWD ARCH=$ARCH "$TOOL" --no-appstream ${UPDATE_INFO:+-u "$UPDATE_INFO"} "$APPDIR" "$NAME")
if [[ -n $UPDATE_INFO && ! -f dist/$NAME.zsync ]]; then
  echo "appimage.sh: appimagetool made no $NAME.zsync (is zsyncmake installed?)" >&2
  exit 1
fi
echo "dist/$NAME"
