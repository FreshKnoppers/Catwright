//! Cards pinned between the transcript and the composer, so they can never
//! scroll out of sight: workspace trust, Vibe's questions, permission
//! requests and the message queue.

use super::*;
use crate::acp::protocol::{PermissionKind, ToolCallContent, ToolKind};
use crate::acp::session::{repeated_tool_call, PendingPermission, TurnState};
use crate::harness::extras::{self, QuestionAnswer};
use crate::i18n::{tr, trf};
use egui::Event;
use serde_json::Value;

pub(super) fn pinned(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    pinned_area(ui, "pinned_cards", |ui| {
        trust_banner(h, ui, sid);
        question_cards(h, ui, Some(sid));
        permission_cards(h, ui, sid);
        loop_guard(h, ui, sid);
        orphan_cards(h, ui);
        queue_rows(h, ui, sid);
    });
}

/// Cards that can appear without an open session (start screen).
pub(super) fn sessionless(h: &mut Harness, ui: &mut egui::Ui) {
    pinned_area(ui, "sessionless_cards", |ui| {
        question_cards(h, ui, None);
        orphan_cards(h, ui);
    });
}

/// The pinned cards scroll once they would take more than half the
/// window: however long a request is (a command with hundreds of lines, a
/// question with many options), the composer below stays on screen.
fn pinned_area(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    let max = (ui.ctx().screen_rect().height() * 0.5).max(220.0);
    let out = egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(max)
        .auto_shrink([false, true])
        .show(ui, add);
    // no card: take back the item spacing the empty area added, so the
    // layout is the same as without the area
    if out.content_size.y <= 0.0 {
        ui.add_space(-ui.spacing().item_spacing.y);
    }
}

// ---------------------------------------------------------------------------
// trust
// ---------------------------------------------------------------------------

fn trust_banner(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let Some(cwd) = h.sessions.get(sid).map(|s| s.cwd.clone()) else {
        return;
    };
    if h.trust_dismissed.contains(&cwd) {
        return;
    }
    let Some(trust) = h.trust.get(&cwd).filter(|t| t.needs_decision()).cloned() else {
        return;
    };
    let mut decision: Option<&str> = None;
    let mut dismiss = false;
    widgets::attention_card(ui, theme::AMBER, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            crate::icons::paint(
                ui.painter(),
                Icon::Shield,
                ui.cursor().min + vec2(0.0, 2.0),
                2.0,
                theme::AMBER,
            );
            ui.add_space(20.0);
            ui.label(
                RichText::new(tr("Trust this project folder?"))
                    .size(14.0)
                    .color(theme::TEXT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(IconButton::new(Icon::Close, tr("Decide later")).side(22.0))
                    .clicked()
                {
                    dismiss = true;
                }
            });
        });
        let files = if trust.detected_files.is_empty() {
            tr("Project files").to_string()
        } else {
            trust.detected_files.join(", ")
        };
        ui.label(
            RichText::new(trf(
                "Vibe found {files} here. Instructions, skills and hooks from the project load only after you allow it. Trust only folders whose content you know.",
                &[("files", &files)],
            ))
            .size(13.0)
            .color(theme::MUTED),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let trust_cwd = trust.decisions.iter().any(|d| d == "trust_cwd");
            let trust_repo = trust.decisions.iter().any(|d| d == "trust_repo");
            if trust_cwd && widgets::button(ui, tr("Trust folder"), ButtonKind::Primary).clicked() {
                decision = Some("trust_cwd");
            }
            if trust_repo
                && widgets::button(
                    ui,
                    tr("Trust the whole repository"),
                    if trust_cwd {
                        ButtonKind::Secondary
                    } else {
                        ButtonKind::Primary
                    },
                )
                .clicked()
            {
                decision = Some("trust_repo");
            }
            if trust.decisions.iter().any(|d| d == "decline")
                && widgets::button(ui, tr("Don’t trust"), ButtonKind::Danger).clicked()
            {
                decision = Some("decline");
            }
        });
    });
    ui.add_space(6.0);
    if dismiss {
        h.trust_dismissed.insert(cwd.clone());
    }
    if let Some(d) = decision {
        h.decide_trust(sid, &cwd, d);
        h.trust_dismissed.insert(cwd);
    }
}

// ---------------------------------------------------------------------------
// questions
// ---------------------------------------------------------------------------

fn question_cards(h: &mut Harness, ui: &mut egui::Ui, sid: Option<&str>) {
    let mut action: Option<(usize, bool)> = None;
    for (idx, q) in h.questions.iter_mut().enumerate() {
        // a question of a session without a view (a subagent, say) must be
        // answerable from wherever the user is: the turn waits for it
        let mine = match (&q.session_id, sid) {
            (Some(own), Some(sid)) => own == sid || !h.sessions.contains_key(own),
            (Some(own), None) => !h.sessions.contains_key(own),
            (None, _) => true,
        };
        if !mine {
            continue;
        }
        widgets::attention_card(ui, theme::YELLOW, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section_header(ui, tr("Vibe is asking"), true);
            });
            ui.add_space(2.0);
            for field in &q.fields {
                if let Some(header) = &field.header {
                    ui.label(theme::mono_label(header));
                }
                ui.add(
                    egui::Label::new(RichText::new(&field.text).size(14.0).color(theme::TEXT))
                        .wrap(),
                );
                let answer = q.answers.entry(field.key.clone()).or_default();
                ui.horizontal_wrapped(|ui| {
                    for option in &field.options {
                        let selected = answer.selected.contains(option);
                        if option_button(ui, option, selected).clicked() {
                            toggle_option(answer, option, field.multi);
                        }
                    }
                });
                if field.hidden_options > 0 {
                    ui.label(
                        RichText::new(trf(
                            "{n} more answers not shown – type your own answer instead.",
                            &[("n", &field.hidden_options)],
                        ))
                        .size(12.0)
                        .color(theme::DIM),
                    );
                }
                ui.add(
                    egui::TextEdit::singleline(&mut answer.other)
                        .hint_text(tr("Your own answer …"))
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);
            }
            if let Some(note) = &q.note {
                ui.label(RichText::new(note).color(theme::DIM).small());
            }
            ui.horizontal(|ui| {
                let ready = q.content().is_some();
                if ui
                    .add_enabled_ui(ready, |ui| {
                        widgets::button(ui, tr("Answer"), ButtonKind::Primary)
                    })
                    .inner
                    .clicked()
                {
                    action = Some((idx, true));
                }
                if widgets::button(ui, tr("Skip"), ButtonKind::Secondary).clicked() {
                    action = Some((idx, false));
                }
            });
        });
        ui.add_space(6.0);
    }
    if let Some((idx, accept)) = action {
        h.answer_question(idx, accept);
    }
}

fn option_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (fill, stroke) = if selected {
        (theme::RAISED, egui::Stroke::new(1.0_f32, theme::ORANGE))
    } else {
        (
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(1.0_f32, theme::BORDER),
        )
    };
    ui.add(
        egui::Button::new(RichText::new(label).size(13.5).color(theme::TEXT))
            .fill(fill)
            .stroke(stroke)
            .rounding(egui::Rounding::same(3.0))
            .min_size(vec2(0.0, 26.0)),
    )
}

fn toggle_option(answer: &mut QuestionAnswer, option: &str, multi: bool) {
    if multi {
        if let Some(pos) = answer.selected.iter().position(|s| s == option) {
            answer.selected.remove(pos);
        } else {
            answer.selected.push(option.to_string());
        }
    } else {
        answer.selected = vec![option.to_string()];
        answer.other.clear();
    }
}

// ---------------------------------------------------------------------------
// permissions
// ---------------------------------------------------------------------------

/// Number keys answer the first card while nothing is being typed. Takes
/// both the key press and the digit's text event, so it never reaches the
/// composer.
fn take_digit(ctx: &egui::Context, index: usize) -> bool {
    const KEYS: [Key; 4] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4];
    let Some(key) = KEYS.get(index).copied() else {
        return false;
    };
    let digit = (index + 1).to_string();
    ctx.input_mut(|i| {
        let Some(pos) = i.events.iter().position(|e| {
            matches!(e, Event::Key { key: k, pressed: true, modifiers, .. }
                if *k == key && modifiers.is_none())
        }) else {
            return false;
        };
        i.events.remove(pos);
        if let Some(pos) = i
            .events
            .iter()
            .position(|e| matches!(e, Event::Text(t) if *t == digit))
        {
            i.events.remove(pos);
        }
        true
    })
}

/// How long a freshly appeared permission card ignores number keys: a
/// digit typed for the composer must not answer a card the user has not
/// seen yet (Bug 6, Handoff 2026-11-09). Measured in *painted frames*, not
/// wall-clock time: a UI stall longer than the grace at the moment the card
/// appears would otherwise expire it before the card was ever drawn (N14).
/// While ANY turn runs, the animated UI repaints at the animation
/// framerate (~60 fps), so the grace must span whole seconds, not three
/// frames ≈ 50 ms — a number the composer types right after the card
/// appears must still be inert.
const PERMISSION_KEY_FRAMES: u32 = 60;

/// Pure decision: number keys unlock a card only after it has actually
/// been painted `frames_shown` times.
fn digits_ready(frames_shown: u32) -> bool {
    frames_shown >= PERMISSION_KEY_FRAMES
}

fn permission_cards(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    // A click into the composer, or text typed while the request is open,
    // means the user writes a message: the digits stay off for this
    // request, even once the draft is empty again (audit M1).
    let first = h
        .sessions
        .get(sid)
        .and_then(|s| s.permissions.first())
        .map(|p| p.request_id.clone());
    if let Some(first) = &first {
        if h.ui.composer_clicked || !h.draft.trim().is_empty() {
            h.ui.digits_blocked = Some(first.clone());
        }
    }
    h.ui.composer_clicked = false;
    let keys_free = h.draft.trim().is_empty()
        && !modal_open(h)
        && composer_or_no_focus(ui.ctx())
        && (first.is_none() || h.ui.digits_blocked != first);
    let mut answers: Vec<(Value, String)> = Vec::new();
    // The lockout counts frames the card was really painted in (N14), so
    // the counters live with the session's pending permissions.
    if let Some(s) = h.sessions.get_mut(sid) {
        s.paint_permission_frames();
    }
    if let Some(s) = h.sessions.get(sid) {
        let digits = keys_free
            && s.permissions
                .first()
                .is_some_and(|p| digits_ready(s.frames_shown(&p.request_id)));
        for (idx, p) in s.permissions.iter().enumerate() {
            widgets::attention_card(ui, theme::YELLOW, |ui| {
                ui.set_width(ui.available_width());
                theme::section_header(ui, tr("Approval required"), true);
                ui.add_space(2.0);
                // number keys only answer a request that is on screen in
                // full: no part of what is approved may be scrolled away
                let mut all_shown = true;
                if let Some(t) = s.tool_for_permission(p) {
                    let headline = match t.kind {
                        ToolKind::Execute => tr("Run command"),
                        ToolKind::Edit => tr("Edit file"),
                        ToolKind::Delete => tr("Delete file"),
                        ToolKind::Move => tr("Move file"),
                        ToolKind::Fetch => tr("Fetch web page"),
                        ToolKind::Read => tr("Read file"),
                        _ => tr("Run action"),
                    };
                    ui.label(RichText::new(headline).size(14.0).color(theme::TEXT));
                    let command = t
                        .raw_input
                        .as_ref()
                        .and_then(|v| v.get("command"))
                        .and_then(Value::as_str);
                    let shown = command.map_or_else(|| t.title.clone(), |c| format!("$ {c}"));
                    all_shown &= request_text(ui, ("permission_cmd", idx), &shown);
                    // what the tool will actually be called with: the title
                    // is Vibe's summary ("Reading README.md"), the arguments
                    // are the truth (full paths, URLs, patterns). ALL of
                    // them are shown — this card is the user's only
                    // safeguard when approving, so an argument that sits
                    // next to `command` (cwd, env, …) must not be hidden.
                    let has_diff = t
                        .content
                        .iter()
                        .any(|c| matches!(c, ToolCallContent::Diff { .. }));
                    let (args, cut) = permission_arguments(t.raw_input.as_ref(), has_diff);
                    // the command itself is the headline above; its key
                    // stays out of the argument list
                    let args: Vec<(String, String)> = if command.is_some() {
                        args.into_iter().filter(|(k, _)| k != "command").collect()
                    } else {
                        args
                    };
                    if !args.is_empty() {
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(trf("Arguments ({n})", &[("n", &args.len())]))
                                .size(12.0)
                                .color(theme::DIM),
                        );
                        all_shown &= argument_rows(ui, ("permission_args", idx), &args);
                        if cut > 0 {
                            ui.label(
                                RichText::new(trf(
                                    "… {n} values cut to {chars} characters",
                                    &[("n", &cut), ("chars", &MAX_ARGUMENT_CHARS)],
                                ))
                                .size(11.0)
                                .color(theme::DIM),
                            );
                        }
                    }
                    for content in &t.content {
                        if let ToolCallContent::Diff {
                            path,
                            old_text,
                            new_text,
                        } = content
                        {
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(relative_path(path, &s.cwd))
                                    .monospace()
                                    .size(12.0)
                                    .color(theme::MUTED),
                            );
                            egui::ScrollArea::vertical()
                                .id_salt(("permission_diff", idx, path))
                                .max_height(180.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    transcript::diff_body(
                                        ui,
                                        old_text.as_deref(),
                                        new_text,
                                        200,
                                        None,
                                    );
                                });
                        }
                    }
                } else {
                    all_shown &= unknown_call(ui, idx, &p.request.tool_call);
                }
                ui.add_space(4.0);
                option_row(ui, p, idx == 0 && digits && all_shown, &mut answers);
            });
            ui.add_space(6.0);
        }
    }
    for (request_id, option_id) in answers {
        h.answer_permission(sid, &request_id, &option_id);
    }
}

/// The body of a permission card whose tool call this session has not seen
/// (yet): vibe-acp sends only an id with the request — the call's entry id,
/// or just the tool's name (`bash`) when it has none. What the request
/// itself carries is shown, plus an honest note; once the call arrives the
/// card shows it in full on the next frame.
/// Returns whether all of it is on screen (see [`request_text`]).
fn unknown_call(
    ui: &mut egui::Ui,
    idx: usize,
    call: &crate::acp::protocol::ToolCallUpdate,
) -> bool {
    ui.label(
        RichText::new(tr("Run action"))
            .size(14.0)
            .color(theme::TEXT),
    );
    let shown = call
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| call.tool_call_id.clone());
    let mut all_shown = true;
    if !shown.trim().is_empty() {
        all_shown &= request_text(ui, ("permission_unknown", idx), &shown);
    }
    let args = permission_arguments(call.raw_input.as_ref(), false).0;
    if !args.is_empty() {
        all_shown &= argument_rows(ui, ("permission_unknown_args", idx), &args);
    }
    if call.title.is_none() && call.raw_input.is_none() {
        ui.label(
            RichText::new(tr(
                "Vibe sent no details for this approval — only the tool or call shown above.",
            ))
            .size(12.0)
            .color(theme::DIM),
        );
    }
    all_shown
}

/// Tallest command box on a permission card; a longer command scrolls.
const MAX_REQUEST_TEXT_HEIGHT: f32 = 160.0;

/// Whether everything `out` scrolls is in view and its viewport lies
/// inside the visible part of `ui` (not scrolled away in the pinned area).
fn fully_shown<R>(ui: &egui::Ui, out: &egui::scroll_area::ScrollAreaOutput<R>) -> bool {
    out.content_size.y <= out.inner_rect.height() + 1.0
        && ui.clip_rect().contains_rect(out.inner_rect)
}

/// The command (or title) a permission card asks about, in a box that
/// scrolls instead of growing: a command padded with hundreds of blank
/// lines must not push its dangerous tail, the buttons and the composer
/// off the window. Hidden characters are made visible, and a command of
/// more than one line says how many it has. Returns whether all of it is
/// on screen.
fn request_text(ui: &mut egui::Ui, id: impl std::hash::Hash, text: &str) -> bool {
    let text = extras::visible_controls(text);
    let out = egui::Frame::none()
        .fill(theme::BG)
        .rounding(egui::Rounding::same(3.0))
        .inner_margin(egui::Margin::symmetric(8.0, 5.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(MAX_REQUEST_TEXT_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(text.as_ref())
                                .monospace()
                                .size(12.5)
                                .color(theme::TEXT),
                        )
                        .wrap(),
                    );
                })
        })
        .inner;
    let lines = text.lines().count();
    if lines > 1 {
        ui.label(
            RichText::new(trf(
                "{n} lines – all of them run. Scroll through the box before you allow it.",
                &[("n", &lines)],
            ))
            .size(12.0)
            .color(theme::AMBER),
        );
    }
    fully_shown(ui, &out)
}

/// The (key, value) argument rows of a permission card in a scrolling box,
/// hidden characters made visible. Returns whether all rows are on screen.
fn argument_rows(ui: &mut egui::Ui, id: impl std::hash::Hash, args: &[(String, String)]) -> bool {
    let out = egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(140.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (key, value) in args {
                ui.horizontal_top(|ui| {
                    ui.label(
                        RichText::new(extras::visible_controls(key).as_ref())
                            .monospace()
                            .size(12.0)
                            .color(theme::DIM),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(extras::visible_controls(value).as_ref())
                                .monospace()
                                .size(12.0)
                                .color(theme::TEXT),
                        )
                        .wrap(),
                    );
                });
            }
        });
    fully_shown(ui, &out)
}

/// Which number key answers an option: only reversible one-shot decisions
/// ("allow once", "deny once") get one; every lasting decision — allow
/// AND reject — stays behind a deliberate click (Bug 6, Handoff
/// 2026-11-09: "lasting options only by click"). The digit is the option's
/// position among the eligible ones, not its place in Vibe's list.
/// The longest argument value shown on a permission card; longer ones are
/// cut with "…" (and counted, so the card can say how many were cut).
const MAX_ARGUMENT_CHARS: usize = 300;

/// The tool call's arguments as (key, value) rows for the permission card,
/// sorted by key (serde_json's map order) — ALL of them, not just the
/// first eight: the card is the user's only safeguard when approving, and
/// an argument that sorts late must not be invisible. A file body that the
/// card already shows as a diff is left out (`content`, `new_string`, …);
/// long values are cut with "…" and the second return value counts the cuts.
fn permission_arguments(raw: Option<&Value>, has_diff: bool) -> (Vec<(String, String)>, usize) {
    const DIFF_BODIES: [&str; 5] = ["content", "new_string", "old_string", "newText", "oldText"];
    let Some(Value::Object(map)) = raw else {
        return (Vec::new(), 0);
    };
    let mut out = Vec::new();
    let mut cut_count = 0;
    for (key, value) in map {
        if has_diff && DIFF_BODIES.contains(&key.as_str()) {
            continue;
        }
        let text = match value {
            Value::String(s) => s.clone(),
            Value::Null => continue,
            other => other.to_string(),
        };
        if text.chars().count() > MAX_ARGUMENT_CHARS {
            let mut t: String = text.chars().take(MAX_ARGUMENT_CHARS).collect();
            t.push('…');
            cut_count += 1;
            out.push((key.clone(), t));
        } else {
            out.push((key.clone(), text));
        }
    }
    (out, cut_count)
}

fn option_digit(kind: PermissionKind, eligible_before: usize) -> Option<usize> {
    match kind {
        PermissionKind::AllowOnce | PermissionKind::RejectOnce => Some(eligible_before + 1),
        PermissionKind::AllowAlways | PermissionKind::RejectAlways | PermissionKind::Other => None,
    }
}

/// The request's options; `digits` also binds number keys to the eligible
/// ones.
fn option_row(
    ui: &mut egui::Ui,
    p: &PendingPermission,
    digits: bool,
    answers: &mut Vec<(Value, String)>,
) {
    ui.horizontal_wrapped(|ui| {
        let mut eligible = 0;
        let shown = p
            .request
            .options
            .iter()
            .take(extras::MAX_PERMISSION_OPTIONS);
        for (i, opt) in shown.enumerate() {
            // label, colour and digit all from the one checked view (H1)
            let view = extras::option_view(&opt.option_id, &opt.kind, &opt.name);
            let label = extras::visible_controls(&view.label).into_owned();
            let digit = view
                .kind
                .as_ref()
                .and_then(|k| option_digit(k.clone(), eligible));
            if digit.is_some() {
                eligible += 1;
            }
            let kind = match view.kind {
                Some(PermissionKind::AllowOnce) if i == 0 => ButtonKind::Primary,
                Some(PermissionKind::RejectOnce | PermissionKind::RejectAlways) => {
                    ButtonKind::Danger
                }
                _ => ButtonKind::Secondary,
            };
            let text = match (digits, digit) {
                (true, Some(n)) => format!("{n}  {label}"),
                _ => label,
            };
            let button = widgets::button(ui, &text, kind);
            // a key never answers through a button scrolled out of view
            let on_screen = ui.clip_rect().contains_rect(button.rect);
            let keyed = digits && on_screen && digit.is_some_and(|n| take_digit(ui.ctx(), n - 1));
            if button.clicked() || keyed {
                answers.push((p.request_id.clone(), opt.option_id.clone()));
            }
        }
    });
}

/// Requests whose session is unknown (Handoff 2.2): always answerable, here
/// on every screen.
fn orphan_cards(h: &mut Harness, ui: &mut egui::Ui) {
    if h.orphan_permissions.is_empty() {
        return;
    }
    let mut answers: Vec<(Value, String)> = Vec::new();
    let mut cancels: Vec<Value> = Vec::new();
    for p in &h.orphan_permissions {
        widgets::attention_card(ui, theme::YELLOW, |ui| {
            ui.set_width(ui.available_width());
            theme::section_header(ui, tr("Approval required"), true);
            ui.label(
                RichText::new(tr(
                    "This request belongs to no open session — probably a subagent. Please answer, or Vibe waits forever.",
                ))
                .size(13.0)
                .color(theme::MUTED),
            );
            ui.label(
                RichText::new(&p.request.session_id)
                    .monospace()
                    .size(12.0)
                    .color(theme::DIM),
            );
            ui.add_space(4.0);
            option_row(ui, p, false, &mut answers);
            if widgets::button(ui, tr("Cancel"), ButtonKind::Danger).clicked() {
                cancels.push(p.request_id.clone());
            }
        });
        ui.add_space(6.0);
    }
    for (request_id, option_id) in answers {
        h.answer_orphan_permission(&request_id, &option_id);
    }
    for request_id in cancels {
        h.cancel_orphan_permission(&request_id);
    }
}

// ---------------------------------------------------------------------------
// loop guard
// ---------------------------------------------------------------------------

/// A running turn in which the agent keeps repeating the same tool call:
/// the detection is `session::repeated_tool_call` (pure), this card only
/// offers the way out. It disappears as soon as the chain breaks.
fn loop_guard(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let Some(s) = h.sessions.get(sid) else {
        return;
    };
    if s.turn != TurnState::Running {
        return;
    }
    let Some(rep) = repeated_tool_call(&s.items, &s.tool_calls) else {
        return;
    };
    let mut stop = false;
    widgets::attention_card(ui, theme::AMBER, |ui| {
        ui.set_width(ui.available_width());
        theme::section_header(ui, tr("Agent repeats the same call — possible loop"), true);
        ui.add_space(2.0);
        ui.label(
            RichText::new(trf(
                "“{title}” has now run {n} times in a row with the same input. Vibe cannot stop itself — you can cancel the running turn here.",
                &[("title", &rep.title), ("n", &rep.count)],
            ))
            .size(13.0)
            .color(theme::MUTED),
        );
        ui.add_space(4.0);
        if widgets::button(ui, tr("Cancel turn"), ButtonKind::Danger).clicked() {
            stop = true;
        }
    });
    ui.add_space(6.0);
    if stop {
        h.cancel();
    }
}

// ---------------------------------------------------------------------------
// queue
// ---------------------------------------------------------------------------

fn queue_rows(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let Some(queued) = h.queue.get(sid).filter(|q| !q.is_empty()).cloned() else {
        return;
    };
    let idle = h
        .sessions
        .get(sid)
        .is_some_and(|s| s.turn == TurnState::Idle);
    let mut remove = None;
    let mut send_next = false;
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(theme::mono_label(if idle {
                tr("In the queue")
            } else {
                tr("Up next")
            }));
            if idle {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, tr("Send now"), ButtonKind::Primary).clicked() {
                        send_next = true;
                    }
                });
            }
        });
        for (i, q) in queued.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(&q.text).size(13.5).color(theme::MUTED))
                        .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(IconButton::new(Icon::Close, tr("Remove from the queue")).side(22.0))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            });
        }
    });
    ui.add_space(6.0);
    if let Some(i) = remove {
        if let Some(q) = h.queue.get_mut(sid) {
            q.remove(i);
        }
    }
    if send_next {
        h.send_next_queued(sid);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn permission_arguments_show_the_real_call() {
        use super::permission_arguments;
        let raw = serde_json::json!({
            "filePath": "/home/u/.ssh/id_ed25519",
            "offset": 0,
            "limit": null,
            "flags": ["-r"]
        });
        assert_eq!(
            permission_arguments(Some(&raw), false).0,
            vec![
                (
                    "filePath".to_string(),
                    "/home/u/.ssh/id_ed25519".to_string()
                ),
                ("flags".to_string(), "[\"-r\"]".to_string()),
                ("offset".to_string(), "0".to_string()),
            ]
        );
        // the body a diff already shows stays out; long values are cut
        // and counted
        let raw = serde_json::json!({"filePath": "a.txt", "content": "x".repeat(5000)});
        assert_eq!(permission_arguments(Some(&raw), true).0.len(), 1);
        let (long, cut) = permission_arguments(Some(&raw), false);
        let body = &long.iter().find(|(k, _)| k == "content").unwrap().1;
        assert!(body.ends_with('…') && body.chars().count() == 301);
        assert_eq!(cut, 1);
        assert!(permission_arguments(None, false).0.is_empty());
        assert!(permission_arguments(Some(&serde_json::json!("x")), false)
            .0
            .is_empty());
        // no 8-key cap: all arguments come back
        let many: serde_json::Map<String, Value> = (0..30)
            .map(|i| (format!("k{i:02}"), Value::from(i)))
            .collect();
        assert_eq!(
            permission_arguments(Some(&Value::Object(many)), false)
                .0
                .len(),
            30
        );
    }

    use super::*;

    #[test]
    fn single_choice_replaces_and_multi_toggles() {
        let mut a = QuestionAnswer {
            other: "alt".to_string(),
            ..Default::default()
        };
        toggle_option(&mut a, "A", false);
        assert_eq!(a.selected, vec!["A"]);
        assert!(a.other.is_empty(), "a picked option replaces free text");
        toggle_option(&mut a, "B", false);
        assert_eq!(a.selected, vec!["B"]);
        let mut m = QuestionAnswer::default();
        toggle_option(&mut m, "A", true);
        toggle_option(&mut m, "B", true);
        toggle_option(&mut m, "A", true);
        assert_eq!(m.selected, vec!["B"]);
    }

    /// Bug 6 (Handoff 2026-11-09): with Vibe's observed option list
    /// (Allow once, session, permanent, Deny) only "Einmal erlauben" and
    /// "Ablehnen" get a number key — the lasting "always" decisions stay
    /// behind a deliberate click. That includes REJECTING for always
    /// (N14): any lasting decision, allow or reject, needs a deliberate
    /// click.
    #[test]
    fn option_digits_skip_lasting_decisions() {
        let kinds = [
            PermissionKind::AllowOnce,
            PermissionKind::AllowAlways,
            PermissionKind::AllowAlways,
            PermissionKind::RejectAlways,
            PermissionKind::RejectAlways,
            PermissionKind::RejectOnce,
        ];
        let mut eligible = 0;
        let digits: Vec<Option<usize>> = kinds
            .iter()
            .map(|k| {
                let d = option_digit(k.clone(), eligible);
                if d.is_some() {
                    eligible += 1;
                }
                d
            })
            .collect();
        assert_eq!(
            digits,
            vec![Some(1), None, None, None, None, Some(2)],
            "one-shot digestions get digits in order, lasting ones never"
        );
    }

    /// Bug 6 / N14: a card that just appeared ignores number keys, so a
    /// digit typed for the composer cannot answer it unseen; from the
    /// third painted frame on the keys work. Measured in frames, never in
    /// wall-clock time, a UI stall cannot expire the grace before the
    /// card was ever drawn.
    #[test]
    fn fresh_cards_ignore_number_keys_for_their_first_frames() {
        assert!(!digits_ready(0), "no frame painted, keys stay inert");
        assert!(!digits_ready(1));
        assert!(!digits_ready(2));
        assert!(
            digits_ready(PERMISSION_KEY_FRAMES),
            "from the third painted frame on the keys work"
        );
        assert!(digits_ready(1000));
    }

    #[test]
    fn digit_keys_are_consumed_with_their_text() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![
                Event::Key {
                    key: Key::Num2,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                },
                Event::Text("2".to_string()),
            ],
            ..Default::default()
        };
        let mut taken = (false, false, 99);
        let _ = ctx.run(input, |ctx| {
            taken.0 = take_digit(ctx, 1);
            taken.1 = take_digit(ctx, 0);
            taken.2 = ctx.input(|i| i.events.len());
        });
        assert!(taken.0, "the pressed digit matches");
        assert!(!taken.1, "other digits do not");
        assert_eq!(taken.2, 0, "neither key nor text is left for the composer");
    }
}
