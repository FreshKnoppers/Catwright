//! The activity feed in the main area: waiting, finished and failed turns
//! across all sessions, newest first, with the actions right on the entry
//! (open the session, answer a waiting permission).

use super::*;
use crate::harness::extras;
use crate::harness::state::{ActivityEvent, ActivityKind};
use crate::i18n::tr;
use crate::widgets::{self, ButtonKind, IconButton};
use egui::Sense;

/// What one entry asks the harness to do after the feed is drawn.
enum FeedOp {
    Open {
        cwd: std::path::PathBuf,
        sid: String,
    },
    Answer {
        sid: String,
        request_id: serde_json::Value,
        option_id: String,
    },
}

pub(super) fn view(h: &mut Harness, ui: &mut egui::Ui) {
    let mut close = false;
    let mut ops: Vec<FeedOp> = Vec::new();
    ui.add_space(6.0);
    column(ui, |ui| {
        ui.horizontal(|ui| {
            theme::section_header(ui, tr("Activity"), true);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(IconButton::new(Icon::Close, tr("Back to the session")).side(22.0))
                    .clicked()
                {
                    close = true;
                }
            });
        });
        ui.add_space(6.0);
        let events: Vec<ActivityEvent> = h.activity.iter().rev().cloned().collect();
        if events.is_empty() {
            ui.label(
                RichText::new(tr("Nothing yet — finished and waiting turns land here."))
                    .color(theme::DIM)
                    .size(13.0),
            );
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for kind in [
                    (ActivityKind::Waiting, tr("Waiting for you")),
                    (ActivityKind::Finished, tr("Finished")),
                    (ActivityKind::Failed, tr("Failed")),
                ] {
                    let entries: Vec<&ActivityEvent> =
                        events.iter().filter(|e| e.kind == kind.0).collect();
                    if entries.is_empty() {
                        continue;
                    }
                    ui.add_space(8.0);
                    ui.label(theme::mono_label(kind.1));
                    ui.add_space(2.0);
                    for e in entries {
                        entry_card(h, ui, e, &mut ops);
                    }
                }
            });
    });
    for op in ops {
        match op {
            FeedOp::Open { cwd, sid } => h.open_session(cwd, &sid),
            FeedOp::Answer {
                sid,
                request_id,
                option_id,
            } => h.answer_permission(&sid, &request_id, &option_id),
        }
    }
    if close {
        h.ui.activity_open = false;
    }
}

/// One feed entry: workspace, title, text, age and its actions.
fn entry_card(h: &Harness, ui: &mut egui::Ui, e: &ActivityEvent, ops: &mut Vec<FeedOp>) {
    egui::Frame::none()
        .fill(theme::CARD)
        .rounding(egui::Rounding::same(4.0))
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER_SOFT))
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), Sense::hover());
            let accent = match e.kind {
                ActivityKind::Waiting => theme::YELLOW,
                ActivityKind::Finished => theme::GREEN,
                ActivityKind::Failed => theme::RED,
            };
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height())),
                0.0,
                accent,
            );
            let mut x = rect.left() + 10.0;
            let ws = if e.cwd.as_os_str().is_empty() {
                String::new()
            } else {
                h.workspace_name(&e.cwd)
            };
            if !ws.is_empty() {
                let g = widgets::line_galley(ui, &ws, widgets::mono(10.5), theme::DIM, 120.0);
                x = widgets::paint_line(ui, x, rect.center().y, g).right() + 6.0;
            }
            let title = if e.title.is_empty() {
                tr("Session").to_string()
            } else {
                e.title.clone()
            };
            let age = Some(extras::human_duration(
                (extras::now_epoch() - e.at).max(0) as u64
            ));
            let mut right = rect.right();
            if let Some(a) = age {
                let g = widgets::line_galley(ui, &a, widgets::mono(10.5), theme::DIM, 70.0);
                right -= g.size().x;
                widgets::paint_line(ui, right, rect.center().y, g);
                right -= 8.0;
            }
            let g = widgets::line_galley(ui, &title, widgets::font(13.0), theme::TEXT, right - x);
            widgets::paint_line(ui, x, rect.center().y, g);
            ui.add_space(2.0);
            ui.label(RichText::new(&e.text).color(theme::MUTED).size(12.5));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if !e.sid.is_empty()
                    && widgets::button(ui, tr("Open"), ButtonKind::Secondary).clicked()
                {
                    let cwd = if e.cwd.as_os_str().is_empty() {
                        h.sessions
                            .get(&e.sid)
                            .map(|v| v.cwd.clone())
                            .unwrap_or_default()
                    } else {
                        e.cwd.clone()
                    };
                    ops.push(FeedOp::Open {
                        cwd,
                        sid: e.sid.clone(),
                    });
                }
                // a waiting permission can be answered right here while it
                // is still open in its session
                if e.kind == ActivityKind::Waiting {
                    if let (Some(rid), Some(v)) = (&e.request_id, h.sessions.get(&e.sid)) {
                        if let Some(p) = v.permissions.iter().find(|p| &p.request_id == rid) {
                            for opt in &p.request.options {
                                let label = if opt.name.is_empty() {
                                    extras::permission_label(&opt.option_id, "")
                                } else {
                                    opt.name.clone()
                                };
                                if widgets::button(ui, &label, ButtonKind::Secondary).clicked() {
                                    ops.push(FeedOp::Answer {
                                        sid: e.sid.clone(),
                                        request_id: rid.clone(),
                                        option_id: opt.option_id.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            });
        });
    ui.add_space(6.0);
}
