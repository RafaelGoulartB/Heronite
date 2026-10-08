#!/usr/bin/env bash
# Builds heronite-rs in release mode and installs it for the current user,
# next to (not replacing) the Electron Heronite.
#   binary   ~/.local/opt/heronite-rs/heronite-rs
#   launcher ~/.local/bin/heronite-rs
#   desktop  ~/.local/share/applications/com.heronite.rs.desktop
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${HERONITE_RS_PREFIX:-$HOME/.local/opt/heronite-rs}"
BIN_DIR="$HOME/.local/bin"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
ICON_SRC="$ROOT/../assets/heroic-local.png"

cd "$ROOT"
cargo build --release -p heronite

mkdir -p "$PREFIX" "$BIN_DIR" "$APP_DIR"
install -m 755 target/release/heronite-rs "$PREFIX/heronite-rs.new"
mv -f "$PREFIX/heronite-rs.new" "$PREFIX/heronite-rs"
ln -sf "$PREFIX/heronite-rs" "$BIN_DIR/heronite-rs"

ICON_NAME=com.heronite.rs
if [[ -f "$ICON_SRC" ]]; then
  mkdir -p "$ICON_DIR"
  install -m 644 "$ICON_SRC" "$ICON_DIR/$ICON_NAME.png"
fi

cat > "$APP_DIR/com.heronite.rs.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Heronite (Rust)
Comment=Game Collection (shares data with Heronite)
Exec=$PREFIX/heronite-rs
Icon=$ICON_NAME
Terminal=false
Categories=Game;
StartupWMClass=com.heronite.rs
DESKTOP

command -v update-desktop-database >/dev/null && update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
echo "Installed heronite-rs to $PREFIX"
