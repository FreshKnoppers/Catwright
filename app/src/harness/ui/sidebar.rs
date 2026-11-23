//! The left sidebar: brand, navigation, the workspace tree (the pinned chat
//! workspace "Allgemein", then the projects) with its sessions (status, age,
//! actions), the account footer and the icon rail it folds into.

use super::*;
use crate::harness::{extras, Conn, SidebarEntry, MAX_RECONNECTS};
use crate::i18n::{self, tr, trf};
use crate::icons;
use crate::theme::GRADIENT;
use crate::widgets::{line_galley, mono, paint_line, paint_row_bg};
use egui::{pos2, Rect, Sense, Stroke};
use std::path::Path;

/// Sessions shown per project before "mehr".
const SESSIONS_SHOWN: usize = 5;

/// What a sidebar interaction asks for; applied after the list is drawn.
enum Op {
    Open(PathBuf, String),
    NewIn(PathBuf),
    Select(PathBuf),
    ToggleCollapsed(PathBuf),
    ToggleAll(PathBuf),
    Rename(String, PathBuf, String),
    Delete(String, PathBuf, String),
    TogglePin(String),
    MarkUnread(String),
    Archive(String),
    Unarchive(String),
    Remove(PathBuf),
    AddProject,
    DeleteWorktree {
        project: PathBuf,
        path: PathBuf,
        branch: String,
    },
}

pub(super) fn show(h: &mut Harness, ctx: &egui::Context, action: &mut HarnessAction) {
    if h.ui.sidebar_collapsed || ctx.screen_rect().width() < COMPACT_WIDTH {
        rail(h, ctx, action);
        return;
    }
    egui::SidePanel::left("harness_sidebar")
        .resizable(true)
        .default_width(272.0)
        .width_range(224.0..=420.0)
        .frame(
            egui::Frame::none()
                .fill(theme::SIDEBAR)
                .inner_margin(egui::Margin {
                    left: 10.0,
                    right: 10.0,
                    top: 10.0,
                    bottom: 6.0,
                }),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            brand_row(h, ui);
            ui.add_space(10.0);
            navigation(h, ui);
            ui.add_space(12.0);
            egui::TopBottomPanel::bottom("harness_sidebar_footer")
                .frame(egui::Frame::none())
                .show_separator_line(false)
                .show_inside(ui, |ui| footer(h, ui, action));
            workspace_list(h, ui);
        });
}

fn brand_row(h: &mut Harness, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        theme::pixel_logo(ui, 2.0);
        ui.add_space(4.0);
        ui.label(
            RichText::new("Catwright")
                .color(theme::TEXT)
                .family(crate::fonts::bold())
                .size(14.5),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(IconButton::new(
                    Icon::PanelLeft,
                    tr("Collapse sidebar (Ctrl B)"),
                ))
                .clicked()
            {
                h.ui.sidebar_collapsed = true;
            }
        });
    });
}

fn navigation(h: &mut Harness, ui: &mut egui::Ui) {
    let on_start = h.active.is_none() && !h.artifacts_open && !h.ui.activity_open;
    if widgets::nav_row(
        ui,
        Icon::Plus,
        tr("New session"),
        Some(tr("Ctrl N")),
        on_start,
    )
    .clicked()
    {
        h.new_session_view();
    }
    if widgets::nav_row(ui, Icon::Search, tr("Search"), Some(tr("Ctrl K")), false).clicked() {
        h.ui.palette = Some(Default::default());
    }
    // the activity feed with its unseen badge
    let unseen_events = h.activity_unseen();
    let resp = widgets::nav_row(ui, Icon::List, tr("Activity"), None, h.ui.activity_open);
    if unseen_events > 0 {
        let label = unseen_events.min(99).to_string();
        let g = line_galley(ui, &label, mono(10.5), theme::SIDEBAR, 30.0);
        let w = (g.size().x + 10.0).max(18.0);
        let badge = Rect::from_min_size(
            pos2(resp.rect.right() - w - 6.0, resp.rect.center().y - 8.0),
            vec2(w, 16.0),
        );
        ui.painter().rect_filled(badge, 2.0, theme::ORANGE);
        ui.painter().galley(
            pos2(
                badge.center().x - g.size().x / 2.0,
                badge.center().y - g.size().y / 2.0,
            ),
            g,
            theme::SIDEBAR,
        );
    }
    if resp.clicked() {
        h.ui.activity_open = true;
        h.artifacts_open = false;
        h.ui.activity_seen = h.activity.len();
    }
    if widgets::nav_row(ui, Icon::Artifacts, tr("Artifacts"), None, h.artifacts_open).clicked() {
        h.artifacts_open = true;
        h.ui.activity_open = false;
    }
}

// ---------------------------------------------------------------------------
// session lists
// ---------------------------------------------------------------------------

fn section_header(ui: &mut egui::Ui, title: &str, add: Option<(&str, &mut bool)>) {
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(theme::mono_label(title));
        if let Some((tip, clicked)) = add {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(IconButton::new(Icon::Plus, tip).side(22.0))
                    .clicked()
                {
                    *clicked = true;
                }
            });
        }
    });
}

/// Every workspace with its sessions: the chat workspace "Allgemein" pinned
/// on top, then the projects.
fn workspace_list(h: &mut Harness, ui: &mut egui::Ui) {
    let mut add = false;
    section_header(ui, tr("Projects"), Some((tr("Add project"), &mut add)));
    let mut ops: Vec<Op> = Vec::new();
    if add {
        ops.push(Op::AddProject);
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            if let Some(dir) = h.chat_dir.clone() {
                workspace_tree(h, ui, &dir, &mut ops);
            }
            if h.projects.is_empty() {
                empty_projects(ui, &mut ops);
            }
            for i in 0..h.projects.len() {
                let p = h.projects[i].clone();
                workspace_tree(h, ui, &p, &mut ops);
            }
        });
    apply_ops(h, ops);
}

/// One workspace row and, unless folded, its newest sessions below it.
fn workspace_tree(h: &mut Harness, ui: &mut egui::Ui, p: &Path, ops: &mut Vec<Op>) {
    let rows = h.sidebar_rows(p, tr("New session"));
    let unseen = rows
        .iter()
        .filter(|e| h.session_state(&e.session_id) == SessionState::Unseen)
        .count();
    project_row(h, ui, p, rows.len(), unseen, ops);
    if h.ui.collapsed_projects.contains(p) {
        return;
    }
    let all = h.ui.expanded_projects.contains(p);
    let shown = if all {
        rows.len()
    } else {
        rows.len().min(SESSIONS_SHOWN)
    };
    for entry in &rows[..shown] {
        session_row(h, ui, p, entry, 20.0, ops);
    }
    if rows.len() > SESSIONS_SHOWN {
        let label = if all {
            tr("Show less").to_string()
        } else {
            trf("{n} more", &[("n", &(rows.len() - SESSIONS_SHOWN))])
        };
        if more_row(ui, &label) {
            ops.push(Op::ToggleAll(p.to_path_buf()));
        }
    }
    // the project's other worktrees (never the main worktree)
    let wts = h.worktrees_of(p);
    if !wts.is_empty() {
        ui.add_space(4.0);
        // indented like the session rows: the list belongs to this project
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            ui.label(theme::mono_label(tr("Worktrees")));
        });
        for w in &wts {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let name = w
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| w.path.to_string_lossy().into_owned());
                let label = if w.branch.is_empty() {
                    name
                } else {
                    format!("{name} ({})", w.branch)
                };
                let g = line_galley(ui, &label, mono(11.0), theme::DIM, 170.0);
                let (rect, _) = ui.allocate_exact_size(vec2(g.size().x, 20.0), Sense::hover());
                paint_line(ui, rect.left(), rect.center().y, g);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(IconButton::new(Icon::Trash, tr("Delete worktree")).side(20.0))
                        .clicked()
                    {
                        ops.push(Op::DeleteWorktree {
                            project: p.to_path_buf(),
                            path: w.path.clone(),
                            branch: w.branch.clone(),
                        });
                    }
                });
            });
        }
    }
    // archived sessions of this workspace, folded behind their own row
    let archived = h.sidebar_rows_archived(p, tr("New session"));
    if !archived.is_empty() {
        let label = trf("Archived ({n})", &[("n", &archived.len().to_string())]);
        if more_row(ui, &label) && !h.ui.archived_expanded.remove(p) {
            h.ui.archived_expanded.insert(p.to_path_buf());
        }
        if h.ui.archived_expanded.contains(p) {
            for entry in &archived {
                session_row(h, ui, p, entry, 20.0, ops);
            }
        }
    }
}

fn empty_projects(ui: &mut egui::Ui, ops: &mut Vec<Op>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr("Add your first project folder."))
                .color(theme::DIM)
                .small(),
        );
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        if widgets::button(ui, tr("Add project"), ButtonKind::Primary).clicked() {
            ops.push(Op::AddProject);
        }
    });
}

/// A workspace row: chevron, icon, name and the session count (or the
/// actions while hovered). The chat workspace has no "remove" action.
fn project_row(
    h: &mut Harness,
    ui: &mut egui::Ui,
    p: &Path,
    count: usize,
    unseen: usize,
    ops: &mut Vec<Op>,
) {
    let key = p.to_string_lossy().into_owned();
    let general = h.is_chat_dir(p);
    let waiting = h.workspace_waiting(p);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let menu_id = ui.make_persistent_id(("project_menu", &key));
    let menu_open = ui.memory(|m| m.is_popup_open(menu_id));
    let hovered = ui.rect_contains_pointer(rect) || menu_open;
    paint_row_bg(ui, rect, hovered, false);
    let collapsed = h.ui.collapsed_projects.contains(p);
    let chevron = if collapsed {
        Icon::ChevronRight
    } else {
        Icon::ChevronDown
    };
    icons::paint(
        ui.painter(),
        chevron,
        pos2(rect.left() + 5.0, rect.center().y - 3.5),
        1.0,
        theme::DIM,
    );
    paint_workspace_icon(
        h,
        ui,
        pos2(rect.left() + 18.0, rect.center().y - 6.0),
        p,
        12.0,
    );
    let is_current = h.current_project.as_deref() == Some(p);
    let name_color = if is_current {
        theme::TEXT
    } else {
        theme::MUTED
    };
    let mut right = rect.right() - 6.0;
    if hovered {
        let more = rect_at_right(rect, right, 22.0);
        right -= 24.0;
        let plus = rect_at_right(rect, right, 22.0);
        right -= 24.0;
        let plus_resp = widgets::place(
            ui,
            plus,
            IconButton::new(Icon::Plus, tr("New session here")).side(22.0),
        );
        if plus_resp.clicked() {
            ops.push(Op::NewIn(p.to_path_buf()));
        }
        let more_resp = widgets::place(
            ui,
            more,
            IconButton::new(Icon::Dots, tr("Project actions")).side(22.0),
        );
        widgets::toggle_popup(ui, &more_resp, menu_id);
        widgets::popup_menu(ui, menu_id, &more_resp, false, 210.0, |ui| {
            if widgets::menu_row(ui, Some(Icon::Plus), tr("New session"), None, false) {
                ops.push(Op::NewIn(p.to_path_buf()));
                widgets::close_popup(ui);
            }
            if widgets::menu_row(ui, Some(Icon::Folder), tr("Open folder"), None, false) {
                open_folder_of(&key);
                widgets::close_popup(ui);
            }
            if widgets::menu_row(ui, Some(Icon::Copy), tr("Copy path"), None, false) {
                ui.ctx().copy_text(key.clone());
                widgets::close_popup(ui);
            }
            if !general
                && widgets::menu_row(ui, Some(Icon::Trash), tr("Remove from list"), None, true)
            {
                ops.push(Op::Remove(p.to_path_buf()));
                widgets::close_popup(ui);
            }
        });
    } else {
        // unseen finished turns: an accent counter, also while folded
        if unseen > 0 {
            let g = line_galley(
                ui,
                &unseen.min(99).to_string(),
                mono(11.0),
                theme::ORANGE,
                40.0,
            );
            right -= g.size().x + 2.0;
            paint_line(ui, right, rect.center().y, g);
            right -= 4.0;
        }
        if count > 0 {
            let g = line_galley(ui, &count.to_string(), mono(11.0), theme::DIM, 40.0);
            right -= g.size().x + 2.0;
            paint_line(ui, right, rect.center().y, g);
            right -= 4.0;
        }
        // a session in here waits for an answer: marked even while the
        // workspace is folded (the session row says WARTET)
        if waiting {
            right -= 6.0;
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(right, rect.center().y - 3.0), vec2(6.0, 6.0)),
                0.0,
                theme::YELLOW,
            );
            right -= 6.0;
        }
    }
    let x = rect.left() + 36.0;
    let g = line_galley(
        ui,
        &h.workspace_name(p),
        widgets::font(14.0),
        name_color,
        right - x,
    );
    paint_line(ui, x, rect.center().y, g);
    if resp.clicked() {
        let on_chevron = resp
            .interact_pointer_pos()
            .is_some_and(|pos| pos.x < rect.left() + 16.0);
        if on_chevron {
            ops.push(Op::ToggleCollapsed(p.to_path_buf()));
        } else {
            ops.push(Op::Select(p.to_path_buf()));
        }
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if general {
        resp.on_hover_text(tr("Sessions without a project folder"));
    }
}

fn rect_at_right(row: Rect, right: f32, size: f32) -> Rect {
    Rect::from_min_size(
        pos2(right - size, row.center().y - size / 2.0),
        vec2(size, size),
    )
}

fn session_row(
    h: &mut Harness,
    ui: &mut egui::Ui,
    cwd: &Path,
    entry: &SidebarEntry,
    indent: f32,
    ops: &mut Vec<Op>,
) {
    let sid = &entry.session_id;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 27.0), Sense::click());
    let menu_id = ui.make_persistent_id(("session_menu", sid));
    let menu_open = ui.memory(|m| m.is_popup_open(menu_id));
    let hovered = ui.rect_contains_pointer(rect) || menu_open;
    let is_active = h.active.as_deref() == Some(sid.as_str()) && !h.artifacts_open;
    let row = Rect::from_min_max(pos2(rect.left() + indent, rect.top()), rect.max);
    paint_row_bg(ui, row, hovered, is_active);
    if indent > 0.0 {
        ui.painter().vline(
            rect.left() + indent / 2.0 + 1.0,
            rect.y_range(),
            Stroke::new(1.0_f32, theme::BORDER_SOFT),
        );
    }
    let state = h.session_state(sid);
    let mut x = row.left() + 10.0;
    match state {
        SessionState::Idle => {}
        SessionState::Running => {
            widgets::paint_busy(ui, pos2(x, rect.center().y - 2.0), 3.0);
            x += 16.0;
        }
        SessionState::Waiting => {
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(x, rect.center().y - 3.0), vec2(6.0, 6.0)),
                0.0,
                theme::YELLOW,
            );
            x += 12.0;
        }
        SessionState::Unseen => {
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(x, rect.center().y - 3.0), vec2(6.0, 6.0)),
                0.0,
                GRADIENT[1],
            );
            x += 12.0;
        }
        SessionState::Failed => {
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(x, rect.center().y - 3.0), vec2(6.0, 6.0)),
                0.0,
                theme::RED,
            );
            x += 12.0;
        }
    }
    // a session with routines gets a small clock
    if h.loops.get(sid).is_some_and(|l| !l.is_empty()) {
        icons::paint(
            ui.painter(),
            Icon::Clock,
            pos2(x, rect.center().y - 3.5),
            1.0,
            theme::DIM,
        );
        x += 14.0;
    }
    let mut right = rect.right() - 6.0;
    if hovered {
        let more = rect_at_right(rect, right, 22.0);
        right -= 26.0;
        let more_resp = widgets::place(
            ui,
            more,
            IconButton::new(Icon::Dots, tr("Session actions")).side(22.0),
        );
        widgets::toggle_popup(ui, &more_resp, menu_id);
        widgets::popup_menu(ui, menu_id, &more_resp, false, 190.0, |ui| {
            session_menu(
                ui,
                cwd,
                entry,
                h.pinned.contains(sid),
                h.archived.contains(sid),
                ops,
            );
        });
    } else if h.pinned.contains(sid) {
        // a pinned row keeps its marker even without hovering
        let pin = rect_at_right(rect, right, 14.0);
        widgets::place(ui, pin, IconButton::new(Icon::Pin, tr("Pinned")).side(14.0));
        right -= 18.0;
    } else {
        let updated = h
            .listed
            .get(cwd)
            .and_then(|l| l.iter().find(|s| &s.session_id == sid))
            .and_then(|s| s.updated_at.as_deref());
        let age = match state {
            SessionState::Waiting => Some(tr("WAITING").to_string()),
            SessionState::Running => Some(tr("RUNNING").to_string()),
            SessionState::Failed => Some(tr("FAILED").to_string()),
            _ => updated.and_then(|u| extras::relative_time(u, extras::now_epoch())),
        };
        if let Some(age) = age {
            let color = match state {
                SessionState::Waiting => theme::YELLOW,
                SessionState::Failed => theme::RED_TEXT,
                _ => theme::DIM,
            };
            let g = line_galley(ui, &age, mono(10.5), color, 60.0);
            right -= g.size().x;
            paint_line(ui, right, rect.center().y, g);
            right -= 8.0;
        }
    }
    let color = if is_active || hovered {
        theme::TEXT
    } else {
        theme::MUTED
    };
    let g = line_galley(ui, &entry.title, widgets::font(13.5), color, right - x);
    paint_line(ui, x, rect.center().y, g);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        ops.push(Op::Open(cwd.to_path_buf(), sid.clone()));
    }
    resp.context_menu(|ui| {
        ui.set_min_width(190.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        session_menu(
            ui,
            cwd,
            entry,
            h.pinned.contains(sid),
            h.archived.contains(sid),
            ops,
        );
    });
}

fn session_menu(
    ui: &mut egui::Ui,
    cwd: &Path,
    entry: &SidebarEntry,
    pinned: bool,
    archived: bool,
    ops: &mut Vec<Op>,
) {
    if widgets::menu_row(
        ui,
        Some(Icon::Pin),
        if pinned { tr("Unpin") } else { tr("Pin") },
        None,
        false,
    ) {
        ops.push(Op::TogglePin(entry.session_id.clone()));
        widgets::close_popup(ui);
        ui.close_menu();
    }
    if widgets::menu_row(ui, Some(Icon::Pencil), tr("Rename"), None, false) {
        ops.push(Op::Rename(
            entry.session_id.clone(),
            cwd.to_path_buf(),
            entry.title.clone(),
        ));
        widgets::close_popup(ui);
        ui.close_menu();
    }
    if !archived && widgets::menu_row(ui, Some(Icon::Eye), tr("Mark as unread"), None, false) {
        ops.push(Op::MarkUnread(entry.session_id.clone()));
        widgets::close_popup(ui);
        ui.close_menu();
    }
    if archived {
        if widgets::menu_row(ui, Some(Icon::Rewind), tr("Unarchive"), None, false) {
            ops.push(Op::Unarchive(entry.session_id.clone()));
            widgets::close_popup(ui);
            ui.close_menu();
        }
    } else if widgets::menu_row(ui, Some(Icon::Artifacts), tr("Archive"), None, false) {
        ops.push(Op::Archive(entry.session_id.clone()));
        widgets::close_popup(ui);
        ui.close_menu();
    }
    if widgets::menu_row(ui, Some(Icon::Trash), tr("Delete"), None, true) {
        ops.push(Op::Delete(
            entry.session_id.clone(),
            cwd.to_path_buf(),
            entry.title.clone(),
        ));
        widgets::close_popup(ui);
        ui.close_menu();
    }
}

fn more_row(ui: &mut egui::Ui, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(rect.left() + 20.0, rect.top()), rect.max),
            3.0,
            theme::HOVER,
        );
    }
    let g = line_galley(
        ui,
        label,
        widgets::font(12.5),
        theme::DIM,
        rect.width() - 40.0,
    );
    paint_line(ui, rect.left() + 30.0, rect.center().y, g);
    resp.clicked()
}

fn apply_ops(h: &mut Harness, ops: Vec<Op>) {
    for op in ops {
        match op {
            Op::Open(cwd, sid) => h.open_session(cwd, &sid),
            Op::NewIn(p) | Op::Select(p) => {
                h.select_workspace(p);
                h.new_session_view();
            }
            Op::ToggleCollapsed(p) => {
                if !h.ui.collapsed_projects.remove(&p) {
                    h.ui.collapsed_projects.insert(p);
                }
            }
            Op::ToggleAll(p) => {
                if !h.ui.expanded_projects.remove(&p) {
                    h.ui.expanded_projects.insert(p);
                }
            }
            Op::Rename(session_id, cwd, title) => {
                h.ui.rename = Some(crate::harness::state::RenameDialog {
                    session_id,
                    cwd,
                    title,
                });
            }
            Op::Delete(session_id, cwd, title) => {
                h.ui.confirm = Some(Confirm::DeleteSession {
                    session_id,
                    cwd,
                    title,
                });
            }
            Op::TogglePin(session_id) => h.toggle_pinned(&session_id),
            Op::MarkUnread(session_id) => h.mark_unread(&session_id),
            Op::Archive(session_id) => h.archive_session(&session_id),
            Op::Unarchive(session_id) => h.unarchive_session(&session_id),
            Op::Remove(p) => h.remove_confirm = Some(p),
            Op::DeleteWorktree {
                project,
                path,
                branch,
            } => {
                let trusted = h.git_trusted(&project);
                let dirty = Harness::worktree_dirty(&path, trusted);
                h.ui.worktree_delete = Some(crate::harness::state::WorktreeDelete {
                    project,
                    path,
                    branch,
                    dirty,
                    delete_branch: false,
                });
            }
            Op::AddProject => {
                h.folder_picker = Some(dialogs::FolderPicker::new(picker_start(h)));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// footer
// ---------------------------------------------------------------------------

fn footer(h: &mut Harness, ui: &mut egui::Ui, action: &mut HarnessAction) {
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(2.0);
    connection_line(h, ui);
    account_row(h, ui, action);
    ui.add_space(2.0);
}

fn connection_line(h: &mut Harness, ui: &mut egui::Ui) {
    // a Vibe outside the tested range: the version turns yellow and its
    // tooltip says why (the same text as the one-time toast)
    let compat_notice = h.vibe_compat.notice();
    let (text, color) = match &h.conn {
        Conn::Ready { agent } if compat_notice.is_some() => (agent.to_uppercase(), theme::YELLOW),
        Conn::Ready { agent } => (agent.to_uppercase(), theme::DIM),
        Conn::Starting => (tr("CONNECTING …").to_string(), theme::DIM),
        Conn::Off => ("OFFLINE".to_string(), theme::DIM),
        Conn::Reconnecting { attempt, .. } => (
            trf(
                "RECONNECTING ({attempt}/{max})",
                &[("attempt", attempt), ("max", &MAX_RECONNECTS)],
            ),
            theme::YELLOW,
        ),
        Conn::Failed(_) => (tr("VIBE NOT CONNECTED").to_string(), theme::RED_TEXT),
    };
    let dot = match &h.conn {
        Conn::Ready { .. } => theme::GREEN,
        Conn::Failed(_) => theme::RED,
        Conn::Reconnecting { .. } => theme::YELLOW,
        _ => theme::DIM,
    };
    let mut retry = false;
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
        ui.painter().rect_filled(r, 0.0, dot);
        let label = ui.add(
            egui::Label::new(RichText::new(text).monospace().size(10.5).color(color)).truncate(),
        );
        let hover = match &h.conn {
            Conn::Ready { .. } => compat_notice,
            Conn::Failed(msg) => Some(msg.clone()),
            _ => None,
        };
        if let Some(hover) = hover {
            label.on_hover_text(hover);
        }
        if let Conn::Failed(_) = &h.conn {
            if ui
                .add(IconButton::new(Icon::Refresh, tr("Reconnect")).side(20.0))
                .clicked()
            {
                retry = true;
            }
        }
    });
    if retry {
        h.retry_connection();
    }
}

fn account_row(h: &mut Harness, ui: &mut egui::Ui, action: &mut HarnessAction) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let popup = ui.make_persistent_id("account_popup");
    let hovered = resp.hovered() || ui.memory(|m| m.is_popup_open(popup));
    paint_row_bg(ui, rect, hovered, false);
    icons::paint(
        ui.painter(),
        Icon::User,
        pos2(rect.left() + 8.0, rect.center().y - 7.0),
        2.0,
        theme::MUTED,
    );
    let name = h
        .identity
        .as_ref()
        .and_then(|i| i.name.clone().or_else(|| i.email.clone()))
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| tr("Account").to_string());
    let gear = rect_at_right(rect, rect.right() - 4.0, 26.0);
    let x = rect.left() + 30.0;
    let g = line_galley(
        ui,
        &name,
        widgets::font(13.5),
        theme::TEXT,
        gear.left() - x - 4.0,
    );
    paint_line(ui, x, rect.top() + 12.0, g);
    let has_meter = h
        .account
        .as_ref()
        .is_some_and(|a| a.usage_percent.is_some());
    if !has_meter {
        let second = h
            .account
            .as_ref()
            .and_then(|a| a.plan.clone())
            .unwrap_or_else(|| tr("Show account").to_string());
        let g = line_galley(
            ui,
            &second,
            widgets::font(12.0),
            theme::DIM,
            gear.left() - x,
        );
        paint_line(ui, x, rect.top() + 27.0, g);
    }
    if let Some(pct) = h.account.as_ref().and_then(|a| a.usage_percent) {
        paint_budget(ui, pos2(x, rect.top() + 24.0), pct, gear.left() - x - 8.0);
    }
    let gear_resp = widgets::place(
        ui,
        gear,
        IconButton::new(Icon::Gear, tr("Settings (Ctrl ,)")),
    );
    if gear_resp.clicked() {
        *action = HarnessAction::OpenSettings;
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() && !gear_resp.hovered() {
        ui.memory_mut(|m| m.toggle_popup(popup));
        if h.account.is_none() {
            h.load_account();
        }
    }
    widgets::popup_menu(ui, popup, &resp, true, 260.0, |ui| {
        account_popup(h, ui, action)
    });
}

/// A short meter of the monthly budget with the exact percentage.
fn paint_budget(ui: &egui::Ui, origin: egui::Pos2, percent: f64, width: f32) {
    // named, so the bare number does not leave "12 % of what?" open
    let label = trf(
        "Quota {pct} %",
        &[("pct", &i18n::decimal(format!("{:.1}", percent)))],
    );
    let g = line_galley(ui, &label, mono(10.5), theme::DIM, 100.0);
    let meter_w = (width - g.size().x - 6.0).max(20.0);
    let blocks = 16;
    let block = ((meter_w + 2.0) / blocks as f32 - 2.0).clamp(2.0, 5.0);
    let filled = theme::meter_blocks((percent / 100.0) as f32, blocks);
    for i in 0..blocks {
        let color = if i >= filled {
            theme::RAISED
        } else if percent >= 90.0 {
            theme::RED
        } else {
            theme::ORANGE_DIM
        };
        ui.painter().rect_filled(
            Rect::from_min_size(
                origin + vec2(i as f32 * (block + 2.0), 2.0),
                vec2(block, 5.0),
            ),
            0.0,
            color,
        );
    }
    ui.painter().galley(
        origin + vec2(blocks as f32 * (block + 2.0) + 4.0, -1.0),
        g,
        theme::DIM,
    );
}

fn account_popup(h: &mut Harness, ui: &mut egui::Ui, action: &mut HarnessAction) {
    ui.add_space(4.0);
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(theme::mono_label(tr("Account")));
            if let Some(i) = &h.identity {
                if let Some(email) = &i.email {
                    ui.add(egui::Label::new(RichText::new(email).size(13.5)).truncate());
                }
                if let Some(w) = &i.workspace {
                    ui.label(RichText::new(w).color(theme::DIM).small());
                }
            }
            match &h.account {
                Some(a) => {
                    if let Some(plan) = &a.plan {
                        ui.label(RichText::new(trf("Plan: {plan}", &[("plan", plan)])).size(13.5));
                    }
                    match a.usage_percent {
                        Some(pct) => {
                            let label = i18n::decimal(trf(
                                "{pct} % of the monthly quota",
                                &[("pct", &format!("{pct:.1}"))],
                            ));
                            ui.label(RichText::new(label).size(13.0).color(theme::MUTED));
                            theme::pixel_meter(
                                ui,
                                theme::meter_blocks((pct / 100.0) as f32, 20),
                                20,
                                8.0,
                            );
                            if let Some(date) = a.reset_at.as_deref().and_then(extras::local_date) {
                                ui.label(
                                    RichText::new(trf("New quota on {date}", &[("date", &date)]))
                                        .color(theme::DIM)
                                        .small(),
                                );
                            }
                            if a.payg {
                                ui.label(
                                    RichText::new(tr(
                                        "Pay-as-you-go active: usage above 100 % is billed.",
                                    ))
                                    .color(theme::YELLOW)
                                    .small(),
                                );
                            }
                        }
                        None => {
                            ui.label(
                                RichText::new(tr("No quota reported."))
                                    .color(theme::DIM)
                                    .small(),
                            );
                        }
                    }
                }
                None if h.account_loading() => {
                    ui.label(RichText::new(tr("Loading …")).color(theme::DIM).small());
                }
                None => {
                    ui.label(
                        RichText::new(tr("Not loaded yet."))
                            .color(theme::DIM)
                            .small(),
                    );
                }
            }
        });
    ui.add_space(4.0);
    if widgets::menu_row(ui, Some(Icon::Refresh), tr("Refresh"), None, false) {
        h.load_account();
    }
    if widgets::menu_row(ui, Some(Icon::Gear), tr("Settings"), None, false) {
        *action = HarnessAction::OpenAccount;
        widgets::close_popup(ui);
    }
    ui.add_space(2.0);
}

// ---------------------------------------------------------------------------
// icon rail (sidebar collapsed or window narrow)
// ---------------------------------------------------------------------------

fn rail(h: &mut Harness, ctx: &egui::Context, action: &mut HarnessAction) {
    egui::SidePanel::left("harness_rail")
        .exact_width(46.0)
        .resizable(false)
        .frame(
            egui::Frame::none()
                .fill(theme::SIDEBAR)
                .inner_margin(egui::Margin::symmetric(8.0, 10.0)),
        )
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                if ui
                    .add(IconButton::new(
                        Icon::PanelLeft,
                        tr("Expand sidebar (Ctrl B)"),
                    ))
                    .clicked()
                {
                    h.ui.sidebar_collapsed = false;
                }
                ui.add_space(6.0);
                if ui
                    .add(IconButton::new(Icon::Plus, tr("New session (Ctrl N)")))
                    .clicked()
                {
                    h.new_session_view();
                }
                if ui
                    .add(IconButton::new(Icon::Search, tr("Search (Ctrl K)")))
                    .clicked()
                {
                    h.ui.palette = Some(Default::default());
                }
                if ui
                    .add(IconButton::new(Icon::Artifacts, tr("Artifacts")))
                    .clicked()
                {
                    h.artifacts_open = true;
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if ui
                        .add(IconButton::new(Icon::Gear, tr("Settings (Ctrl ,)")))
                        .clicked()
                    {
                        *action = HarnessAction::OpenSettings;
                    }
                });
            });
        });
}
