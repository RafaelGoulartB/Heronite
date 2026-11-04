#!/usr/bin/env bash
# Builds heronite-rs in release mode and installs it for the current user.
# It is a separate app from Heroic/Heronite with its own data folder
# (~/.local/share/heronite); the first start copies an existing Heroic setup.
#   binary   ~/.local/opt/heronite-rs/heronite-rs
#   launcher ~/.local/bin/heronite-rs
#   desktop  ~/.local/share/applications/app.heronite.Heronite.desktop
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${HERONITE_RS_PREFIX:-$HOME/.local/opt/heronite-rs}"
BIN_DIR="$HOME/.local/bin"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
ICON_SRC="$ROOT/packaging/linux/app.heronite.Heronite.png"

cd "$ROOT"
cargo build --release -p heronite -p heronite-epic-login

mkdir -p "$PREFIX" "$BIN_DIR" "$APP_DIR"
# The menu entry and icon of the app id up to 1.0.
rm -f "$APP_DIR/com.heronite.rs.desktop" "$ICON_DIR/com.heronite.rs.png"
install -m 755 target/release/heronite-rs "$PREFIX/heronite-rs.new"
mv -f "$PREFIX/heronite-rs.new" "$PREFIX/heronite-rs"
# Epic login window (WebKit), a separate binary the app starts when needed.
install -m 755 target/release/heronite-epic-login "$PREFIX/heronite-epic-login"
ln -sf "$PREFIX/heronite-rs" "$BIN_DIR/heronite-rs"

ICON_NAME=app.heronite.Heronite
if [[ -f "$ICON_SRC" ]]; then
  mkdir -p "$ICON_DIR"
  install -m 644 "$ICON_SRC" "$ICON_DIR/$ICON_NAME.png"
fi

cat > "$APP_DIR/app.heronite.Heronite.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Heronite (Dev)
Comment=Game Collection
Exec=$PREFIX/heronite-rs %U
Icon=$ICON_NAME
Terminal=false
Categories=Game;
MimeType=x-scheme-handler/heronite;x-scheme-handler/heronite-rs;
StartupWMClass=app.heronite.Heronite
DESKTOP

# heronite:// links, and heronite-rs:// from before the rename (heroic:// stays with Heroic).
command -v xdg-mime >/dev/null && xdg-mime default app.heronite.Heronite.desktop x-scheme-handler/heronite x-scheme-handler/heronite-rs >/dev/null 2>&1 || true
command -v update-desktop-database >/dev/null && update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
echo "Installed heronite-rs to $PREFIX"
