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
                        // git scans only run in folders Vibe reported as
                        // trusted (a shipped .git/config is executable by git)
                        let trusted = h.trust.get(&cwd).is_some_and(|t| t.trusted);
                        if let Some(git) = h.watcher.git(&cwd, trusted).cloned() {
                            git_chip(h, ui, &git, &cwd);
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

fn git_chip(
    h: &mut Harness,
    ui: &mut egui::Ui,
    git: &crate::harness::watch::GitInfo,
    cwd: &std::path::Path,
) {
    let Some(branch) = &git.branch else {
        return;
    };
    ui.add_space(4.0);
    if git.changed > 0 {
        let resp = ui
            .add(
                egui::Label::new(
                    RichText::new(trf("{n} changed", &[("n", &git.changed)]))
                        .color(theme::AMBER)
                        .size(12.0),
                )
                .sense(Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr(
                "Changed files in the working directory (git status) — click for the list",
            ));
        changed_files_menu(h, ui, &resp, git, cwd);
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

/// Popup id of the header's changed-files menu for the workspace `cwd`.
pub(crate) fn changed_files_popup(cwd: &std::path::Path) -> egui::Id {
    egui::Id::new(("changed_files", cwd))
}

/// The popup of the header's "N changed": every changed path from
/// `git status`, relative to the workspace when it lies inside. A click on
/// a file shows its diff in the preview pane (where it can be commented);
/// an untracked folder opens in the file manager.
fn changed_files_menu(
    h: &mut Harness,
    ui: &mut egui::Ui,
    anchor: &egui::Response,
    git: &crate::harness::watch::GitInfo,
    cwd: &std::path::Path,
) {
    // a global id: one menu per workspace, wherever the header sits
    let popup = changed_files_popup(cwd);
    widgets::toggle_popup(ui, anchor, popup);
    let root = git.root.clone().unwrap_or_else(|| cwd.to_path_buf());
    let mut pick: Option<(std::path::PathBuf, bool)> = None;
    widgets::popup_menu(ui, popup, anchor, false, 380.0, |ui| {
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for f in &git.files {
                    let abs = root.join(f.path.trim_end_matches('/'));
                    let shown = abs
                        .strip_prefix(cwd)
                        .map(|r| r.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| f.path.clone());
                    let folder = f.path.ends_with('/');
                    if changed_row(ui, &f.code, &shown, folder) {
                        pick = Some((abs, folder));
                    }
                }
                if git.changed > git.files.len() {
                    ui.label(
                        RichText::new(trf(
                            "… {n} more",
                            &[("n", &(git.changed - git.files.len()))],
                        ))
                        .color(theme::DIM)
                        .size(12.0),
                    );
                }
            });
    });
    if let Some((path, folder)) = pick {
        if folder {
            open_folder_of(&path.to_string_lossy());
        } else {
            h.open_preview_diff(&path);
        }
        widgets::close_popup(ui);
    }
}

/// One row of the changed-files popup: git's status letter in its colour
/// (modified, added/untracked, deleted) and the path. True on click.
fn changed_row(ui: &mut egui::Ui, code: &str, path: &str, folder: bool) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::RAISED);
    }
    let letter = code.trim();
    let letter = if letter == "??" { "U" } else { letter };
    let color = match letter.chars().next() {
        Some('A' | 'U') => theme::GREEN,
        Some('D') => theme::RED_TEXT,
        _ => theme::AMBER,
    };
    let g = line_galley(ui, letter, widgets::mono(12.0), color, 22.0);
    widgets::paint_line(ui, rect.left() + 8.0, rect.center().y, g);
    let icon = if folder { Icon::Folder } else { Icon::File };
    crate::icons::paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 32.0, rect.center().y - 7.0),
        2.0,
        theme::DIM,
    );
    let g = line_galley(
        ui,
        path,
        widgets::mono(12.0),
        theme::TEXT,
        rect.right() - rect.left() - 64.0,
    );
    widgets::paint_line(ui, rect.left() + 54.0, rect.center().y, g);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(path)
        .clicked()
}
