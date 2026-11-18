//! Pixel icon set: 7×7 bitmaps drawn with painter rects, so the app needs no
//! image assets and no emoji glyphs (which many systems cannot render). Every
//! icon carries a meaning next to a label or a tooltip (docs/design.md).

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect};

/// Width and height of every bitmap in cells.
pub const GRID: usize = 7;

/// All icons of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Plus,
    Search,
    Artifacts,
    Folder,
    File,
    Gear,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    Close,
    Dots,
    Send,
    Stop,
    Branch,
    Fork,
    Rewind,
    Copy,
    Check,
    Trash,
    Pencil,
    PanelLeft,
    PanelRight,
    Terminal,
    User,
    Spark,
    Globe,
    Shield,
    Eye,
    List,
    Refresh,
    ArrowDown,
    Chat,
    Pin,
    Clock,
}

impl Icon {
    #[cfg(test)]
    pub const ALL: [Icon; 34] = [
        Icon::Plus,
        Icon::Search,
        Icon::Artifacts,
        Icon::Folder,
        Icon::File,
        Icon::Gear,
        Icon::ChevronRight,
        Icon::ChevronDown,
        Icon::ChevronUp,
        Icon::Close,
        Icon::Dots,
        Icon::Send,
        Icon::Stop,
        Icon::Branch,
        Icon::Fork,
        Icon::Rewind,
        Icon::Copy,
        Icon::Check,
        Icon::Trash,
        Icon::Pencil,
        Icon::PanelLeft,
        Icon::PanelRight,
        Icon::Terminal,
        Icon::User,
        Icon::Spark,
        Icon::Globe,
        Icon::Shield,
        Icon::Eye,
        Icon::List,
        Icon::Refresh,
        Icon::ArrowDown,
        Icon::Chat,
        Icon::Pin,
        Icon::Clock,
    ];

    /// The bitmap: `#` = filled, `.` = empty.
    pub fn bitmap(self) -> [&'static str; GRID] {
        match self {
            Icon::Plus => [
                "...#...", "...#...", "...#...", "#######", "...#...", "...#...", "...#...",
            ],
            Icon::Search => [
                ".###...", "#...#..", "#...#..", "#...#..", ".###...", "....##.", ".....##",
            ],
            Icon::Artifacts => [
                "..#####", "..#...#", "#####.#", "#...#.#", "#...###", "#...#..", "#####..",
            ],
            Icon::Folder => [
                ".......", "###....", "#..####", "#.....#", "#.....#", "#.....#", "#######",
            ],
            Icon::File => [
                "####...", "#..##..", "#...##.", "#....#.", "#....#.", "#....#.", "######.",
            ],
            Icon::Gear => [
                "...#...", ".#####.", ".#...#.", "##.#.##", ".#...#.", ".#####.", "...#...",
            ],
            Icon::ChevronRight => [
                ".#.....", "..#....", "...#...", "....#..", "...#...", "..#....", ".#.....",
            ],
            Icon::ChevronDown => [
                ".......", ".......", "#.....#", ".#...#.", "..#.#..", "...#...", ".......",
            ],
            Icon::ChevronUp => [
                ".......", "...#...", "..#.#..", ".#...#.", "#.....#", ".......", ".......",
            ],
            Icon::Close => [
                "#.....#", ".#...#.", "..#.#..", "...#...", "..#.#..", ".#...#.", "#.....#",
            ],
            Icon::Dots => [
                ".......", ".......", ".......", "#..#..#", ".......", ".......", ".......",
            ],
            Icon::Send => [
                "...#...", "..###..", ".#####.", "##.#.##", "...#...", "...#...", "...#...",
            ],
            Icon::Stop => [
                ".......", ".#####.", ".#####.", ".#####.", ".#####.", ".#####.", ".......",
            ],
            Icon::Branch => [
                ".#...#.", ".#...#.", ".#...#.", ".#..#..", ".###...", ".#.....", ".#.....",
            ],
            Icon::Fork => [
                "#.....#", "#.....#", ".#...#.", "..###..", "...#...", "...#...", "...#...",
            ],
            Icon::Rewind => [
                "...#...", "..##...", ".#####.", "..##..#", "...#..#", "......#", "..####.",
            ],
            Icon::Copy => [
                "..#####", "..#...#", "###...#", "#.#...#", "#.#####", "#...#..", "#####..",
            ],
            Icon::Clock => [
                "..###..", ".#...#.", ".#..#..", ".#.#...", ".#...#.", "..###..", ".......",
            ],
            Icon::Pin => [
                "..###..", ".#####.", ".#####.", "...#...", "...#...", "...#...", "...#...",
            ],
            Icon::Check => [
                ".......", "......#", ".....#.", "#...#..", ".#.#...", "..#....", ".......",
            ],
            Icon::Trash => [
                "..###..", "#######", ".#...#.", ".#.#.#.", ".#.#.#.", ".#...#.", ".#####.",
            ],
            Icon::Pencil => [
                ".....##", "....#.#", "...#.#.", "..#.#..", ".#.#...", "#.#....", "##.....",
            ],
            Icon::PanelLeft => [
                "#######", "#.#...#", "#.#...#", "#.#...#", "#.#...#", "#.#...#", "#######",
            ],
            Icon::PanelRight => [
                "#######", "#...#.#", "#...#.#", "#...#.#", "#...#.#", "#...#.#", "#######",
            ],
            Icon::Terminal => [
                "#######", "#.....#", "#.#...#", "#..#..#", "#.#.###", "#.....#", "#######",
            ],
            Icon::User => [
                "..###..", "..###..", "..###..", ".......", ".#####.", "#######", "#######",
            ],
            Icon::Spark => [
                "...#...", "...#...", ".#.#.#.", "..###..", ".#.#.#.", "...#...", "...#...",
            ],
            Icon::Globe => [
                ".#####.", "#.#.#.#", "#.#.#.#", "#######", "#.#.#.#", "#.#.#.#", ".#####.",
            ],
            Icon::Shield => [
                "#######", "#..#..#", "#..#..#", "#.###.#", "#..#..#", ".#...#.", "..###..",
            ],
            Icon::Eye => [
                ".......", "..###..", ".#...#.", "#..#..#", ".#...#.", "..###..", ".......",
            ],
            Icon::List => [
                ".......", "#.#####", ".......", "#.#####", ".......", "#.#####", ".......",
            ],
            Icon::Refresh => [
                "...#...", "...##..", ".#####.", "#..##..", "#..#...", "#......", ".####..",
            ],
            Icon::ArrowDown => [
                "...#...", "...#...", "...#...", "#..#..#", ".#.#.#.", "..###..", "...#...",
            ],
            Icon::Chat => [
                "#######", "#.....#", "#.###.#", "#.....#", "#######", ".##....", ".#.....",
            ],
        }
    }
}

/// Paint `icon` with its top-left corner at `origin`, `pixel` points per cell.
pub fn paint(painter: &Painter, icon: Icon, origin: Pos2, pixel: f32, color: Color32) {
    let origin = pos2(origin.x.round(), origin.y.round());
    for (row, line) in icon.bitmap().iter().enumerate() {
        for (col, cell) in line.bytes().enumerate() {
            if cell == b'#' {
                painter.rect_filled(
                    Rect::from_min_size(
                        origin + vec2(col as f32 * pixel, row as f32 * pixel),
                        vec2(pixel, pixel),
                    ),
                    0.0,
                    color,
                );
            }
        }
    }
}

/// Paint `icon` centred in `rect`.
pub fn paint_centered(painter: &Painter, icon: Icon, rect: Rect, pixel: f32, color: Color32) {
    let size = GRID as f32 * pixel;
    paint(
        painter,
        icon,
        rect.center() - vec2(size, size) / 2.0,
        pixel,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bitmap_is_a_full_grid() {
        for icon in Icon::ALL {
            let bitmap = icon.bitmap();
            assert_eq!(bitmap.len(), GRID, "{icon:?}");
            for row in bitmap {
                assert_eq!(row.len(), GRID, "{icon:?}: row {row:?}");
                assert!(row.bytes().all(|c| c == b'#' || c == b'.'), "{icon:?}");
            }
            assert!(
                bitmap.iter().any(|r| r.contains('#')),
                "{icon:?} must not be blank"
            );
        }
    }

    #[test]
    fn bitmaps_are_distinct() {
        for (i, a) in Icon::ALL.iter().enumerate() {
            for b in &Icon::ALL[i + 1..] {
                assert_ne!(a.bitmap(), b.bitmap(), "{a:?} and {b:?} look the same");
            }
        }
    }
}
