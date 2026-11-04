#!/usr/bin/env bash
# Renders the app's icon from its sources in packaging/icons/ (graphite.svg,
# and graphite-small.svg without the controller's buttons for the sizes where
# they would be specks). Run it again when an SVG changes and commit what it
# writes; builds do not run it.
#   app/assets/app-icon/graphite.png        256 px, the full icon (the X11 window)
#   app/assets/app-icon/graphite-small.png  64 px, the small one (the .ico's 16 to 32 px)
#   packaging/linux/app.heronite.Heronite.png  the full icon, 256 px
#   packaging/windows/heronite.ico          through scripts/windows/make-icon.py
#
#   usage: scripts/make-app-icons.sh      (needs rsvg-convert and Pillow)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/packaging/icons"
OUT="$ROOT/app/assets/app-icon"

mkdir -p "$OUT"
rsvg-convert -w 256 -h 256 "$SRC/graphite.svg" -o "$OUT/graphite.png"
rsvg-convert -w 64 -h 64 "$SRC/graphite-small.svg" -o "$OUT/graphite-small.png"
cp "$OUT/graphite.png" "$ROOT/packaging/linux/app.heronite.Heronite.png"
python3 "$ROOT/scripts/windows/make-icon.py"
