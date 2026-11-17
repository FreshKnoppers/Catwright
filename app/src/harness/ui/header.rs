//! The session header: breadcrumb with the session menu, git state and the
//! actions on the session.

use super::*;
use crate::harness::{display_title, without_ui_hint};
use crate::i18n::{tr, trf};
use crate::widgets::line_galley;
use egui::{pos2, Sense};

pub(super) fn show(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let Some((title, cwd, idle)) = h.sessions.get(sid).map(|s| {
        (
            s.title.as_deref().map_or_else(
                || tr("New session").to_string(),
                |t| display_title(t, tr("New session")),
            ),
            s.cwd.clone(),
            s.turn == TurnState::Idle,
        )
    }) else {
        return;
    };
    let is_chat = h.is_chat_dir(&cwd);
    egui::TopBottomPanel::top("harness_session_header")
        .frame(
            egui::Frame::none()
                .fill(theme::BG)
                .inner_margin(egui::Margin::symmetric(14.0, 7.0)),
        )
        .show_separator_line(true)
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(28.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let wide = ui.ctx().screen_rect().width() >= PANEL_MIN_WINDOW;
                    if wide
                        && ui
                            .add(IconButton::new(Icon::PanelRight, tr("Work panel (Ctrl J)")))
                            .clicked()
                    {
                        toggle_panel(h);
                    }
                    if !is_chat
                        && ui
                            .add(IconButton::new(Icon::Folder, tr("Open project folder")))
                            .clicked()
                    {
                        open_folder_of(&cwd.to_string_lossy());
                    }
                    if ui
                        .add(
                            IconButton::new(Icon::Fork, tr("Fork session"))
                                .enabled(idle && !h.fork_in_flight()),
                        )
                        .clicked()
                    {
                        h.fork_active_session();
                    }
                    if !is_chat {
                        if let Some(git) = h.watcher.git(&cwd).cloned() {
                            git_chip(ui, &git);
                        }
                    }
                    ui.allocate_ui_with_layout(
                        ui.available_size(),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| crumbs(h, ui, sid, &title, &cwd),
                    );
                });
            });
        });
}

fn git_chip(ui: &mut egui::Ui, git: &crate::harness::watch::GitInfo) {
    let Some(branch) = &git.branch else {
        return;
    };
    ui.add_space(4.0);
    if git.changed > 0 {
        ui.label(
            RichText::new(trf("{n} changed", &[("n", &git.changed)]))
                .color(theme::AMBER)
                .size(12.0),
        )
        .on_hover_text(tr("Changed files in the working directory (git status)"));
    }
    let g = line_galley(ui, branch, widgets::mono(12.0), theme::MUTED, 180.0);
    let (rect, resp) = ui.allocate_exact_size(g.size() + vec2(20.0, 6.0), Sense::hover());
    crate::icons::paint(
        ui.painter(),
        Icon::Branch,
        pos2(rect.left(), rect.center().y - 3.5),
        1.0,
        theme::DIM,
    );
    widgets::paint_line(ui, rect.left() + 12.0, rect.center().y, g);
    resp.on_hover_text(tr("Current Git branch"));
}

/// Workspace crumb ("Allgemein" or the project) › session title as a menu.
fn crumbs(h: &mut Harness, ui: &mut egui::Ui, sid: &str, title: &str, cwd: &std::path::Path) {
    let key = cwd.to_string_lossy().into_owned();
    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    paint_workspace_icon(h, ui, rect.min, cwd, 12.0);
    ui.add(
        egui::Label::new(theme::mono_tag(&h.workspace_name(cwd), theme::MUTED).size(12.0))
            .truncate(),
    )
    .on_hover_text(key);
    crate::icons::paint(
        ui.painter(),
        Icon::ChevronRight,
        pos2(ui.cursor().min.x, ui.max_rect().center().y - 3.5),
        1.0,
        theme::DIM,
    );
    ui.add_space(10.0);
    let popup = ui.make_persistent_id(("session_menu_header", sid));
    let max_text = (ui.available_width() - 60.0).max(80.0);
    let resp = widgets::chip_limited(ui, None, title, true, tr("Session actions"), max_text);
    widgets::toggle_popup(ui, &resp, popup);
    let mut close = false;
    widgets::popup_menu(ui, popup, &resp, false, 230.0, |ui| {
        let idle = h
            .sessions
            .get(sid)
            .is_some_and(|s| s.turn == TurnState::Idle);
        if widgets::menu_row(ui, Some(Icon::Pencil), tr("Rename"), None, false) {
            h.ui.rename = Some(crate::harness::state::RenameDialog {
                session_id: sid.to_string(),
                cwd: cwd.to_path_buf(),
                title: title.to_string(),
            });
            close = true;
        }
        if idle
            && !h.fork_in_flight()
            && widgets::menu_row(ui, Some(Icon::Fork), tr("Fork"), None, false)
        {
            h.fork_active_session();
            close = true;
        }
        if widgets::menu_row(
            ui,
            Some(Icon::Copy),
            tr("Copy transcript (Markdown)"),
            None,
            false,
        ) {
            if let Some(s) = h.sessions.get(sid) {
                ui.ctx().copy_text(s.transcript_markdown(without_ui_hint));
            }
            h.status = tr("Transcript copied to the clipboard.").to_string();
            close = true;
        }
        if widgets::menu_row(ui, Some(Icon::Trash), tr("Delete"), None, true) {
            h.ui.confirm = Some(Confirm::DeleteSession {
                session_id: sid.to_string(),
                cwd: cwd.to_path_buf(),
                title: title.to_string(),
            });
            close = true;
        }
    });
    if close {
        widgets::close_popup(ui);
    }
}
