#!/bin/sh
# Removes the program, icon and menu entries. Settings, sign-ins and worlds stay.
set -eu

if [ -f /.flatpak-info ]; then
    exec flatpak-spawn --host sh "$(cd "$(dirname "$0")" && pwd)/uninstall.sh"
fi

data=${XDG_DATA_HOME:-$HOME/.local/share}
desktop=$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")

rm -f "$HOME/.local/bin/lattice" \
      "$data/applications/lattice.desktop" \
      "$data/icons/hicolor/scalable/apps/lattice.svg" \
      "$data/icons/hicolor/256x256/apps/lattice.png" \
      "$desktop/Lattice.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true

echo "Lattice is removed. Your settings, sign-ins and worlds are still there."
