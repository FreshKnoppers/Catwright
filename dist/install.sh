#!/bin/sh
# Install catwright for the current user (no root):
#   binary     -> ~/.local/bin/catwright
#   menu entry -> ~/.local/share/applications/catwright.desktop
#   icon       -> ~/.local/share/icons/hicolor (16-128 px, freedesktop set)/catwright.png
# Usage: dist/install.sh [path-to-release-binary]
# Without an argument it takes the binary next to dist/ (release archive),
# else the one cargo built (source checkout).
set -e
: "${HOME:?HOME is not set}"
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
# icon -> the full freedesktop hicolor set: titlebars pick 16-32 px.
# One `install` per regular file, not `cp -R`: install replaces whatever
# sits at the target (cp would write THROUGH a symlink planted there, e.g.
# ~/.local/share/icons/.../catwright.png -> ~/.bashrc), and symlinks in
# the archive itself are skipped (find does not follow or list them).
(cd "$dir/icons/hicolor" && find . -type f) | while IFS= read -r f; do
    install -D -m 0644 "$dir/icons/hicolor/$f" "$HOME/.local/share/icons/hicolor/$f"
done
# overwriting an icon leaves its folder's mtime alone, so an existing
# icon-theme.cache still counts as fresh: refresh it (when the tool is
# there) and the desktop's app database, so a new icon shows up
if command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    [ -f "$HOME/.local/share/icons/hicolor/index.theme" ]; then
    gtk-update-icon-cache -f -q "$HOME/.local/share/icons/hicolor" || true
fi
if command -v kbuildsycoca6 >/dev/null 2>&1; then
    kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
fi
echo "installed:"
echo "  ~/.local/bin/catwright"
echo "  ~/.local/share/applications/catwright.desktop"
echo "  ~/.local/share/icons/hicolor/<size>/apps/catwright.png"
