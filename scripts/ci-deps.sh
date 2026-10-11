#!/usr/bin/env bash
# System packages needed to build Heronite on Ubuntu (CI and release).
set -euo pipefail
sudo=""; [[ $(id -u) -ne 0 ]] && sudo=sudo
$sudo apt-get update
$sudo apt-get install -y --no-install-recommends \
  build-essential pkg-config clang lld libclang-dev cmake curl ca-certificates file \
  libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libx11-xcb-dev libwayland-dev \
  libudev-dev libfontconfig-dev libfreetype-dev \
  libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev
