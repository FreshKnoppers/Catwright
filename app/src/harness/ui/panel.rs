//! The right "work" panel: collapsible cards for context, plan, changes,
//! subagents, routines, connectors and the environment.

use super::transcript::{plan_progress, plan_rows};
use super::*;
use crate::acp::protocol::{ToolCallContent, ToolStatus};
use crate::acp::session::{diff_stat_cached, Item, SessionView, SubagentInfo, SubagentState};
use crate::harness::extras;
use crate::harness::format_tokens;
use crate::i18n::{self, tr, trf};
use crate::icons;
use crate::widgets::{line_galley, mono, paint_line};
use egui::{pos2, Color32, Rect, Sense, Stroke};
use std::collections::HashMap;

pub(super) fn show(h: &mut Harness, ctx: &egui::Context) {
    let Some(sid) = h.active.clone() else {
        return;
    };
    let used = h
        .sessions
        .get(&sid)
        .and_then(|s| s.usage.as_ref().map(|u| u.used));
    let sizes = used.and_then(|u| h.context_sizes(&sid, u));
    egui::SidePanel::right("harness_right")
        .resizable(true)
        .default_width(300.0)
        .width_range(250.0..=440.0)
        .frame(
            // flat sections reach the panel's edges; their hairlines
            // separate them (no boxes inside the box)
            egui::Frame::none()
                .fill(theme::SIDEBAR)
                .inner_margin(egui::Margin::symmetric(0.0, 2.0)),
        )
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    context_card(h, ui, &sid, sizes);
                    plan_card(h, ui, &sid);
                    changes_card(h, ui, &sid);
                    subagents_card(h, ui, &sid);
                    routines_card(h, ui, &sid);
                    connectors_card(h, ui, &sid);
                    environment_card(h, ui, &sid);
                });
        });
}

/// A collapsible section of the panel: a header row (title, summary on
/// the right, also while folded) and a hairline under it across the panel.
/// `key` stores the open state.
fn card(
    sections: &mut HashMap<&'static str, bool>,
    ui: &mut egui::Ui,
    key: &'static str,
    title: &str,
    summary: &str,
    default_open: bool,
    add: impl FnOnce(&mut egui::Ui),
) -> bool {
    let open = *sections.get(key).unwrap_or(&default_open);
    let mut toggled = false;
    let section = egui::Frame::none()
        .inner_margin(egui::Margin {
            left: 14.0,
            right: 14.0,
            top: 8.0,
            bottom: 8.0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            let icon = if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            };
            icons::paint(
                ui.painter(),
                icon,
                pos2(rect.left(), rect.center().y - 3.5),
                1.0,
                theme::DIM,
            );
            let g = line_galley(
                ui,
                &title.to_uppercase(),
                mono(11.5),
                theme::MUTED,
                rect.width() * 0.6,
            );
            paint_line(ui, rect.left() + 12.0, rect.center().y, g);
            if !summary.is_empty() {
                let g = line_galley(ui, summary, mono(11.0), theme::DIM, rect.width() * 0.5);
                paint_line(ui, rect.right() - g.size().x, rect.center().y, g);
            }
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                toggled = true;
            }
            if open {
                add(ui);
                ui.add_space(2.0);
            }
        });
    let r = section.response.rect;
    ui.painter().hline(
        r.x_range(),
        r.bottom(),
        Stroke::new(1.0_f32, theme::BORDER_SOFT),
    );
    if toggled {
        sections.insert(key, !open);
    }
    open
}

// ---------------------------------------------------------------------------
// context
// ---------------------------------------------------------------------------

const CONTEXT_COLORS: [Color32; 4] = [
    Color32::from_rgb(0x14, 0xb8, 0xa6), // system prompt
    theme::ORANGE,                       // tools
    Color32::from_rgb(0xa8, 0x55, 0xf7), // skills
    Color32::from_rgb(0x3b, 0x82, 0xf6), // conversation
];

/// From this fill level on, the context card offers "Compact context".
const COMPACT_FROM: f32 = 0.5;

fn context_card(
    h: &mut Harness,
    ui: &mut egui::Ui,
    sid: &str,
    sizes: Option<crate::harness::context::ContextSizes>,
) {
    let Some(s) = h.sessions.get(sid) else {
        return;
    };
    let summary = match (&s.usage, s.context_fraction()) {
        (Some(u), Some(frac)) => format!(
            "{} % · {}/{}",
            (frac * 100.0).round() as u32,
            format_tokens(u.used),
            format_tokens(u.size)
        ),
        _ => "–".to_string(),
    };
    let idle = s.turn == TurnState::Idle;
    let has_compact = s.commands.iter().any(|c| c.name == "compact");
    let mut compact = false;
    let s_ref: &SessionView = s;
    card(
        &mut h.ui.panel_sections,
        ui,
        "context",
        tr("Context"),
        &summary,
        true,
        |ui| {
            context_body(ui, s_ref, sizes);
            // offered once it matters; below, the button only competes
            // (/compact stays available as a command)
            let worth = s_ref.context_fraction().unwrap_or(0.0) >= COMPACT_FROM;
            if has_compact && worth && s_ref.usage.as_ref().is_some_and(|u| u.used > 0) {
                ui.add_space(2.0);
                ui.add_enabled_ui(idle, |ui| {
                    if widgets::button(ui, tr("Compact context"), ButtonKind::Secondary)
                        .on_hover_text(tr(
                            "Summarises the conversation so far and frees context (/compact)",
                        ))
                        .clicked()
                    {
                        compact = true;
                    }
                });
            }
        },
    );
    if compact {
        h.run_command("/compact");
    }
}

fn context_body(
    ui: &mut egui::Ui,
    s: &SessionView,
    sizes: Option<crate::harness::context::ContextSizes>,
) {
    let Some(u) = &s.usage else {
        ui.label(
            RichText::new(tr("No measurement yet."))
                .color(theme::DIM)
                .small(),
        );
        return;
    };
    let size = u.size.max(1);
    let frac = s.context_fraction().unwrap_or(0.0);
    let parts = sizes.map(|z| crate::harness::context::split_tokens(u.used, &z));
    let block = ((ui.available_width() + 2.0) / 20.0 - 2.0).clamp(4.0, 10.0);
    let segments: Option<Vec<(Color32, f32)>> = parts.map(|parts| {
        parts
            .iter()
            .zip(CONTEXT_COLORS)
            .map(|(t, c)| (c, *t as f32 / size as f32))
            .collect()
    });
    match segments {
        // the split in colour only while every part fills a whole block:
        // the stacked meter gives a smaller part one block anyway, and at
        // 4 % that would light 3 blocks (15 %); then the meter shows the
        // total and the legend below carries the split
        Some(segments) if segments.iter().all(|(_, f)| *f == 0.0 || *f >= 1.0 / 20.0) => {
            theme::stacked_meter(ui, &segments, 20, block);
        }
        _ => {
            theme::pixel_meter(ui, theme::meter_blocks(frac, 20), 20, block);
        }
    }
    ui.add_space(2.0);
    let legend = |ui: &mut egui::Ui, color: Color32, label: &str, tokens: u64| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().rect_filled(r, 0.0, color);
            ui.label(RichText::new(label).color(theme::MUTED).size(12.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let pct = tokens as f64 * 100.0 / size as f64;
                ui.label(theme::mono_tag(
                    &i18n::decimal(format!("{pct:.1} %")),
                    theme::DIM,
                ));
                ui.label(theme::mono_tag(&format_tokens(tokens), theme::DIM));
            });
        });
    };
    if let Some(parts) = parts {
        for ((tokens, color), label) in parts
            .iter()
            .zip(CONTEXT_COLORS)
            .zip(crate::harness::context::labels())
        {
            if *tokens > 0 {
                legend(ui, color, label, *tokens);
            }
        }
    }
    legend(ui, theme::RAISED, tr("Free"), u.size.saturating_sub(u.used));
    if parts.is_some() {
        ui.label(
            RichText::new(tr("Split estimated (from Vibe’s measurement)"))
                .color(theme::DIM)
                .small(),
        );
    }
    if let Some(t) = &s.last_turn_usage {
        ui.label(theme::mono_tag(
            &trf(
                "Last answer ↑ {input} ↓ {output}",
                &[
                    ("input", &format_tokens(t.input_tokens)),
                    ("output", &format_tokens(t.output_tokens)),
                ],
            ),
            theme::DIM,
        ));
    }
    if let Some(cost) = &u.cost {
        ui.label(theme::mono_tag(
            &trf(
                "Cost (turn): {amount} {currency}",
                &[
                    ("amount", &i18n::decimal(format!("{:.4}", cost.amount))),
                    ("currency", &cost.currency),
                ],
            ),
            theme::DIM,
        ));
    }
}

// ---------------------------------------------------------------------------
// plan
// ---------------------------------------------------------------------------

fn plan_card(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let Some(s) = h.sessions.get(sid) else {
        return;
    };
    // only the CURRENT round's plan: a plan item must follow the last user
    // message, else the panel would show a finished round's stale plan
    let plan_current = s
        .items
        .iter()
        .rposition(|i| matches!(i, Item::User { .. }))
        .is_some_and(|u| s.items[u..].iter().any(|i| matches!(i, Item::Plan)));
    if s.plan.is_empty() || !plan_current {
        return;
    }
    let (done, total) = plan_progress(&s.plan);
    let plan = s.plan.clone();
    card(
        &mut h.ui.panel_sections,
        ui,
        "plan",
        tr("Plan"),
        &format!("{done}/{total}"),
        true,
        |ui| {
            plan_rows(ui, &plan, 13.0);
        },
    );
}

// ---------------------------------------------------------------------------
// changes (review)
// ---------------------------------------------------------------------------

/// Every edited file with its summed stats, newest edit last.
fn edited_files(s: &SessionView) -> Vec<(String, usize, usize)> {
    let mut out: Vec<(String, usize, usize)> = Vec::new();
    for item in &s.items {
        let Item::Tool(id) = item else { continue };
        let Some(tc) = s.tool_calls.get(id) else {
            continue;
        };
        if tc.status != ToolStatus::Completed {
            continue;
        }
        for c in &tc.content {
            if let ToolCallContent::Diff {
                path,
                old_text,
                new_text,
            } = c
            {
                let stat = diff_stat_cached(old_text.as_deref(), new_text);
                match out.iter_mut().find(|(p, _, _)| p == path) {
                    Some(f) => {
                        f.1 += stat.added;
                        f.2 += stat.removed;
                    }
                    None => out.push((path.clone(), stat.added, stat.removed)),
                }
            }
        }
    }
    out
}

fn changes_card(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let Some(s) = h.sessions.get(sid) else {
        return;
    };
    let cwd = s.cwd.clone();
    let idle = s.turn == TurnState::Idle;
    let mut files = edited_files(s);
    let pending = h.review.get(sid).cloned().unwrap_or_default();
    // files Vibe still tracks but the transcript does not show (a rewind, or
    // edits from a shell command) stay reachable, just without stats
    for f in &pending {
        if !files.iter().any(|(p, _, _)| p == &f.path) {
            files.push((f.path.clone(), 0, 0));
        }
    }
    if files.is_empty() && pending.is_empty() {
        return;
    }
    let summary = if pending.is_empty() {
        if files.len() == 1 {
            tr("1 file").to_string()
        } else {
            trf("{n} files", &[("n", &files.len())])
        }
    } else {
        trf("{n} open", &[("n", &pending.len())])
    };
    let mut revert_file: Option<String> = None;
    let mut approve_file: Option<String> = None;
    let mut revert_all = false;
    let mut approve_all = false;
    // a click on a row shows the file's diff in the preview pane
    let mut show_diff: Option<String> = None;
    card(
        &mut h.ui.panel_sections,
        ui,
        "changes",
        // "session": Vibe's edits here, not the header's git count
        tr("Session changes"),
        &summary,
        true,
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (path, added, removed) in &files {
                let open = pending.iter().find(|f| &f.path == path);
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                let hovered = ui.rect_contains_pointer(rect);
                let mut button_clicked = false;
                if hovered {
                    ui.painter().rect_filled(rect, 3.0, theme::RAISED);
                }
                let name = crate::artifacts::file_name(path);
                let mut right = rect.right();
                if hovered && open.is_some() && idle {
                    let r =
                        Rect::from_min_size(pos2(right - 22.0, rect.top() + 1.0), vec2(22.0, 22.0));
                    if widgets::place(
                        ui,
                        r,
                        IconButton::new(Icon::Rewind, tr("Revert change")).side(22.0),
                    )
                    .clicked()
                    {
                        revert_file = Some(path.clone());
                        button_clicked = true;
                    }
                    right -= 24.0;
                    let r =
                        Rect::from_min_size(pos2(right - 22.0, rect.top() + 1.0), vec2(22.0, 22.0));
                    if widgets::place(
                        ui,
                        r,
                        IconButton::new(Icon::Check, tr("Mark as reviewed")).side(22.0),
                    )
                    .clicked()
                    {
                        approve_file = Some(path.clone());
                        button_clicked = true;
                    }
                    right -= 24.0;
                } else if hovered {
                    let r =
                        Rect::from_min_size(pos2(right - 22.0, rect.top() + 1.0), vec2(22.0, 22.0));
                    if widgets::place(
                        ui,
                        r,
                        IconButton::new(Icon::Folder, tr("Open folder")).side(22.0),
                    )
                    .clicked()
                    {
                        open_folder_of(path);
                        button_clicked = true;
                    }
                    right -= 24.0;
                } else {
                    let stats = if *added == 0 && *removed == 0 {
                        Vec::new()
                    } else if *removed > 0 {
                        vec![
                            line_galley(
                                ui,
                                &format!("−{removed}"),
                                mono(11.5),
                                theme::RED_TEXT,
                                60.0,
                            ),
                            line_galley(ui, &format!("+{added}"), mono(11.5), theme::GREEN, 60.0),
                        ]
                    } else {
                        vec![line_galley(
                            ui,
                            &format!("+{added}"),
                            mono(11.5),
                            theme::GREEN,
                            60.0,
                        )]
                    };
                    for g in stats {
                        right -= g.size().x;
                        paint_line(ui, right, rect.center().y, g);
                        right -= 6.0;
                    }
                }
                let mut x = rect.left();
                if let Some(f) = open {
                    let tag = extras::review_status_label(&f.status);
                    let g = line_galley(ui, tag, mono(10.5), theme::AMBER, 80.0);
                    let r = paint_line(ui, x, rect.center().y, g);
                    x = r.right() + 6.0;
                }
                let g = line_galley(ui, name, widgets::font(13.0), theme::TEXT, right - x - 4.0);
                paint_line(ui, x, rect.center().y, g);
                let resp = resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(relative_path(path, &cwd));
                if resp.clicked() && !button_clicked {
                    show_diff = Some(path.clone());
                }
            }
            if !pending.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled_ui(idle, |ui| {
                        if widgets::button(ui, tr("Accept all"), ButtonKind::Secondary)
                            .on_hover_text(tr("Marks all open changes as reviewed"))
                            .clicked()
                        {
                            approve_all = true;
                        }
                        if widgets::button(ui, tr("Discard all"), ButtonKind::Danger)
                            .on_hover_text(tr("Reverts all open changes in the project"))
                            .clicked()
                        {
                            revert_all = true;
                        }
                    });
                });
            }
        },
    );
    if let Some(path) = show_diff {
        let abs = if std::path::Path::new(&path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            cwd.join(path)
        };
        h.open_preview_diff(&abs);
    }
    if let Some(path) = revert_file {
        h.ui.confirm = Some(Confirm::RevertFile {
            session_id: sid.to_string(),
            path,
        });
    }
    if let Some(path) = approve_file {
        h.review_action(sid, true, Harness::review_file_target(&path));
    }
    if revert_all {
        h.ui.confirm = Some(Confirm::RevertAll {
            session_id: sid.to_string(),
            files: pending.len(),
        });
    }
    if approve_all {
        h.review_action(sid, true, Harness::review_all_target());
    }
}

// ---------------------------------------------------------------------------
// subagents
// ---------------------------------------------------------------------------

fn subagents_card(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let Some(s) = h.sessions.get(sid) else {
        return;
    };
    let subagents = s.subagents();
    if subagents.is_empty() {
        return;
    }
    let running = subagents
        .iter()
        .filter(|a| a.state == SubagentState::Running)
        .count();
    let summary = if running > 0 {
        trf("{n} running", &[("n", &running)])
    } else {
        subagents.len().to_string()
    };
    card(
        &mut h.ui.panel_sections,
        ui,
        "subagents",
        "Subagents",
        &summary,
        true,
        |ui| {
            let time = ui.input(|i| i.time);
            let (attempts, started): (Vec<_>, Vec<_>) = subagents
                .iter()
                .partition(|a| a.state == SubagentState::Failed && a.child_session_id.is_none());
            for a in &started {
                subagent(ui, a, time);
            }
            if !attempts.is_empty() {
                ui.label(
                    RichText::new(if attempts.len() == 1 {
                        tr("1 start attempt failed").to_string()
                    } else {
                        trf("{n} start attempts failed", &[("n", &attempts.len())])
                    })
                    .color(theme::DIM)
                    .small(),
                );
            }
        },
    );
}

/// One subagent: name, agent, goal, the cat's status bar and the state. The
/// child session cannot be opened (Vibe rejects `session/load` for it).
fn subagent(ui: &mut egui::Ui, a: &SubagentInfo, time: f64) {
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(&a.name).color(theme::TEXT).size(13.5)).truncate())
            .on_hover_text(&a.name);
        // the type only when it tells subagents apart: Vibe starts nearly
        // all of them as "generic", and a tag on every card says nothing
        if let Some(agent) = a
            .agent
            .as_deref()
            .filter(|t| !t.trim().is_empty() && !t.eq_ignore_ascii_case("generic"))
        {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::mono_tag(agent, theme::DIM));
            });
        }
    });
    if !a.task.is_empty() {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &a.task,
            0.0,
            egui::TextFormat {
                font_id: widgets::font(12.5),
                color: theme::DIM,
                ..Default::default()
            },
        );
        job.wrap.max_rows = 3;
        job.wrap.break_anywhere = true;
        ui.add(egui::Label::new(job).truncate())
            .on_hover_text(&a.task);
    }
    // each cat on its own phase, so running subagents do not walk in step
    let time = time + theme::cat_stagger(&a.tool_call_id);
    theme::subagent_bar(ui, &a.state, time, ui.available_width());
    let (text, color) = match a.state {
        SubagentState::Running => (tr("Running …").to_string(), theme::YELLOW),
        SubagentState::Done => (
            match a.turns {
                Some(n) => trf("Done · {n} turns", &[("n", &n)]),
                None => tr("Done").to_string(),
            },
            theme::GREEN,
        ),
        SubagentState::Failed => (tr("Failed").to_string(), theme::RED_TEXT),
        SubagentState::Cancelled => (tr("Cancelled").to_string(), theme::DIM),
    };
    ui.horizontal(|ui| {
        ui.label(theme::mono_tag(&text, color));
        // the cat's run is a loop, not progress: the real measure of a
        // running subagent is how long it has been at it
        if a.state == SubagentState::Running && a.started > 0 {
            let secs = (extras::now_epoch() as u64).saturating_sub(a.started);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(extras::elapsed_label(secs))
                        .monospace()
                        .size(11.0)
                        .color(theme::DIM),
                )
                .on_hover_text(tr("Running time so far"));
            });
        }
    });
    ui.add_space(4.0);
}

// ---------------------------------------------------------------------------
// routines (loops)
// ---------------------------------------------------------------------------

fn routines_card(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let loops = h.loops.get(sid).cloned().unwrap_or_default();
    let summary = if loops.is_empty() {
        tr("none").to_string()
    } else {
        loops.len().to_string()
    };
    let mut delete: Option<String> = None;
    let mut create = false;
    let mut clear_all = false;
    let form = &mut h.ui.loop_form;
    card(
        &mut h.ui.panel_sections,
        ui,
        "routines",
        tr("Routines"),
        &summary,
        false,
        |ui| {
            ui.label(
            RichText::new(tr("Recurring tasks in this session. They only run while it is open and nothing else is running."))
                .color(theme::DIM)
                .small(),
        );
            for l in &loops {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(
                            egui::Label::new(RichText::new(l.name()).size(13.5).color(theme::TEXT))
                                .truncate(),
                        );
                        let next = (l.next_fire_at - extras::now_epoch() as f64).max(0.0) as u64;
                        ui.label(theme::mono_tag(
                            &trf(
                                "{schedule} · in {time}",
                                &[
                                    ("schedule", &l.schedule()),
                                    ("time", &extras::human_duration(next)),
                                ],
                            ),
                            theme::DIM,
                        ));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(IconButton::new(Icon::Trash, tr("Delete routine")).side(22.0))
                            .clicked()
                        {
                            delete = Some(l.id.clone());
                        }
                    });
                });
            }
            if form.open {
                ui.add(
                    egui::TextEdit::multiline(&mut form.prompt)
                        .hint_text(tr("What should Vibe do regularly?\n(first line = name)"))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Every")).color(theme::MUTED).size(13.0));
                    ui.add(
                        egui::TextEdit::singleline(&mut form.interval)
                            .desired_width(60.0)
                            .hint_text("10m"),
                    );
                    ui.label(
                        RichText::new(tr("e.g. 30s, 10m, 2h"))
                            .color(theme::DIM)
                            .small(),
                    );
                });
                ui.horizontal(|ui| {
                    // Vibe needs both: without them the button stays off and
                    // says why, instead of a click that does nothing
                    let ready = !form.prompt.trim().is_empty() && !form.interval.trim().is_empty();
                    let resp = ui
                        .add_enabled_ui(ready, |ui| {
                            widgets::button(ui, tr("Create"), ButtonKind::Primary)
                        })
                        .inner;
                    if resp
                        .on_disabled_hover_text(tr("Enter a task and an interval first."))
                        .clicked()
                    {
                        create = true;
                    }
                    if widgets::button(ui, tr("Cancel"), ButtonKind::Secondary).clicked() {
                        form.open = false;
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    if widgets::button(ui, tr("New routine"), ButtonKind::Secondary).clicked() {
                        form.open = true;
                    }
                    if !loops.is_empty()
                        && widgets::button(ui, tr("Delete all"), ButtonKind::Danger).clicked()
                    {
                        clear_all = true;
                    }
                });
            }
        },
    );
    if create {
        let (interval, prompt) = (
            h.ui.loop_form.interval.clone(),
            h.ui.loop_form.prompt.clone(),
        );
        if !prompt.trim().is_empty() && !interval.trim().is_empty() {
            h.create_loop(sid, &interval, &prompt);
            h.ui.loop_form = Default::default();
        }
    }
    if let Some(id) = delete {
        h.delete_loop(sid, &id);
    }
    if clear_all {
        h.ui.confirm = Some(Confirm::ClearRoutines {
            session_id: sid.to_string(),
            count: loops.len(),
        });
    }
}

// ---------------------------------------------------------------------------
// connectors
// ---------------------------------------------------------------------------

fn connectors_card(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let list = h.connectors.get(sid).cloned();
    let summary = list
        .as_ref()
        .map(|l| {
            let connected = l.iter().filter(|c| c.enabled && c.connected).count();
            trf("{n} connected", &[("n", &connected)])
        })
        .unwrap_or_default();
    let mut toggle: Option<(String, bool)> = None;
    let opened = card(
        &mut h.ui.panel_sections,
        ui,
        "connectors",
        tr("Connectors"),
        &summary,
        false,
        |ui| match &list {
            None => {
                ui.label(RichText::new(tr("Loading …")).color(theme::DIM).small());
            }
            Some(l) if l.is_empty() => {
                ui.label(
                    RichText::new(tr("No connectors set up."))
                        .color(theme::DIM)
                        .small(),
                );
            }
            Some(l) => {
                for c in l {
                    ui.horizontal(|ui| {
                        let color = if c.enabled && c.connected {
                            theme::GREEN
                        } else if c.enabled {
                            theme::YELLOW
                        } else {
                            theme::DIM
                        };
                        let (r, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
                        ui.painter().rect_filled(r, 0.0, color);
                        let label = ui.add(
                            egui::Label::new(RichText::new(&c.name).size(13.5).color(theme::TEXT))
                                .truncate(),
                        );
                        match (&c.error, c.tools) {
                            (Some(e), _) => {
                                label.on_hover_text(e.clone());
                            }
                            (None, n) => {
                                label.on_hover_text(if n == 1 {
                                    tr("1 tool").to_string()
                                } else {
                                    trf("{n} tools", &[("n", &n)])
                                });
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::toggle(
                                ui,
                                c.enabled,
                                if c.enabled {
                                    tr("Disable")
                                } else {
                                    tr("Enable")
                                },
                            )
                            .clicked()
                            {
                                toggle = Some((c.name.clone(), c.enabled));
                            }
                        });
                    });
                }
            }
        },
    );
    if opened {
        h.request_connectors(sid);
    }
    if let Some((name, disabled)) = toggle {
        h.toggle_connector(sid, &name, disabled);
    }
}

// ---------------------------------------------------------------------------
// environment
// ---------------------------------------------------------------------------

fn environment_card(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let Some(s) = h.sessions.get(sid) else {
        return;
    };
    let cwd = s.cwd.clone();
    let idle = s.turn == TurnState::Idle;
    let extra: Vec<String> = h
        .listed
        .get(&cwd)
        .map(|l| {
            l.iter()
                .filter(|e| e.session_id == sid)
                .flat_map(|e| e.additional_directories.iter().cloned())
                .collect()
        })
        .unwrap_or_default();
    let trust = h.trust.get(&cwd).cloned();
    let fork_busy = h.fork_in_flight();
    let is_chat = h.is_chat_dir(&cwd);
    let git = if is_chat {
        None
    } else {
        // git scans only run in folders Vibe reported as trusted
        let trusted = trust.as_ref().is_some_and(|t| t.trusted);
        h.watcher.git(&cwd, trusted).cloned()
    };
    let mut fork = false;
    // folded, the section still says where the session works
    let summary = git
        .as_ref()
        .and_then(|g| g.branch.clone())
        .into_iter()
        .chain(trust.as_ref().map(|t| {
            if t.trusted {
                tr("Trusted").to_string()
            } else {
                tr("Not trusted").to_string()
            }
        }))
        .collect::<Vec<_>>()
        .join(" · ");
    card(
        &mut h.ui.panel_sections,
        ui,
        "environment",
        tr("Environment"),
        &summary,
        false,
        |ui| {
            let key = cwd.to_string_lossy().into_owned();
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                widgets::paint_project_icon(ui, r.min, &key, 12.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(&key)
                            .monospace()
                            .size(11.5)
                            .color(theme::MUTED),
                    )
                    .truncate(),
                )
                .on_hover_text(&key);
            });
            if let Some(branch) = git.as_ref().and_then(|g| g.branch.as_ref()) {
                ui.label(theme::mono_tag(
                    &trf("Branch {branch}", &[("branch", branch)]),
                    theme::DIM,
                ));
            } else if git.as_ref().is_some_and(|g| g.refused) {
                ui.label(theme::mono_tag(
                    tr("Git off for this repository"),
                    theme::YELLOW,
                ))
                .on_hover_text(super::header::git_refused_why());
            }
            if let Some(t) = &trust {
                let (text, color) = if t.trusted {
                    (tr("Trusted"), theme::GREEN)
                } else {
                    (tr("Not trusted"), theme::DIM)
                };
                ui.label(theme::mono_tag(text, color)).on_hover_text(
                tr("Vibe loads AGENTS.md, skills and hooks from this folder only when it is trusted."),
            );
            }
            for d in &extra {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("+ {d}"))
                            .monospace()
                            .size(11.5)
                            .color(theme::DIM),
                    )
                    .truncate(),
                )
                .on_hover_text(d);
            }
            ui.add_enabled_ui(idle && !fork_busy, |ui| {
                if widgets::button(ui, tr("Fork session"), ButtonKind::Secondary)
                    .on_hover_text(tr("Continue this conversation as a new session (fork)"))
                    .clicked()
                {
                    fork = true;
                }
            });
        },
    );
    if fork {
        h.fork_active_session();
    }
}
