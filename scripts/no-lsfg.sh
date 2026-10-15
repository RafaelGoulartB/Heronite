#!/usr/bin/env bash
# Opens heronite-rs as if lsfg-vk (Lossless Scaling frame generation) were
# not installed, without uninstalling it: in a private mount namespace
# (`unshare -rm`) every Vulkan layer folder holding an lsfg-vk manifest gets
# a throwaway overlay with those files removed. Nothing changes outside the
# app; the overlays go away when it closes. With --no-app or --old-app the
# layer stays and Lossless Scaling is hidden instead.
#
# The data folder is a throwaway copy of yours (without tools/ and
# images-cache/, which are linked), so frame generation can be turned on for
# a game without touching your settings. Steam games are the exception:
# their settings are Steam's launch options, so use another game.
#
#   usage: scripts/no-lsfg.sh [--no-app | --old-app] [game] [binary]
#     --no-app   lsfg-vk installed, Lossless Scaling not found
#     --old-app  Lossless Scaling without lsfg-vk.dll (a copy older than lsfg-vk 2)
#     game       open that game's settings on the Other tab: its
#                `runner_appName` (e.g. legendary_Calluna, sideload_<id>)
#     binary     default target/debug/heronite-rs
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)

MODE=layer
case "${1:-}" in
  --no-app) MODE=app; shift ;;
  --old-app) MODE=old-app; shift ;;
esac
GAME=${1:-}
BIN=${2:-$ROOT/target/debug/heronite-rs}
[ -x "$BIN" ] || { echo "$BIN not found: run cargo build first" >&2; exit 1; }

T=$(mktemp -d /tmp/heronite-no-lsfg-XXXX)
# Overlay work folders come back without permissions: give them back first.
cleanup() { chmod -R u+rwx "$T" 2>/dev/null || true; rm -rf "$T"; }
trap cleanup EXIT

DATA=${XDG_DATA_HOME:-$HOME/.local/share}/heronite
if [ -d "$DATA" ]; then
  rsync -a --exclude tools --exclude images-cache "$DATA/" "$T/data/"
  for d in tools images-cache; do
    [ -e "$DATA/$d" ] && ln -s "$DATA/$d" "$T/data/$d"
  done
else
  mkdir -p "$T/data"
fi

# Each entry is "folder|file to remove from it".
HIDE=()
if [ "$MODE" = layer ]; then
  # The implicit layer folders the Vulkan loader (and Heronite) look in.
  BASES=("${XDG_CONFIG_HOME:-$HOME/.config}" /etc/xdg /etc "${XDG_DATA_HOME:-$HOME/.local/share}")
  IFS=: read -ra XDD <<<"${XDG_DATA_DIRS:-}"
  BASES+=("${XDD[@]}" /usr/local/share /usr/share)
  for b in "${BASES[@]}"; do
    d="${b%/}/vulkan/implicit_layer.d"
    [ -d "$d" ] || continue
    for f in "$d"/VkLayer_LSFGVK_frame_generation*.json "$d"/VkLayer_LS_frame_generation*.json; do
      [ -e "$f" ] && HIDE+=("$(readlink -f "$d")|$(basename "$f")")
    done
  done
else
  # Every Steam library (the main folders and those in libraryfolders.vdf).
  LIBS=("$HOME/.local/share/Steam" "$HOME/.steam/steam" "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam")
  for vdf in "$HOME/.local/share/Steam/steamapps/libraryfolders.vdf" "$HOME/.steam/steam/steamapps/libraryfolders.vdf"; do
    [ -f "$vdf" ] || continue
    while IFS= read -r p; do LIBS+=("$p"); done < <(sed -n 's/^[[:space:]]*"path"[[:space:]]*"\(.*\)"/\1/p' "$vdf")
  done
  for l in "${LIBS[@]}"; do
    app="$l/steamapps/common/Lossless Scaling"
    [ -d "$app" ] || continue
    if [ "$MODE" = app ]; then
      HIDE+=("$(readlink -f "$l/steamapps/common")|Lossless Scaling")
    elif [ -e "$app/lsfg-vk.dll" ]; then
      HIDE+=("$(readlink -f "$app")|lsfg-vk.dll")
    fi
  done
fi
# The same folder may be reached twice (symlinked Steam folders).
mapfile -t HIDE < <(printf '%s\n' "${HIDE[@]}" | sort -u | sed '/^$/d')
[ ${#HIDE[@]} -eq 0 ] && echo "nothing to hide: it is missing anyway; opening as is" >&2

echo "Hiding:"
for h in "${HIDE[@]}"; do echo "  ${h%%|*}/${h#*|}"; done
echo "Throwaway data folder: $T/data"

ENV=(HERONITE_DATA_DIR="$T/data")
if [ -n "$GAME" ]; then
  ENV+=(HERONITE_DEBUG_SELECT="$GAME" HERONITE_DEBUG_DIALOG=gamesettings:other)
fi
env "${ENV[@]}" \
  unshare -rm bash -c '
    t=$1 bin=$2
    shift 2
    i=0
    # One overlay per folder, then the files of that folder are removed
    # (`userxattr`: removing a whole folder needs it in a user namespace).
    declare -A mounted=()
    for h in "$@"; do
      d=${h%%|*} f=${h#*|}
      if [ -z "${mounted[$d]:-}" ]; then
        mkdir -p "$t/ov$i/up" "$t/ov$i/work"
        mount -t overlay overlay -o "userxattr,lowerdir=$d,upperdir=$t/ov$i/up,workdir=$t/ov$i/work" "$d"
        mounted[$d]=1
        i=$((i + 1))
      fi
      rm -rf "${d:?}/$f"
      if [ -e "$d/$f" ]; then
        echo "$d/$f is still there" >&2
        exit 1
      fi
    done
    exec "$bin"
  ' bash "$T" "$BIN" "${HIDE[@]}"
