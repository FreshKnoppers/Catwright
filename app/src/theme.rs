//! Visual theme for the pixel style from docs/design.md: colour tokens plus
//! reusable pixel widgets drawn with `egui::Painter` rects (no image assets).

use egui::{Color32, RichText, Stroke};

// Colour tokens (docs/design.md).
pub const BG: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1e);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x17, 0x17, 0x1b);
pub const CARD: Color32 = Color32::from_rgb(0x10, 0x10, 0x10);
pub const RAISED: Color32 = Color32::from_rgb(0x29, 0x29, 0x2d);
pub const INPUT: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2e);
pub const BORDER: Color32 = Color32::from_rgb(0x40, 0x40, 0x40);
/// Quiet 1 px line between panels and around cards.
pub const BORDER_SOFT: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x34);
/// Row background under the pointer.
pub const HOVER: Color32 = Color32::from_rgb(0x23, 0x23, 0x28);
pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xec, 0xec);
/// Secondary text that must stay comfortably readable.
pub const MUTED: Color32 = Color32::from_rgb(0xb4, 0xb4, 0xba);
pub const DIM: Color32 = Color32::from_rgb(0x8a, 0x8a, 0x8f);
pub const ORANGE: Color32 = Color32::from_rgb(0xfa, 0x50, 0x0f);
pub const ORANGE_DIM: Color32 = Color32::from_rgb(0xce, 0x61, 0x14);
pub const YELLOW: Color32 = Color32::from_rgb(0xff, 0xd8, 0x00);
pub const AMBER: Color32 = Color32::from_rgb(0xff, 0xaf, 0x00);
pub const RED: Color32 = Color32::from_rgb(0xe1, 0x05, 0x00);
/// RED lightened for text on dark backgrounds (contrast).
pub const RED_TEXT: Color32 = Color32::from_rgb(0xff, 0x6e, 0x62);
pub const GREEN: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);
/// Line backgrounds of the diff view.
pub const DIFF_ADD_BG: Color32 = Color32::from_rgb(0x12, 0x28, 0x1a);
pub const DIFF_DEL_BG: Color32 = Color32::from_rgb(0x30, 0x15, 0x14);

/// Cool accents for states the warm gradient has no place for.
pub const BLUE: Color32 = Color32::from_rgb(0x5b, 0x9c, 0xf8);
pub const PURPLE: Color32 = Color32::from_rgb(0xb0, 0x7c, 0xf8);

/// Pixel gradient, left to right (red to yellow).
pub const GRADIENT: [Color32; 5] = [
    Color32::from_rgb(0xe1, 0x05, 0x00),
    Color32::from_rgb(0xfa, 0x50, 0x0f),
    Color32::from_rgb(0xff, 0x82, 0x05),
    Color32::from_rgb(0xff, 0xaf, 0x00),
    Color32::from_rgb(0xff, 0xd8, 0x00),
];

/// Palette for project icons: the 5 gradient colours plus 4 cool colours.
pub const PROJECT_PALETTE: [Color32; 9] = [
    GRADIENT[0],
    GRADIENT[1],
    GRADIENT[2],
    GRADIENT[3],
    GRADIENT[4],
    Color32::from_rgb(0x3b, 0x82, 0xf6),
    Color32::from_rgb(0x22, 0xc5, 0x5e),
    Color32::from_rgb(0xa8, 0x55, 0xf7),
    Color32::from_rgb(0x14, 0xb8, 0xa6),
];

/// Series colours of charts, in order: the project palette re-ordered so
/// that neighbouring series never share a hue family. Neighbouring
/// gradient steps are hard to tell apart (OKLab ΔE ≈ 10); in this order
/// every adjacent pair stays above 15, for normal vision and simulated
/// protan/deutan vision alike. Red leads, amber follows, so two series
/// still read as the pixel style.
pub const CHART_SERIES: [Color32; 9] = [
    PROJECT_PALETTE[0], // red
    PROJECT_PALETTE[3], // amber
    PROJECT_PALETTE[5], // blue
    PROJECT_PALETTE[6], // green
    PROJECT_PALETTE[7], // purple
    PROJECT_PALETTE[1], // orange-red
    PROJECT_PALETTE[4], // yellow
    PROJECT_PALETTE[8], // teal
    PROJECT_PALETTE[2], // orange
];

/// Our own pixel mark: a little pixel window with two yellow eyes on a red
/// sill. `o` = ORANGE, `y` = YELLOW, `r` = RED, `.` = empty. The app icons
/// (`assets/icon.png`, `dist/icons/`) are rendered from this bitmap by
/// `tools/icons/render.py`.
pub const LOGO: [&str; 5] = [
    "ooooooooo",
    "o.......o",
    "o.y...y.o",
    "o.......o",
    "rrrrrrrrr",
];

/// Colour of one `LOGO` cell; `None` for an empty one.
pub fn logo_color(cell: char) -> Option<Color32> {
    match cell {
        'o' => Some(ORANGE),
        'y' => Some(YELLOW),
        'r' => Some(RED),
        _ => None,
    }
}

/// Install the dark pixel theme on the context: colours, the type scale
/// (egui's default `Small` of 9 pt is unreadable) and the spacing.
pub fn apply(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    // text fields use extreme_bg_color: INPUT keeps them visible on CARD
    // backgrounds (settings modal, dialogs)
    visuals.extreme_bg_color = INPUT;
    visuals.faint_bg_color = SIDEBAR;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_rounding = egui::Rounding::same(4.0);
    visuals.menu_rounding = egui::Rounding::same(4.0);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 8.0),
        blur: 24.0,
        spread: 0.0,
        color: Color32::from_black_alpha(140),
    };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.selection.bg_fill = ORANGE;
    visuals.selection.stroke = Stroke::new(1.0_f32, TEXT);
    visuals.hyperlink_color = AMBER;
    visuals.text_cursor.stroke = Stroke::new(2.0_f32, ORANGE);
    // Body text colour via the widget fg strokes, not override_text_color:
    // the override would also brighten hint texts (placeholders).
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_SOFT);
    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.weak_bg_fill = RAISED;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x34, 0x34, 0x3a);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x34, 0x34, 0x3a);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0x3c, 0x3c, 0x42);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0x3c, 0x3c, 0x42);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.rounding = egui::Rounding::same(3.0);
    }
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(13.5, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(19.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.5, FontFamily::Monospace),
            ),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(6.0, 5.0);
        style.spacing.button_padding = egui::vec2(9.0, 4.0);
        style.spacing.interact_size = egui::vec2(36.0, 22.0);
        style.spacing.scroll = egui::style::ScrollStyle {
            bar_width: 6.0,
            floating: true,
            floating_width: 2.0,
            floating_allocated_width: 0.0,
            handle_min_length: 24.0,
            bar_inner_margin: 2.0,
            bar_outer_margin: 2.0,
            foreground_color: false,
            dormant_background_opacity: 0.0,
            active_background_opacity: 0.0,
            interact_background_opacity: 0.1,
            dormant_handle_opacity: 0.25,
            active_handle_opacity: 0.5,
            interact_handle_opacity: 0.8,
        };
        style.interaction.tooltip_delay = 0.3;
        style.animation_time = 0.1;
    });
}

/// Monospace UPPERCASE small label in DIM (section headers, meter labels, …).
pub fn mono_label(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .monospace()
        .size(11.5)
        .color(DIM)
}

/// One-line section header: a small `■` in ORANGE when active, plus the label.
pub fn section_header(ui: &mut egui::Ui, text: &str, active: bool) -> egui::Response {
    ui.horizontal(|ui| {
        if active {
            ui.label(RichText::new("■").color(ORANGE).monospace().size(11.5));
        }
        ui.label(mono_label(text));
    })
    .response
}

/// Monospace UPPERCASE label in an explicit colour (badges, states).
pub fn mono_tag(text: &str, color: Color32) -> RichText {
    RichText::new(text.to_uppercase())
        .monospace()
        .size(11.0)
        .color(color)
}

/// Number of filled blocks for a fraction: `round(fraction * total)` on
/// 0..=1 (NaN counts as 0), but any fraction > 0 yields at least one block.
pub fn meter_blocks(fraction: f32, total: usize) -> usize {
    let f = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    };
    let n = (f * total as f32).round() as usize;
    if f > 0.0 {
        n.max(1)
    } else {
        n
    }
}

/// Row of square pixel blocks; `filled` of `total` blocks in ORANGE_DIM.
pub fn pixel_meter(ui: &mut egui::Ui, filled: usize, total: usize, block: f32) -> egui::Response {
    let gap = block * 0.3;
    let width = total as f32 * block + total.saturating_sub(1) as f32 * gap;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, block), egui::Sense::hover());
    let painter = ui.painter();
    for i in 0..total {
        let pos = egui::pos2(rect.left() + i as f32 * (block + gap), rect.top());
        let color = if i < filled { ORANGE_DIM } else { RAISED };
        painter.rect_filled(
            egui::Rect::from_min_size(pos, egui::vec2(block, block)),
            egui::Rounding::same(0.0),
            color,
        );
    }
    response
}

/// The pixel logo in its own colours (see `LOGO`).
pub fn pixel_logo(ui: &mut egui::Ui, pixel: f32) -> egui::Response {
    let cols = LOGO[0].chars().count() as f32;
    let rows = LOGO.len() as f32;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(cols * pixel, rows * pixel), egui::Sense::hover());
    let painter = ui.painter();
    for (row, line) in LOGO.iter().enumerate() {
        for (col, c) in line.chars().enumerate() {
            if let Some(color) = logo_color(c) {
                let pos = egui::pos2(
                    rect.left() + col as f32 * pixel,
                    rect.top() + row as f32 * pixel,
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(pos, egui::vec2(pixel, pixel)),
                    egui::Rounding::same(0.0),
                    color,
                );
            }
        }
    }
    response
}

/// Colour of a session mode, from calm to risky: Plan asks before every
/// action, Accept Edits writes files without asking, Smart Approve decides
/// for itself, Auto Approve asks nothing. Custom Vibe profiles share one
/// colour.
pub fn mode_color(id: &str) -> Color32 {
    match id {
        "ask" => BLUE,
        "accept-edits" => GREEN,
        "smart-approve" => AMBER,
        "auto-approve" => RED_TEXT,
        _ => PURPLE,
    }
}

/// Colour for a tool-call status square.
pub fn status_color(status: &crate::acp::protocol::ToolStatus) -> Color32 {
    match status {
        crate::acp::protocol::ToolStatus::Pending => DIM,
        crate::acp::protocol::ToolStatus::InProgress => YELLOW,
        crate::acp::protocol::ToolStatus::Completed => GREEN,
        crate::acp::protocol::ToolStatus::Failed => RED,
        crate::acp::protocol::ToolStatus::Other => DIM,
    }
}

/// Deterministic 3×3 colour raster derived from the project key (FNV-1a-64).
pub fn project_icon_colors(key: &str) -> [Color32; 9] {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut out = [PROJECT_PALETTE[0]; 9];
    for (i, cell) in out.iter_mut().enumerate() {
        let index = ((hash >> (i * 7)) % 9) as usize;
        *cell = PROJECT_PALETTE[index];
    }
    out
}

// ---------------------------------------------------------------------------
// pixel cat (our own mascot, docs/design.md backlog)
// ---------------------------------------------------------------------------

/// Our own 12×8 pixel cat, facing right. `#` = fur (ORANGE), `o` = eye
/// (CARD), `.` = empty. Two walking frames: all four legs stride (spread,
/// then closed under the body) and the tail tip sways.
pub const CAT_WALK: [[&str; 8]; 2] = [
    [
        "#........#.#",
        "#........###",
        "#........#o#",
        ".#.......###",
        "..########..",
        "..#########.",
        "..#.#...#.#.",
        ".#...#.#...#",
    ],
    [
        ".#.......#.#",
        "#........###",
        "#........#o#",
        ".#.......###",
        "..########..",
        "..#########.",
        "...##...##..",
        "...##...##..",
    ],
];

/// Sitting cat, eyes open: full body, tail curled round the front.
pub const CAT_SIT: [&str; 8] = [
    "........#.#.",
    "........###.",
    "........#o#.",
    ".......####.",
    "......#####.",
    "#....######.",
    "#...#######.",
    ".##########.",
];

/// Sitting cat with closed eyes (same bitmap, eye cells are fur).
pub const CAT_BLINK: [&str; 8] = [
    "........#.#.",
    "........###.",
    "........###.",
    ".......####.",
    "......#####.",
    "#....######.",
    "#...#######.",
    ".##########.",
];

/// Which bitmap the cat shows right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatPose {
    /// Walking frame index (0 or 1).
    Walk(usize),
    Sit,
    Blink,
}

/// Walking frame changes every 0.2 s; while idle the cat sits and blinks for
/// 0.15 s every 4 s. Pure function of `time`.
pub fn cat_frame(time: f64, running: bool) -> CatPose {
    if running {
        // epsilon: 0.2 / 0.2 must land on step 1, not 0.9999…
        let step = ((time + 1e-9) / 0.2).floor() as i64;
        return CatPose::Walk(step.rem_euclid(2) as usize);
    }
    if time.rem_euclid(4.0) < 0.15 {
        CatPose::Blink
    } else {
        CatPose::Sit
    }
}

/// Bitmap rows for a pose.
fn cat_bitmap(pose: CatPose) -> &'static [&'static str; 8] {
    match pose {
        CatPose::Walk(0) => &CAT_WALK[0],
        CatPose::Walk(_) => &CAT_WALK[1],
        CatPose::Sit => &CAT_SIT,
        CatPose::Blink => &CAT_BLINK,
    }
}

/// Draw a cat bitmap with `pixel`-sized painter rects.
/// Paint the cat bitmap for `pose` with its top-left corner at `origin`.
fn paint_cat(painter: &egui::Painter, origin: egui::Pos2, pose: CatPose, pixel: f32) {
    for (row, line) in cat_bitmap(pose).iter().enumerate() {
        for (col, c) in line.chars().enumerate() {
            let color = match c {
                '#' => ORANGE,
                'o' => CARD,
                _ => continue,
            };
            let pos = egui::pos2(origin.x + col as f32 * pixel, origin.y + row as f32 * pixel);
            painter.rect_filled(
                egui::Rect::from_min_size(pos, egui::vec2(pixel, pixel)),
                egui::Rounding::same(0.0),
                color,
            );
        }
    }
}

/// The cat as a widget: walking while `running`, otherwise sitting and
/// blinking. Requests repaints only while it moves.
pub fn pixel_cat(ui: &mut egui::Ui, time: f64, running: bool, pixel: f32) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(12.0 * pixel, 8.0 * pixel), egui::Sense::hover());
    paint_cat(ui.painter(), rect.min, cat_frame(time, running), pixel);
    // repaint only when the frame changes: walking every 100 ms, idle at
    // the next blink transition
    let wait = if running {
        0.1
    } else {
        let phase = time.rem_euclid(4.0);
        if phase < 0.15 {
            0.15 - phase
        } else {
            4.0 - phase
        }
    };
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.01)));
    response
}

/// Status bar of a subagent card: a 4 px track of pixel blocks (RAISED) with
/// the cat (2 px pixels) on top. Running: the cat walks from left to right
/// and starts over at the left, the track behind it is ORANGE_DIM. Done: the
/// whole track is GREEN and the cat sits at the right end. Failed: RED track,
/// cat sits at the left. Cancelled: DIM track, cat sits at the left.
pub fn subagent_bar(
    ui: &mut egui::Ui,
    state: &crate::acp::session::SubagentState,
    time: f64,
    width: f32,
) -> egui::Response {
    use crate::acp::session::SubagentState;

    const TRACK_H: f32 = 4.0;
    const BLOCK: f32 = 4.0;
    const GAP: f32 = 2.0;
    const PIXEL: f32 = 2.0;
    const CAT_W: f32 = 12.0 * PIXEL;
    const CAT_H: f32 = 8.0 * PIXEL;

    let width = width.max(CAT_W);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, CAT_H), egui::Sense::hover());
    let track = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - TRACK_H / 2.0),
        egui::vec2(width, TRACK_H),
    );
    let painter = ui.painter();

    let running = *state == SubagentState::Running;
    let cat_x = if running {
        // walk inside the track (the cat is CAT_W wide), then start over
        ((time * 40.0) % (width - CAT_W).max(1.0) as f64) as f32
    } else if *state == SubagentState::Done {
        width - CAT_W
    } else {
        0.0
    };

    // track: pixel blocks, filled up to the cat while running
    let mut x = track.left();
    while x < track.right() {
        let w = BLOCK.min(track.right() - x);
        let color = match state {
            SubagentState::Running => {
                if x + w <= track.left() + cat_x {
                    ORANGE_DIM
                } else {
                    RAISED
                }
            }
            SubagentState::Done => GREEN,
            SubagentState::Failed => RED,
            SubagentState::Cancelled => DIM,
        };
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, track.top()), egui::vec2(w, TRACK_H)),
            egui::Rounding::same(0.0),
            color,
        );
        x += BLOCK + GAP;
    }

    // the cat, drawn on top of the track
    paint_cat(
        painter,
        egui::pos2(rect.left() + cat_x, rect.top()),
        cat_frame(time, running),
        PIXEL,
    );

    if running {
        ui.ctx().request_repaint();
    }
    response
}

/// Pixel meter split into coloured segments: `segments` are (colour,
/// fraction of the whole) in order; blocks beyond them are RAISED. A segment
/// with a non-zero share gets at least one block while blocks are left.
pub fn stacked_meter(
    ui: &mut egui::Ui,
    segments: &[(Color32, f32)],
    total: usize,
    block: f32,
) -> egui::Response {
    let colors = stacked_blocks(segments, total);
    let gap = block * 0.3;
    let width = total as f32 * block + total.saturating_sub(1) as f32 * gap;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, block), egui::Sense::hover());
    let painter = ui.painter();
    for (i, c) in colors.iter().enumerate() {
        let pos = egui::pos2(rect.left() + i as f32 * (block + gap), rect.top());
        painter.rect_filled(
            egui::Rect::from_min_size(pos, egui::vec2(block, block)),
            egui::Rounding::same(0.0),
            c.unwrap_or(RAISED),
        );
    }
    response
}

/// Block colours of a stacked meter (None = empty block). Pure, for tests.
pub fn stacked_blocks(segments: &[(Color32, f32)], total: usize) -> Vec<Option<Color32>> {
    let mut out = Vec::with_capacity(total);
    let mut cumulative = 0.0_f32;
    for (color, frac) in segments {
        let frac = if frac.is_nan() {
            0.0
        } else {
            frac.clamp(0.0, 1.0)
        };
        if frac <= 0.0 {
            continue;
        }
        cumulative = (cumulative + frac).min(1.0);
        let until = ((cumulative * total as f32).round() as usize).max(out.len() + 1);
        while out.len() < until.min(total) {
            out.push(Some(*color));
        }
    }
    while out.len() < total {
        out.push(None);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_blocks_fill_in_order_with_minimum_one() {
        let a = Color32::RED;
        let b = Color32::GREEN;
        let blocks = stacked_blocks(&[(a, 0.25), (b, 0.01)], 20);
        assert_eq!(blocks.len(), 20);
        assert_eq!(blocks.iter().filter(|c| **c == Some(a)).count(), 5);
        assert_eq!(
            blocks.iter().filter(|c| **c == Some(b)).count(),
            1,
            "tiny share still shows"
        );
        assert!(blocks[6..].iter().all(|c| c.is_none()));
        assert!(stacked_blocks(&[], 4).iter().all(|c| c.is_none()));
        assert_eq!(stacked_blocks(&[(a, 2.0)], 3), vec![Some(a); 3]);
    }
    use crate::acp::protocol::ToolStatus;

    #[test]
    fn meter_blocks_cases() {
        assert_eq!(meter_blocks(0.0, 20), 0);
        assert_eq!(meter_blocks(1.0, 20), 20);
        assert_eq!(meter_blocks(0.001, 20), 1);
        assert_eq!(meter_blocks(0.5, 20), 10);
        assert_eq!(meter_blocks(1.7, 20), 20); // clamped to 1
        assert_eq!(meter_blocks(f32::NAN, 20), 0);
    }

    #[test]
    fn logo_bitmap_shape() {
        assert_eq!(LOGO.len(), 5);
        for row in LOGO {
            assert_eq!(row.chars().count(), LOGO[0].chars().count());
            // every cell is empty or has a colour: no unknown letters
            assert!(row.chars().all(|c| c == '.' || logo_color(c).is_some()));
        }
        // all three colours appear, and the mark is closed on both sides
        for c in ['o', 'y', 'r'] {
            assert!(LOGO.iter().any(|row| row.contains(c)), "{c} is used");
        }
        assert!(LOGO
            .iter()
            .all(|row| !row.starts_with('.') && !row.ends_with('.')));
        assert_eq!(logo_color('.'), None);
    }

    #[test]
    fn project_icon_colors_deterministic_and_in_palette() {
        let a = project_icon_colors("/home/a/proj1");
        assert_eq!(a, project_icon_colors("/home/a/proj1"));
        assert_ne!(a, project_icon_colors("/home/a/proj2"));
        for color in a {
            assert!(PROJECT_PALETTE.contains(&color));
        }
    }

    #[test]
    fn every_mode_has_its_own_colour() {
        let ids = [
            "ask",
            "accept-edits",
            "smart-approve",
            "auto-approve",
            "eigenes-profil",
        ];
        let colors: Vec<Color32> = ids.iter().map(|id| mode_color(id)).collect();
        for (i, a) in colors.iter().enumerate() {
            assert!(!colors[i + 1..].contains(a), "{} shares a colour", ids[i]);
        }
        assert_eq!(mode_color("noch-ein-profil"), PURPLE);
    }

    #[test]
    fn status_color_variants() {
        assert_eq!(status_color(&ToolStatus::Pending), DIM);
        assert_eq!(status_color(&ToolStatus::InProgress), YELLOW);
        assert_eq!(status_color(&ToolStatus::Completed), GREEN);
        assert_eq!(status_color(&ToolStatus::Failed), RED);
        assert_eq!(status_color(&ToolStatus::Other), DIM);
    }

    #[test]
    fn apply_sets_tokens() {
        let ctx = egui::Context::default();
        apply(&ctx);
        assert_eq!(ctx.style().visuals.selection.bg_fill, ORANGE);
        assert_eq!(ctx.style().visuals.panel_fill, BG);
    }

    #[test]
    fn cat_bitmaps_are_12x8() {
        let frames: Vec<&[&str; 8]> = vec![&CAT_WALK[0], &CAT_WALK[1], &CAT_SIT, &CAT_BLINK];
        for frame in frames {
            assert_eq!(frame.len(), 8);
            for row in frame {
                assert_eq!(row.chars().count(), 12, "row {row:?} is not 12 wide");
                assert!(
                    row.chars().all(|c| c == '#' || c == 'o' || c == '.'),
                    "row {row:?} has an unknown cell"
                );
            }
        }
        // the blink frame is the sitting cat with the eye closed
        assert_eq!(CAT_SIT.len(), CAT_BLINK.len());
        assert!(CAT_SIT.iter().any(|r| r.contains('o')));
        assert!(!CAT_BLINK.iter().any(|r| r.contains('o')));
        // walking moves the hind legs (left half) AND the front legs
        let legs = |f: usize, cols: std::ops::Range<usize>| -> Vec<String> {
            CAT_WALK[f][6..]
                .iter()
                .map(|r| r[cols.clone()].to_string())
                .collect()
        };
        assert_ne!(legs(0, 0..6), legs(1, 0..6), "hind legs stand still");
        assert_ne!(legs(0, 6..12), legs(1, 6..12), "front legs stand still");
    }

    #[test]
    fn cat_frame_walks_and_blinks() {
        // running: two frames, switching every 0.2 s
        assert_eq!(cat_frame(0.0, true), CatPose::Walk(0));
        assert_eq!(cat_frame(0.19, true), CatPose::Walk(0));
        assert_eq!(cat_frame(0.2, true), CatPose::Walk(1));
        assert_eq!(cat_frame(0.4, true), CatPose::Walk(0));
        assert_eq!(cat_frame(0.6, true), CatPose::Walk(1));
        // idle: sitting, blinking for 0.15 s every 4 s
        assert_eq!(cat_frame(0.5, false), CatPose::Sit);
        assert_eq!(cat_frame(0.0, false), CatPose::Blink);
        assert_eq!(cat_frame(0.14, false), CatPose::Blink);
        assert_eq!(cat_frame(0.15, false), CatPose::Sit);
        assert_eq!(cat_frame(4.0, false), CatPose::Blink);
        assert_eq!(cat_frame(4.2, false), CatPose::Sit);
        assert_eq!(cat_frame(8.05, false), CatPose::Blink);
    }
}
