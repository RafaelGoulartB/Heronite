#!/usr/bin/env bash
# Opens heronite-rs on Hyprland workspace 7 (silently), captures its window
# with `grim -T` (works even when the window is off-screen), prints memory,
# then closes ONLY the window it opened (safe with concurrent runs).
#   usage: scripts/shot.sh out.png [wait_seconds] [binary]
#   env:   SHOT_ENV="VAR=value ..." extra environment for the app
set -euo pipefail
OUT=${1:-/tmp/heronite-rs.png}; WAIT=${2:-3}
BIN=${3:-$(cd "$(dirname "$0")/.." && pwd)/target/debug/heronite-rs}
LOG=$(mktemp /tmp/heronite-rs-XXXX.log)
list() { hyprctl clients -j | python3 -I -c "
import json,sys
for c in json.load(sys.stdin):
  if c['class']=='app.heronite.Heronite': print(c['stableId'], c['pid'])"; }
BEFORE=$(list | cut -d' ' -f1 | tr '\n' ' ')
hyprctl eval "hl.exec_cmd(\"env HERONITE_NO_ANIMATIONS=1 HERONITE_NO_SINGLE_INSTANCE=1 HERONITE_NO_TRAY=1 HERONITE_NO_ONBOARDING=1 HERONITE_NO_GAMEPAD=1 ${SHOT_ENV:-} $BIN > $LOG 2>&1\", { workspace = \"7 silent\" })" >/dev/null
ID=""; P=""
for _ in $(seq 1 200); do
  while read -r sid pid; do
    [[ " $BEFORE " == *" $sid "* ]] && continue
    ID=$sid; P=$pid
  done < <(list)
  [ -n "$ID" ] && break; sleep 0.05
done
[ -z "$ID" ] && { echo "window did not appear"; cat "$LOG"; exit 1; }
# Closed even when a step below fails (a copy left running keeps a window and
# a GPU context on the hidden workspace).
trap 'kill "$P" 2>/dev/null || true' EXIT
sleep "$WAIT"
# On a hidden workspace the window only repaints after a first capture wakes
# the compositor: capture once to warm up, then the real one.
grim -T "$ID" /dev/null 2>/dev/null || grim -T "$ID" "$OUT"; sleep 1
grim -T "$ID" "$OUT"
echo "PSS: $(awk '/^Pss:/{print int($2/1024)}' /proc/$P/smaps_rollup) MB"
grep -E "first render|on screen|ERROR heronite|panick" "$LOG" | head -5 || true
cp "$LOG" /tmp/heronite-rs.log
kill "$P" 2>/dev/null || true
