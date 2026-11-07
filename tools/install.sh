#!/bin/sh
# Installs Lattice for this user: the program, its icon, and entries in the
# application menu and on the desktop. Run it again to update.
# Settings, sign-ins and worlds live elsewhere and aren't touched.
set -eu

here=$(cd "$(dirname "$0")" && pwd)

# From a downloaded release everything sits beside this script. From the source, it's built first.
if [ -x "$here/lattice" ]; then
    built=$here/lattice
    icons=$here
else
    root=$(cd "$here/.." && pwd)
    if [ "${1:-}" != "--built" ]; then
        (cd "$root" && cargo build --release -p lattice)
    fi
    built=$root/target/release/lattice
    icons=$root/crates/lattice/assets/icon
fi

# Inside a sandboxed editor's terminal the folders below would be the sandbox's
# own, and the real menu would never see them.
if [ -f /.flatpak-info ]; then
    exec flatpak-spawn --host sh "$here/install.sh" --built
fi

data=${XDG_DATA_HOME:-$HOME/.local/share}
program=$HOME/.local/bin/lattice

# Replaced in one step, so it works while Lattice is open.
mkdir -p "$(dirname "$program")"
cp "$built" "$program.new"
chmod 755 "$program.new"
mv -f "$program.new" "$program"

mkdir -p "$data/icons/hicolor/scalable/apps" "$data/icons/hicolor/256x256/apps" "$data/applications"
cp "$icons/lattice.svg" "$data/icons/hicolor/scalable/apps/lattice.svg"
cp "$icons/lattice-256.png" "$data/icons/hicolor/256x256/apps/lattice.png"
touch "$data/icons/hicolor"

entry=$data/applications/lattice.desktop
cat > "$entry" <<DESKTOP
[Desktop Entry]
Type=Application
Version=1.5
Name=Lattice
GenericName=Minecraft client
Comment=Play on Minecraft: Java Edition servers and your own worlds
Exec="$program"
TryExec=$program
Icon=lattice
Terminal=false
Categories=Game;
Keywords=minecraft;java;launcher;game;
StartupNotify=true
StartupWMClass=lattice
SingleMainWindow=true
DESKTOP

desktop=$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")
if [ -d "$desktop" ] && [ "$desktop" != "$HOME" ]; then
    cp "$entry" "$desktop/Lattice.desktop"
    chmod 755 "$desktop/Lattice.desktop"
fi

command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true

echo "Lattice is installed. Open it from your application menu, under Games, or from your desktop."
