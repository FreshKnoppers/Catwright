//! Reusable UI components of the pixel design (docs/design.md): rows with
//! hover and selection states, icon buttons, chips, cards and menus.
//! Everything is drawn from `theme` tokens.

use crate::icons::{self, Icon};
use crate::theme;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    pos2, vec2, Align2, Color32, FontFamily, FontId, Galley, Rect, Response, RichText, Rounding,
    Sense, Stroke, TextFormat, Ui, Widget,
};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

/// One line of text, cut with an ellipsis at `max_width`.
pub fn line_galley(
    ui: &Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(
        text.replace('\n', " "),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts(|f| f.layout_job(job))
}

/// A proportional font id of `size`.
pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// A monospace font id of `size`.
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Paint a galley with its left edge at `x`, vertically centred on `center_y`.
pub fn paint_line(ui: &Ui, x: f32, center_y: f32, galley: Arc<Galley>) -> Rect {
    let rect = Rect::from_min_size(pos2(x, center_y - galley.size().y / 2.0), galley.size());
    ui.painter().galley(rect.min, galley, theme::TEXT);
    rect
}

// ---------------------------------------------------------------------------
// rows
// ---------------------------------------------------------------------------

/// Add a widget at `rect` without disturbing the layout around it, e.g. a
/// hover button on an already allocated row. `Ui::put` must not be used for
/// that: it moves the parent's cursor to the widget's bottom edge, and
/// everything after it jumps.
pub fn place(ui: &mut Ui, rect: Rect, widget: impl Widget) -> Response {
    ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
    ))
    .add(widget)
}

/// Background of a list row: selected (RAISED with an ORANGE edge), hovered,
/// or nothing.
pub fn paint_row_bg(ui: &Ui, rect: Rect, hovered: bool, selected: bool) {
    if selected {
        ui.painter().rect_filled(rect, 3.0, theme::RAISED);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(0.0, 5.0), vec2(2.0, rect.height() - 10.0)),
            0.0,
            theme::ORANGE,
        );
    } else if hovered {
        ui.painter().rect_filled(rect, 3.0, theme::HOVER);
    }
}

/// Icon, label and optional trailing text in a full-width row.
pub fn nav_row(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    trailing: Option<&str>,
    selected: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    paint_row_bg(ui, rect, resp.hovered(), selected);
    let color = if selected || resp.hovered() {
        theme::TEXT
    } else {
        theme::MUTED
    };
    icons::paint_centered(
        ui.painter(),
        icon,
        Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(14.0, 14.0)),
        2.0,
        color,
    );
    let mut right = rect.right() - 8.0;
    if let Some(t) = trailing {
        let g = line_galley(ui, t, mono(11.0), theme::DIM, 90.0);
        right -= g.size().x;
        paint_line(ui, right, rect.center().y, g);
        right -= 6.0;
    }
    let x = rect.left() + 30.0;
    let g = line_galley(ui, label, font(14.0), color, right - x);
    paint_line(ui, x, rect.center().y, g);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of a popup or context menu. Returns true on click.
pub fn menu_row(
    ui: &mut Ui,
    icon: Option<Icon>,
    label: &str,
    detail: Option<&str>,
    danger: bool,
) -> bool {
    let w = ui.available_width().max(170.0);
    let height = if detail.is_some() { 42.0 } else { 28.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, height), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::RAISED);
    }
    let color = if danger { theme::RED_TEXT } else { theme::TEXT };
    let mut x = rect.left() + 8.0;
    let top_line_y = if detail.is_some() {
        rect.top() + 14.0
    } else {
        rect.center().y
    };
    if let Some(icon) = icon {
        icons::paint(ui.painter(), icon, pos2(x, top_line_y - 7.0), 2.0, color);
        x += 22.0;
    }
    let g = line_galley(ui, label, font(13.5), color, rect.right() - x - 8.0);
    paint_line(ui, x, top_line_y, g);
    if let Some(detail) = detail {
        let g = line_galley(ui, detail, font(12.0), theme::DIM, rect.right() - x - 8.0);
        paint_line(ui, x, rect.top() + 30.0, g);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A row of a selection list: the check mark keeps its column even when it
/// is off, so the labels line up. Returns true on click.
pub fn menu_check_row(ui: &mut Ui, checked: bool, label: &str, detail: Option<&str>) -> bool {
    choice_row(ui, None, checked, label, detail)
}

/// A selection row led by a shield in the mode's colour; the check mark sits
/// at the right edge. Returns true on click.
pub fn menu_mode_row(
    ui: &mut Ui,
    color: Color32,
    checked: bool,
    label: &str,
    detail: Option<&str>,
) -> bool {
    choice_row(ui, Some(color), checked, label, detail)
}

fn choice_row(
    ui: &mut Ui,
    shield: Option<Color32>,
    checked: bool,
    label: &str,
    detail: Option<&str>,
) -> bool {
    let w = ui.available_width().max(170.0);
    let height = if detail.is_some() { 42.0 } else { 28.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, height), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::RAISED);
    }
    let top_line_y = if detail.is_some() {
        rect.top() + 14.0
    } else {
        rect.center().y
    };
    let lead = pos2(rect.left() + 8.0, top_line_y - 7.0);
    let mut right = rect.right() - 8.0;
    match shield {
        Some(color) => {
            icons::paint(ui.painter(), Icon::Shield, lead, 2.0, color);
            if checked {
                right -= 22.0;
                icons::paint(
                    ui.painter(),
                    Icon::Check,
                    pos2(right + 8.0, top_line_y - 7.0),
                    2.0,
                    theme::ORANGE,
                );
            }
        }
        None if checked => icons::paint(ui.painter(), Icon::Check, lead, 2.0, theme::ORANGE),
        None => {}
    }
    let x = rect.left() + 30.0;
    let g = line_galley(ui, label, font(13.5), theme::TEXT, right - x);
    paint_line(ui, x, top_line_y, g);
    if let Some(detail) = detail {
        let g = line_galley(ui, detail, font(12.0), theme::DIM, right - x);
        paint_line(ui, x, rect.top() + 30.0, g);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

// ---------------------------------------------------------------------------
// buttons
// ---------------------------------------------------------------------------

/// A square button holding a pixel icon, with a hover background and a
/// tooltip (an icon alone never carries the meaning).
pub struct IconButton<'a> {
    icon: Icon,
    tooltip: &'a str,
    side: f32,
    pixel: f32,
    color: Color32,
    fill: Option<Color32>,
    enabled: bool,
}

impl<'a> IconButton<'a> {
    pub fn new(icon: Icon, tooltip: &'a str) -> Self {
        Self {
            icon,
            tooltip,
            side: 26.0,
            pixel: 2.0,
            color: theme::MUTED,
            fill: None,
            enabled: true,
        }
    }

    pub fn side(mut self, side: f32) -> Self {
        self.side = side;
        self
    }

    pub fn color(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }

    pub fn fill(mut self, fill: Color32) -> Self {
        self.fill = Some(fill);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(self.side, self.side), sense);
        if let Some(fill) = self.fill {
            ui.painter().rect_filled(rect, 3.0, fill);
        } else if self.enabled && resp.hovered() {
            ui.painter().rect_filled(rect, 3.0, theme::RAISED);
        }
        let color = if !self.enabled {
            theme::BORDER
        } else if resp.hovered() && self.fill.is_none() {
            theme::TEXT
        } else {
            self.color
        };
        icons::paint_centered(ui.painter(), self.icon, rect, self.pixel, color);
        let resp = resp.on_hover_text(self.tooltip);
        if self.enabled {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            resp
        }
    }
}

/// Visual weight of a text button.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Filled ORANGE: the main action of a card.
    Primary,
    /// RAISED: everything else.
    Secondary,
    /// RED outline: deny, delete.
    Danger,
}

/// A text button in the pixel style.
pub fn button(ui: &mut Ui, label: &str, kind: ButtonKind) -> Response {
    let (fill, text, stroke) = match kind {
        ButtonKind::Primary => (theme::ORANGE, Color32::WHITE, Stroke::NONE),
        ButtonKind::Secondary => (theme::RAISED, theme::TEXT, Stroke::NONE),
        ButtonKind::Danger => (
            Color32::TRANSPARENT,
            theme::RED_TEXT,
            Stroke::new(1.0_f32, theme::RED),
        ),
    };
    ui.add(
        egui::Button::new(RichText::new(label).color(text).size(13.5))
            .fill(fill)
            .stroke(stroke)
            .rounding(Rounding::same(3.0))
            .min_size(vec2(0.0, 28.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A pill showing a state with an optional icon and chevron; opens a menu in
/// the composer and the header.
pub fn chip(
    ui: &mut Ui,
    icon: Option<(Icon, Color32)>,
    label: &str,
    chevron: bool,
    tooltip: &str,
) -> Response {
    chip_limited(ui, icon, label, chevron, tooltip, 240.0)
}

/// `chip` with an explicit maximum text width.
pub fn chip_limited(
    ui: &mut Ui,
    icon: Option<(Icon, Color32)>,
    label: &str,
    chevron: bool,
    tooltip: &str,
    max_text: f32,
) -> Response {
    let g = line_galley(ui, label, font(13.0), theme::TEXT, max_text);
    let mut w = g.size().x + 16.0;
    if icon.is_some() {
        w += 18.0;
    }
    if chevron {
        w += 12.0;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::RAISED);
    }
    let mut x = rect.left() + 8.0;
    if let Some((icon, color)) = icon {
        icons::paint(
            ui.painter(),
            icon,
            pos2(x, rect.center().y - 7.0),
            2.0,
            color,
        );
        x += 18.0;
    }
    let right = paint_line(ui, x, rect.center().y, g).right();
    if chevron {
        icons::paint(
            ui.painter(),
            Icon::ChevronDown,
            pos2(right + 4.0, rect.center().y - 3.5),
            1.0,
            theme::DIM,
        );
    }
    let resp = if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    };
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small read-only tag in a colour, e.g. `NEU`.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let g = line_galley(ui, text, mono(10.5), color, 160.0);
    let (rect, resp) = ui.allocate_exact_size(g.size() + vec2(8.0, 4.0), Sense::hover());
    ui.painter()
        .rect_stroke(rect, 2.0, Stroke::new(1.0_f32, color.gamma_multiply(0.5)));
    ui.painter().galley(rect.min + vec2(4.0, 2.0), g, color);
    resp
}

/// A pixel on/off switch. Returns the response; the caller flips the value.
pub fn toggle(ui: &mut Ui, on: bool, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 14.0), Sense::click());
    ui.painter().rect_filled(
        rect,
        2.0,
        if on { theme::ORANGE_DIM } else { theme::RAISED },
    );
    let knob_x = if on {
        rect.right() - 12.0
    } else {
        rect.left() + 2.0
    };
    ui.painter().rect_filled(
        Rect::from_min_size(pos2(knob_x, rect.top() + 2.0), vec2(10.0, 10.0)),
        0.0,
        if on { theme::TEXT } else { theme::DIM },
    );
    let resp = if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    };
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---------------------------------------------------------------------------
// containers
// ---------------------------------------------------------------------------

/// A CARD with a soft border and the standard padding.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::default()
        .fill(theme::CARD)
        .rounding(Rounding::same(4.0))
        .stroke(Stroke::new(1.0_f32, theme::BORDER_SOFT))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, add)
        .inner
}

/// A card with a coloured 2 px top edge: it asks for a decision.
pub fn attention_card<R>(ui: &mut Ui, edge: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let out = egui::Frame::default()
        .fill(theme::CARD)
        .rounding(Rounding::same(4.0))
        .stroke(Stroke::new(1.0_f32, theme::BORDER_SOFT))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, add);
    let r = out.response.rect;
    ui.painter().rect_filled(
        Rect::from_min_size(r.min, vec2(r.width(), 2.0)),
        Rounding {
            nw: 4.0,
            ne: 4.0,
            sw: 0.0,
            se: 0.0,
        },
        edge,
    );
    out.inner
}

/// Open or close a popup anchored to `anchor`; the caller draws the content
/// through `popup_menu`.
pub fn toggle_popup(ui: &Ui, anchor: &Response, id: egui::Id) {
    if anchor.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
}

/// A popup menu above or below `anchor`, styled like every other card.
pub fn popup_menu<R>(
    ui: &Ui,
    id: egui::Id,
    anchor: &Response,
    above: bool,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let side = if above {
        egui::AboveOrBelow::Above
    } else {
        egui::AboveOrBelow::Below
    };
    egui::popup::popup_above_or_below_widget(
        ui,
        id,
        anchor,
        side,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        },
    )
}

/// Close whichever popup is open.
pub fn close_popup(ui: &Ui) {
    ui.memory_mut(|m| m.close_popup());
}

/// Paint the 3×3 project raster of `key` into a `size` square at `origin`.
pub fn paint_project_icon(ui: &Ui, origin: egui::Pos2, key: &str, size: f32) {
    let cell = size / 3.0;
    let origin = pos2(origin.x.round(), origin.y.round());
    for (i, color) in theme::project_icon_colors(key).iter().enumerate() {
        ui.painter().rect_filled(
            Rect::from_min_size(
                origin + vec2((i % 3) as f32 * cell, (i / 3) as f32 * cell),
                vec2(cell, cell),
            ),
            0.0,
            *color,
        );
    }
}

/// Three small blocks running through the pixel gradient (busy state).
pub fn paint_busy(ui: &Ui, origin: egui::Pos2, block: f32) {
    let time = ui.input(|i| i.time);
    let base = (time * 6.0).floor() as usize;
    for i in 0..3 {
        ui.painter().rect_filled(
            Rect::from_min_size(
                origin + vec2(i as f32 * (block + 1.0), 0.0),
                vec2(block, block),
            ),
            0.0,
            theme::GRADIENT[base.wrapping_add(i) % 5],
        );
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(100));
}

/// A row of floating pills (icon + label), centred with its bottom edge at
/// `center_bottom` above everything else. When the full labels do not fit
/// into `max_width`, the pills shrink to their icons and the label moves
/// into the hover text, so the row never spills over its surroundings.
/// Returns the index of the clicked pill.
pub fn floating_pills(
    ctx: &egui::Context,
    id: &str,
    center_bottom: egui::Pos2,
    max_width: f32,
    pills: &[(Icon, &str)],
) -> Option<usize> {
    const H: f32 = 30.0;
    const GAP: f32 = 8.0;
    let mut clicked = None;
    egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(center_bottom)
        .show(ctx, |ui| {
            let galleys: Vec<_> = pills
                .iter()
                .map(|(_, label)| line_galley(ui, label, font(13.0), theme::TEXT, 240.0))
                .collect();
            let gaps = GAP * pills.len().saturating_sub(1) as f32;
            let full: f32 = galleys.iter().map(|g| g.size().x + 40.0).sum::<f32>() + gaps;
            let compact = full > max_width;
            ui.spacing_mut().item_spacing.x = GAP;
            ui.horizontal(|ui| {
                for (i, ((icon, label), g)) in pills.iter().zip(galleys).enumerate() {
                    let w = if compact { H } else { g.size().x + 40.0 };
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, H), Sense::click());
                    ui.painter().rect(
                        rect,
                        3.0,
                        if resp.hovered() {
                            theme::RAISED
                        } else {
                            theme::CARD
                        },
                        Stroke::new(1.0_f32, theme::BORDER),
                    );
                    let icon_x = if compact {
                        rect.center().x - 7.0
                    } else {
                        rect.left() + 12.0
                    };
                    icons::paint(
                        ui.painter(),
                        *icon,
                        pos2(icon_x, rect.center().y - 7.0),
                        2.0,
                        theme::MUTED,
                    );
                    let resp = if compact {
                        resp.on_hover_text(*label)
                    } else {
                        paint_line(ui, rect.left() + 30.0, rect.center().y, g);
                        resp
                    };
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hover button on a finished row must leave everything after it where
    /// it was (`Ui::put` pulled the next rows up by a few points).
    #[test]
    fn place_leaves_the_layout_alone() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (row, _) = ui.allocate_exact_size(vec2(200.0, 30.0), Sense::hover());
                let before = ui.cursor().min;
                let button = Rect::from_min_size(row.min, vec2(22.0, 22.0));
                place(ui, button, IconButton::new(Icon::Plus, "x").side(22.0));
                assert_eq!(ui.cursor().min, before, "the next row stays in place");
            });
        });
    }
}
