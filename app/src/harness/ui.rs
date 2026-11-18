//! egui rendering of the harness: the shell and its keyboard shortcuts. The
//! parts live in submodules (sidebar, header, transcript, composer, cards,
//! right panel, command palette, artifacts). Everything shown comes from
//! real ACP events folded into `Harness` (harness/mod.rs); UI strings are
//! German.

mod activity;
mod artifacts;
mod cards;
mod composer;
mod header;
mod palette;
mod panel;
mod sidebar;
/// The transcript renderer; `pub(crate)` so the harness tests can check
/// its fold-state key rules directly.
pub(crate) mod transcript;

use super::state::Confirm;
use super::{project_name, Harness, SessionState};
use crate::acp::session::TurnState;
use crate::dialogs::{self, ModalClose, PickerResult};
use crate::i18n::{tr, trf};
use crate::icons::Icon;
use crate::theme;
use crate::widgets::{self, ButtonKind, IconButton};
use egui::{vec2, Key, Modifiers, RichText};
use std::path::PathBuf;
use std::time::Duration;

/// Below this window width the sidebar folds into its icon rail.
const COMPACT_WIDTH: f32 = 880.0;
/// Below this window width the right panel stays hidden.
const PANEL_MIN_WINDOW: f32 = 1120.0;
/// Width of the conversation column.
pub(crate) const COLUMN_WIDTH: f32 = 820.0;

/// What the harness UI wants the app shell to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessAction {
    None,
    OpenSettings,
    OpenAccount,
}

/// Is the right panel on screen in this frame? The file preview pane takes
/// its place while it is open.
fn panel_visible(h: &Harness, ctx: &egui::Context) -> bool {
    h.active.is_some()
        && !h.artifacts_open
        && !h.ui.activity_open
        && h.ui.preview.is_none()
        && !h.ui.panel_closed
        && ctx.screen_rect().width() >= PANEL_MIN_WINDOW
}

fn toggle_panel(h: &mut Harness) {
    h.ui.panel_closed = !h.ui.panel_closed;
}

/// A dialog owned by the harness or the app shell has the keyboard.
fn modal_open(h: &Harness) -> bool {
    h.ui.settings_open
        || h.ui.palette.is_some()
        || h.ui.rename.is_some()
        || h.ui.rewind.is_some()
        || h.ui.confirm.is_some()
        || h.ui.file_picker.is_some()
        || h.folder_picker.is_some()
        || h.remove_confirm.is_some()
}

/// May Strg-K toggle the command palette in this frame? Only when the
/// keyboard belongs to it: no modal dialog is open, or the palette itself
/// is the modal that has it (so it can always be closed again).
fn palette_shortcut_allowed(h: &Harness) -> bool {
    h.ui.palette.is_some() || !modal_open(h)
}

fn active_running(h: &Harness) -> bool {
    h.active
        .as_deref()
        .and_then(|sid| h.sessions.get(sid))
        .is_some_and(|s| s.turn != TurnState::Idle)
}

pub fn show(h: &mut Harness, ctx: &egui::Context) -> HarnessAction {
    let mut action = HarnessAction::None;
    swap_draft(h);
    shortcuts(h, ctx, &mut action);
    sidebar::show(h, ctx, &mut action);
    if h.ui.preview.is_some() && !h.artifacts_open && h.active.is_some() {
        artifacts::preview_panel(h, ctx);
    } else if panel_visible(h, ctx) {
        panel::show(h, ctx);
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(theme::BG))
        .show(ctx, |ui| {
            if h.artifacts_open {
                artifacts::view(h, ui);
                return;
            }
            if h.ui.activity_open {
                activity::view(h, ui);
                return;
            }
            match h.active.clone().filter(|s| h.sessions.contains_key(s)) {
                None => composer::start_screen(h, ui),
                Some(sid) => session_view(h, ui, &sid),
            }
        });
    palette::show(h, ctx, &mut action);
    toast(h, ctx);
    window_state(h, ctx);
    escape_cancels(h, ctx);
    if h.worktree_running() || h.sessions.values().any(|s| s.turn != TurnState::Idle) {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    h.handle_ui_events();
    action
}

/// One open session: header, transcript, pinned cards and composer.
fn session_view(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    header::show(h, ui, sid);
    egui::TopBottomPanel::bottom("harness_composer_panel")
        .frame(egui::Frame::none().fill(theme::BG))
        .show_separator_line(false)
        .show_inside(ui, |ui| {
            ui.add_space(4.0);
            column(ui, |ui| {
                cards::pinned(h, ui, sid);
                composer::composer(h, ui);
            });
            ui.add_space(8.0);
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(theme::BG))
        .show_inside(ui, |ui| transcript::show(h, ui, sid));
}

/// Centre `add` in a column of at most `COLUMN_WIDTH`.
pub(super) fn column<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let avail = ui.available_width();
    let w = COLUMN_WIDTH.min((avail - 32.0).max(120.0));
    let pad = ((avail - w) / 2.0).max(0.0);
    egui::Frame::none()
        .inner_margin(egui::Margin {
            left: pad,
            right: pad,
            top: 0.0,
            bottom: 0.0,
        })
        .show(ui, |ui| {
            ui.set_width(w);
            add(ui)
        })
        .inner
}

// ---------------------------------------------------------------------------
// keyboard
// ---------------------------------------------------------------------------

fn shortcuts(h: &mut Harness, ctx: &egui::Context, action: &mut HarnessAction) {
    let pressed = |key: Key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key));
    // Ctrl K opens the palette, but only when the keyboard is free: an
    // open dialog (settings, rename, rewind, a picker…) is the one focus
    // owner of the frame — the palette must not open on top of it
    // (review N15). The palette itself may always be closed, dialog or
    // not, so it can never get stuck.
    if pressed(Key::K) && palette_shortcut_allowed(h) {
        h.ui.palette = match h.ui.palette {
            Some(_) => None,
            None => Some(Default::default()),
        };
    }
    if modal_open(h) {
        return;
    }
    // typing anywhere goes to the composer
    let typing = ctx.input(|i| {
        !i.modifiers.command
            && i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Text(t) if !t.trim().is_empty()))
    });
    if typing && !h.artifacts_open && !h.ui.activity_open && ctx.memory(|m| m.focused().is_none()) {
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("harness_composer")));
    }
    if pressed(Key::N) {
        h.new_session_view();
    }
    if pressed(Key::B) {
        h.ui.sidebar_collapsed = !h.ui.sidebar_collapsed;
    }
    if pressed(Key::J) {
        toggle_panel(h);
    }
    if pressed(Key::Comma) {
        *action = HarnessAction::OpenSettings;
    }
    if pressed(Key::L) {
        h.ui.focus_composer = true;
    }
}

/// The keyboard is free for the app: nothing or the composer has the focus
/// (not the routine form or a question's text field).
fn composer_or_no_focus(ctx: &egui::Context) -> bool {
    ctx.memory(|m| {
        m.focused()
            .is_none_or(|id| id == egui::Id::new("harness_composer"))
    })
}

/// Esc stops the running turn, unless something else is waiting for it (a
/// dialog, the palette, an open menu or the composer's list).
fn escape_cancels(h: &mut Harness, ctx: &egui::Context) {
    if h.ui.composer_used_escape {
        h.ui.composer_used_escape = false;
        return;
    }
    let pressed = ctx.input(|i| i.key_pressed(Key::Escape));
    if pressed
        && active_running(h)
        && !modal_open(h)
        && !ctx.memory(|m| m.any_popup_open())
        && composer_or_no_focus(ctx)
    {
        h.cancel();
        h.ui.focus_composer = true;
    }
}

// ---------------------------------------------------------------------------
// window title, attention, toasts
// ---------------------------------------------------------------------------

/// Every session (and the start screen) keeps its own draft and attachments.
/// `pub(super)`: the harness tests exercise the parked drafts through this
/// swap (the same function the frames run).
pub(super) fn swap_draft(h: &mut Harness) {
    if h.ui.draft_owner == h.active {
        return;
    }
    let current = (
        std::mem::take(&mut h.draft),
        std::mem::take(&mut h.attachments),
    );
    let owner = std::mem::replace(&mut h.ui.draft_owner, h.active.clone());
    if !current.0.is_empty() || !current.1.is_empty() {
        h.ui.parked_drafts.insert(owner, current);
    }
    if let Some((draft, attachments)) = h.ui.parked_drafts.remove(&h.active) {
        h.draft = draft;
        h.attachments = attachments;
    }
}

fn window_state(h: &mut Harness, ctx: &egui::Context) {
    let states: Vec<SessionState> = h.sessions.keys().map(|s| h.session_state(s)).collect();
    let waiting = states.contains(&SessionState::Waiting) || !h.orphan_permissions.is_empty();
    let running = states.contains(&SessionState::Running);
    let title = if waiting {
        tr("■ Waiting for you – Catwright")
    } else if running {
        tr("Vibe is working … – Catwright")
    } else {
        "Catwright"
    };
    if h.ui.window_title != title {
        h.ui.window_title = title.to_string();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.to_string()));
    }
    // ask for attention once per finished turn or waiting request
    let focused = ctx.input(|i| i.focused);
    let turn_ended = h.ui.was_running && !running;
    h.ui.was_running = running;
    if focused {
        h.ui.attention_sent = false;
    } else if (waiting || turn_ended) && !h.ui.attention_sent {
        h.ui.attention_sent = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Informational,
        ));
    }
}

/// Status lines that describe a failure stay until they are dismissed.
/// Matched by wording in both UI languages: a status keeps the language it
/// was written in when the user switches languages.
fn is_error_status(text: &str) -> bool {
    [
        "failed",
        "Error",
        "error",
        "not signed in",
        "not connected",
        "interrupted",
        "could not",
        "not possible",
        "not saved",
        "fehlgeschlagen",
        "Fehler",
        "nicht angemeldet",
        "nicht verbunden",
        "unterbrochen",
        "konnte nicht",
        "nicht möglich",
        "nicht gespeichert",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// The status line as a toast above the content: info fades after six
/// seconds, errors stay until dismissed.
fn toast(h: &mut Harness, ctx: &egui::Context) {
    if h.status != h.ui.toast.text {
        h.ui.toast.text = h.status.clone();
        h.ui.toast.since = (!h.status.is_empty()).then(std::time::Instant::now);
    }
    if h.status.is_empty() {
        return;
    }
    let error = is_error_status(&h.status);
    if !error {
        let age =
            h.ui.toast
                .since
                .map_or(Duration::ZERO, |t: std::time::Instant| t.elapsed());
        if age >= Duration::from_secs(6) {
            h.status.clear();
            return;
        }
        ctx.request_repaint_after(Duration::from_secs(6) - age);
    }
    let screen = ctx.screen_rect();
    let mut close = false;
    egui::Area::new(egui::Id::new("harness_toast"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(screen.center().x, 54.0))
        .show(ctx, |ui| {
            ui.set_max_width((screen.width() - 80.0).min(640.0));
            widgets::attention_card(ui, if error { theme::RED } else { theme::ORANGE }, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&h.status).color(theme::TEXT).size(13.5))
                            .wrap(),
                    );
                    ui.add_space(4.0);
                    if ui
                        .add(IconButton::new(Icon::Close, tr("Close")).side(22.0))
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        });
    if close {
        h.status.clear();
    }
}

// ---------------------------------------------------------------------------
// dialogs
// ---------------------------------------------------------------------------

/// The harness' modal dialogs, rendered on top of everything. The app shell
/// calls this exactly once per frame (after the settings modal), so the
/// dialogs also work when the settings window is open.
pub fn show_dialogs(h: &mut Harness, ctx: &egui::Context) {
    if let Some(picker) = h.folder_picker.as_mut() {
        match picker.show(ctx) {
            Some(PickerResult::Picked(p)) => {
                let path = p.to_string_lossy().into_owned();
                h.folder_picker = None;
                h.add_project(&path);
            }
            Some(PickerResult::Cancelled) => h.folder_picker = None,
            None => {}
        }
    }
    if let Some(picker) = h.ui.file_picker.as_mut() {
        match picker.show(ctx) {
            Some(PickerResult::Picked(p)) => {
                h.ui.file_picker = None;
                if !h.attachments.contains(&p) {
                    h.attachments.push(p);
                }
                h.ui.focus_composer = true;
            }
            Some(PickerResult::Cancelled) => h.ui.file_picker = None,
            None => {}
        }
    }
    if let Some(p) = h.remove_confirm.clone() {
        let text = trf(
            "Remove “{name}” from the list? The files in the folder stay untouched.",
            &[("name", &project_name(&p))],
        );
        match dialogs::confirm(
            ctx,
            "harness_remove_project",
            tr("Remove project"),
            &text,
            tr("Remove"),
            true,
        ) {
            Some(true) => {
                h.remove_project(&p);
                h.remove_confirm = None;
            }
            Some(false) => h.remove_confirm = None,
            None => {}
        }
    }
    rename_dialog(h, ctx);
    rewind_dialog(h, ctx);
    confirm_dialog(h, ctx);
}

fn rename_dialog(h: &mut Harness, ctx: &egui::Context) {
    let Some(mut d) = h.ui.rename.take() else {
        return;
    };
    let mut done = false;
    let mut apply = false;
    let (_r, close) = dialogs::modal(ctx, "harness_rename", tr("Rename session"), |ui| {
        ui.set_min_width(420.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut d.title)
                .desired_width(f32::INFINITY)
                .hint_text(tr("New title")),
        );
        resp.request_focus();
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            apply = true;
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, tr("Cancel"), ButtonKind::Secondary).clicked() {
                done = true;
            }
            if widgets::button(ui, tr("Save"), ButtonKind::Primary).clicked() {
                apply = true;
            }
        });
    });
    if apply {
        h.rename_session(&d.session_id, &d.cwd, &d.title);
    } else if !done && close == ModalClose::Open {
        h.ui.rename = Some(d);
    }
}

fn rewind_dialog(h: &mut Harness, ctx: &egui::Context) {
    let Some(mut d) = h.ui.rewind.take() else {
        return;
    };
    let mut done = false;
    let mut apply = false;
    let (_r, close) = dialogs::modal(ctx, "harness_rewind", tr("Rewind to here"), |ui| {
        ui.set_max_width(460.0);
        ui.label(
            RichText::new(
                "Die Unterhaltung wird auf den Stand vor dieser Nachricht gesetzt. \
                 Die Nachricht steht danach wieder im Eingabefeld.",
            )
            .size(14.0),
        );
        ui.add_space(8.0);
        if d.paths.is_empty() {
            ui.label(
                RichText::new(tr("No files affected."))
                    .color(theme::DIM)
                    .small(),
            );
        } else {
            ui.checkbox(
                &mut d.restore_files,
                if d.paths.len() == 1 {
                    tr("Restore 1 file").to_string()
                } else {
                    trf("Restore {n} files", &[("n", &d.paths.len())])
                },
            );
            for p in d.paths.iter().take(8) {
                ui.add(
                    egui::Label::new(RichText::new(p).monospace().size(12.0).color(theme::MUTED))
                        .truncate(),
                );
            }
            if d.paths.len() > 8 {
                ui.label(
                    RichText::new(trf("… and {n} more", &[("n", &(d.paths.len() - 8))]))
                        .color(theme::DIM)
                        .small(),
                );
            }
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, tr("Cancel"), ButtonKind::Secondary).clicked() {
                done = true;
            }
            if widgets::button(ui, tr("Rewind"), ButtonKind::Primary).clicked() {
                apply = true;
            }
        });
    });
    if apply {
        h.confirm_rewind(&d.session_id, &d.message_id, d.restore_files);
    } else if !done && close == ModalClose::Open {
        h.ui.rewind = Some(d);
    }
}

fn confirm_dialog(h: &mut Harness, ctx: &egui::Context) {
    let Some(c) = h.ui.confirm.clone() else {
        return;
    };
    let (title, text, label) = match &c {
        Confirm::DeleteSession { title, .. } => (
            tr("Delete session"),
            trf(
                "Delete “{title}” for good? This cannot be undone.",
                &[("title", title)],
            ),
            tr("Delete"),
        ),
        Confirm::RevertFile { path, .. } => (
            tr("Revert changes"),
            trf(
                "Revert all open changes to “{file}”? The file is reset in the project.",
                &[("file", &crate::artifacts::file_name(path))],
            ),
            tr("Revert"),
        ),
        Confirm::RevertAll { files, .. } => (
            tr("Revert all changes"),
            trf(
                "Revert open changes to {files} file(s)? The files are reset in the project.",
                &[("files", files)],
            ),
            tr("Revert all"),
        ),
        Confirm::AutoApprove { .. } => (
            tr("Run automatically"),
            tr("Let Vibe run without asking? File changes and commands are no longer confirmed.")
                .to_string(),
            tr("Automatic"),
        ),
    };
    match dialogs::confirm(ctx, "harness_confirm", title, &text, label, true) {
        Some(true) => {
            h.ui.confirm = None;
            match c {
                Confirm::DeleteSession {
                    session_id, cwd, ..
                } => h.delete_session(&session_id, &cwd),
                Confirm::RevertFile { session_id, path } => {
                    h.review_action(&session_id, false, Harness::review_file_target(&path));
                }
                Confirm::RevertAll { session_id, .. } => {
                    h.review_action(&session_id, false, Harness::review_all_target());
                }
                Confirm::AutoApprove { session_id } => match session_id {
                    Some(sid) => h.set_mode(&sid, "auto-approve"),
                    None => h.start_mode = "auto-approve".to_string(),
                },
            }
        }
        Some(false) => h.ui.confirm = None,
        None => {}
    }
}

/// Open the folder containing `path` in the file manager (never the file
/// itself: opening an agent-written file could run it).
pub(super) fn open_folder_of(path: &str) {
    let dir = std::path::Path::new(path);
    let dir = if dir.is_dir() {
        dir
    } else {
        dir.parent().unwrap_or(dir)
    };
    if dir.is_dir() {
        spawn_xdg_open(&dir.to_string_lossy());
    }
}

/// Start `xdg-open` on its own reaping thread (review N11): dropping the
/// child after `spawn` left one zombie per click until app exit. The
/// child is spawned inside the thread and waited for there, so neither
/// the spawn nor the wait ever blocks the UI thread. I/O is discarded —
/// the viewer is disposable, its failures unobservable either way.
pub(super) fn spawn_xdg_open(target: &str) {
    let target = target.to_string();
    std::thread::spawn(move || {
        if let Ok(mut child) = std::process::Command::new("xdg-open")
            .arg(&target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            // reap: no zombie outlives the helper (usually it exits at
            // once; exotic viewers may keep it alive, the thread waits)
            let _ = child.wait();
        }
    });
}

/// The start folder for the folder picker: the current project, else Home
/// (never the hidden chat workspace: a project lives somewhere else).
pub(super) fn picker_start(h: &Harness) -> PathBuf {
    h.current_project
        .clone()
        .filter(|p| !h.is_chat_dir(p))
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The icon of a workspace, `size` points wide: a speech bubble for the chat
/// workspace "Allgemein", the coloured blocks of its path for a project.
pub(super) fn paint_workspace_icon(
    h: &Harness,
    ui: &egui::Ui,
    origin: egui::Pos2,
    dir: &std::path::Path,
    size: f32,
) {
    if h.is_chat_dir(dir) {
        let pixel = (size / crate::icons::GRID as f32).round().max(1.0);
        let offset = (size - pixel * crate::icons::GRID as f32) / 2.0;
        crate::icons::paint(
            ui.painter(),
            Icon::Chat,
            origin + vec2(offset, offset),
            pixel,
            theme::ORANGE,
        );
    } else {
        widgets::paint_project_icon(ui, origin, &dir.to_string_lossy(), size);
    }
}

/// A path relative to the project, when it lies inside it.
pub(super) fn relative_path(path: &str, cwd: &std::path::Path) -> String {
    std::path::Path::new(path)
        .strip_prefix(cwd)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// A centred fixed-height spacer so short content sits in the optical middle.
pub(super) fn vertical_center_offset(available: f32, content: f32) -> f32 {
    ((available - content) * 0.4).max(20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N15: Strg-K toggles the command palette only when the keyboard is
    /// free — with an open dialog it must not open a second focus owner
    /// on top of it, but an open palette can always be closed again.
    /// Runs the real shortcut layer headless (egui `Context::run`), with
    /// a Ctrl-K key event per frame.
    #[test]
    fn ctrl_k_toggles_the_palette_only_when_the_keyboard_is_free() {
        let ctx = egui::Context::default();
        crate::fonts::install_fallback_fonts(&ctx);
        let press_k = |h: &mut Harness| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1360.0, 860.0),
                )),
                events: vec![egui::Event::Key {
                    key: Key::K,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::COMMAND,
                }],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                let _ = show(h, ctx);
            });
        };
        let mut h = Harness::new(Vec::new(), "ask".to_string());
        press_k(&mut h);
        assert!(
            h.ui.palette.is_some(),
            "no dialog open: Strg-K opens the palette"
        );
        press_k(&mut h);
        assert!(
            h.ui.palette.is_none(),
            "still no dialog: Strg-K closes the palette again"
        );
        // a dialog owns the keyboard: the palette must not open on top
        h.ui.settings_open = true;
        press_k(&mut h);
        assert!(
            h.ui.palette.is_none(),
            "with the settings dialog open the palette stays closed"
        );
        assert!(h.ui.settings_open, "the dialog keeps the keyboard");
        // the palette itself may always be closed, dialog or not
        h.ui.palette = Some(Default::default());
        press_k(&mut h);
        assert!(h.ui.palette.is_none(), "an open palette can be closed");
        assert!(h.ui.settings_open, "the dialog survives untouched");
    }

    #[test]
    fn drafts_stay_with_their_session() {
        let mut h = Harness::new(Vec::new(), "ask".to_string());
        h.draft = "Entwurf Start".to_string();
        h.active = Some("a".to_string());
        swap_draft(&mut h);
        assert!(
            h.draft.is_empty(),
            "session a starts with its own empty draft"
        );
        h.draft = "Entwurf A".to_string();
        h.attachments.push(PathBuf::from("/x"));
        h.active = None;
        swap_draft(&mut h);
        assert_eq!(h.draft, "Entwurf Start");
        assert!(h.attachments.is_empty());
        h.active = Some("a".to_string());
        swap_draft(&mut h);
        assert_eq!(h.draft, "Entwurf A");
        assert_eq!(h.attachments, vec![PathBuf::from("/x")]);
    }

    #[test]
    fn error_statuses_are_recognised() {
        assert!(is_error_status("Löschen fehlgeschlagen: x"));
        assert!(is_error_status("Fehler (-32602): boom"));
        assert!(is_error_status("Vibe ist nicht verbunden."));
        assert!(!is_error_status("Sitzung gelöscht."));
    }

    #[test]
    fn relative_paths_strip_only_the_project_prefix() {
        let cwd = std::path::Path::new("/p/app");
        assert_eq!(relative_path("/p/app/src/a.rs", cwd), "src/a.rs");
        assert_eq!(relative_path("/other/a.rs", cwd), "/other/a.rs");
    }

    #[test]
    fn centre_offset_never_goes_negative() {
        assert_eq!(vertical_center_offset(100.0, 900.0), 20.0);
        assert_eq!(vertical_center_offset(900.0, 100.0), 320.0);
    }

    #[test]
    fn layout_constants_keep_the_column_inside_the_panel_threshold() {
        // the column plus both side panels must fit at the panel threshold
        const { assert!(COLUMN_WIDTH < PANEL_MIN_WINDOW) };
        const { assert!(COMPACT_WIDTH < PANEL_MIN_WINDOW) };
    }
}
