#!/bin/sh
# Install catwright for the current user (no root):
#   binary     -> ~/.local/bin/catwright
#   menu entry -> ~/.local/share/applications/catwright.desktop
#   icon       -> ~/.local/share/icons/hicolor (16-128 px, freedesktop set)/catwright.png
# Usage: dist/install.sh [path-to-release-binary]
# Without an argument it takes the binary next to dist/ (release archive),
# else the one cargo built (source checkout).
set -e
dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ -n "$1" ]; then
    bin="$1"
elif [ -f "$dir/../catwright" ]; then
    bin="$dir/../catwright"
else
    bin="$dir/../app/target/release/catwright"
fi
if [ ! -f "$bin" ]; then
    echo "binary not found: $bin  (build it first: cd app && cargo build --release)" >&2
    exit 1
fi
install -D -m 0755 "$bin" "$HOME/.local/bin/catwright"
install -D -m 0644 "$dir/catwright.desktop" \
    "$HOME/.local/share/applications/catwright.desktop"
# icon -> the full freedesktop hicolor set: titlebars pick 16-32 px
# (cp -R of "hicolor/." lands INSIDE hicolor: the size dirs belong under it)
mkdir -p "$HOME/.local/share/icons/hicolor"
cp -R "$dir/icons/hicolor/." "$HOME/.local/share/icons/hicolor/"
echo "installed:"
echo "  ~/.local/bin/catwright"
echo "  ~/.local/share/applications/catwright.desktop"
echo "  ~/.local/share/icons/hicolor/<size>/apps/catwright.png"
