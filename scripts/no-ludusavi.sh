#!/usr/bin/env bash
# Opens heronite-rs as if Ludusavi were not installed, without uninstalling
# it: in a private mount namespace (`unshare -rm`) every folder holding a
# `ludusavi` gets a throwaway overlay with that one file removed. Nothing
# changes outside the app; the overlay goes away when it closes, and the
# rest of /usr/bin (xdg-open for "Download Ludusavi") still works.
#
# The data folder is a new, empty one (a fresh install, onboarding
# included), so it never forwards to a Heronite you have open.
#
#   usage: scripts/no-ludusavi.sh [--with-settings] [page] [binary]
#     --with-settings  copy your settings.json into the throwaway data folder
#     page             open on a page (HERONITE_DEBUG_PAGE, e.g. settings:saves);
#                      default: the app opens as usual (Collection)
#     binary           default target/debug/heronite-rs
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)

WITH_SETTINGS=0
if [ "${1:-}" = "--with-settings" ]; then
  WITH_SETTINGS=1
  shift
fi
PAGE=${1:-}
BIN=${2:-$ROOT/target/debug/heronite-rs}
[ -x "$BIN" ] || { echo "$BIN not found: run cargo build first" >&2; exit 1; }

T=$(mktemp -d /tmp/heronite-no-ludusavi-XXXX)
# Overlay work folders come back without permissions: give them back first.
cleanup() { chmod -R u+rwx "$T" 2>/dev/null || true; rm -rf "$T"; }
trap cleanup EXIT

mkdir -p "$T/data/local_library"
if [ "$WITH_SETTINGS" = 1 ]; then
  SRC=${XDG_DATA_HOME:-$HOME/.local/share}/heronite/local_library/settings.json
  if [ -f "$SRC" ]; then
    cp "$SRC" "$T/data/local_library/"
  else
    echo "no settings at $SRC; starting empty" >&2
  fi
fi

# Every folder Heronite looks in (resolve_binary), plus wherever $PATH has one.
DIRS=()
add() { for d in "${DIRS[@]}"; do [ "$d" = "$1" ] && return; done; DIRS+=("$1"); }
for d in /usr/bin /usr/local/bin "$HOME/.local/bin"; do
  [ -e "$d/ludusavi" ] && add "$d"
done
while IFS= read -r p; do
  add "$(dirname "$p")"
done < <(which -a ludusavi 2>/dev/null || true)
[ ${#DIRS[@]} -eq 0 ] && echo "ludusavi is not installed anyway; opening as is" >&2

echo "Hiding ludusavi in: ${DIRS[*]:-(none)}"
echo "Throwaway data folder: $T/data"

ENV=(HERONITE_DATA_DIR="$T/data")
[ -n "$PAGE" ] && ENV+=(HERONITE_DEBUG_PAGE="$PAGE")
env "${ENV[@]}" \
  unshare -rm bash -c '
    t=$1 bin=$2
    shift 2
    i=0
    for d in "$@"; do
      mkdir -p "$t/ov$i/up" "$t/ov$i/work"
      mount -t overlay overlay -o "lowerdir=$d,upperdir=$t/ov$i/up,workdir=$t/ov$i/work" "$d"
      rm -f "$d/ludusavi"
      i=$((i + 1))
    done
    if command -v ludusavi >/dev/null; then
      echo "ludusavi is still found at $(command -v ludusavi)" >&2
      exit 1
    fi
    exec "$bin"
  ' bash "$T" "$BIN" "${DIRS[@]}"
