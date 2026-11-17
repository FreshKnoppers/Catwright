//! Cards pinned between the transcript and the composer, so they can never
//! scroll out of sight: workspace trust, Vibe's questions, permission
//! requests and the message queue.

use super::*;
use crate::acp::protocol::{PermissionKind, ToolCallContent, ToolKind};
use crate::acp::session::{repeated_tool_call, PendingPermission, TurnState};
use crate::harness::extras::{self, QuestionAnswer};
use crate::i18n::tr;
use egui::Event;
use serde_json::Value;

pub(super) fn pinned(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    trust_banner(h, ui, sid);
    question_cards(h, ui, Some(sid));
    permission_cards(h, ui, sid);
    loop_guard(h, ui, sid);
    orphan_cards(h, ui);
    queue_rows(h, ui, sid);
}

/// Cards that can appear without an open session (start screen).
pub(super) fn sessionless(h: &mut Harness, ui: &mut egui::Ui) {
    question_cards(h, ui, None);
    orphan_cards(h, ui);
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
            RichText::new(format!(
                "Vibe hat hier {files} gefunden. Anweisungen, Skills und Hooks aus dem Projekt \
                 werden erst nach deiner Freigabe geladen. Vertraue nur Ordnern, deren Inhalt du kennst."
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
    let keys_free = h.draft.trim().is_empty() && !modal_open(h) && composer_or_no_focus(ui.ctx());
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
                    egui::Frame::none()
                        .fill(theme::BG)
                        .rounding(egui::Rounding::same(3.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(shown)
                                        .monospace()
                                        .size(12.5)
                                        .color(theme::TEXT),
                                )
                                .wrap(),
                            );
                        });
                    // what the tool will actually be called with: the title
                    // is Vibe's summary ("Reading README.md"), the arguments
                    // are the truth (full paths, URLs, patterns)
                    let has_diff = t
                        .content
                        .iter()
                        .any(|c| matches!(c, ToolCallContent::Diff { .. }));
                    let args = if command.is_some() {
                        Vec::new()
                    } else {
                        permission_arguments(t.raw_input.as_ref(), has_diff)
                    };
                    if !args.is_empty() {
                        ui.add_space(2.0);
                        for (key, value) in &args {
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    RichText::new(key).monospace().size(12.0).color(theme::DIM),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(value)
                                            .monospace()
                                            .size(12.0)
                                            .color(theme::TEXT),
                                    )
                                    .wrap(),
                                );
                            });
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
                                    transcript::diff_body(ui, old_text.as_deref(), new_text, 200);
                                });
                        }
                    }
                }
                ui.add_space(4.0);
                option_row(ui, p, idx == 0 && digits, &mut answers);
            });
            ui.add_space(6.0);
        }
    }
    for (request_id, option_id) in answers {
        h.answer_permission(sid, &request_id, &option_id);
    }
}

/// Which number key answers an option: only reversible one-shot decisions
/// ("allow once", "deny once") get one; every lasting decision — allow
/// AND reject — stays behind a deliberate click (Bug 6, Handoff
/// 2026-11-09: "lasting options only by click"). The digit is the option's
/// position among the eligible ones, not its place in Vibe's list.
/// Most arguments a permission card lists, and the longest value shown.
const MAX_SHOWN_ARGUMENTS: usize = 8;
const MAX_ARGUMENT_CHARS: usize = 300;

/// The tool call's arguments as (key, value) rows for the permission card,
/// sorted by key (serde_json's map order). A file body that the card
/// already shows as a diff is left out (`content`, `new_string`, …); long
/// values are cut with "…".
fn permission_arguments(raw: Option<&Value>, has_diff: bool) -> Vec<(String, String)> {
    const DIFF_BODIES: [&str; 5] = ["content", "new_string", "old_string", "newText", "oldText"];
    let Some(Value::Object(map)) = raw else {
        return Vec::new();
    };
    let cut = |text: String| -> String {
        if text.chars().count() > MAX_ARGUMENT_CHARS {
            let mut t: String = text.chars().take(MAX_ARGUMENT_CHARS).collect();
            t.push('…');
            t
        } else {
            text
        }
    };
    let mut out = Vec::new();
    for (key, value) in map {
        if has_diff && DIFF_BODIES.contains(&key.as_str()) {
            continue;
        }
        let text = match value {
            Value::String(s) => s.clone(),
            Value::Null => continue,
            other => other.to_string(),
        };
        out.push((key.clone(), cut(text)));
        if out.len() == MAX_SHOWN_ARGUMENTS {
            break;
        }
    }
    out
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
        for (i, opt) in p.request.options.iter().enumerate() {
            let label = extras::permission_label(&opt.option_id, &opt.name);
            let digit = option_digit(opt.kind.clone(), eligible);
            if digit.is_some() {
                eligible += 1;
            }
            let kind = match opt.kind {
                PermissionKind::AllowOnce if i == 0 => ButtonKind::Primary,
                PermissionKind::AllowOnce | PermissionKind::AllowAlways => ButtonKind::Secondary,
                PermissionKind::RejectOnce | PermissionKind::RejectAlways => ButtonKind::Danger,
                PermissionKind::Other => ButtonKind::Secondary,
            };
            let text = match (digits, digit) {
                (true, Some(n)) => format!("{n}  {label}"),
                _ => label,
            };
            let clicked = widgets::button(ui, &text, kind).clicked();
            if clicked || (digits && digit.is_some_and(|n| take_digit(ui.ctx(), n - 1))) {
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
                RichText::new(
                    "Diese Anfrage gehört zu keinem geöffneten Verlauf – vermutlich ein \
                     Subagent. Bitte antworten, sonst wartet Vibe endlos.",
                )
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
            RichText::new(format!(
                "„{}“ läuft zum {}. Mal hintereinander mit denselben Eingaben. Vibe kann sich \
                 selbst nicht bremsen – du kannst den laufenden Turn hier abbrechen.",
                rep.title, rep.count
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
            permission_arguments(Some(&raw), false),
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
        let raw = serde_json::json!({"filePath": "a.txt", "content": "x".repeat(5000)});
        assert_eq!(permission_arguments(Some(&raw), true).len(), 1);
        let long = permission_arguments(Some(&raw), false);
        let body = &long.iter().find(|(k, _)| k == "content").unwrap().1;
        assert!(body.ends_with('…') && body.chars().count() == 301);
        assert!(permission_arguments(None, false).is_empty());
        assert!(permission_arguments(Some(&serde_json::json!("x")), false).is_empty());
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
