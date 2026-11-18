#!/usr/bin/env python3
"""Render the Catwright app icons from the pixel logo in app/src/theme.rs.

The mark (`LOGO`) and its colours (`ORANGE`, `YELLOW`, `RED`, `SIDEBAR`,
`BORDER_SOFT`) are read from theme.rs, so the icons cannot drift from the
mark drawn in the sidebar. The mark sits on a dark rounded tile, drawn with
whole-pixel cells (no smoothing), so it stays crisp at 16 px.

Writes app/assets/icon.png (window icon, 128 px) and the freedesktop set in
dist/icons/hicolor/<size>/apps/catwright.png. Needs Pillow.

Usage: python3 tools/icons/render.py
"""
import re
import sys
from pathlib import Path

from PIL import Image, ImageDraw

REPO = Path(__file__).resolve().parents[2]
THEME = REPO / "app" / "src" / "theme.rs"
SIZES = (16, 22, 32, 48, 64, 128)
# LOGO letters -> theme.rs colour constants (see theme::logo_color)
LETTERS = {"o": "ORANGE", "y": "YELLOW", "r": "RED"}


def theme_color(src: str, name: str) -> tuple:
    m = re.search(
        rf"pub const {name}: Color32 = Color32::from_rgb\((0x\w+), (0x\w+), (0x\w+)\);", src
    )
    if not m:
        sys.exit(f"{name} not found in {THEME}")
    return tuple(int(v, 16) for v in m.groups())


def theme_logo(src: str) -> list:
    m = re.search(r"pub const LOGO: \[&str; \d+\] = \[(.*?)\];", src, re.S)
    if not m:
        sys.exit(f"LOGO not found in {THEME}")
    rows = re.findall(r'"([^"]*)"', m.group(1))
    if not rows or any(len(r) != len(rows[0]) for r in rows):
        sys.exit("LOGO rows must all have the same width")
    return rows


def render(size: int, logo: list, colors: dict, tile: tuple, edge: tuple) -> Image.Image:
    # the tile with rounded corners, supersampled for smooth edges
    ss = 4
    big = Image.new("RGBA", (size * ss, size * ss), (0, 0, 0, 0))
    ImageDraw.Draw(big).rounded_rectangle(
        [0, 0, size * ss - 1, size * ss - 1],
        radius=round(size * 0.2) * ss,
        fill=tile + (255,),
        outline=edge + (255,),
        width=max(1, size // 32) * ss,
    )
    img = big.resize((size, size), Image.LANCZOS)
    # the mark in whole pixels: larger share of the tile at small sizes
    cols, rows = len(logo[0]), len(logo)
    share = 0.86 if size <= 32 else 0.76
    cell = max(1, int(size * share) // cols)
    ox, oy = (size - cols * cell) // 2, (size - rows * cell) // 2
    draw = ImageDraw.Draw(img)
    for r, line in enumerate(logo):
        for c, ch in enumerate(line):
            if ch in colors:
                x, y = ox + c * cell, oy + r * cell
                draw.rectangle([x, y, x + cell - 1, y + cell - 1], fill=colors[ch] + (255,))
    return img


def main() -> None:
    src = THEME.read_text(encoding="utf-8")
    logo = theme_logo(src)
    unknown = {ch for row in logo for ch in row} - set(LETTERS) - {"."}
    if unknown:
        sys.exit(f"LOGO uses letters without a colour here: {sorted(unknown)}")
    colors = {k: theme_color(src, v) for k, v in LETTERS.items()}
    tile, edge = theme_color(src, "SIDEBAR"), theme_color(src, "BORDER_SOFT")
    for size in SIZES:
        out = REPO / "dist" / "icons" / "hicolor" / f"{size}x{size}" / "apps" / "catwright.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        render(size, logo, colors, tile, edge).save(out, optimize=True)
    render(128, logo, colors, tile, edge).save(REPO / "app" / "assets" / "icon.png", optimize=True)
    print(f"icons written for sizes {', '.join(map(str, SIZES))}")


if __name__ == "__main__":
    main()
