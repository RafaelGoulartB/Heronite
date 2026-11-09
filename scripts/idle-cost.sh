#!/usr/bin/env bash
# Measures what an idle heronite-rs costs: CPU time, wakeups (context
# switches), threads and memory over a quiet window. Opens the app on
# Hyprland workspace 7 (silently) against a fixture and closes only the
# process it started.
#   usage: scripts/idle-cost.sh <binary> <data_fixture_dir> [settle_s] [measure_s]
#   env:   IDLE_ENV="VAR=value ..." extra environment (e.g. HERONITE_DEBUG_START_HIDDEN=1)
set -euo pipefail
BIN=$1; DIR=$2; SETTLE=${3:-12}; MEASURE=${4:-60}
LOG=$(mktemp /tmp/heronite-idle-XXXX.log)
BASE="HERONITE_NO_ANIMATIONS=1 HERONITE_NO_SINGLE_INSTANCE=1 HERONITE_NO_TRAY=1 HERONITE_DISABLE_IDLE_JOBS=1 HERONITE_WINE_OFFLINE=1 HERONITE_DEBUG_FAKE_LAUNCH=1 HERONITE_NO_GAMEPAD=1"
hyprctl eval "hl.exec_cmd(\"env $BASE ${IDLE_ENV:-} HERONITE_DATA_DIR=$DIR $BIN > $LOG 2>&1\", { workspace = \"7 silent\" })" >/dev/null
P=""
for _ in $(seq 1 100); do P=$(pgrep -nf "^$BIN" || true); [ -n "$P" ] && break; sleep 0.1; done
[ -z "$P" ] && { echo "no pid"; cat "$LOG"; exit 1; }
sleep "$SETTLE"
# A window on a hidden workspace is never presented, and GPUI retries the
# first present every 10 ms. A capture presents it, as being on screen would.
ID=$(hyprctl clients -j | python3 -I -c "
import json,sys
for c in json.load(sys.stdin):
  if c['pid']==$P: print(c['stableId'])" | head -1)
if [ -n "$ID" ]; then grim -T "$ID" /dev/null 2>/dev/null || true; sleep 1; grim -T "$ID" /dev/null 2>/dev/null || true; sleep 2; fi
snap() { # cpu_ticks ctx_switches
  local t=0 c=0
  for s in /proc/$P/task/*/stat; do read -r -a f < "$s" 2>/dev/null || continue; t=$((t + f[13] + f[14])); done
  for s in /proc/$P/task/*/status; do c=$((c + $(awk '/ctxt_switches/{s+=$2} END{print s+0}' "$s" 2>/dev/null))); done
  echo "$t $c"
}
per_thread() { for s in /proc/$P/task/*; do echo "$(basename "$s") $(awk '/ctxt_switches/{s+=$2} END{print s+0}' "$s/status" 2>/dev/null) $(cat "$s/comm" 2>/dev/null)"; done; }
read -r t0 c0 < <(snap); per_thread > "$LOG.t0"
sleep "$MEASURE"
read -r t1 c1 < <(snap); per_thread > "$LOG.t1"
HZ=$(getconf CLK_TCK)
threads=$(ls /proc/$P/task | wc -l)
mem=$(grep -E "^(Rss|Pss|Pss_Anon|Private_Dirty)" /proc/$P/smaps_rollup | awk '{printf "%s %.1fMB ", $1, $2/1024}')
awk -v t=$((t1 - t0)) -v hz="$HZ" -v m="$MEASURE" -v c=$((c1 - c0)) -v th="$threads" \
  'BEGIN{printf "cpu %.2f%% (%.2fs in %ss)  wakeups %.1f/s  threads %d\n", 100*t/hz/m, t/hz, m, c/m, th}'
gpu=$(nvidia-smi 2>/dev/null | awk -v p="$P" '$5==p {print $(NF-1)}')
echo "$mem${gpu:+ GPU: $gpu}"
# Per-thread wakeups during the window, busiest first.
join -a2 -e0 -o '2.3,2.2,1.2' <(sort "$LOG.t0") <(sort "$LOG.t1") | awk '{d=$2-$3; if (d>0) printf "  %6.1f/s %s\n", d/'"$MEASURE"', $1}' | sort -rn | head -8
kill "$P" 2>/dev/null || true
sleep 1
kill -9 "$P" 2>/dev/null || true
