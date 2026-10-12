#!/usr/bin/env bash
# Signs the AppImages in a folder for the in-app updater and writes their
# checksums: <file>.minisig (minisign, trusted comment "version:<v> file:<name>",
# which the app checks against the release it installs) and <file>.sha256.
# The signatures are then verified with the public key built into the app
# (crates/update/src/lib.rs), so a secret that does not match it fails here
# instead of in every user's updater.
#   usage: scripts/sign-release.sh <version> [dir]   (dir defaults to dist)
#   env:   MINISIGN_SECRET_KEY=contents of the minisign secret key file
#          (made with `minisign -G -W`, no password)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION=${1:?usage: sign-release.sh <version> [dir]}
DIR=${2:-dist}
: "${MINISIGN_SECRET_KEY:?MINISIGN_SECRET_KEY is not set (the repository secret holding the release signing key)}"

PUBLIC_KEY=$(sed -n 's/^pub const PUBLIC_KEY: &str = "\(.*\)";$/\1/p' "$ROOT/crates/update/src/lib.rs")
[[ -n $PUBLIC_KEY ]] || { echo "sign-release.sh: PUBLIC_KEY not found in crates/update/src/lib.rs" >&2; exit 1; }

KEY=$(mktemp)
trap 'rm -f "$KEY"' EXIT
chmod 600 "$KEY"
printf '%s\n' "$MINISIGN_SECRET_KEY" > "$KEY"

shopt -s nullglob
files=("$DIR"/*.AppImage)
[[ ${#files[@]} -gt 0 ]] || { echo "sign-release.sh: no AppImage in $DIR" >&2; exit 1; }
for f in "${files[@]}"; do
  name=$(basename "$f")
  minisign -S -s "$KEY" -m "$f" -x "$f.minisig" -t "version:$VERSION file:$name"
  minisign -V -P "$PUBLIC_KEY" -m "$f" -x "$f.minisig" -q
  (cd "$(dirname "$f")" && sha256sum "$name" > "$name.sha256")
  echo "signed $name"
done
