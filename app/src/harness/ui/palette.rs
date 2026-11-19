//! The command palette (Strg K): search over sessions and projects, app
//! actions and Vibe's slash commands, fully usable from the keyboard.

use super::*;
use crate::harness::{extras, filter_entries, matches_query};
use crate::i18n::tr;
use crate::icons;
use crate::widgets::{line_galley, paint_line};
use egui::{pos2, Id, Rect, Sense};

pub(crate) enum Action {
    File(PathBuf, String),
    NewSession,
    ToggleSidebar,
    TogglePanel,
    Settings,
    Artifacts,
    Fork,
    Open(PathBuf, String),
    Project(PathBuf),
    AddProject,
    Command(String),
    Worktree,
}

pub(crate) struct Entry {
    pub icon: Icon,
    pub label: String,
    pub detail: String,
    pub action: Action,
}

pub(crate) fn entries(h: &mut Harness, query: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut push = |icon, label: &str, detail: &str, action| {
        if matches_query(label, query) || matches_query(detail, query) {
            out.push(Entry {
                icon,
                label: label.to_string(),
                detail: detail.to_string(),
                action,
            });
        }
    };
    let has_session = h.active.is_some();
    push(
        Icon::Plus,
        tr("New session"),
        tr("Ctrl N"),
        Action::NewSession,
    );
    push(
        Icon::Plus,
        tr("Add project"),
        tr("Choose a folder"),
        Action::AddProject,
    );
    push(
        Icon::Artifacts,
        tr("Open Artifacts"),
        tr("Edited files"),
        Action::Artifacts,
    );
    push(
        Icon::Branch,
        tr("Worktree for project"),
        tr("Isolated copy"),
        Action::Worktree,
    );
    push(Icon::Gear, tr("Settings"), tr("Ctrl ,"), Action::Settings);
    push(
        Icon::PanelLeft,
        tr("Toggle sidebar"),
        tr("Ctrl B"),
        Action::ToggleSidebar,
    );
    if has_session {
        push(
            Icon::PanelRight,
            tr("Toggle work panel"),
            tr("Ctrl J"),
            Action::TogglePanel,
        );
        push(Icon::Fork, tr("Fork session"), "Fork", Action::Fork);
    }
    for c in h.available_commands() {
        let label = format!("/{}", c.name);
        let detail = extras::command_description(&c.name, &c.description);
        push(
            Icon::Terminal,
            &label,
            &detail,
            Action::Command(c.name.clone()),
        );
    }
    // files of the current project (git ls-files, cached by the watcher;
    // fuzzy match like the @-mention in the composer) — only while
    // searching: unfiltered, a dozen arbitrary files would push the
    // sessions down
    let cwd = h
        .active
        .as_deref()
        .and_then(|s| h.sessions.get(s))
        .map(|s| s.cwd.clone())
        .or_else(|| h.current_project.clone());
    if let Some(cwd) = cwd.as_ref().filter(|_| !query.trim().is_empty()) {
        let name = h.workspace_name(cwd);
        for rel in crate::harness::watch::filter_files(h.watcher.files(cwd), query, 12) {
            out.push(Entry {
                icon: Icon::File,
                label: rel.to_string(),
                detail: name.clone(),
                action: Action::File(cwd.clone(), rel.to_string()),
            });
        }
    }
    // the sessions of every workspace ("Allgemein" first, then the
    // projects); while searching they come before the actions
    let actions = std::mem::take(&mut out);
    for dir in h.workspaces() {
        let general = h.is_chat_dir(&dir);
        let name = h.workspace_name(&dir);
        let rows = h.sidebar_rows(&dir, tr("New session"));
        let Some(rows) = filter_entries(&name, rows, query) else {
            continue;
        };
        for row in rows {
            out.push(Entry {
                icon: Icon::Chat,
                label: row.title.clone(),
                detail: name.clone(),
                action: Action::Open(dir.clone(), row.session_id.clone()),
            });
        }
        if matches_query(&name, query) {
            out.push(Entry {
                icon: if general { Icon::Chat } else { Icon::Folder },
                label: name.clone(),
                detail: if general {
                    tr("Sessions without a project folder").to_string()
                } else {
                    tr("Project").to_string()
                },
                action: Action::Project(dir.clone()),
            });
        }
    }
    if query.trim().is_empty() {
        let mut all = actions;
        all.append(&mut out);
        all
    } else {
        out.extend(actions);
        out
    }
}

pub(crate) fn apply(h: &mut Harness, action: Action, out: &mut HarnessAction) {
    match action {
        Action::File(cwd, rel) => {
            // Enter opens the preview: beside the transcript in the session,
            // in the Artifacts view when no session is open
            if let Some(sid) = h.active.clone() {
                h.open_preview(&sid, &rel);
            } else {
                h.artifacts_open = true;
                h.artifact_selected = Some(cwd.join(&rel).to_string_lossy().into_owned());
            }
        }
        Action::NewSession => h.new_session_view(),
        Action::ToggleSidebar => h.ui.sidebar_collapsed = !h.ui.sidebar_collapsed,
        Action::TogglePanel => toggle_panel(h),
        Action::Settings => *out = HarnessAction::OpenSettings,
        Action::Artifacts => h.artifacts_open = true,
        Action::Fork => h.fork_active_session(),
        Action::Worktree => {
            // create dialog with the optional start ref
            if h.current_project.is_some() {
                h.ui.worktree_dialog = Some(crate::harness::state::WorktreeDialog {
                    start_ref: String::new(),
                });
            } else {
                h.status = tr("No project open.").to_string();
            }
        }
        Action::Open(cwd, sid) => h.open_session(cwd, &sid),
        Action::Project(p) => {
            h.select_workspace(p);
            h.new_session_view();
        }
        Action::AddProject => {
            h.folder_picker = Some(dialogs::FolderPicker::new(picker_start(h)));
        }
        Action::Command(name) => {
            h.draft = format!("/{name} ");
            h.ui.focus_composer = true;
        }
    }
}

pub(super) fn show(h: &mut Harness, ctx: &egui::Context, out: &mut HarnessAction) {
    let Some(mut state) = h.ui.palette.take() else {
        return;
    };
    let list = entries(h, &state.query);
    state.selected = state.selected.min(list.len().saturating_sub(1));
    let mut close = false;
    let mut chosen: Option<usize> = None;

    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) && !list.is_empty() {
            state.selected = (state.selected + 1) % list.len();
            state.scroll_to_selected = true;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) && !list.is_empty() {
            state.selected = (state.selected + list.len() - 1) % list.len();
            state.scroll_to_selected = true;
        }
        if i.consume_key(Modifiers::NONE, Key::Enter) && !list.is_empty() {
            chosen = Some(state.selected);
        }
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            close = true;
        }
    });

    let screen = ctx.screen_rect();
    let backdrop = egui::Area::new(Id::new("palette_backdrop"))
        .order(egui::Order::Middle)
        .anchor(egui::Align2::LEFT_TOP, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(150));
            resp.clicked()
        });
    if backdrop.inner {
        close = true;
    }
    egui::Area::new(Id::new("palette"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(pos2(screen.center().x, (screen.height() * 0.14).min(120.0)))
        .show(ctx, |ui| {
            let width = 620.0_f32.min(screen.width() - 32.0);
            egui::Frame::popup(ui.style())
                .fill(theme::CARD)
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        icons::paint(
                            ui.painter(),
                            Icon::Search,
                            ui.cursor().min + vec2(2.0, 3.0),
                            2.0,
                            theme::DIM,
                        );
                        ui.add_space(22.0);
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut state.query)
                                .id(Id::new("palette_input"))
                                .frame(false)
                                .font(widgets::font(15.0))
                                .desired_width(f32::INFINITY)
                                .hint_text(tr("Search sessions, projects or commands …")),
                        );
                        resp.request_focus();
                        if resp.changed() {
                            state.selected = 0;
                            state.scroll_to_selected = true;
                        }
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(380.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if list.is_empty() {
                                ui.add_space(8.0);
                                ui.label(RichText::new(tr("No matches.")).color(theme::DIM));
                                ui.add_space(8.0);
                            }
                            for (i, e) in list.iter().enumerate() {
                                let (rect, resp) = ui.allocate_exact_size(
                                    vec2(ui.available_width(), 32.0),
                                    Sense::click(),
                                );
                                let selected = i == state.selected;
                                if selected || resp.hovered() {
                                    ui.painter().rect_filled(rect, 3.0, theme::RAISED);
                                }
                                if selected {
                                    if std::mem::take(&mut state.scroll_to_selected) {
                                        ui.scroll_to_rect(rect, None);
                                    }
                                    ui.painter().rect_filled(
                                        Rect::from_min_size(
                                            rect.min + vec2(0.0, 7.0),
                                            vec2(2.0, 18.0),
                                        ),
                                        0.0,
                                        theme::ORANGE,
                                    );
                                }
                                icons::paint(
                                    ui.painter(),
                                    e.icon,
                                    pos2(rect.left() + 10.0, rect.center().y - 7.0),
                                    2.0,
                                    theme::MUTED,
                                );
                                let mut right = rect.right() - 10.0;
                                if !e.detail.is_empty() {
                                    let g = line_galley(
                                        ui,
                                        &e.detail,
                                        widgets::font(12.5),
                                        theme::DIM,
                                        rect.width() * 0.45,
                                    );
                                    right -= g.size().x;
                                    paint_line(ui, right, rect.center().y, g);
                                    right -= 12.0;
                                }
                                let x = rect.left() + 32.0;
                                let g = line_galley(
                                    ui,
                                    &e.label,
                                    widgets::font(14.0),
                                    theme::TEXT,
                                    right - x,
                                );
                                paint_line(ui, x, rect.center().y, g);
                                if resp.clicked() {
                                    chosen = Some(i);
                                }
                            }
                        });
                });
        });

    if let Some(i) = chosen {
        if let Some(entry) = list.into_iter().nth(i) {
            apply(h, entry.action, out);
        }
        h.ui.palette = None;
        return;
    }
    if close {
        h.ui.palette = None;
        h.ui.focus_composer = true;
        return;
    }
    h.ui.palette = Some(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(h: &mut Harness, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            let mut action = HarnessAction::None;
            show(h, ctx, &mut action);
        });
    }

    fn arrow_down() -> egui::Event {
        egui::Event::Key {
            key: Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    /// Scrolling to the selection happens once per key press; repeating it
    /// every frame pulled the list back while the mouse wheel moved it.
    #[test]
    fn the_keyboard_scrolls_once_and_leaves_the_wheel_alone() {
        let ctx = egui::Context::default();
        let mut h = Harness::new(Vec::new(), "ask".to_string());
        h.ui.palette = Some(Default::default());
        frame(&mut h, &ctx, vec![arrow_down()]);
        let state = h.ui.palette.as_ref().expect("still open");
        assert_eq!(state.selected, 1);
        assert!(!state.scroll_to_selected, "the request was used this frame");
        frame(&mut h, &ctx, Vec::new());
        let state = h.ui.palette.as_ref().expect("still open");
        assert!(!state.scroll_to_selected, "no new request without a key");
    }
}
