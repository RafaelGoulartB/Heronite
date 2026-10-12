#!/usr/bin/env bash
# Builds the system packages of a release into dist/ with nfpm
# (packaging/nfpm.yaml): heronite_<v>-1_amd64.deb, heronite-<v>-1.x86_64.rpm
# and heronite-<v>-1-x86_64.pkg.tar.zst. They hold the same binaries as the
# AppImage; users install the downloaded file (apt install ./file.deb,
# dnf install ./file.rpm, pacman -U file), and the app updates them later
# through the same package manager.
#   usage: scripts/packages.sh [version]
#   env:   NFPM=path to nfpm (default: on PATH)
#          BIN=folder of the binaries (default: target/release)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
VERSION=${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}
NFPM=${NFPM:-nfpm}
BIN=${BIN:-target/release}
[[ $BIN == /* ]] || BIN="$ROOT/$BIN"

[[ -x $BIN/heronite-rs && -x $BIN/heronite-epic-login ]] || cargo build --release -p heronite -p heronite-epic-login

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
MARKERS=$WORK/markers
mkdir -p "$MARKERS"
for format in deb rpm arch; do
  printf '%s\n' "$format" > "$MARKERS/package-format-$format"
done
# nfpm does not expand variables in file paths: fill the config in.
sed -e "s|\${VERSION}|$VERSION|g" -e "s|\${BIN}|$BIN|g" -e "s|\${MARKERS}|$MARKERS|g" packaging/nfpm.yaml > "$WORK/nfpm.yaml"

mkdir -p dist
rm -f dist/heronite_*.deb dist/heronite-*.rpm dist/heronite-*.pkg.tar.zst
for packager in deb rpm archlinux; do
  "$NFPM" package --config "$WORK/nfpm.yaml" --packager "$packager" --target dist/
done
ls dist/heronite_*.deb dist/heronite-*.rpm dist/heronite-*.pkg.tar.zst
