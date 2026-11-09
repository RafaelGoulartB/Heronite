#!/usr/bin/env bash
# Writes the winget manifests of a published release into
# dist/winget/manifests/h/Heronite/Heronite/<version>/, laid out like
# microsoft/winget-pkgs, for the package's first pull request (later versions
# are opened by the winget job of release.yml). See packaging/winget/README.md.
#
#   scripts/winget-manifest.sh 1.2.0
#
# The installer's SHA-256 comes from the release's SHA256SUMS, so the release
# must be out already. Needs curl.
set -euo pipefail

version=${1:?usage: scripts/winget-manifest.sh <version, like 1.2.0>}
version=${version#v}
repo=RafaelGoulartB/Heronite
id=Heronite.Heronite
schema=1.10.0
setup=Heronite-Setup-x86_64.exe
base="https://github.com/$repo/releases/download/v$version"

sums=$(curl -fsSL "$base/SHA256SUMS") || {
    echo "No SHA256SUMS for v$version: is that release published?" >&2
    exit 1
}
sha=$(awk -v f="$setup" '{ n = $2; sub(/^\*/, "", n) } n == f { print toupper($1) }' <<<"$sums")
[[ $sha =~ ^[0-9A-F]{64}$ ]] || {
    echo "SHA256SUMS of v$version does not list $setup" >&2
    exit 1
}
date=$(curl -fsSL "https://api.github.com/repos/$repo/releases/tags/v$version" | sed -n 's/.*"published_at": *"\([0-9-]*\)T.*/\1/p' | head -n1)

root=$(cd "$(dirname "$0")/.." && pwd)
out="$root/dist/winget/manifests/h/Heronite/Heronite/$version"
mkdir -p "$out"

cat >"$out/$id.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.$schema.schema.json
PackageIdentifier: $id
PackageVersion: $version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: $schema
EOF

# The setup (packaging/windows/heronite.iss) installs per user, and Inno
# Setup files the copy under its AppId plus "_is1" in Apps & features.
cat >"$out/$id.installer.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.$schema.schema.json
PackageIdentifier: $id
PackageVersion: $version
MinimumOSVersion: 10.0.0.0
InstallerType: inno
Scope: user
InstallModes:
  - interactive
  - silent
  - silentWithProgress
UpgradeBehavior: install
ProductCode: '{104041C7-B50B-4E70-8560-A6CFF660777E}_is1'
${date:+ReleaseDate: $date
}Installers:
  - Architecture: x64
    InstallerUrl: $base/$setup
    InstallerSha256: $sha
ManifestType: installer
ManifestVersion: $schema
EOF

cat >"$out/$id.locale.en-US.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.$schema.schema.json
PackageIdentifier: $id
PackageVersion: $version
PackageLocale: en-US
Publisher: Heronite
PublisherUrl: https://heronite.app
PublisherSupportUrl: https://github.com/$repo/issues
Author: RafaGoulartB
PackageName: Heronite
PackageUrl: https://heronite.app
License: GPL-3.0-only
LicenseUrl: https://github.com/$repo/blob/main/LICENSE
ShortDescription: Every game you own, in one fast native launcher.
Description: |-
  Heronite brings your games from Steam, Epic Games, GOG, Amazon Games, Xbox and PC Game Pass,
  Battle.net, EA app, Ubisoft Connect and Rockstar Games, plus emulated and other Windows games,
  into one Collection. It launches them, records every session, and shows your play time,
  statistics and a year in review. It also has a controller-first console mode.
Moniker: heronite
Tags:
  - emulator
  - epic-games
  - game-launcher
  - games
  - gog
  - launcher
  - steam
ReleaseNotesUrl: https://github.com/$repo/releases/tag/v$version
ManifestType: defaultLocale
ManifestVersion: $schema
EOF

echo "Wrote $out"
ls -1 "$out"
