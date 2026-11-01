#!/usr/bin/env bash
# Builds the Flatpak (packaging/flatpak/app.heronite.Heronite.yml) into
# dist/Heronite-x86_64.flatpak, a single-file bundle users install with
# `flatpak install --user ./Heronite-x86_64.flatpak` (the GNOME runtime
# comes from Flathub). The build is offline: packaging/flatpak/
# cargo-sources.json, made here from Cargo.lock by flatpak-cargo-generator
# (pinned below), lists every crate for flatpak-builder to download first.
#   usage: scripts/flatpak.sh [--install]
#          --install also installs the build for this user
#   needs: flatpak, flatpak-builder (else the org.flatpak.Builder Flatpak),
#          python3; the SDK and its extensions are installed from Flathub
#          for this user when missing
#   env:   FLATPAK_BUILDER=command of flatpak-builder (default: on PATH,
#          else flatpak run org.flatpak.Builder)
#          FLATPAK_STATE=flatpak-builder's cache folder (default:
#          target/flatpak/state; CI keeps it between runs)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
ID=app.heronite.Heronite
MANIFEST=packaging/flatpak/$ID.yml
NAME=Heronite-x86_64.flatpak
INSTALL=false
[[ ${1:-} == --install ]] && INSTALL=true

# flatpak-builder-tools' cargo/flatpak-cargo-generator.py at this commit.
GENERATOR_COMMIT=41c20aa10819cdb2a4f3ca171758a96d1955c018
GENERATOR_SHA256=0a2db6be87d75910facef28ab46d4d6460802e8419ab850d0caa6a364d26b380

if [[ -n ${FLATPAK_BUILDER:-} ]]; then
  read -ra BUILDER <<<"$FLATPAK_BUILDER"
elif command -v flatpak-builder >/dev/null; then
  BUILDER=(flatpak-builder)
else
  BUILDER=(flatpak run org.flatpak.Builder)
fi

OUT=target/flatpak
STATE=${FLATPAK_STATE:-$OUT/state}
mkdir -p "$OUT"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# The crates of Cargo.lock as flatpak-builder sources. The generator runs in
# a virtual environment of its own, with the libraries it needs.
curl -fsSL -o "$WORK/flatpak-cargo-generator.py" \
  "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$GENERATOR_COMMIT/cargo/flatpak-cargo-generator.py"
echo "$GENERATOR_SHA256  $WORK/flatpak-cargo-generator.py" | sha256sum -c --quiet -
python3 -m venv "$WORK/venv"
"$WORK/venv/bin/pip" install --quiet --disable-pip-version-check 'aiohttp>=3.9.5,<4' 'PyYAML>=6.0.2,<7' 'tomlkit>=0.13.3,<1'
"$WORK/venv/bin/python" -I "$WORK/flatpak-cargo-generator.py" Cargo.lock -o packaging/flatpak/cargo-sources.json

# The repository is the build's output; the bundle is made from it.
"${BUILDER[@]}" --user --install-deps-from=flathub --force-clean --disable-rofiles-fuse \
  --state-dir="$STATE" --repo="$OUT/repo" "$OUT/build" "$MANIFEST"

mkdir -p dist
rm -f "dist/$NAME"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo "$OUT/repo" "dist/$NAME" "$ID"
if $INSTALL; then
  flatpak install --user --noninteractive --reinstall "dist/$NAME"
fi
echo "dist/$NAME"
