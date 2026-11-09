#!/usr/bin/env bash
# Signs a release for the in-app updater: SHA256SUMS lists the SHA-256 of
# every file people download (AppImages, system packages, the Flatpak, the
# Windows setup and portable zip), and SHA256SUMS.minisig signs it (minisign,
# trusted comment "version:<v> file:SHA256SUMS", which the app checks
# against the release it installs). The signature is then verified with the
# public key built into the app (crates/update/src/lib.rs), so a secret that
# does not match it fails here instead of in every user's updater.
# Transition: versions up to 1.0 only read a signature per file
# (<file>.minisig). Until most users run a version that reads SHA256SUMS,
# every file is also signed on its own (PER_FILE_SIGNATURES=1, the default);
# then the default becomes 0, and later this part goes.
#   usage: scripts/sign-release.sh <version> [dir]   (dir defaults to dist)
#   env:   MINISIGN_SECRET_KEY=contents of the minisign secret key file
#          (made with `minisign -G -W`, no password)
#          PER_FILE_SIGNATURES=1 also signs each file (default 1)
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
files=("$DIR"/*.AppImage "$DIR"/*.flatpak "$DIR"/*.deb "$DIR"/*.rpm "$DIR"/*.pkg.tar.zst
  "$DIR"/Heronite-Setup-*.exe "$DIR"/Heronite-Portable-*.zip)
[[ ${#files[@]} -gt 0 ]] || { echo "sign-release.sh: nothing to sign in $DIR" >&2; exit 1; }

names=()
for f in "${files[@]}"; do names+=("$(basename "$f")"); done
(cd "$DIR" && sha256sum -- "${names[@]}") > "$DIR/SHA256SUMS"
minisign -S -s "$KEY" -m "$DIR/SHA256SUMS" -x "$DIR/SHA256SUMS.minisig" -t "version:$VERSION file:SHA256SUMS"
minisign -V -P "$PUBLIC_KEY" -m "$DIR/SHA256SUMS" -x "$DIR/SHA256SUMS.minisig" -q
(cd "$DIR" && sha256sum --check --quiet SHA256SUMS)
echo "signed SHA256SUMS (${#names[@]} files)"

if [[ ${PER_FILE_SIGNATURES:-1} == 1 ]]; then
  for f in "${files[@]}"; do
    # The Flatpak is never installed by the app: only the list names it.
    [[ $f == *.flatpak ]] && continue
    name=$(basename "$f")
    minisign -S -s "$KEY" -m "$f" -x "$f.minisig" -t "version:$VERSION file:$name"
    minisign -V -P "$PUBLIC_KEY" -m "$f" -x "$f.minisig" -q
    echo "signed $name"
  done
fi
