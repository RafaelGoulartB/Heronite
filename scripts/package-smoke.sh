#!/usr/bin/env bash
# Installs the packages in dist/ on clean containers of the oldest and newest
# supported systems and checks that every library resolves, the app starts
# (`--help`) and the update marker is in place. Needs docker.
#   usage: scripts/package-smoke.sh [deb|rpm|arch ...]   (default: all)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST=$ROOT/dist
formats=("$@")
[[ ${#formats[@]} -gt 0 ]] || formats=(deb rpm arch)

# Inside the container: no missing library, the app runs, the marker says $1.
# The Epic login helper only needs its libraries where the package manager
# installs recommended packages (apt, dnf).
CHECK='
set -e
missing=$(ldd /usr/bin/heronite-rs | grep "not found" || true)
[ -z "$missing" ] || { echo "heronite-rs: $missing"; exit 1; }
if [ "$2" = with-recommends ]; then
  missing=$(ldd /usr/bin/heronite-epic-login | grep "not found" || true)
  [ -z "$missing" ] || { echo "heronite-epic-login: $missing"; exit 1; }
fi
heronite-rs --help | head -1
[ "$(cat /usr/share/heronite/package-format)" = "$1" ]
echo "ok: $1 on $(. /etc/os-release; echo "$PRETTY_NAME")"
'

run() {
  local image=$1 format=$2 recommends=$3 install=$4
  echo "== $image ($format)"
  docker run --rm -v "$DIST:/dist:ro" "$image" sh -c "$install && sh -c '$CHECK' check $format $recommends"
}

for format in "${formats[@]}"; do
  case $format in
    deb)
      for image in ubuntu:22.04 debian:12 ubuntu:24.04; do
        run "$image" deb with-recommends \
          'export DEBIAN_FRONTEND=noninteractive; apt-get update -qq && apt-get install -y -qq /dist/heronite_*_amd64.deb >/dev/null'
      done
      ;;
    rpm)
      run fedora:41 rpm with-recommends 'dnf install -y -q /dist/heronite-*.x86_64.rpm >/dev/null'
      ;;
    arch)
      run archlinux:latest arch no-recommends 'pacman -Syu --noconfirm --needed >/dev/null && pacman -U --noconfirm /dist/heronite-*-x86_64.pkg.tar.zst >/dev/null'
      ;;
    *) echo "unknown format: $format" >&2; exit 2 ;;
  esac
done
