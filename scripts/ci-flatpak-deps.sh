#!/usr/bin/env bash
# What scripts/flatpak.sh needs on Ubuntu (CI and release): flatpak, Flathub
# for this user and Flathub's flatpak-builder (org.flatpak.Builder, which
# brings the tools it runs, like eu-strip). The SDK comes from Flathub when
# the script runs.
set -euo pipefail
sudo=""; [[ $(id -u) -ne 0 ]] && sudo=sudo
$sudo apt-get update
$sudo apt-get install -y --no-install-recommends flatpak python3-venv curl ca-certificates
# Ubuntu 24.04 keeps unprivileged programs from making user namespaces,
# which flatpak-builder's bubblewrap needs.
if [[ -e /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]]; then
  $sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
fi
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive flathub org.flatpak.Builder
