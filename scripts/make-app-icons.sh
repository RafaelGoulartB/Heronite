#!/usr/bin/env bash
# Renders the app's icons from their sources in packaging/icons/ (one SVG per
# icon, and a "-small" SVG without the controller's buttons for the sizes
# where they would be specks). Run it again when an SVG changes and commit
# what it writes; builds do not run it.
#   app/assets/app-icon/<name>.png        256 px, the full icon (Settings →
#                                         Appearance, the window, the desktop)
#   app/assets/app-icon/<name>-small.png  64 px, the small one (the 16
#                                         and 32 px window icons)
#   packaging/linux/app.heronite.Heronite.png  the default icon (graphite), 256 px
#   packaging/windows/heronite.ico        through scripts/windows/make-icon.py
#
#   usage: scripts/make-app-icons.sh      (needs rsvg-convert and Pillow)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/packaging/icons"
OUT="$ROOT/app/assets/app-icon"
DEFAULT=graphite

mkdir -p "$OUT"
for name in graphite light extruded; do
  rsvg-convert -w 256 -h 256 "$SRC/$name.svg" -o "$OUT/$name.png"
  rsvg-convert -w 64 -h 64 "$SRC/$name-small.svg" -o "$OUT/$name-small.png"
done
cp "$OUT/$DEFAULT.png" "$ROOT/packaging/linux/app.heronite.Heronite.png"
python3 "$ROOT/scripts/windows/make-icon.py"
