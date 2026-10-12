#!/usr/bin/env bash
# Runs Heronite as a brand-new user: an empty HOME and XDG folders in a
# temporary directory, so nothing of your own setup (Heronite data, Heroic,
# Steam, settings) is read or written.
#
#   usage: scripts/fresh-start.sh [options] [binary or AppImage]
#     --steam    let the app see your Steam libraries (read-only symlinks)
#     --keep     reuse the previous fresh home instead of starting empty
#     --dir DIR  where the fresh home lives (default /tmp/heronite-fresh)
#
# Without a binary it builds and runs `target/quick/heronite-rs`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR=/tmp/heronite-fresh
STEAM=0
KEEP=0
BIN=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --steam) STEAM=1 ;;
    --keep) KEEP=1 ;;
    --dir) DIR=${2:?--dir needs a folder}; shift ;;
    -h|--help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit 0 ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) BIN=$1 ;;
  esac
  shift
done

if [[ -z $BIN ]]; then
  (cd "$ROOT" && cargo build --profile quick -p heronite)
  BIN="$ROOT/target/quick/heronite-rs"
fi
BIN=$(realpath "$BIN")
[[ -x $BIN ]] || { echo "not executable: $BIN" >&2; exit 1; }

# Never wipe a real home by mistake: only a folder that holds nothing but
# what a fresh home has.
is_fresh_home() {
  local entry
  for entry in "$1"/* "$1"/.*; do
    [[ -e $entry || -L $entry ]] || continue
    case ${entry##*/} in
      . | .. | .config | .local | .cache | .steam | .var | .heronite-fresh | heronite.log) ;;
      *) return 1 ;;
    esac
  done
}
if [[ $KEEP -eq 0 && -e $DIR ]]; then
  is_fresh_home "$DIR" || { echo "$DIR holds other files, so it is not wiped; pick another --dir" >&2; exit 1; }
  rm -rf "$DIR"
fi
mkdir -p "$DIR/.config" "$DIR/.local/share" "$DIR/.cache" "$DIR/.local/state"
touch "$DIR/.heronite-fresh"

if [[ $STEAM -eq 1 ]]; then
  for p in .steam .local/share/Steam .var/app/com.valvesoftware.Steam; do
    if [[ -e $HOME/$p && ! -e $DIR/$p ]]; then
      mkdir -p "$(dirname "$DIR/$p")"
      ln -s "$HOME/$p" "$DIR/$p"
    fi
  done
fi

echo "Fresh home: $DIR (log in $DIR/heronite.log)"
exec env HOME="$DIR" \
  XDG_CONFIG_HOME="$DIR/.config" XDG_DATA_HOME="$DIR/.local/share" \
  XDG_CACHE_HOME="$DIR/.cache" XDG_STATE_HOME="$DIR/.local/state" \
  HERONITE_NO_SINGLE_INSTANCE=1 \
  "$BIN" > "$DIR/heronite.log" 2>&1
