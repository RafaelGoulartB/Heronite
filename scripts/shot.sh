#!/usr/bin/env bash
# Opens heronite-rs on Hyprland workspace 2 (silently), captures its window
# with `grim -T` (works even when the window is off-screen), prints memory,
# then closes it.  usage: scripts/shot.sh out.png [wait_seconds] [binary]
set -euo pipefail
OUT=${1:-/tmp/heronite-rs.png}; WAIT=${2:-3}
BIN=${3:-$(cd "$(dirname "$0")/.." && pwd)/target/debug/heronite-rs}
pkill -x heronite-rs 2>/dev/null || true
LOG=/tmp/heronite-rs.log
hyprctl eval "hl.exec_cmd(\"env ${SHOT_ENV:-} $BIN > $LOG 2>&1\", { workspace = \"2 silent\" })" >/dev/null
for _ in $(seq 1 100); do
  ID=$(hyprctl clients -j | python3 -I -c "
import json,sys
for c in json.load(sys.stdin):
  if c['class']=='com.heronite.rs': print(c['stableId']); break")
  [ -n "$ID" ] && break; sleep 0.05
done
sleep "$WAIT"
grim -T "$ID" "$OUT"
P=$(pgrep -x heronite-rs)
echo "PSS: $(awk '/^Pss:/{print int($2/1024)}' /proc/$P/smaps_rollup) MB"
grep -E "first render|ERROR|WARN|panick" $LOG | head -5 || true
pkill -x heronite-rs || true
