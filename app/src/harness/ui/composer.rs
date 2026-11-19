//! The composer and the start screen: auto-growing input, attachments, the
//! slash and @ lists with keyboard control, mode / model / thinking menus.

use super::*;
use crate::acp::protocol::ConfigOption;
use crate::harness::state::Dictation;
use crate::harness::watch::{filter_files, mention_at};
use crate::harness::{
    current_mode_label, extras, filter_commands, mode_label, mode_selectable, start_modes, Conn,
    MAX_RECONNECTS,
};
use crate::i18n::{tr, trf};
use crate::icons;
use crate::widgets::{line_galley, mono, paint_line};
use egui::text::{CCursor, CCursorRange};
use egui::{pos2, Id, Rect, Sense, Stroke};

/// Most rows the slash / @ list shows.
const LIST_ROWS: usize = 8;

/// One row of the slash / @ list.
struct ListRow {
    title: String,
    detail: String,
    kind: ListKind,
}

enum ListKind {
    Command(String),
    /// Char index of the `@` and the file's relative path.
    File(usize, String),
}

fn composer_id() -> Id {
    Id::new("harness_composer")
}

/// The caret's char index in the composer.
fn cursor_index(ctx: &egui::Context) -> Option<usize> {
    egui::TextEdit::load_state(ctx, composer_id())
        .and_then(|s| s.cursor.char_range())
        .map(|r| r.primary.index)
}

fn set_cursor(ctx: &egui::Context, index: usize) {
    if let Some(mut state) = egui::TextEdit::load_state(ctx, composer_id()) {
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
        egui::TextEdit::store_state(ctx, composer_id(), state);
    }
}

fn list_rows(h: &mut Harness, ctx: &egui::Context) -> Vec<ListRow> {
    if h.draft.starts_with('/') {
        let cmds = h.available_commands();
        return filter_commands(&cmds, &h.draft)
            .into_iter()
            .take(LIST_ROWS)
            .map(|c| ListRow {
                title: format!("/{}", c.name),
                detail: extras::command_description(&c.name, &c.description),
                kind: ListKind::Command(c.name.clone()),
            })
            .collect();
    }
    let Some(cursor) = cursor_index(ctx) else {
        return Vec::new();
    };
    let Some((at, query)) = mention_at(&h.draft, cursor) else {
        return Vec::new();
    };
    let cwd = h
        .active
        .as_deref()
        .and_then(|s| h.sessions.get(s))
        .map(|s| s.cwd.clone())
        .or_else(|| h.current_project.clone());
    let Some(cwd) = cwd else {
        return Vec::new();
    };
    // the file list is a git scan too: only in trusted folders
    let trusted = h.trust.get(&cwd).is_some_and(|t| t.trusted);
    filter_files(h.watcher.files(&cwd, trusted), &query, LIST_ROWS)
        .into_iter()
        .map(|rel| ListRow {
            title: format!("@{rel}"),
            detail: String::new(),
            kind: ListKind::File(at, rel.to_string()),
        })
        .collect()
}

fn accept_row(h: &mut Harness, ctx: &egui::Context, row: &ListRow) {
    match &row.kind {
        ListKind::Command(name) => {
            h.draft = format!("/{name} ");
            set_cursor(ctx, h.draft.chars().count());
        }
        ListKind::File(at, rel) => {
            let cursor = cursor_index(ctx).unwrap_or(h.draft.chars().count());
            let chars: Vec<char> = h.draft.chars().collect();
            let mut out: String = chars[..*at].iter().collect();
            out.push('@');
            out.push_str(rel);
            out.push(' ');
            let after = out.chars().count();
            out.extend(chars[cursor.min(chars.len())..].iter());
            h.draft = out;
            set_cursor(ctx, after);
            let cwd = h
                .active
                .as_deref()
                .and_then(|s| h.sessions.get(s))
                .map(|s| s.cwd.clone())
                .or_else(|| h.current_project.clone());
            if let Some(cwd) = cwd {
                let path = cwd.join(rel);
                if !h.attachments.contains(&path) {
                    h.attachments.push(path);
                }
            }
        }
    }
    h.ui.focus_composer = true;
    h.ui.composer_popup = Default::default();
}

pub(super) fn composer(h: &mut Harness, ui: &mut egui::Ui) {
    let sid = h.active.clone().filter(|s| h.sessions.contains_key(s));
    let running = sid
        .as_deref()
        .and_then(|s| h.sessions.get(s))
        .is_some_and(|s| s.turn != TurnState::Idle);
    let id = composer_id();
    let focused = ui.memory(|m| m.has_focus(id));

    // keyboard for the list comes first, so its keys never reach the text
    let rows = if focused {
        list_rows(h, ui.ctx())
    } else {
        Vec::new()
    };
    if rows.is_empty() {
        h.ui.composer_popup = Default::default();
    }
    h.ui.composer_popup.selected =
        h.ui.composer_popup
            .selected
            .min(rows.len().saturating_sub(1));
    let list_open = !rows.is_empty() && !h.ui.composer_popup.dismissed;
    let mut accepted = None;
    if list_open {
        let len = rows.len();
        let mut escape = false;
        ui.input_mut(|i| {
            let popup = &mut h.ui.composer_popup;
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                popup.selected = (popup.selected + 1) % len;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                popup.selected = (popup.selected + len - 1) % len;
            }
            if i.consume_key(Modifiers::NONE, Key::Tab)
                || (!i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter))
            {
                accepted = Some(popup.selected);
            }
            if i.key_pressed(Key::Escape) {
                popup.dismissed = true;
                escape = true;
            }
        });
        if escape {
            h.ui.composer_used_escape = true;
        }
    }
    let mut send_now = false;
    if focused && accepted.is_none() {
        send_now =
            ui.input_mut(|i| !i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter));
    }

    let stroke = Stroke::new(
        1.0_f32,
        if focused {
            theme::ORANGE_DIM
        } else {
            theme::BORDER_SOFT
        },
    );
    let frame = egui::Frame::none()
        .fill(theme::INPUT)
        .rounding(egui::Rounding::same(6.0))
        .stroke(stroke)
        .inner_margin(egui::Margin {
            left: 12.0,
            right: 8.0,
            top: 9.0,
            bottom: 6.0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            attachment_chips(h, ui);
            let in_chat_dir = sid
                .as_deref()
                .and_then(|s| h.sessions.get(s))
                .map(|s| s.cwd.clone())
                .or_else(|| h.current_project.clone())
                .is_some_and(|dir| h.is_chat_dir(&dir));
            let hint = if running {
                tr("Queue the next message …")
            } else if in_chat_dir {
                tr("Message to Vibe …")
            } else {
                tr("What do you want to build?")
            };
            egui::ScrollArea::vertical()
                .id_salt("composer_scroll")
                .max_height(220.0)
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let edit = egui::TextEdit::multiline(&mut h.draft)
                        .id(id)
                        .frame(false)
                        .font(widgets::font(14.5))
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .lock_focus(true)
                        .hint_text(hint)
                        .show(ui);
                    if h.ui.focus_composer && !modal_open(h) {
                        edit.response.request_focus();
                        h.ui.focus_composer = false;
                    }
                });
            ui.add_space(4.0);
            toolbar(h, ui, sid.as_deref(), running, &mut send_now);
        });
    // drag & drop onto the composer: dropped files attach exactly like the
    // "+" picker (PNG as image when the agent takes images, else a
    // resource_link); while files are dragged over the window the composer
    // shows a hint
    let (hovered, dropped) = ui.ctx().input(|i| {
        (
            !i.raw.hovered_files.is_empty(),
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect::<Vec<_>>(),
        )
    });
    attach_dropped(h, dropped);
    if hovered {
        let rect = frame.response.rect;
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            egui::Rounding::same(6.0),
            theme::ORANGE.gamma_multiply(0.12),
        );
        painter.rect(
            rect,
            egui::Rounding::same(6.0),
            egui::Color32::TRANSPARENT,
            Stroke::new(1.5_f32, theme::ORANGE),
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tr("Drop to attach"),
            widgets::font(15.0),
            theme::ORANGE,
        );
    }
    if send_now {
        h.send();
        h.ui.focus_composer = true;
    }
    hint_row(h, ui, sid.as_deref(), running);

    if list_open {
        let anchor = frame.response.rect.left_top() - vec2(0.0, 6.0);
        if let Some(i) = list_popup(h, ui, &rows, anchor, frame.response.rect.width()) {
            accepted = Some(i);
        }
    }
    if let Some(i) = accepted {
        accept_row(h, ui.ctx(), &rows[i]);
    }
}

/// Attach dropped files, deduplicated by path — the same list the "+" picker
/// and the @-mention fill, so `prompt_blocks` turns them into image /
/// resource_link blocks on send.
pub(crate) fn attach_dropped(h: &mut Harness, paths: Vec<std::path::PathBuf>) {
    for p in paths {
        if !h.attachments.contains(&p) {
            h.attachments.push(p);
        }
    }
}

fn attachment_chips(h: &mut Harness, ui: &mut egui::Ui) {
    if h.attachments.is_empty() {
        return;
    }
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        for (i, a) in h.attachments.iter().enumerate() {
            let name = project_name(a);
            let g = line_galley(ui, &name, widgets::font(12.5), theme::TEXT, 220.0);
            let (rect, _) = ui.allocate_exact_size(vec2(g.size().x + 40.0, 24.0), Sense::hover());
            ui.painter().rect_filled(rect, 2.0, theme::RAISED);
            icons::paint(
                ui.painter(),
                Icon::File,
                pos2(rect.left() + 6.0, rect.center().y - 3.5),
                1.0,
                theme::DIM,
            );
            paint_line(ui, rect.left() + 18.0, rect.center().y, g);
            let x = Rect::from_min_size(
                pos2(rect.right() - 22.0, rect.top() + 2.0),
                vec2(20.0, 20.0),
            );
            if widgets::place(
                ui,
                x,
                IconButton::new(Icon::Close, tr("Remove attachment")).side(20.0),
            )
            .on_hover_text(a.to_string_lossy())
            .clicked()
            {
                remove = Some(i);
            }
        }
    });
    if let Some(i) = remove {
        h.attachments.remove(i);
    }
    ui.add_space(4.0);
}

/// The microphone button of the composer: `None` while the setting is off.
/// The tooltip says what a click does; the icon is ORANGE while Vibe
/// records.
pub(crate) fn dictation_button(h: &Harness) -> Option<(Icon, &'static str, egui::Color32)> {
    if !h.voice_dictation {
        return None;
    }
    Some(match h.dictation {
        Dictation::Recording => (Icon::Mic, tr("Stop dictation"), theme::ORANGE),
        _ => (Icon::Mic, tr("Dictate"), theme::MUTED),
    })
}

fn toolbar(
    h: &mut Harness,
    ui: &mut egui::Ui,
    sid: Option<&str>,
    running: bool,
    send_now: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        if ui
            .add(IconButton::new(Icon::Plus, tr("Attach a file (or type @)")))
            .clicked()
        {
            let start = h
                .active
                .as_deref()
                .and_then(|s| h.sessions.get(s))
                .map(|s| s.cwd.clone())
                .unwrap_or_else(|| picker_start(h));
            h.ui.file_picker = Some(dialogs::FilePicker::new(start));
        }
        if let Some((icon, tooltip, color)) = dictation_button(h) {
            if ui
                .add(IconButton::new(icon, tooltip).color(color))
                .clicked()
            {
                h.toggle_dictation();
            }
        }
        mode_chip(h, ui, sid);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let has_text = !h.draft.trim().is_empty() || !h.attachments.is_empty();
            if running {
                if ui
                    .add(
                        IconButton::new(Icon::Stop, tr("Stop (Esc)"))
                            .side(28.0)
                            .color(theme::RED_TEXT)
                            .fill(theme::RAISED),
                    )
                    .clicked()
                {
                    h.cancel();
                }
                if has_text
                    && ui
                        .add(
                            IconButton::new(Icon::Send, tr("Queue (↵)"))
                                .side(28.0)
                                .color(egui::Color32::WHITE)
                                .fill(theme::ORANGE_DIM),
                        )
                        .clicked()
                {
                    *send_now = true;
                }
            } else {
                let fill = if has_text {
                    theme::ORANGE
                } else {
                    theme::RAISED
                };
                let color = if has_text {
                    egui::Color32::WHITE
                } else {
                    theme::DIM
                };
                if ui
                    .add(
                        IconButton::new(Icon::Send, tr("Send (↵)"))
                            .side(28.0)
                            .color(color)
                            .fill(fill),
                    )
                    .clicked()
                {
                    *send_now = true;
                }
            }
            ui.add_space(2.0);
            thinking_chip(h, ui, sid);
            model_chip(h, ui, sid);
        });
    });
}

/// (id, label, description), current id, current label.
type ModeList = (Vec<(String, String, String)>, String, String);

fn mode_list(h: &Harness, sid: Option<&str>) -> Option<ModeList> {
    if let Some(modes) = sid
        .and_then(|s| h.sessions.get(s))
        .and_then(|s| s.modes.as_ref())
    {
        let rows = modes
            .available_modes
            .iter()
            .filter(|m| mode_selectable(&m.id))
            .map(|m| {
                (
                    m.id.clone(),
                    mode_label(&m.id, &m.name).to_string(),
                    extras::mode_description(&m.id, m.description.as_deref()),
                )
            })
            .collect();
        return Some((
            rows,
            modes.current_mode_id.clone(),
            current_mode_label(modes),
        ));
    }
    if sid.is_some() {
        return None;
    }
    let rows: Vec<(String, String, String)> = start_modes(&h.known_modes)
        .into_iter()
        .map(|(id, name)| {
            let desc = extras::mode_description(&id, None);
            (id, name, desc)
        })
        .collect();
    let current = h.start_mode.clone();
    let label = rows
        .iter()
        .find(|(id, _, _)| *id == current)
        .map(|(_, n, _)| n.clone())
        .unwrap_or_else(|| current.clone());
    Some((rows, current, label))
}

fn mode_chip(h: &mut Harness, ui: &mut egui::Ui, sid: Option<&str>) {
    let Some((rows, current, label)) = mode_list(h, sid) else {
        return;
    };
    let tip = if current == "auto-approve" {
        tr("Mode: Vibe runs everything without asking")
    } else {
        tr("Mode: when Vibe asks")
    };
    let color = theme::mode_color(&current);
    let resp = widgets::chip(ui, Some((Icon::Shield, color)), &label, true, tip);
    let popup = ui.make_persistent_id(("mode_menu", sid.unwrap_or("start")));
    widgets::toggle_popup(ui, &resp, popup);
    let mut chosen = None;
    widgets::popup_menu(ui, popup, &resp, true, 390.0, |ui| {
        for (id, name, desc) in &rows {
            if widgets::menu_mode_row(
                ui,
                theme::mode_color(id),
                id == &current,
                name,
                (!desc.is_empty()).then_some(desc.as_str()),
            ) {
                chosen = Some(id.clone());
                widgets::close_popup(ui);
            }
        }
    });
    if let Some(id) = chosen {
        if id == "auto-approve" {
            // never switch to executing everything without a deliberate
            // confirmation (Handoff Ideenliste 7)
            h.ui.confirm = Some(crate::harness::state::Confirm::AutoApprove {
                session_id: sid.map(|s| s.to_string()),
            });
        } else {
            match sid {
                Some(s) => h.set_mode(s, &id),
                None => h.start_mode = id,
            }
        }
    }
}

/// A select option's (value, name, description) rows and current value.
type SelectValues = (Vec<(String, String, String)>, String);

fn select_values(h: &Harness, sid: Option<&str>, config: &str) -> Option<SelectValues> {
    sid.and_then(|s| h.sessions.get(s)).and_then(|s| {
        s.config_options.iter().find_map(|o| match o {
            ConfigOption::Select {
                id,
                options,
                current_value,
                ..
            } if id == config => Some((
                options
                    .iter()
                    .map(|x| {
                        (
                            x.value.clone(),
                            x.name.clone(),
                            x.description.clone().unwrap_or_default(),
                        )
                    })
                    .collect(),
                current_value.clone(),
            )),
            _ => None,
        })
    })
}

fn model_chip(h: &mut Harness, ui: &mut egui::Ui, sid: Option<&str>) {
    let (models, current) = match select_values(h, sid, "model") {
        Some((m, c)) => (m, Some(c)),
        None => (
            h.known_models
                .iter()
                .map(|(v, n)| (v.clone(), n.clone(), String::new()))
                .collect(),
            h.start_model.clone().or_else(|| h.default_model.clone()),
        ),
    };
    if models.is_empty() {
        return;
    }
    let selected = current
        .as_ref()
        .and_then(|c| {
            models
                .iter()
                .find(|(v, _, _)| v == c)
                .map(|(_, n, _)| n.clone())
        })
        .unwrap_or_else(|| tr("Vibe default").to_string());
    let resp = widgets::chip_limited(
        ui,
        Some((Icon::Spark, theme::ORANGE)),
        &selected,
        true,
        // Handoff 2.8.9: switching writes Vibe's global default
        tr("Switching the model changes Vibe’s globally active model (“active_model” in Vibe’s config.toml) – the Vibe CLI uses it afterwards too."),
        150.0,
    );
    let popup = ui.make_persistent_id(("model_menu", sid.unwrap_or("start")));
    widgets::toggle_popup(ui, &resp, popup);
    let mut chosen = None;
    widgets::popup_menu(ui, popup, &resp, true, 320.0, |ui| {
        ui.add_space(4.0);
        ui.label(theme::mono_label(tr("Vibe (subscription)")));
        ui.add_space(2.0);
        for (value, name, desc) in &models {
            if widgets::menu_check_row(
                ui,
                current.as_deref() == Some(value.as_str()),
                name,
                (!desc.is_empty()).then_some(desc.as_str()),
            ) {
                chosen = Some(value.clone());
                widgets::close_popup(ui);
            }
        }
    });
    if let Some(v) = chosen {
        match sid {
            Some(s) => h.set_config(s, "model", &v),
            None => h.choose_start_model(v),
        }
    }
}

fn thinking_chip(h: &mut Harness, ui: &mut egui::Ui, sid: Option<&str>) {
    let (levels, current): (Vec<String>, Option<String>) = match select_values(h, sid, "thinking") {
        Some((l, c)) => (l.into_iter().map(|(v, _, _)| v).collect(), Some(c)),
        None => {
            // the levels of the model the next session starts with
            let model = h.start_model.clone().or_else(|| h.default_model.clone());
            (
                h.thinking_levels_for(model.as_deref()),
                h.start_thinking
                    .clone()
                    .or_else(|| h.thinking_of_model(model.as_deref()))
                    .or_else(|| h.default_thinking.clone()),
            )
        }
    };
    if levels.is_empty() {
        return;
    }
    let idx = current
        .as_ref()
        .and_then(|c| levels.iter().position(|l| l == c))
        .unwrap_or(levels.len() - 1);
    let label = extras::thinking_label(&levels[idx]);
    // pixel meter plus word: the meter never carries the meaning alone
    let g = line_galley(ui, &label, widgets::font(13.0), theme::TEXT, 90.0);
    let meter_w = levels.len() as f32 * 8.0;
    let (rect, resp) =
        ui.allocate_exact_size(vec2(g.size().x + meter_w + 24.0, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::RAISED);
    }
    for i in 0..levels.len() {
        let color = if i <= idx {
            theme::ORANGE_DIM
        } else {
            theme::BORDER
        };
        ui.painter().rect_filled(
            Rect::from_min_size(
                pos2(rect.left() + 8.0 + i as f32 * 8.0, rect.center().y - 3.0),
                vec2(6.0, 6.0),
            ),
            0.0,
            color,
        );
    }
    paint_line(ui, rect.left() + 12.0 + meter_w, rect.center().y, g);
    let resp = resp
        .on_hover_text(tr("How deeply the model thinks"))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let popup = ui.make_persistent_id(("thinking_menu", sid.unwrap_or("start")));
    widgets::toggle_popup(ui, &resp, popup);
    let mut chosen = None;
    widgets::popup_menu(ui, popup, &resp, true, 160.0, |ui| {
        ui.add_space(4.0);
        ui.label(theme::mono_label(tr("Thinking")));
        ui.add_space(2.0);
        for (i, level) in levels.iter().enumerate() {
            if widgets::menu_check_row(ui, i == idx, &extras::thinking_label(level), None) {
                chosen = Some(level.clone());
                widgets::close_popup(ui);
            }
        }
    });
    if let Some(level) = chosen {
        match sid {
            Some(s) => h.set_config(s, "thinking", &level),
            None => h.start_thinking = Some(level),
        }
    }
}

fn hint_row(h: &mut Harness, ui: &mut egui::Ui, sid: Option<&str>, running: bool) {
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.set_height(18.0);
        ui.add_space(2.0);
        if ui.available_width() > 520.0 {
            let enter = if running { tr("Queue") } else { tr("Send") };
            for (key, what) in [
                ("↵", enter),
                ("⇧↵", tr("Line")),
                ("/", tr("Commands")),
                ("@", tr("Files")),
            ] {
                ui.label(
                    RichText::new(key)
                        .monospace()
                        .size(11.0)
                        .color(theme::MUTED),
                );
                ui.label(RichText::new(what).size(11.5).color(theme::DIM));
                ui.add_space(4.0);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(s) = sid.and_then(|s| h.sessions.get(s)) {
                if let (Some(u), Some(frac)) = (&s.usage, s.context_fraction()) {
                    ui.label(
                        RichText::new(trf(
                            "{pct} % CONTEXT · {used}",
                            &[
                                ("pct", &((frac * 100.0).round() as u32)),
                                ("used", &crate::harness::format_tokens(u.used)),
                            ],
                        ))
                        .monospace()
                        .size(10.5)
                        .color(if frac > 0.85 {
                            theme::YELLOW
                        } else {
                            theme::DIM
                        }),
                    )
                    .on_hover_text(tr("Context window usage"));
                    theme::pixel_meter(ui, theme::meter_blocks(frac, 10), 10, 4.0);
                }
            }
        });
    });
}

/// The floating slash / @ list. Returns the row clicked this frame.
fn list_popup(
    h: &Harness,
    ui: &mut egui::Ui,
    rows: &[ListRow],
    anchor: egui::Pos2,
    width: f32,
) -> Option<usize> {
    let mut picked = None;
    egui::Area::new(composer_id().with("list"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(anchor)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(theme::CARD)
                .inner_margin(egui::Margin::same(4.0))
                .show(ui, |ui| {
                    ui.set_width(width - 8.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, row) in rows.iter().enumerate() {
                        let (rect, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                        if i == h.ui.composer_popup.selected || resp.hovered() {
                            ui.painter().rect_filled(rect, 3.0, theme::RAISED);
                        }
                        let g = line_galley(
                            ui,
                            &row.title,
                            mono(12.5),
                            theme::TEXT,
                            rect.width() * 0.5,
                        );
                        let r = paint_line(ui, rect.left() + 8.0, rect.center().y, g);
                        if !row.detail.is_empty() {
                            let x = r.right() + 12.0;
                            let g = line_galley(
                                ui,
                                &row.detail,
                                widgets::font(12.5),
                                theme::DIM,
                                rect.right() - x - 8.0,
                            );
                            paint_line(ui, x, rect.center().y, g);
                        }
                        // a click moves the focus away from the text field on
                        // press, so the row reacts to the press itself
                        if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
                            picked = Some(i);
                        }
                    }
                });
        });
    picked
}

// ---------------------------------------------------------------------------
// start screen
// ---------------------------------------------------------------------------

/// The start screen of a new session: greeting with the workspace picker,
/// composer and the workspace's recent sessions.
pub(super) fn start_screen(h: &mut Harness, ui: &mut egui::Ui) {
    let available = ui.available_height();
    let dir = h.current_project.clone();
    let chat = dir.as_deref().is_some_and(|d| h.is_chat_dir(d));
    let recent = dir
        .as_ref()
        .map(|d| h.sidebar_rows(d, tr("New session")))
        .unwrap_or_default();
    let shown = recent.len().min(5);
    let content = 268.0
        + if shown > 0 {
            34.0 + shown as f32 * 32.0
        } else {
            0.0
        };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(vertical_center_offset(available, content));
            let avail = ui.available_width();
            let w = 720.0_f32.min((avail - 32.0).max(120.0));
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
                    let time = ui.input(|i| i.time);
                    theme::pixel_cat(ui, time, false, 3.0);
                    ui.add_space(10.0);
                    greeting(h, ui, chat);
                    ui.add_space(14.0);
                    connection_card(h, ui);
                    cards::sessionless(h, ui);
                    composer(h, ui);
                    if shown > 0 {
                        ui.add_space(18.0);
                        ui.label(theme::mono_label(if chat {
                            tr("Recent chats")
                        } else {
                            tr("Recent")
                        }));
                        ui.add_space(4.0);
                        let dir = dir.clone().expect("rows come from this folder");
                        let mut open = None;
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for entry in &recent[..shown] {
                            let updated = h
                                .listed
                                .get(&dir)
                                .and_then(|l| l.iter().find(|s| s.session_id == entry.session_id))
                                .and_then(|s| s.updated_at.as_deref())
                                .and_then(|u| extras::relative_time(u, extras::now_epoch()));
                            let (icon, trailing) = (Icon::ChevronRight, updated);
                            if widgets::nav_row(ui, icon, &entry.title, trailing.as_deref(), false)
                                .clicked()
                            {
                                open = Some(entry.session_id.clone());
                            }
                        }
                        if let Some(sid) = open {
                            h.open_session(dir, &sid);
                        }
                    }
                });
        });
}

/// "Was baust du heute in [project ▾]" — or, in the chat workspace, "Worüber
/// möchtest du chatten? [Allgemein ▾]": the picker chooses the workspace the
/// new session starts in.
fn greeting(h: &mut Harness, ui: &mut egui::Ui, chat: bool) {
    ui.horizontal_wrapped(|ui| {
        let size = 23.0;
        let text = if chat {
            tr("What do you want to chat about?")
        } else {
            tr("What are you building today in")
        };
        ui.label(RichText::new(text).size(size).color(theme::TEXT));
        match h.current_project.clone() {
            Some(p) => workspace_picker(h, ui, &p),
            None => {
                ui.label(
                    RichText::new(tr("your project?"))
                        .size(size)
                        .color(theme::DIM),
                );
            }
        }
    });
    if chat {
        ui.label(
            RichText::new(tr(
                "Your Vibe subscription · web search and tools with approval",
            ))
            .color(theme::DIM)
            .size(13.5),
        );
    }
    if h.projects.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(tr("For code, Vibe works in a project folder."))
                .color(theme::MUTED)
                .size(14.0),
        );
        ui.add_space(6.0);
        let kind = if chat {
            ButtonKind::Secondary
        } else {
            ButtonKind::Primary
        };
        if widgets::button(ui, tr("Add project folder"), kind).clicked() {
            h.folder_picker = Some(dialogs::FolderPicker::new(picker_start(h)));
        }
    }
}

/// The chip that picks the workspace of the next session: "Allgemein" (the
/// chat workspace) first, then the projects.
fn workspace_picker(h: &mut Harness, ui: &mut egui::Ui, current: &std::path::Path) {
    let key = current.to_string_lossy().into_owned();
    let name = h.workspace_name(current);
    let g = line_galley(ui, &name, widgets::font(22.0), theme::TEXT, 320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 54.0, 38.0), Sense::click());
    ui.painter().rect_filled(
        rect,
        4.0,
        if resp.hovered() {
            theme::HOVER
        } else {
            theme::RAISED
        },
    );
    paint_workspace_icon(
        h,
        ui,
        pos2(rect.left() + 10.0, rect.center().y - 9.0),
        current,
        18.0,
    );
    paint_line(ui, rect.left() + 36.0, rect.center().y, g);
    icons::paint(
        ui.painter(),
        Icon::ChevronDown,
        pos2(rect.right() - 16.0, rect.center().y - 3.5),
        1.0,
        theme::DIM,
    );
    let popup = ui.make_persistent_id("start_project_picker");
    let resp = resp
        .on_hover_text(key)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    widgets::toggle_popup(ui, &resp, popup);
    let mut choose = None;
    let mut add = false;
    widgets::popup_menu(ui, popup, &resp, false, 300.0, |ui| {
        for p in h.workspaces() {
            let detail = h.is_chat_dir(&p).then_some(tr("no project"));
            if widgets::menu_check_row(ui, p == current, &h.workspace_name(&p), detail) {
                choose = Some(p);
                widgets::close_popup(ui);
            }
        }
        if widgets::menu_row(ui, Some(Icon::Plus), tr("Add project …"), None, false) {
            add = true;
            widgets::close_popup(ui);
        }
    });
    if let Some(p) = choose {
        h.select_workspace(p);
        h.ui.focus_composer = true;
    }
    if add {
        h.folder_picker = Some(dialogs::FolderPicker::new(picker_start(h)));
    }
}

/// When Vibe is not reachable, say so where the user is looking.
fn connection_card(h: &mut Harness, ui: &mut egui::Ui) {
    let (title, detail, retry) = match &h.conn {
        Conn::Failed(msg) => (
            tr("Vibe could not be started"),
            format!(
                "{msg}\n{}",
                tr("Check in a terminal that `vibe --version` runs and that you are signed in.")
            ),
            true,
        ),
        Conn::Reconnecting { attempt, .. } => (
            tr("Connection to Vibe interrupted"),
            trf(
                "Retry {attempt} of {max} …",
                &[("attempt", attempt), ("max", &MAX_RECONNECTS)],
            ),
            false,
        ),
        _ => return,
    };
    let mut do_retry = false;
    widgets::attention_card(ui, theme::RED, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).size(14.0).color(theme::TEXT));
        ui.label(RichText::new(detail).size(13.0).color(theme::MUTED));
        if retry {
            ui.add_space(4.0);
            if widgets::button(ui, tr("Reconnect"), ButtonKind::Primary).clicked() {
                do_retry = true;
            }
        }
    });
    ui.add_space(8.0);
    if do_retry {
        h.retry_connection();
    }
}
