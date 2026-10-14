#!/usr/bin/env bash
# Packs the release binaries into dist/Heronite-x86_64.AppImage (the name the
# in-app updater and AppImageUpdate look for in the latest release) and
# dist/Heronite-x86_64.AppImage.zsync. The name carries no version, so the
# README and every release link to the same file.
# The two binaries go in with the libraries the app links that a system may
# lack (BUNDLED_LIBS: not on the AppImage excludelist, so a clean system, like
# the AppImageHub test, has none of them). The rest comes from the system:
# glibc, libxcb and the libraries loaded at run time (Vulkan, Wayland,
# fontconfig). The Epic login window also uses the system's WebKitGTK 4.1;
# without it the app offers the browser login.
# The AppImage carries update information, so AppImageUpdate-based tools
# (Gear Lever, AppImageLauncher) update it from the .zsync of the latest
# release, downloading only the changed blocks.
#   usage: scripts/appimage.sh
#   needs: patchelf, readelf, and the BUNDLED_LIBS installed (the release
#          builds on the oldest supported Ubuntu: their versions run on every
#          newer system)
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
# Linked by heronite-rs, directly or through each other: keyboard input
# (xkbcommon) and controllers (udev).
BUNDLED_LIBS=(libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1 libudev.so.1)
# What may stay a system library: glibc and the rest of the AppImage
# excludelist that the app links.
SYSTEM_LIBS=" ld-linux-x86-64.so.2 libc.so.6 libm.so.6 libdl.so.2 libpthread.so.0 librt.so.1 libgcc_s.so.1 libxcb.so.1 "

install -Dm755 $BIN/heronite-rs "$APPDIR/usr/bin/heronite-rs"
install -Dm755 $BIN/heronite-epic-login "$APPDIR/usr/bin/heronite-epic-login"

# The bundled libraries are found through RUNPATH, not LD_LIBRARY_PATH, which
# would leak into the games the app launches. Only heronite-rs uses them: the
# Epic login window takes everything from the system, like its WebKitGTK.
mkdir -p "$APPDIR/usr/lib"
for lib in "${BUNDLED_LIBS[@]}"; do
  path=$(ldd "$APPDIR/usr/bin/heronite-rs" | awk -v lib="$lib" '$1 == lib { print $3 }')
  [[ -f $path ]] || { echo "appimage.sh: $lib not found on this system" >&2; exit 1; }
  install -m644 "$path" "$APPDIR/usr/lib/$lib"
  patchelf --set-rpath '$ORIGIN' "$APPDIR/usr/lib/$lib"
done
patchelf --set-rpath '$ORIGIN/../lib' "$APPDIR/usr/bin/heronite-rs"
# Fails when the app starts linking a library that is neither bundled nor on
# every system, which would make the AppImage quit at start on some of them.
for elf in "$APPDIR/usr/bin/heronite-rs" "$APPDIR"/usr/lib/*; do
  for lib in $(readelf -d "$elf" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
    [[ -f $APPDIR/usr/lib/$lib || $SYSTEM_LIBS == *" $lib "* ]] || {
      echo "appimage.sh: ${elf#"$APPDIR"/} links $lib: add it to BUNDLED_LIBS" >&2
      exit 1
    }
  done
done

install -Dm644 packaging/linux/$ID.desktop "$APPDIR/usr/share/applications/$ID.desktop"
# AppStream data for AppImageHub, which only looks for the *.appdata.xml name.
install -Dm644 packaging/linux/$ID.metainfo.xml "$APPDIR/usr/share/metainfo/$ID.appdata.xml"
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
