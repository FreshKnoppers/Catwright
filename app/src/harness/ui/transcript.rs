//! The conversation: user and agent messages with their hover actions,
//! thoughts, tool groups, real line diffs, the plan and the busy row.

use super::*;
use crate::acp::protocol::{
    PlanEntry, PlanStatus, ToolCall, ToolCallContent, ToolKind, ToolStatus,
};
use crate::acp::session::{
    diff_stat_cached, round_key, tool_group_summary, tool_groups, tool_title, Item, Round,
    SessionView,
};
use crate::diff::{self, LineKind, Segment};
use crate::harness::state::{CommentEdit, DiffComment, Narration, TurnDiffView, TurnDiffs};
use crate::harness::{extras, stop_reason_note};
use crate::i18n::{tr, trf};
use crate::icons;
use crate::model::UiEvent;
use crate::render::RenderCtx;
use crate::widgets::{line_galley, mono, paint_line};
use egui::{pos2, Rect, Sense, Stroke};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Actions from message rows, applied after the transcript is drawn.
pub(crate) enum MessageOp {
    Rewind(String),
    Fork(String),
    /// Vibe's `/retry` for the finished turn.
    Retry,
    /// Open a file path in the Artefakte preview.
    OpenFile(String),
    /// Load one file's diff for the turn (`_review/turnDiff`).
    TurnDiff {
        turn_id: String,
        path: String,
    },
    /// Start writing a comment on one diff line.
    CommentAt {
        key: String,
        line: usize,
    },
    /// Turn all comments of one diff into a composer draft.
    SendComments {
        key: String,
        path: String,
    },
    /// The speaker of the agent answer at this item position: read it aloud,
    /// or stop the narration of the answer being read.
    Narrate(usize),
}

pub(super) fn show(h: &mut Harness, ui: &mut egui::Ui, sid: &str) {
    let jump = std::mem::take(&mut h.ui.scroll_to_end);
    let jump_round =
        h.ui.scroll_to_round
            .take()
            .filter(|(jsid, _)| jsid == sid)
            .map(|(_, y)| y);
    // every session keeps its own scroll position
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("transcript", sid))
        .auto_shrink([false, false])
        // Off for the frame of a round jump: egui (0.29) keeps a view that
        // is stuck to the end there even after a programmatic offset and
        // pulls it back down at the end of the frame. Without sticking that
        // frame, egui re-evaluates "stuck" from the new offset (no longer
        // the end), and the view stays on the round.
        .stick_to_bottom(jump || jump_round.is_none());
    // A jump to the end scrolls to the content's last row (below). A huge
    // offset like f32::MAX must not be used: egui lays the frame out at
    // that offset before clamping, and positions near 3e38 lose all
    // precision, so the measured content height comes out wrong.
    if let (false, Some(y)) = (jump, jump_round) {
        area = area.vertical_scroll_offset(y);
    }
    let mut ops = Vec::new();
    let mut round_tops = Vec::new();
    let output = area.show(ui, |ui| {
        // round tops are drawn in screen space; the scroll offset is in
        // content space: subtract the content's top edge
        let origin = ui.max_rect().top();
        ui.add_space(16.0);
        column(ui, |ui| {
            round_tops = render_items(h, ui, sid, &mut ops);
        });
        for top in &mut round_tops {
            *top -= origin;
        }
        // room under the last row for the navigation pills, so they never
        // cover it (the busy row with the cat sits there)
        ui.add_space(NAV_RESERVE);
        if jump {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }
    });
    for op in ops {
        match op {
            MessageOp::Rewind(message_id) => h.begin_rewind(sid, &message_id),
            MessageOp::Fork(message_id) => h.fork_active_at(Some(message_id)),
            MessageOp::TurnDiff { turn_id, path } => h.load_turn_diff(sid, &turn_id, &path),
            MessageOp::Retry => h.run_command("/retry"),
            MessageOp::OpenFile(path) => h.open_preview(sid, &path),
            MessageOp::CommentAt { key, line } => {
                h.ui.comment_edit = Some(CommentEdit {
                    key,
                    line,
                    text: String::new(),
                });
            }
            MessageOp::SendComments { key, path } => send_comments(h, &key, &path),
            MessageOp::Narrate(index) => h.toggle_narration(sid, index),
        }
    }
    // the pills of the NEXT frame jump to the positions this frame drew
    let current = output.state.offset.y;
    let max_offset = (output.content_size.y - output.inner_rect.height()).max(0.0);
    let prev = round_jump_target(&round_tops, current, max_offset, true);
    let next = round_jump_target(&round_tops, current, max_offset, false);
    let to_end = max_offset - current > 160.0;
    // one centred row: it follows the window width and shrinks to icons
    // when the labels do not fit
    let mut pills: Vec<(Icon, &str, NavJump)> = Vec::new();
    if let Some(y) = prev {
        pills.push((Icon::ChevronUp, tr("Previous round"), NavJump::Round(y)));
    }
    if let Some(y) = next {
        pills.push((Icon::ArrowDown, tr("Next round"), NavJump::Round(y)));
    }
    if to_end {
        pills.push((Icon::ArrowDown, tr("To the end"), NavJump::End));
    }
    // no navigation pills over a modal (settings, rename, …): they are
    // Foreground like the dialog and would sit on top of it
    if pills.is_empty() || super::modal_open(h) {
        return;
    }
    let r = output.inner_rect;
    let shown: Vec<(Icon, &str)> = pills.iter().map(|(i, l, _)| (*i, *l)).collect();
    let clicked = widgets::floating_pills(
        ui.ctx(),
        "transcript_nav",
        pos2(r.center().x, r.bottom() - 10.0),
        (r.width() - 32.0).max(0.0),
        &shown,
    );
    match clicked.map(|i| pills[i].2) {
        Some(NavJump::Round(y)) => h.ui.scroll_to_round = Some((sid.to_string(), y)),
        Some(NavJump::End) => h.ui.scroll_to_end = true,
        None => {}
    }
}

/// Space under the transcript's last row: the navigation pills (30 px,
/// 10 px above the bottom edge) plus a gap.
const NAV_RESERVE: f32 = 56.0;

/// What a navigation pill does when clicked.
#[derive(Clone, Copy)]
enum NavJump {
    /// Scroll so this content-space y is the top of the viewport.
    Round(f32),
    End,
}

/// Where a round-navigation jump goes: the nearest round top strictly
/// above (`up`) or below the current scroll position, in content
/// coordinates. The small dead zone keeps a pill from jumping to the
/// round the viewport already shows. A top below `max_offset` can only be
/// scrolled to `max_offset`: when that is where the view already is (the
/// end), there is no next round to go to.
fn round_jump_target(tops: &[f32], current: f32, max_offset: f32, up: bool) -> Option<f32> {
    let mut best: Option<f32> = None;
    for &t in tops {
        let t = t.min(max_offset);
        if up && t < current - 0.5 {
            best = Some(best.map_or(t, |b: f32| b.max(t)));
        } else if !up && t > current + 0.5 {
            best = Some(best.map_or(t, |b: f32| b.min(t)));
        }
    }
    best
}

/// " · 1 tool call" / " · {n} tool calls" — singular and plural are separate
/// msgids so the German catalog covers each.
fn tool_calls_suffix(n: usize) -> String {
    if n == 1 {
        tr(" · 1 tool call").to_string()
    } else {
        trf(" · {n} tool calls", &[("n", &n)])
    }
}

fn render_items(
    h: &mut Harness,
    ui: &mut egui::Ui,
    sid: &str,
    ops: &mut Vec<MessageOp>,
) -> Vec<f32> {
    let compact_plan = panel_visible(h, ui.ctx());
    // The transcript redraws every frame, so this head must not clone per
    // frame. `review`, `review_scopes` and `turn_diffs` are independent
    // fields of the harness: disjoint borrows replace the old deep clones,
    // whose per-frame cost was the worst of the three (turn diffs carry
    // whole file contents as baselines).
    let Harness {
        sessions,
        ui_events,
        form_values,
        scripts,
        open_groups,
        diff_open,
        review,
        review_scopes,
        turn_diffs,
        questions,
        turn_started,
        retrying,
        last_turn_secs,
        voice_narration,
        narration,
        ui: h_ui,
        ..
    } = h;
    let detail = h_ui.transcript_detail.clone();
    let review: &[extras::ReviewFile] = review.get(sid).map_or(&[], Vec::as_slice);
    let scopes: &[extras::ReviewScope] = review_scopes.get(sid).map_or(&[], Vec::as_slice);
    let diffs: &TurnDiffs = turn_diffs.get(sid).unwrap_or(TurnDiffs::empty());
    let has_question = questions
        .iter()
        .any(|q| q.session_id.as_deref() == Some(sid));
    let Some(s) = sessions.get_mut(sid) else {
        return Vec::new();
    };
    let running = s.turn != TurnState::Idle;
    // the detail level's auto-open defaults for a running turn
    let auto_group = running && matches!(detail.as_str(), "detailed" | "verbose");
    let auto_output = running && detail == "verbose";
    // compact: the running round collapses to a single summary line
    let compact = running && detail == "compact";
    // retention hint (audit N1): the oldest items left the view's memory
    // once the transcript outgrew its bound; Vibe keeps the real history
    if s.trimmed_front > 0 {
        ui.label(
            RichText::new(trf(
                "… {count} older transcript items are not shown",
                &[("count", &s.trimmed_front.to_string())],
            ))
            .color(theme::DIM)
            .italics()
            .size(13.5),
        );
    }
    // a finished round folds to its last answer; the running turn's rounds
    // stay open, older ones stay folded meanwhile
    let rounds = s.folded_rounds();
    let groups = tool_groups(&s.items);
    let cwd = s.cwd.as_path();
    // turnId -> round: Vibe's turn ids are the user message ids (rewind and
    // `_rewind/to` take them), so a round is found by its own message id
    let mut round_of: HashMap<&str, &Round> = HashMap::new();
    for round in &rounds {
        if let Some(id) = &round.user_message_id {
            round_of.insert(id.as_str(), round);
        }
    }
    let comments = &mut Comments {
        map: &mut h_ui.diff_comments,
        edit: &mut h_ui.comment_edit,
        decision_open: has_question || !s.permissions.is_empty(),
    };
    let mut turn_ops: Vec<MessageOp> = Vec::new();
    let mut turn_changes = TurnChanges {
        cwd,
        review,
        scopes,
        diffs,
        round_of: &round_of,
        ops: &mut turn_ops,
    };
    let mut round_tops: Vec<f32> = Vec::with_capacity(rounds.len());
    let mut compact_line_shown = false;
    for round in &rounds {
        // where this round starts on screen: the round navigation jumps
        // here (also for a folded round, whose summary row is its top)
        round_tops.push(ui.cursor().top());
        let key = round_key(sid, round);
        let expanded = open_groups.contains(&key);
        let detail_key = format!("detail::{key}");
        if round.shows_summary() {
            let (fold, detail) = round_summary(
                ui,
                &s.items,
                round,
                expanded,
                open_groups.contains(&detail_key),
            );
            if detail {
                toggle_key(open_groups, detail_key.clone());
            } else if fold {
                toggle_key(open_groups, key.clone());
            }
        }
        if open_groups.contains(&detail_key) {
            round_detail(ui, s, round);
        }
        // compact mode: the running round's tool calls collapse into one
        // line at their position in the round — busy marker, tool count,
        // current step; agent text and changed files stay visible
        let is_running_round = running && std::ptr::eq(round, rounds.last().unwrap_or(round));
        if compact && is_running_round {
            let step = s
                .items
                .iter()
                .filter_map(|i| match i {
                    Item::Tool(id) => s.tool_calls.get(id),
                    _ => None,
                })
                .find(|c| matches!(c.status, ToolStatus::InProgress | ToolStatus::Pending))
                .map(|c| tool_title(c, s.has_subagents()).into_owned())
                .unwrap_or_default();
            let mut label = tr("Working").to_string();
            if round.tool_calls > 0 {
                label.push_str(&tool_calls_suffix(round.tool_calls));
            }
            if !step.is_empty() {
                label.push_str(&trf(" · {step}", &[("step", &step)]));
            }
            let mut ctx = ItemCtx {
                sid,
                running,
                compact_plan,
                cwd,
                plan: &s.plan,
                open_groups,
                closed_groups: &mut h_ui.closed_groups,
                auto_group: false,
                auto_output: false,
                skip_tools: true,
                compact_line: Some(label),
                diff_open,
                comments,
                ui_events,
                form_values,
                scripts,
                ops,
                voice_narration: *voice_narration,
                narration,
            };
            render_range(&mut ctx, ui, s, &groups, round.range.clone());
            // painted where the tool calls would be; without any tool call
            // yet the busy row after the loop stays the working marker
            compact_line_shown = ctx.compact_line.is_none();
            if let Some(turn_id) = &round.user_message_id {
                turn_changes.round_files(ui, turn_id);
            }
            continue;
        }
        let range = if round.collapsed && !expanded {
            round
                .answer
                .map_or(round.range.end..round.range.end, |a| a..a + 1)
        } else {
            round.range.clone()
        };
        let mut ctx = ItemCtx {
            sid,
            running,
            compact_plan,
            cwd,
            plan: &s.plan,
            open_groups,
            closed_groups: &mut h_ui.closed_groups,
            auto_group,
            auto_output,
            skip_tools: false,
            compact_line: None,
            diff_open,
            comments,
            ui_events,
            form_values,
            scripts,
            ops,
            voice_narration: *voice_narration,
            narration,
        };
        render_range(&mut ctx, ui, s, &groups, range);
        // the files this round changed, under its user message (also under
        // a folded round: it is the round's outcome)
        if let Some(turn_id) = &round.user_message_id {
            turn_changes.round_files(ui, turn_id);
        }
    }
    turn_changes.unassigned_files(ui);
    ops.append(&mut turn_ops);
    let waiting_user = !s.permissions.is_empty() || has_question;
    if running {
        // in compact mode the running round already shows its one Working
        // line at the tool calls; the busy row with the cat would be a
        // second one — only without any tool call yet does it stay
        if !compact || !compact_line_shown {
            ui.add_space(4.0);
            busy_row(
                ui,
                s,
                turn_started.get(sid).map(|t| t.elapsed().as_secs()),
                retrying.get(sid),
                waiting_user,
            );
        }
    } else {
        turn_footer(ui, s, last_turn_secs.get(sid).copied(), ops);
    }
    round_tops
}

/// What one `render_range` pass needs besides the session view. Bundled so
/// the per-item helpers keep short argument lists.
struct ItemCtx<'a, 'c> {
    sid: &'a str,
    running: bool,
    compact_plan: bool,
    cwd: &'a Path,
    plan: &'a [PlanEntry],
    open_groups: &'a mut HashSet<String>,
    closed_groups: &'a mut HashSet<String>,
    /// Detail-level default: auto-open tool groups and thoughts while running.
    auto_group: bool,
    /// Detail-level default: auto-open tool outputs while running (verbose).
    auto_output: bool,
    /// Compact mode: tool groups are not rendered at all (one summary line
    /// instead); agent text and changed files still are.
    skip_tools: bool,
    /// Compact mode: the one summary line, painted where the first tool
    /// group would be. `None` once painted (or in non-compact rounds).
    compact_line: Option<String>,
    diff_open: &'a mut HashSet<String>,
    comments: &'a mut Comments<'c>,
    ui_events: &'a mut Vec<UiEvent>,
    form_values: &'a mut HashMap<String, String>,
    scripts: &'a mut HashMap<String, crate::script::ScriptTool>,
    ops: &'a mut Vec<MessageOp>,
    /// The speaker buttons are shown (Settings → General → Read answers
    /// aloud).
    voice_narration: bool,
    /// The answer being read aloud, if any.
    narration: &'a Narration,
}

/// Draw `items[range]`, tool calls through their groups. Ranges start at a
/// round start, so a tool group is never cut: `tool_groups` is computed over
/// the whole transcript and every group lies inside one round. The second
/// lifetime is the comments' inner borrow: `&mut` is invariant there, so a
/// single lifetime would pin every field borrow to the whole loop.
fn render_range(
    ctx: &mut ItemCtx<'_, '_>,
    ui: &mut egui::Ui,
    s: &SessionView,
    groups: &[std::ops::Range<usize>],
    range: std::ops::Range<usize>,
) {
    let mut i = range.start;
    while i < range.end {
        if let Some(g) = groups.iter().find(|g| g.start == i) {
            let calls: Vec<&ToolCall> = s.items[g.clone()]
                .iter()
                .filter_map(|it| match it {
                    Item::Tool(id) => s.tool_calls.get(id),
                    _ => None,
                })
                .collect();
            if ctx.skip_tools {
                // compact: one summary line where the tool calls would be
                if let Some(label) = ctx.compact_line.take() {
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                    widgets::paint_busy(ui, pos2(rect.left() + 5.0, rect.center().y - 1.0), 3.0);
                    let g = line_galley(
                        ui,
                        &label,
                        widgets::font(13.5),
                        theme::MUTED,
                        rect.width() - 30.0,
                    );
                    paint_line(ui, rect.left() + 22.0, rect.center().y, g);
                }
                // the changed files stay visible at every detail level; in
                // the other modes tool_group draws them below the calls
                let files = group_diffs(&calls);
                let any_file = !files.is_empty();
                for file in files {
                    diff_chip(
                        ui,
                        ctx.sid,
                        ctx.cwd,
                        &file,
                        ctx.diff_open,
                        ctx.comments,
                        ctx.ops,
                    );
                }
                if any_file {
                    ui.add_space(8.0);
                }
                i = g.end;
                continue;
            }
            tool_group(
                ui,
                ctx.sid,
                g.start,
                &calls,
                ctx.running,
                ctx.cwd,
                ctx.open_groups,
                ctx.closed_groups,
                ctx.auto_group,
                ctx.auto_output,
                ctx.diff_open,
                ctx.comments,
                ctx.ops,
                s.has_subagents(),
            );
            i = g.end;
            continue;
        }
        match &s.items[i] {
            Item::User { text, message_id } => {
                let images = user_message(ui, text, message_id.as_deref(), !ctx.running, ctx.ops);
                if !images.is_empty() {
                    attachment_rows(
                        ui,
                        ctx.sid,
                        i,
                        message_id.as_deref(),
                        &images,
                        ctx.open_groups,
                    );
                }
            }
            Item::Agent { text, message_id } => {
                let blocks = crate::markdown::parse_cached(text);
                let top = ui.cursor().top();
                // path-like words naming a file of the session's workspace
                // render as chips; a click opens the preview pane
                let mut chip_opens: Vec<String> = Vec::new();
                {
                    let mut rctx = RenderCtx::new(ctx.ui_events, ctx.form_values, ctx.sid)
                        .with_scripts(ctx.scripts);
                    rctx.file_chips = Some(crate::render::FileChips {
                        cwd: ctx.cwd,
                        open: &mut chip_opens,
                    });
                    crate::render::render_blocks(ui, &blocks, i, &mut rctx);
                }
                for p in chip_opens {
                    ctx.ops.push(MessageOp::OpenFile(p));
                }
                let reading = ctx.narration.is_answer(ctx.sid, message_id.as_deref(), i);
                let speaker = speaker_button(ctx.voice_narration, reading);
                if agent_actions(ui, top, text, speaker) {
                    ctx.ops.push(MessageOp::Narrate(i));
                }
            }
            Item::Thought { text, message_id } => {
                let streaming = ctx.running && i + 1 == s.items.len();
                thought_row(
                    ui,
                    ctx.sid,
                    i,
                    message_id.as_deref(),
                    text,
                    streaming,
                    ctx.open_groups,
                    ctx.closed_groups,
                    ctx.auto_group,
                );
            }
            Item::Plan => {
                if !ctx.plan.is_empty() {
                    inline_plan(ui, ctx.sid, ctx.plan, ctx.compact_plan, ctx.open_groups);
                }
            }
            Item::Tool(_) => {} // drawn through its group above
        }
        i += 1;
    }
}

/// Flip one `open_groups` entry: present means expanded.
fn toggle_key(set: &mut HashSet<String>, key: String) {
    if !set.remove(&key) {
        set.insert(key);
    }
}

/// Effective open state of a collapsible row: an explicit open wins, then an
/// explicit close, then the detail level's default for a running turn.
pub(crate) fn effective_open(
    open: &HashSet<String>,
    closed: &HashSet<String>,
    key: &str,
    auto: bool,
) -> bool {
    open.contains(key) || (auto && !closed.contains(key))
}

/// Toggle a row that may be auto-open: closing records the key in `closed`
/// so the auto default stops re-opening it, opening clears that record.
pub(crate) fn toggle_effective(
    open: &mut HashSet<String>,
    closed: &mut HashSet<String>,
    key: &str,
    currently_open: bool,
) {
    if currently_open {
        open.remove(key);
        closed.insert(key.to_string());
    } else {
        closed.remove(key);
        open.insert(key.to_string());
    }
}

/// The composer draft built from a diff's comments: one line per comment
/// with its new-side line number.
pub(crate) fn comments_prompt(path: &str, comments: &[DiffComment]) -> String {
    let mut out = trf(
        "Please address these review comments on {path}:",
        &[("path", &path)],
    );
    for c in comments {
        out.push_str(&format!("\nL{}: {}", c.line, c.text));
    }
    out
}

/// Put the comments of the diff `key` into the composer as one draft.
pub(crate) fn send_comments(h: &mut Harness, key: &str, path: &str) {
    let comments = h.ui.diff_comments.get(key).cloned().unwrap_or_default();
    if !comments.is_empty() {
        h.draft = comments_prompt(path, &comments);
        h.ui.focus_composer = true;
        h.status = tr("Comments are in the input field — add to them and send.").to_string();
    }
}

/// Borrowed comment state of a diff view (transcript or preview pane): the
/// stored comments plus the comment being written.
pub(crate) struct Comments<'a> {
    map: &'a mut HashMap<String, Vec<DiffComment>>,
    edit: &'a mut Option<CommentEdit>,
    /// The session waits for an approval or an answer: that card holds the
    /// one Primary action on screen, so "Send to Vibe" steps back.
    decision_open: bool,
}

impl<'a> Comments<'a> {
    pub(crate) fn new(
        map: &'a mut HashMap<String, Vec<DiffComment>>,
        edit: &'a mut Option<CommentEdit>,
        decision_open: bool,
    ) -> Self {
        Self {
            map,
            edit,
            decision_open,
        }
    }
}

// ---------------------------------------------------------------------------
// fold-state keys: bound to content identity, not transcript position
// ---------------------------------------------------------------------------

/// Key of a tool group's fold state: the group's FIRST call id. Item
/// indices are positions in the transcript — after a rewind a later run
/// can occupy them with different content, and an index key would hand
/// a stale fold state to whatever ends up there (review N2). A group
/// without a single call cannot occur in practice; its position stays
/// as the fallback, like `round_key` does without a message id.
pub(crate) fn tool_group_key(owner: &str, start: usize, calls: &[&ToolCall]) -> String {
    match calls.first() {
        Some(call) => format!("{owner}#grp#{}", call.tool_call_id),
        None => format!("{owner}#grp@{start}"),
    }
}

/// Key of one thought row's fold state: the thought's message id when Vibe
/// sent one, the item index as the fallback otherwise.
pub(crate) fn thought_key(sid: &str, message_id: Option<&str>, index: usize) -> String {
    match message_id {
        Some(id) => format!("{sid}#thought#{id}"),
        None => format!("{sid}#thought@{index}"),
    }
}

/// The per-round detail view: every tool call of the round with its status
/// and client-side duration (arrival timestamps, ToolCall::first_seen /
/// last_update).
fn round_detail(ui: &mut egui::Ui, s: &SessionView, round: &Round) {
    let now = extras::now_epoch().max(0) as u64;
    let has_subagents = s.has_subagents();
    indented(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for it in &s.items[round.range.clone()] {
            let Item::Tool(id) = it else { continue };
            let Some(call) = s.tool_calls.get(id) else {
                continue;
            };
            ui.horizontal(|ui| {
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                let color = theme::status_color(&call.status);
                let square =
                    Rect::from_min_size(pos2(r.left() + 4.0, r.center().y - 3.0), vec2(6.0, 6.0));
                if call.status == ToolStatus::Pending {
                    ui.painter()
                        .rect_stroke(square, 0.0, Stroke::new(1.0_f32, color));
                } else {
                    ui.painter().rect_filled(square, 0.0, color);
                }
                icons::paint(
                    ui.painter(),
                    kind_icon(call),
                    pos2(r.left() + 16.0, r.center().y - 3.5),
                    1.0,
                    theme::DIM,
                );
                let status = match call.status {
                    ToolStatus::Pending => tr("Pending"),
                    ToolStatus::InProgress => tr("Running"),
                    ToolStatus::Completed => tr("Done"),
                    ToolStatus::Failed => tr("Failed"),
                    ToolStatus::Other => tr("Done"),
                };
                let dur = call
                    .duration_secs(now)
                    .map(extras::human_duration)
                    .unwrap_or_else(|| "–".to_string());
                let title = tool_title(call, has_subagents);
                let g = line_galley(ui, &title, mono(12.0), theme::MUTED, r.width() - 200.0);
                paint_line(ui, r.left() + 30.0, r.center().y, g);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(dur).monospace().size(10.5).color(theme::DIM));
                    ui.add_space(8.0);
                    ui.label(RichText::new(status).size(10.5).color(color));
                });
            });
        }
    });
    ui.add_space(4.0);
}

/// Key of one attachment row's fold state: the message's id (the index as
/// the fallback) plus the image's slot inside that message.
pub(crate) fn attachment_key(
    sid: &str,
    message_id: Option<&str>,
    index: usize,
    n: usize,
) -> String {
    match message_id {
        Some(id) => format!("{sid}#img#{id}#{n}"),
        None => format!("{sid}#img@{index}#{n}"),
    }
}

/// Scroll id of a tool's output box: the owning view, the tool call's id
/// and the box's slot — never the content. A content-derived key (its
/// length) made two equally long outputs share one scroll state, and a
/// streamed output got a new one per chunk, jumping the box to the top
/// (review N16).
pub(crate) fn output_scroll_id(owner: &str, call_id: &str, slot: usize) -> egui::Id {
    egui::Id::new((owner, "tool_output", call_id, slot))
}

/// The one line that stands for a folded round: the user message shortened
/// plus how many tool calls it took. Clicking expands or folds it again; a
/// small button on the right opens the per-round detail (tool calls with
/// status and duration). Returns (fold clicked, detail clicked).
fn round_summary(
    ui: &mut egui::Ui,
    items: &[Item],
    round: &Round,
    expanded: bool,
    detail_open: bool,
) -> (bool, bool) {
    let title = items
        .get(round.range.start)
        .and_then(|i| match i {
            Item::User { text, .. } => {
                let visible = crate::harness::without_image_blocks(text);
                Some(crate::harness::without_ui_hint(&visible).trim().to_string())
            }
            _ => None,
        })
        .unwrap_or_default();
    // unfolded, the user message itself follows right below this row:
    // the row then names its action instead of repeating the message
    let label = if expanded {
        tr("Fold round").to_string()
    } else if title.is_empty() {
        tr("Round").to_string()
    } else {
        trf("“{title}”", &[("title", &shorten(&title, 90))])
    };
    // the tool count is its own run: a long message shortens, the count
    // always stays readable
    let suffix = if round.tool_calls > 0 {
        tool_calls_suffix(round.tool_calls)
    } else {
        String::new()
    };
    // a quiet card (fill + 1 px edge + accent strip), so a folded round
    // reads as something to open at first glance, not as plain text
    ui.add_space(2.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let fill = if resp.hovered() {
        theme::RAISED
    } else {
        theme::HOVER
    };
    ui.painter()
        .rect(rect, 3.0, fill, Stroke::new(1.0_f32, theme::BORDER_SOFT));
    ui.painter().rect_filled(
        Rect::from_min_size(rect.left_top(), vec2(3.0, rect.height())),
        egui::Rounding {
            nw: 3.0,
            sw: 3.0,
            ne: 0.0,
            se: 0.0,
        },
        theme::ORANGE_DIM,
    );
    icons::paint(
        ui.painter(),
        Icon::Rewind,
        pos2(rect.left() + 12.0, rect.center().y - 7.0),
        2.0,
        theme::DIM,
    );
    let color = if resp.hovered() {
        theme::TEXT
    } else {
        theme::MUTED
    };
    // room on the right: the fold chevron, plus the detail button when the
    // round has tool calls
    let reserve = if round.tool_calls > 0 { 100.0 } else { 70.0 };
    let tail = line_galley(ui, &suffix, widgets::font(13.5), color, rect.width());
    let g = line_galley(
        ui,
        &label,
        widgets::font(13.5),
        color,
        (rect.width() - reserve - tail.size().x).max(40.0),
    );
    let r = paint_line(ui, rect.left() + 30.0, rect.center().y, g);
    let r = if suffix.is_empty() {
        r
    } else {
        paint_line(ui, r.right(), rect.center().y, tail)
    };
    chevron(ui, pos2(r.right() + 6.0, rect.center().y - 3.5), expanded);
    // right side: the per-round detail toggle (real widget over the row)
    let mut detail_clicked = false;
    if round.tool_calls > 0 {
        let b = widgets::place(
            ui,
            Rect::from_min_size(
                pos2(rect.right() - 30.0, rect.center().y - 10.0),
                vec2(24.0, 24.0),
            ),
            IconButton::new(Icon::List, tr("Round details"))
                .side(20.0)
                .color(if detail_open {
                    theme::ORANGE
                } else {
                    theme::DIM
                }),
        );
        detail_clicked = b.clicked();
    }
    ui.add_space(6.0);
    let fold = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if expanded {
            tr("Fold round")
        } else {
            tr("Unfold round")
        })
        .clicked()
        && !detail_clicked;
    (fold, detail_clicked)
}

/// `text` shortened to at most `max` characters, with an ellipsis.
fn shorten(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let mut out: String = one_line.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// messages
// ---------------------------------------------------------------------------

/// The user's bubble, right-aligned, with copy / rewind / fork on hover.
/// Vibe's `<image>` description blocks (what the describer made of an
/// attached image, meant for the model) never become bubble text: they are
/// returned for `attachment_rows` to draw as collapsible rows instead.
fn user_message(
    ui: &mut egui::Ui,
    text: &str,
    message_id: Option<&str>,
    idle: bool,
    ops: &mut Vec<MessageOp>,
) -> Vec<crate::harness::ImageBlock> {
    let raw = crate::harness::without_ui_hint(text).trim();
    let (visible, images) = crate::harness::split_image_blocks(raw);
    if !visible.is_empty() {
        let font = widgets::font(14.5);
        let max_text = (ui.available_width() * 0.82 - 24.0).clamp(80.0, 620.0);
        let galley = ui.fonts(|f| f.layout(visible.to_string(), font, theme::TEXT, max_text));
        let size = galley.size() + vec2(24.0, 16.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), size.y), Sense::hover());
        let bubble = Rect::from_min_size(pos2(row.right() - size.x, row.top()), size);
        ui.painter().rect_filled(bubble, 6.0, theme::RAISED);
        widgets::place(
            ui,
            bubble.shrink2(vec2(12.0, 8.0)),
            egui::Label::new(galley).selectable(true),
        );
        ui.add_space(2.0);
        if ui.rect_contains_pointer(row) {
            // the actions sit left of the bubble, so they cost no vertical space
            let mut x = bubble.left() - 4.0;
            let y = bubble.top() + 1.0;
            let mut place = |ui: &mut egui::Ui, icon: Icon, tip: &str| {
                x -= 24.0;
                let r = Rect::from_min_size(pos2(x, y), vec2(22.0, 22.0));
                widgets::place(ui, r, IconButton::new(icon, tip).side(22.0)).clicked()
            };
            if place(ui, Icon::Copy, tr("Copy")) {
                ui.ctx().copy_text(visible.to_string());
            }
            if let (Some(id), true) = (message_id, idle) {
                if place(ui, Icon::Rewind, tr("Rewind to here")) {
                    ops.push(MessageOp::Rewind(id.to_string()));
                }
                if place(ui, Icon::Fork, tr("Fork after this round")) {
                    ops.push(MessageOp::Fork(id.to_string()));
                }
            }
        }
    }
    images
}

/// One collapsible row per attached image: the alias as the label, Vibe's
/// description of the picture (what the model sees) behind the chevron.
fn attachment_rows(
    ui: &mut egui::Ui,
    sid: &str,
    index: usize,
    message_id: Option<&str>,
    images: &[crate::harness::ImageBlock],
    open_groups: &mut HashSet<String>,
) {
    for (n, img) in images.iter().enumerate() {
        let key = attachment_key(sid, message_id, index, n);
        let id = egui::Id::new(key.as_str());
        let open = open_groups.contains(&key);
        let label = trf("Image: {file}", &[("file", &img.alias)]);
        if collapsible_row(ui, Icon::File, &label, open, theme::MUTED) {
            toggle_key(open_groups, key);
        }
        if open {
            indented(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt(id)
                    .max_height(200.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(
                            RichText::new(img.description.as_str())
                                .monospace()
                                .size(12.0)
                                .color(theme::MUTED),
                        ));
                    });
            });
        }
        ui.add_space(2.0);
    }
}

/// The speaker button of one agent answer: `None` while the setting is off.
/// The tooltip says what a click does; the icon is ORANGE while this answer
/// is being read, and then the button stays visible without hovering.
pub(crate) fn speaker_button(voice_narration: bool, reading: bool) -> Option<SpeakerButton> {
    if !voice_narration {
        return None;
    }
    Some(if reading {
        SpeakerButton {
            icon: Icon::Stop,
            tooltip: tr("Stop reading"),
            color: theme::ORANGE,
            pinned: true,
        }
    } else {
        SpeakerButton {
            icon: Icon::Speaker,
            tooltip: tr("Read aloud"),
            color: theme::MUTED,
            pinned: false,
        }
    })
}

/// The speaker button of one agent answer, as `agent_actions` draws it.
pub(crate) struct SpeakerButton {
    pub icon: Icon,
    pub tooltip: &'static str,
    pub color: egui::Color32,
    /// Shown permanently instead of only while the answer is hovered.
    pub pinned: bool,
}

/// Height of the action row under every agent answer.
const ANSWER_ACTIONS_H: f32 = 20.0;

/// The action row under an agent answer: copy and, with the setting on, the
/// speaker left of it, right-aligned at the END of the answer, so a long
/// answer needs no scrolling back up. The row is always reserved (the
/// buttons only show while the answer is hovered), so nothing below jumps
/// when they appear; while the answer is read the speaker shows without
/// hovering. Returns true when the speaker was clicked; the caller decides
/// what that means.
fn agent_actions(ui: &mut egui::Ui, top: f32, text: &str, speaker: Option<SpeakerButton>) -> bool {
    let (row, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), ANSWER_ACTIONS_H),
        egui::Sense::hover(),
    );
    let right = row.right();
    let body = Rect::from_min_max(pos2(row.left(), top), pos2(right, row.bottom()));
    let hovered = ui.rect_contains_pointer(body);
    let side = ANSWER_ACTIONS_H;
    let mut clicked = false;
    // The speaker keeps its place left of the copy button, so it does not
    // jump away from the pointer when the hover actions appear.
    if let Some(speaker) = speaker {
        if hovered || speaker.pinned {
            let r = Rect::from_min_size(pos2(right - 2.0 * side, row.top()), vec2(side, side));
            if widgets::place(
                ui,
                r,
                IconButton::new(speaker.icon, speaker.tooltip)
                    .side(side)
                    .color(speaker.color)
                    .fill(theme::BG),
            )
            .clicked()
            {
                clicked = true;
            }
        }
    }
    if hovered {
        let r = Rect::from_min_size(pos2(right - side, row.top()), vec2(side, side));
        if widgets::place(
            ui,
            r,
            IconButton::new(Icon::Copy, tr("Copy answer"))
                .side(side)
                .fill(theme::BG),
        )
        .clicked()
        {
            ui.ctx().copy_text(text.to_string());
        }
    }
    clicked
}

#[allow(clippy::too_many_arguments)]
fn thought_row(
    ui: &mut egui::Ui,
    sid: &str,
    index: usize,
    message_id: Option<&str>,
    text: &str,
    streaming: bool,
    open_groups: &mut HashSet<String>,
    closed_groups: &mut HashSet<String>,
    auto: bool,
) {
    let key = thought_key(sid, message_id, index);
    let open = effective_open(open_groups, closed_groups, &key, auto);
    if collapsible_row(
        ui,
        Icon::Spark,
        if streaming {
            tr("Thinking …")
        } else {
            tr("Thoughts")
        },
        open,
        theme::DIM,
    ) {
        toggle_effective(open_groups, closed_groups, &key, open);
    }
    if open {
        indented(ui, |ui| {
            ui.label(RichText::new(text).color(theme::DIM).italics().size(13.5));
        });
    }
    ui.add_space(4.0);
}

/// A quiet row with icon, label and chevron. Returns true on click.
fn collapsible_row(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    open: bool,
    color: egui::Color32,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::HOVER);
    }
    icons::paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 4.0, rect.center().y - 7.0),
        2.0,
        color,
    );
    let g = line_galley(ui, label, widgets::font(13.5), color, rect.width() - 50.0);
    let r = paint_line(ui, rect.left() + 26.0, rect.center().y, g);
    chevron(ui, pos2(r.right() + 6.0, rect.center().y - 3.5), open);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn chevron(ui: &egui::Ui, pos: egui::Pos2, open: bool) {
    let icon = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    icons::paint(ui.painter(), icon, pos, 1.0, theme::DIM);
}

/// Content indented under a row, with a guide line on the left.
fn indented<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let out = egui::Frame::none()
        .inner_margin(egui::Margin {
            left: 14.0,
            right: 0.0,
            top: 2.0,
            bottom: 2.0,
        })
        .show(ui, add);
    let r = out.response.rect;
    ui.painter().vline(
        r.left() + 6.0,
        r.y_range(),
        Stroke::new(1.0_f32, theme::BORDER_SOFT),
    );
    out.inner
}

// ---------------------------------------------------------------------------
// plan
// ---------------------------------------------------------------------------

/// Pixel checkbox of a plan entry.
fn plan_check(ui: &egui::Ui, origin: egui::Pos2, status: &PlanStatus) {
    let rect = Rect::from_min_size(origin, vec2(11.0, 11.0));
    match status {
        PlanStatus::Completed => {
            ui.painter().rect_filled(rect, 0.0, theme::GREEN);
            icons::paint(
                ui.painter(),
                Icon::Check,
                origin + vec2(2.0, 2.0),
                1.0,
                theme::CARD,
            );
        }
        PlanStatus::InProgress => {
            ui.painter()
                .rect_stroke(rect, 0.0, Stroke::new(1.0_f32, theme::YELLOW));
            ui.painter()
                .rect_filled(rect.shrink(3.0), 0.0, theme::YELLOW);
        }
        _ => {
            ui.painter()
                .rect_stroke(rect, 0.0, Stroke::new(1.0_f32, theme::DIM));
        }
    }
}

/// The plan's checklist rows (shared with the right panel).
pub(super) fn plan_rows(ui: &mut egui::Ui, plan: &[PlanEntry], size: f32) {
    for e in plan {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(13.0, size + 4.0), Sense::hover());
            plan_check(ui, rect.min + vec2(0.0, 3.0), &e.status);
            let color = match e.status {
                PlanStatus::Completed => theme::DIM,
                PlanStatus::InProgress => theme::TEXT,
                _ => theme::MUTED,
            };
            ui.add(egui::Label::new(RichText::new(&e.content).size(size).color(color)).wrap());
        });
    }
}

/// `(done, total)` of a plan.
pub(super) fn plan_progress(plan: &[PlanEntry]) -> (usize, usize) {
    let done = plan
        .iter()
        .filter(|e| e.status == PlanStatus::Completed)
        .count();
    (done, plan.len())
}

/// The plan's place in the conversation: the full card, or one line when
/// the right panel already shows the checklist.
fn inline_plan(
    ui: &mut egui::Ui,
    sid: &str,
    plan: &[PlanEntry],
    compact: bool,
    open_groups: &mut HashSet<String>,
) {
    if !compact {
        plan_card(ui, plan);
        ui.add_space(10.0);
        return;
    }
    let (done, total) = plan_progress(plan);
    let key = format!("{sid}#plan");
    let open = open_groups.contains(&key);
    let label = match plan.iter().find(|e| e.status == PlanStatus::InProgress) {
        Some(e) => format!("{} {done}/{total} · {}", tr("Plan"), e.content),
        None => format!("{} {done}/{total}", tr("Plan")),
    };
    if collapsible_row(ui, Icon::List, &label, open, theme::MUTED) {
        if open {
            open_groups.remove(&key);
        } else {
            open_groups.insert(key);
        }
    }
    if open {
        indented(ui, |ui| plan_rows(ui, plan, 14.0));
    }
    ui.add_space(6.0);
}

fn plan_card(ui: &mut egui::Ui, plan: &[PlanEntry]) {
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        let (done, total) = plan_progress(plan);
        ui.horizontal(|ui| {
            theme::section_header(ui, "Plan", done < total);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::mono_tag(&format!("{done}/{total}"), theme::DIM));
            });
        });
        ui.add_space(4.0);
        plan_rows(ui, plan, 14.0);
    });
}

// ---------------------------------------------------------------------------
// tools and diffs
// ---------------------------------------------------------------------------

fn kind_icon(call: &ToolCall) -> Icon {
    match call.kind {
        ToolKind::Read => Icon::Eye,
        ToolKind::Edit => Icon::Pencil,
        ToolKind::Delete => Icon::Trash,
        ToolKind::Move => Icon::File,
        ToolKind::Search => Icon::Search,
        ToolKind::Execute => Icon::Terminal,
        ToolKind::Think => Icon::Spark,
        ToolKind::Fetch => Icon::Globe,
        ToolKind::SwitchMode => Icon::Shield,
        ToolKind::Other => Icon::Dots,
    }
}

/// One file's diffs inside a tool group.
struct DiffFile<'a> {
    path: &'a str,
    call_id: &'a str,
    parts: Vec<(Option<&'a str>, &'a str)>,
    added: usize,
    removed: usize,
}

fn group_diffs<'a>(calls: &[&'a ToolCall]) -> Vec<DiffFile<'a>> {
    let mut out: Vec<DiffFile<'a>> = Vec::new();
    for call in calls {
        if call.status == ToolStatus::Failed {
            continue;
        }
        for content in &call.content {
            let ToolCallContent::Diff {
                path,
                old_text,
                new_text,
            } = content
            else {
                continue;
            };
            let stat = diff_stat_cached(old_text.as_deref(), new_text);
            let part = (old_text.as_deref(), new_text.as_str());
            match out.iter_mut().find(|f| f.path == path) {
                Some(f) => {
                    f.parts.push(part);
                    f.added += stat.added;
                    f.removed += stat.removed;
                }
                None => out.push(DiffFile {
                    path,
                    call_id: &call.tool_call_id,
                    parts: vec![part],
                    added: stat.added,
                    removed: stat.removed,
                }),
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn tool_group(
    ui: &mut egui::Ui,
    owner: &str,
    start: usize,
    calls: &[&ToolCall],
    running: bool,
    cwd: &Path,
    open_groups: &mut HashSet<String>,
    closed_groups: &mut HashSet<String>,
    auto_group: bool,
    auto_output: bool,
    diff_open: &mut HashSet<String>,
    comments: &mut Comments,
    ops: &mut Vec<MessageOp>,
    has_subagents: bool,
) {
    let active = calls
        .iter()
        .find(|c| matches!(c.status, ToolStatus::InProgress | ToolStatus::Pending));
    let busy = running && active.is_some();
    let failed = calls.iter().any(|c| c.status == ToolStatus::Failed);
    let key = tool_group_key(owner, start, calls);
    let open = effective_open(open_groups, closed_groups, &key, auto_group);

    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::HOVER);
    }
    let marker = pos2(rect.left() + 5.0, rect.center().y - 4.0);
    if busy {
        widgets::paint_busy(ui, marker + vec2(0.0, 2.0), 3.0);
    } else {
        let color = if failed { theme::RED } else { theme::GREEN };
        ui.painter()
            .rect_filled(Rect::from_min_size(marker, vec2(8.0, 8.0)), 0.0, color);
    }
    let summary = match active.filter(|_| busy) {
        Some(c) => tool_title(c, has_subagents).into_owned(),
        None => tool_group_summary(calls),
    };
    let color = if resp.hovered() {
        theme::TEXT
    } else {
        theme::MUTED
    };
    let g = line_galley(
        ui,
        &summary,
        widgets::font(13.5),
        color,
        rect.width() - 60.0,
    );
    let r = paint_line(ui, rect.left() + 22.0, rect.center().y, g);
    chevron(ui, pos2(r.right() + 6.0, rect.center().y - 3.5), open);
    if failed {
        ui.painter().galley(
            pos2(rect.right() - 62.0, rect.center().y - 7.0),
            line_galley(ui, tr("ERROR"), mono(10.5), theme::RED_TEXT, 60.0),
            theme::RED_TEXT,
        );
    }
    let clicked = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked();
    if clicked {
        toggle_effective(open_groups, closed_groups, &key, open);
    }
    if open {
        indented(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for call in calls {
                tool_row(
                    ui,
                    owner,
                    call,
                    open_groups,
                    closed_groups,
                    auto_output,
                    ops,
                    has_subagents,
                );
            }
        });
    }
    for file in group_diffs(calls) {
        diff_chip(ui, owner, cwd, &file, diff_open, comments, ops);
    }
    ui.add_space(8.0);
}

#[allow(clippy::too_many_arguments)]
fn tool_row(
    ui: &mut egui::Ui,
    owner: &str,
    call: &ToolCall,
    open_groups: &mut HashSet<String>,
    closed_groups: &mut HashSet<String>,
    auto_output: bool,
    ops: &mut Vec<MessageOp>,
    has_subagents: bool,
) {
    let output: Vec<&str> = call
        .content
        .iter()
        .filter_map(|c| match c {
            ToolCallContent::Content { content } => content.as_text(),
            _ => None,
        })
        .collect();
    let key = format!("{owner}#tool#{}", call.tool_call_id);
    let open = effective_open(open_groups, closed_groups, &key, auto_output);
    let has_output = output.iter().any(|t| !t.trim().is_empty());
    let (rect, resp) = ui.allocate_exact_size(
        vec2(ui.available_width(), 24.0),
        if has_output {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if has_output && resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::HOVER);
    }
    let square = Rect::from_min_size(
        pos2(rect.left() + 4.0, rect.center().y - 3.0),
        vec2(6.0, 6.0),
    );
    let color = theme::status_color(&call.status);
    if call.status == ToolStatus::Pending {
        ui.painter()
            .rect_stroke(square, 0.0, Stroke::new(1.0_f32, color));
    } else {
        ui.painter().rect_filled(square, 0.0, color);
    }
    icons::paint(
        ui.painter(),
        kind_icon(call),
        pos2(rect.left() + 16.0, rect.center().y - 3.5),
        1.0,
        theme::DIM,
    );
    let file = call
        .locations
        .first()
        .map(|l| l.path.clone())
        .filter(|_| call.kind != ToolKind::Fetch);
    let title = tool_title(call, has_subagents);
    let g = line_galley(
        ui,
        &title,
        mono(12.5),
        theme::MUTED,
        rect.width() - if file.is_some() { 78.0 } else { 54.0 },
    );
    let r = paint_line(ui, rect.left() + 30.0, rect.center().y, g);
    if let Some(path) = file {
        // clickable path (Handoff Ideenliste 5): the file opens in the
        // Artefakte preview
        let btn = Rect::from_min_size(
            pos2(rect.right() - 26.0, rect.center().y - 11.0),
            vec2(22.0, 22.0),
        );
        if widgets::place(
            ui,
            btn,
            IconButton::new(Icon::Eye, tr("Open file preview")).side(22.0),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        {
            ops.push(MessageOp::OpenFile(path));
        }
    }
    if has_output {
        chevron(ui, pos2(r.right() + 6.0, rect.center().y - 3.5), open);
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            toggle_effective(open_groups, closed_groups, &key, open);
        }
    }
    if open && has_output {
        output_box(
            ui,
            output_scroll_id(owner, &call.tool_call_id, 0),
            &output.join("\n"),
        );
    }
}

/// A tool's text output in a scrollable mono box with a copy button.
fn output_box(ui: &mut egui::Ui, id: egui::Id, text: &str) {
    const MAX_LINES: usize = 400;
    egui::Frame::none()
        .fill(theme::CARD)
        .rounding(egui::Rounding::same(3.0))
        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
        .outer_margin(egui::Margin {
            left: 28.0,
            right: 0.0,
            top: 2.0,
            bottom: 4.0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::mono_label(tr("Output")));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(IconButton::new(Icon::Copy, tr("Copy output")).side(20.0))
                        .clicked()
                    {
                        ui.ctx().copy_text(text.to_string());
                    }
                });
            });
            let total = text.lines().count();
            let shown: String = text.lines().take(MAX_LINES).collect::<Vec<_>>().join("\n");
            // both directions: tool output is terminal text, so long lines
            // keep their length and scroll instead of wrapping (cell wrap
            // stays possible for tables, which can fit the width). The id
            // is identity-based, so a streamed output keeps its scroll
            // position chunk by chunk.
            egui::ScrollArea::both()
                .id_salt(id)
                .max_height(260.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(shown)
                                .monospace()
                                .size(12.0)
                                .color(theme::MUTED),
                        )
                        .extend(),
                    );
                    if total > MAX_LINES {
                        ui.label(
                            RichText::new(trf("… {n} more lines", &[("n", &(total - MAX_LINES))]))
                                .monospace()
                                .size(12.0)
                                .color(theme::DIM),
                        );
                    }
                });
        });
}

/// One edited file: path, `+a −r`, and the diff when expanded.
#[allow(clippy::too_many_arguments)]
fn diff_chip(
    ui: &mut egui::Ui,
    owner: &str,
    cwd: &Path,
    file: &DiffFile,
    diff_open: &mut HashSet<String>,
    comments: &mut Comments,
    ops: &mut Vec<MessageOp>,
) {
    let key = format!("{owner}#{}::{}", file.call_id, file.path);
    let open = diff_open.contains(&key);
    let rel = relative_path(file.path, cwd);
    ui.add_space(3.0);
    egui::Frame::none()
        .fill(theme::CARD)
        .rounding(egui::Rounding::same(4.0))
        .stroke(Stroke::new(1.0_f32, theme::BORDER_SOFT))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 4.0, theme::HOVER);
            }
            icons::paint(
                ui.painter(),
                Icon::File,
                pos2(rect.left() + 9.0, rect.center().y - 7.0),
                2.0,
                theme::DIM,
            );
            let stats = stat_galleys(ui, file.added, file.removed);
            let stats_w = stats.iter().map(|g| g.size().x).sum::<f32>() + 8.0;
            // the preview button sits between the stats and the chevron
            let prev_btn = Rect::from_min_size(
                pos2(rect.right() - 48.0, rect.center().y - 11.0),
                vec2(22.0, 22.0),
            );
            if widgets::place(
                ui,
                prev_btn,
                IconButton::new(Icon::Eye, tr("Open file preview")).side(22.0),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            {
                ops.push(MessageOp::OpenFile(file.path.to_string()));
            }
            let mut right = rect.right() - 74.0;
            let g = line_galley(
                ui,
                &rel,
                mono(12.5),
                theme::TEXT,
                right - stats_w - rect.left() - 40.0,
            );
            paint_line(ui, rect.left() + 28.0, rect.center().y, g);
            for s in stats.into_iter().rev() {
                right -= s.size().x;
                paint_line(ui, right, rect.center().y, s);
                right -= 6.0;
            }
            chevron(ui, pos2(rect.right() - 16.0, rect.center().y - 3.5), open);
            let resp = resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(file.path);
            if resp.clicked() {
                if open {
                    diff_open.remove(&key);
                } else {
                    diff_open.insert(key.clone());
                }
            }
            if open {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, (old, new)) in file.parts.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(6.0);
                    }
                    diff_body(
                        ui,
                        *old,
                        new,
                        400,
                        Some((key.as_str(), &mut *comments, &mut *ops)),
                    );
                }
                comment_section(ui, &key, file.path, comments, ops);
                ui.add_space(4.0);
            }
        });
}

/// The review comments of one open diff: the editor for the line being
/// commented, the stored comments, and the "Send to Vibe" button that turns
/// them into a composer draft.
pub(crate) fn comment_section(
    ui: &mut egui::Ui,
    key: &str,
    path: &str,
    comments: &mut Comments,
    ops: &mut Vec<MessageOp>,
) {
    // the comments themselves hang under their lines (line_comments); here
    // only the count and the one action that sends them all to the composer
    let n = comments.map.get(key).map_or(0, Vec::len);
    if n == 0 {
        return;
    }
    ui.add_space(6.0);
    let indent = ui
        .data(|d| d.get_temp::<f32>(egui::Id::new(("diff_text_x", key))))
        .unwrap_or(26.0);
    // as tall as the button, so the label sits on its middle
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 28.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_space(indent);
            let label = if n == 1 {
                tr("1 comment on this file").to_string()
            } else {
                trf("{n} comments on this file", &[("n", &n.to_string())])
            };
            ui.label(RichText::new(label).color(theme::DIM).size(12.0));
            // while the editor is open the primary action is the editor's own
            // send, and while an approval or a question waits that card's; this
            // one stays Secondary then, so the screen never shows two primaries
            let kind = if comments.edit.is_some() || comments.decision_open {
                ButtonKind::Secondary
            } else {
                ButtonKind::Primary
            };
            if widgets::button(ui, tr("Send to Vibe"), kind).clicked() {
                ops.push(MessageOp::SendComments {
                    key: key.to_string(),
                    path: path.to_string(),
                });
            }
        },
    );
}

// ---------------------------------------------------------------------------
// changes per turn
// ---------------------------------------------------------------------------

/// The files a turn changed, drawn under the round that turn belongs to
/// (`_review/state` `scopes[]`). A click on a file loads its turn diff
/// (`_review/turnDiff`) and shows it through `diff_body`. Borrows only —
/// the transcript redraws every frame and must not copy per-frame data.
struct TurnChanges<'a> {
    cwd: &'a Path,
    /// Undecided files: an unassigned path shows even when its scope is
    /// gone, so no open change is lost from the transcript.
    review: &'a [extras::ReviewFile],
    scopes: &'a [extras::ReviewScope],
    diffs: &'a TurnDiffs,
    /// turnId -> round of the transcript on screen.
    round_of: &'a HashMap<&'a str, &'a Round>,
    ops: &'a mut Vec<MessageOp>,
}

impl TurnChanges<'_> {
    /// The files of the round started by `turn_id` (nothing when Vibe
    /// reported no changes for it).
    fn round_files(&mut self, ui: &mut egui::Ui, turn_id: &str) {
        let files: Vec<&str> = self
            .scopes
            .iter()
            .filter(|s| s.turn_id.as_deref() == Some(turn_id))
            .flat_map(|s| s.files.iter().map(String::as_str))
            .collect();
        if files.is_empty() {
            return;
        }
        indented(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for path in files {
                self.file_row(ui, Some(turn_id), path);
            }
        });
    }

    /// Changes Vibe cannot tie to a round on screen: a rewind cut their
    /// round, or the session was loaded without a transcript for them.
    /// Shown at the end of the transcript instead of being dropped.
    fn unassigned_files(&mut self, ui: &mut egui::Ui) {
        let mut orphans: Vec<&str> = Vec::new();
        for scope in self.scopes {
            let known = scope
                .turn_id
                .as_deref()
                .is_some_and(|id| self.round_of.contains_key(id));
            if known {
                continue;
            }
            for path in &scope.files {
                if !orphans.contains(&path.as_str()) {
                    orphans.push(path);
                }
            }
        }
        // a scope-less state (older Vibe, a loaded session) still lists its
        // open files: they stay reachable, without a round
        for f in self.review {
            if !orphans.contains(&f.path.as_str()) {
                orphans.push(&f.path);
            }
        }
        if orphans.is_empty() {
            return;
        }
        ui.add_space(6.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        let g = line_galley(
            ui,
            tr("Changes without a round"),
            widgets::font(12.5),
            theme::DIM,
            rect.width(),
        );
        paint_line(ui, rect.left() + 4.0, rect.center().y, g);
        ui.spacing_mut().item_spacing.y = 2.0;
        for path in orphans {
            self.file_row(ui, None, path);
        }
        ui.add_space(4.0);
    }

    /// One changed file: name, status badge, `+a −r`; a click expands the
    /// turn diff. Without a `turn_id` (no round) the row only opens the
    /// file's folder.
    fn file_row(&mut self, ui: &mut egui::Ui, turn_id: Option<&str>, path: &str) {
        let rel = relative_path(path, self.cwd);
        let open = self.review.iter().find(|f| f.path == path);
        let view = turn_id.and_then(|id| self.diffs.get(id, path));
        let expanded = view.is_some_and(|v| *v != TurnDiffView::Loading);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 3.0, theme::HOVER);
        }
        // the preview button sits left of the stats / chevron
        let prev_btn = Rect::from_min_size(
            pos2(rect.right() - 44.0, rect.center().y - 11.0),
            vec2(22.0, 22.0),
        );
        if widgets::place(
            ui,
            prev_btn,
            IconButton::new(Icon::Eye, tr("Open file preview")).side(22.0),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
        {
            self.ops.push(MessageOp::OpenFile(path.to_string()));
        }
        let mut right = rect.right() - 70.0;
        if let Some(TurnDiffView::Loaded(d)) = view {
            let stat = diff_stat_cached(d.baseline.as_deref(), d.current.as_deref().unwrap_or(""));
            for g in stat_galleys(ui, stat.added, stat.removed).into_iter().rev() {
                right -= g.size().x;
                paint_line(ui, right, rect.center().y, g);
                right -= 6.0;
            }
        }
        let mut x = rect.left() + 4.0;
        if let Some(f) = open {
            let g = line_galley(
                ui,
                extras::review_status_label(&f.status),
                mono(9.5),
                theme::AMBER,
                80.0,
            );
            x = paint_line(ui, x, rect.center().y, g).right() + 8.0;
        }
        let g = line_galley(ui, &rel, mono(12.0), theme::TEXT, right - x - 4.0);
        paint_line(ui, x, rect.center().y, g);
        if turn_id.is_some() {
            chevron(
                ui,
                pos2(rect.right() - 16.0, rect.center().y - 3.5),
                expanded,
            );
        }
        let resp = resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(path);
        if resp.clicked() {
            match turn_id {
                Some(id) => self.ops.push(MessageOp::TurnDiff {
                    turn_id: id.to_string(),
                    path: path.to_string(),
                }),
                None => open_folder_of(path),
            }
        }
        match view {
            Some(TurnDiffView::Loading) => {
                indented(ui, |ui| {
                    ui.label(
                        RichText::new(tr("Loading diff …"))
                            .size(12.0)
                            .color(theme::DIM),
                    );
                });
            }
            Some(TurnDiffView::Failed(note)) => {
                indented(ui, |ui| {
                    ui.label(RichText::new(note).size(12.0).color(theme::RED_TEXT));
                });
            }
            Some(TurnDiffView::Loaded(d)) if expanded => {
                indented(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    diff_body(
                        ui,
                        d.baseline.as_deref(),
                        d.current.as_deref().unwrap_or(""),
                        400,
                        None,
                    );
                    ui.add_space(4.0);
                });
            }
            _ => {}
        }
    }
}

fn stat_galleys(ui: &egui::Ui, added: usize, removed: usize) -> Vec<std::sync::Arc<egui::Galley>> {
    let mut out = Vec::new();
    if added > 0 || removed == 0 {
        out.push(line_galley(
            ui,
            &format!("+{added}"),
            mono(12.0),
            theme::GREEN,
            70.0,
        ));
    }
    if removed > 0 {
        out.push(line_galley(
            ui,
            &format!("−{removed}"),
            mono(12.0),
            theme::RED_TEXT,
            70.0,
        ));
    }
    out
}

/// The real line diff of one text pair, folded to the changed hunks.
/// With `comment` (`key` + state + ops), every line gets a "+" gutter button
/// that starts a review comment on its new-side line number.
/// New-side line number of every segment: `Some(n)` for a line, `None` for a
/// folded block. A skipped block still advances the number — otherwise every
/// line after it would be numbered too low (and "Send to Vibe" would name the
/// wrong line).
pub(crate) fn line_numbers(segments: &[Segment]) -> Vec<Option<usize>> {
    let mut new_line = 0usize;
    segments
        .iter()
        .map(|seg| match seg {
            Segment::Skipped(n) => {
                new_line += n;
                None
            }
            Segment::Line(l) => match l.kind {
                LineKind::Removed => Some(new_line.max(1)),
                _ => {
                    new_line += 1;
                    Some(new_line)
                }
            },
        })
        .collect()
}

/// How `diff_rows` lays out its lines: the transcript wraps long lines in
/// its narrow column; the preview pane shows line numbers and scrolls
/// sideways instead, so code keeps its shape.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct DiffLayout {
    /// Old-side numbers on removed lines, new-side numbers on all others.
    pub numbers: bool,
    /// Wrap long lines; without, a row is as wide as the widest line.
    pub wrap: bool,
}

impl DiffLayout {
    pub(crate) const TRANSCRIPT: DiffLayout = DiffLayout {
        numbers: false,
        wrap: true,
    };
    pub(crate) const PREVIEW: DiffLayout = DiffLayout {
        numbers: true,
        wrap: false,
    };
}

/// The number shown in front of each segment: the old-side number for a
/// removed line, the new-side one for every other line, `None` for a folded
/// block (which advances both sides).
pub(crate) fn display_numbers(segments: &[Segment]) -> Vec<Option<usize>> {
    let (mut old, mut new) = (0usize, 0usize);
    segments
        .iter()
        .map(|seg| match seg {
            Segment::Skipped(n) => {
                old += n;
                new += n;
                None
            }
            Segment::Line(l) => match l.kind {
                LineKind::Removed => {
                    old += 1;
                    Some(old)
                }
                LineKind::Added => {
                    new += 1;
                    Some(new)
                }
                LineKind::Same => {
                    old += 1;
                    new += 1;
                    Some(new)
                }
            },
        })
        .collect()
}

/// The diff in the transcript's layout (wrapped, no line numbers).
pub(crate) fn diff_body(
    ui: &mut egui::Ui,
    old: Option<&str>,
    new: &str,
    max_rows: usize,
    comment: Option<(&str, &mut Comments, &mut Vec<MessageOp>)>,
) {
    diff_rows(ui, old, new, max_rows, comment, DiffLayout::TRANSCRIPT);
}

/// Where the parts of a diff row start, shared by all rows of one diff.
#[derive(Clone, Copy)]
struct DiffGeometry {
    /// Width of the line-number column (0 without numbers).
    numbers_w: f32,
    sign_x: f32,
    text_x: f32,
    /// The rows' width when lines do not wrap (the widest line), else
    /// `None`: as wide as the column.
    row_w: Option<f32>,
}

pub(crate) fn diff_rows(
    ui: &mut egui::Ui,
    old: Option<&str>,
    new: &str,
    max_rows: usize,
    mut comment: Option<(&str, &mut Comments, &mut Vec<MessageOp>)>,
    layout: DiffLayout,
) {
    let segments = diff::folded_cached(old, new);
    let total = segments.len();
    let numbers = line_numbers(&segments);
    let shown_numbers = display_numbers(&segments);
    ui.scope(|ui| {
        // rows touch: an added block is one band, not stripes
        ui.spacing_mut().item_spacing.y = 0.0;
        let gutter = if comment.is_some() { 22.0 } else { 0.0 };
        let numbers_w = if layout.numbers {
            let widest = shown_numbers.iter().flatten().max().copied().unwrap_or(1);
            let digits = widest.to_string().len().max(2);
            let g = line_galley(ui, &"0".repeat(digits), mono(11.0), theme::DIM, 200.0);
            g.size().x + 10.0
        } else {
            0.0
        };
        let sign_x = gutter + numbers_w + if layout.numbers { 4.0 } else { 12.0 };
        let text_x = sign_x + 14.0;
        let row_w = (!layout.wrap).then(|| {
            let widest = segments
                .iter()
                .take(max_rows)
                .filter_map(|seg| match seg {
                    Segment::Line(l) => Some(
                        ui.fonts(|f| f.layout_no_wrap(l.text.clone(), mono(12.0), theme::TEXT))
                            .size()
                            .x,
                    ),
                    Segment::Skipped(_) => None,
                })
                .fold(0.0_f32, f32::max);
            (text_x + widest + 12.0).max(ui.available_width())
        });
        let geo = DiffGeometry {
            numbers_w,
            sign_x,
            text_x,
            row_w,
        };
        // the comment footer under the diff (comment_section) starts where
        // the code does
        if let Some((key, _, _)) = comment.as_ref() {
            let id = egui::Id::new(("diff_text_x", *key));
            ui.data_mut(|d| d.insert_temp(id, text_x));
        }
        let row_width = |ui: &egui::Ui| geo.row_w.unwrap_or_else(|| ui.available_width());
        for ((seg, line_no), shown) in segments
            .iter()
            .zip(numbers.iter())
            .zip(shown_numbers.iter())
            .take(max_rows)
        {
            match seg {
                Segment::Skipped(n) => {
                    let n = *n;
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(row_width(ui), 18.0), Sense::hover());
                    let text = if n == 1 {
                        tr("··· 1 unchanged line").to_string()
                    } else {
                        trf("··· {n} unchanged lines", &[("n", &n)])
                    };
                    let g = line_galley(ui, &text, mono(11.0), theme::DIM, rect.width());
                    paint_line(ui, rect.left() + geo.sign_x + 14.0, rect.center().y, g);
                }
                Segment::Line(line) => {
                    let line_no = line_no.expect("line_numbers pairs with every segment");
                    // the "+" gutter only on lines that exist on the new side
                    // (never on removed ones)
                    let commented = comment.as_ref().is_some_and(|(key, state, _)| {
                        state
                            .map
                            .get(*key)
                            .is_some_and(|l| l.iter().any(|c| c.line == line_no))
                    });
                    let gutter = if line.kind == LineKind::Removed {
                        None
                    } else {
                        comment
                            .as_mut()
                            .map(|(key, _state, ops)| (*key, line_no, &mut **ops))
                    };
                    let number = (*shown).filter(|_| layout.numbers);
                    diff_line(ui, line.kind, &line.text, gutter, commented, number, geo);
                    // comments and the editor hang directly under their line
                    if let Some((key, state, _ops)) = comment.as_mut() {
                        line_comments(ui, key, line_no, state, geo);
                    }
                }
            }
        }
        if total > max_rows {
            let (rect, _) = ui.allocate_exact_size(vec2(row_width(ui), 18.0), Sense::hover());
            let g = line_galley(
                ui,
                &trf("… {n} more lines", &[("n", &(total - max_rows))]),
                mono(11.0),
                theme::DIM,
                rect.width(),
            );
            paint_line(ui, rect.left() + geo.sign_x + 14.0, rect.center().y, g);
        }
    });
}

fn diff_line(
    ui: &mut egui::Ui,
    kind: LineKind,
    text: &str,
    gutter: Option<(&str, usize, &mut Vec<MessageOp>)>,
    commented: bool,
    number: Option<usize>,
    geo: DiffGeometry,
) {
    let (bg, sign, color) = match kind {
        LineKind::Added => (Some(theme::DIFF_ADD_BG), "+", theme::TEXT),
        LineKind::Removed => (Some(theme::DIFF_DEL_BG), "−", theme::TEXT),
        LineKind::Same => (None, " ", theme::DIM),
    };
    let width = geo.row_w.unwrap_or_else(|| ui.available_width());
    let galley = ui.fonts(|f| match geo.row_w {
        Some(_) => f.layout_no_wrap(text.to_string(), mono(12.0), color),
        None => f.layout(
            text.to_string(),
            mono(12.0),
            color,
            (width - geo.text_x - 8.0).max(40.0),
        ),
    });
    let (rect, _) = ui.allocate_exact_size(vec2(width, galley.size().y.max(17.0)), Sense::hover());
    if let Some(bg) = bg {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    // a commented line gets a yellow edge on the left
    if commented {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), vec2(3.0, rect.height())),
            0.0,
            theme::YELLOW,
        );
    }
    if let Some(n) = number {
        let g = line_galley(ui, &n.to_string(), mono(11.0), theme::DIM, geo.numbers_w);
        let x = rect.left() + geo.sign_x - 6.0 - g.size().x;
        ui.painter()
            .galley(pos2(x, rect.top() + 1.0), g, theme::DIM);
    }
    let sign_color = match kind {
        LineKind::Added => theme::GREEN,
        LineKind::Removed => theme::RED_TEXT,
        LineKind::Same => theme::DIM,
    };
    let g = line_galley(ui, sign, mono(12.0), sign_color, 12.0);
    ui.painter()
        .galley(rect.left_top() + vec2(geo.sign_x, 0.0), g, sign_color);
    ui.painter()
        .galley(rect.left_top() + vec2(geo.text_x, 0.0), galley, color);
    if let Some((key, line, ops)) = gutter {
        let btn = Rect::from_min_size(
            rect.left_top() + vec2(2.0, 1.0),
            vec2(18.0, rect.height() - 2.0),
        );
        let id = ui.make_persistent_id(("diff_comment", key, line));
        let resp = ui.interact(btn, id, Sense::click());
        // the "+" only shows on hover, but the button stays clickable
        if resp.hovered() {
            ui.painter().rect_filled(btn, 3.0, theme::HOVER);
            let g = line_galley(ui, "+", mono(12.0), theme::TEXT, 18.0);
            ui.painter()
                .galley(btn.center() - g.size() / 2.0, g, theme::TEXT);
        }
        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            ops.push(MessageOp::CommentAt {
                key: key.to_string(),
                line,
            });
        }
    }
}

/// The comments of one diff line, hanging directly under it, plus the editor
/// when this line is being commented. Both line up with the diff: the yellow
/// edge continues the commented line's edge, text and editor start where
/// the code does.
fn line_comments(
    ui: &mut egui::Ui,
    key: &str,
    line_no: usize,
    comments: &mut Comments,
    geo: DiffGeometry,
) {
    let decision_open = comments.decision_open;
    // what is visible of the row: a non-wrapping diff can be wider than
    // the pane, and the editor and the delete button belong in view
    let visible_w = ui
        .available_width()
        .min(ui.clip_rect().right() - ui.cursor().left());
    let row_w = geo.row_w.unwrap_or(visible_w);
    // the editor for this line
    let mut save = false;
    let mut cancel = false;
    if let Some(e) = comments.edit.as_ref() {
        if e.key == key && e.line == line_no {
            let text = comments.edit.as_mut().unwrap();
            ui.add_space(4.0);
            // the frame's inner margin brings its content to the code; an
            // outer margin instead of a horizontal row keeps it top-down
            let indent = (geo.text_x - 8.0).max(0.0);
            let inner_w = (visible_w - indent - 8.0 - 16.0).max(120.0);
            egui::Frame::none()
                .outer_margin(egui::Margin {
                    left: indent,
                    right: 8.0,
                    top: 0.0,
                    bottom: 0.0,
                })
                .fill(theme::RAISED)
                .rounding(egui::Rounding::same(4.0))
                .stroke(Stroke::new(1.0_f32, theme::BORDER_SOFT))
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    ui.set_width(inner_w);
                    {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.label(
                            RichText::new(trf(
                                "Comment on line {n}",
                                &[("n", &line_no.to_string())],
                            ))
                            .color(theme::DIM)
                            .size(12.0),
                        );
                        // the input field gets its own visible frame
                        egui::Frame::none()
                            .fill(theme::BG)
                            .stroke(Stroke::new(1.0_f32, theme::BORDER))
                            .rounding(egui::Rounding::same(2.0))
                            .inner_margin(egui::Margin::symmetric(6.0, 4.0))
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut text.text)
                                        .frame(false)
                                        .desired_rows(2)
                                        .desired_width(f32::INFINITY)
                                        .hint_text(tr("Your comment …")),
                                );
                            });
                        // an open approval or question keeps the one Primary
                        let kind = if decision_open {
                            ButtonKind::Secondary
                        } else {
                            ButtonKind::Primary
                        };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if widgets::button(ui, tr("Comment"), kind).clicked() {
                                save = true;
                            }
                            if widgets::button(ui, tr("Cancel"), ButtonKind::Secondary).clicked() {
                                cancel = true;
                            }
                        });
                    }
                });
            ui.add_space(4.0);
        }
    }
    if save {
        if let Some(e) = comments.edit.take() {
            if !e.text.trim().is_empty() {
                comments
                    .map
                    .entry(key.to_string())
                    .or_default()
                    .push(DiffComment {
                        line: e.line,
                        text: e.text.trim().to_string(),
                    });
            }
        }
    }
    if cancel {
        *comments.edit = None;
    }
    let list: Vec<DiffComment> = comments
        .map
        .get(key)
        .map(|l| l.iter().filter(|c| c.line == line_no).cloned().collect())
        .unwrap_or_default();
    for c in &list {
        // one row per comment: a quiet inset under its line, the text
        // wrapped to what is visible, the delete button at its right end
        let text_w = (visible_w - geo.text_x - 34.0).max(60.0);
        let galley = ui.fonts(|f| {
            f.layout(
                trf(
                    "L{line}: {text}",
                    &[("line", &c.line.to_string()), ("text", &c.text)],
                ),
                widgets::font(12.5),
                theme::TEXT,
                text_w,
            )
        });
        let height = (galley.size().y + 10.0).max(24.0);
        let (rect, _) = ui.allocate_exact_size(vec2(row_w, height), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, theme::HOVER);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), vec2(3.0, rect.height())),
            0.0,
            theme::YELLOW,
        );
        let text_pos = pos2(
            rect.left() + geo.text_x,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, theme::TEXT);
        let right = rect.left() + visible_w;
        let btn = Rect::from_min_size(pos2(right - 26.0, rect.center().y - 10.0), vec2(20.0, 20.0));
        if widgets::place(
            ui,
            btn,
            IconButton::new(Icon::Trash, tr("Delete comment")).side(20.0),
        )
        .clicked()
        {
            if let Some(v) = comments.map.get_mut(key) {
                v.retain(|x| x != c);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// turn state
// ---------------------------------------------------------------------------

fn busy_row(
    ui: &mut egui::Ui,
    s: &SessionView,
    elapsed: Option<u64>,
    retry: Option<&String>,
    waiting_user: bool,
) {
    ui.horizontal(|ui| {
        let time = ui.input(|i| i.time);
        theme::pixel_cat(ui, time, !waiting_user, 2.0);
        ui.add_space(4.0);
        let cancelling = s.turn == TurnState::Cancelling;
        let label = if cancelling {
            tr("Cancelling …")
        } else if waiting_user {
            tr("Waiting for you")
        } else {
            tr("Working")
        };
        ui.label(RichText::new(label).size(13.5).color(theme::MUTED));
        if let Some(secs) = elapsed {
            ui.label(
                RichText::new(extras::elapsed_label(secs))
                    .monospace()
                    .size(12.0)
                    .color(theme::DIM),
            );
        }
        if let Some(note) = retry {
            ui.label(RichText::new(note).size(12.5).color(theme::YELLOW));
        }
        if !cancelling {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::mono_tag(tr("Esc = Cancel"), theme::DIM));
            });
        }
    });
}

/// Stop reason and duration after the turn.
fn turn_footer(ui: &mut egui::Ui, s: &SessionView, secs: Option<u64>, ops: &mut Vec<MessageOp>) {
    let note = s.last_stop_reason.as_ref().and_then(stop_reason_note);
    let mut parts: Vec<String> = Vec::new();
    if let Some(secs) = secs {
        parts.push(trf(
            "Done in {time}",
            &[("time", &extras::elapsed_label(secs))],
        ));
    }
    if let Some(t) = &s.last_turn_usage {
        parts.push(format!(
            "↑ {} ↓ {}",
            crate::harness::format_tokens(t.input_tokens),
            crate::harness::format_tokens(t.output_tokens)
        ));
    }
    if parts.is_empty() && note.is_none() {
        return;
    }
    ui.horizontal(|ui| {
        if let Some(note) = note {
            ui.label(theme::mono_tag(note, theme::AMBER));
        }
        if !parts.is_empty() {
            ui.label(theme::mono_tag(&parts.join(" · "), theme::DIM));
        }
        // Vibe's own retry (a failed turn can be repeated without
        // retyping the message)
        if widgets::button(ui, tr("Try again"), widgets::ButtonKind::Secondary).clicked() {
            ops.push(MessageOp::Retry);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::protocol::PlanPriority;

    fn call(id: &str) -> ToolCall {
        serde_json::from_value(serde_json::json!({ "toolCallId": id })).unwrap()
    }

    /// N2: fold keys bind to content identity (message/tool-call ids),
    /// with the position only as the fallback — after a rewind a later
    /// run occupies the old indices with different content, and an
    /// index key would hand a stale fold state to whatever ends up
    /// there. N16: a tool output's scroll id is content-independent,
    /// so streamed output keeps its scroll position chunk by chunk.
    #[test]
    fn effective_open_prefers_explicit_over_auto() {
        let mut open = HashSet::new();
        let mut closed = HashSet::new();
        // auto default opens the row
        assert!(super::effective_open(&open, &closed, "k", true));
        // no auto default: closed
        assert!(!super::effective_open(&open, &closed, "k", false));
        // an explicit close beats the auto default
        closed.insert("k".to_string());
        assert!(!super::effective_open(&open, &closed, "k", true));
        // an explicit open beats an explicit close
        open.insert("k".to_string());
        assert!(super::effective_open(&open, &closed, "k", false));
    }

    #[test]
    fn toggle_effective_records_the_override() {
        let mut open = HashSet::new();
        let mut closed = HashSet::new();
        // close an auto-opened row: recorded in closed, gone from open
        super::toggle_effective(&mut open, &mut closed, "k", true);
        assert!(!super::effective_open(&open, &closed, "k", true));
        // re-open: the close record is cleared again
        super::toggle_effective(&mut open, &mut closed, "k", false);
        assert!(super::effective_open(&open, &closed, "k", false));
        assert!(closed.is_empty());
    }

    #[test]
    fn line_numbers_advance_across_folded_blocks() {
        // 40 old lines, one change at the very end: the fold skips the
        // first 36 lines, and the line after the skipped block must still
        // get its real new-side number (40), not 4
        let old: String = (0..40).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l39\n", "changed\n");
        let segments = super::diff::folded_cached(Some(&old), &new);
        let numbers = super::line_numbers(&segments);
        assert!(matches!(segments[0], super::Segment::Skipped(36)));
        let added = segments
            .iter()
            .zip(numbers.iter())
            .find(|(seg, _)| {
                matches!(seg, super::Segment::Line(l) if l.kind == super::LineKind::Added)
            })
            .map(|(_, n)| *n)
            .expect("there is an added line");
        assert_eq!(added, Some(40), "the line after the fold keeps its number");
    }

    #[test]
    fn display_numbers_use_the_old_side_for_removed_lines() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\nd\n";
        let segments = super::diff::folded_cached(Some(old), new);
        let shown: Vec<(super::LineKind, usize)> = segments
            .iter()
            .zip(super::display_numbers(&segments))
            .filter_map(|(seg, n)| match seg {
                super::Segment::Line(l) => Some((l.kind, n.unwrap())),
                super::Segment::Skipped(_) => None,
            })
            .collect();
        use super::LineKind::*;
        assert_eq!(
            shown,
            [(Same, 1), (Removed, 2), (Added, 2), (Same, 3), (Added, 4)],
            "{shown:?}"
        );
        // after a folded block both sides go on with their real numbers
        let old: String = (0..40).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l39\n", "changed\n");
        let segments = super::diff::folded_cached(Some(&old), &new);
        let numbers = super::display_numbers(&segments);
        let last_two: Vec<Option<usize>> = numbers.iter().rev().take(2).rev().copied().collect();
        assert_eq!(
            last_two,
            [Some(40), Some(40)],
            "removed old 40, added new 40"
        );
    }

    #[test]
    fn comments_prompt_lists_every_comment_with_its_line() {
        let comments = vec![
            super::DiffComment {
                line: 48,
                text: "append ^ for outliers".to_string(),
            },
            super::DiffComment {
                line: 72,
                text: "take the CSV path from argv".to_string(),
            },
        ];
        let prompt = super::comments_prompt("src/analysis.py", &comments);
        assert!(prompt.contains("src/analysis.py"));
        assert!(prompt.contains("L48: append ^ for outliers"));
        assert!(prompt.contains("L72: take the CSV path from argv"));
        assert_eq!(prompt.matches('\n').count(), 2);
    }

    #[test]
    fn fold_and_scroll_keys_bind_to_identity_not_position() {
        let t1 = call("t1");
        let t2 = call("t2");
        // tool group: the first call's id names the group
        assert_eq!(tool_group_key("s", 4, &[&t1]), "s#grp#t1");
        assert_eq!(
            tool_group_key("s", 9, &[&t1]),
            tool_group_key("s", 4, &[&t1]),
            "the item index does not participate"
        );
        assert_ne!(
            tool_group_key("s", 4, &[&t1]),
            tool_group_key("s", 4, &[&t2])
        );
        assert_ne!(
            tool_group_key("s", 4, &[&t1]),
            tool_group_key("other", 4, &[&t1])
        );
        // a group without a single call falls back to its position
        assert_eq!(tool_group_key("s", 4, &[]), "s#grp@4");
        // thought: the message id wins, the position is the fallback
        assert_eq!(thought_key("s", Some("m7"), 3), "s#thought#m7");
        assert_eq!(thought_key("s", None, 3), "s#thought@3");
        assert_ne!(
            thought_key("s", Some("m7"), 3),
            thought_key("s", Some("m8"), 3)
        );
        // attachments: message id and image slot
        assert_eq!(attachment_key("s", Some("m7"), 3, 2), "s#img#m7#2");
        assert_eq!(attachment_key("s", None, 3, 2), "s#img@3#2");
        assert_ne!(
            attachment_key("s", Some("m7"), 3, 2),
            attachment_key("s", None, 3, 2)
        );
        // tool output scroll: never the content, but owner, call and slot
        assert_eq!(
            output_scroll_id("s", "t1", 0),
            output_scroll_id("s", "t1", 0)
        );
        assert_ne!(
            output_scroll_id("s", "t1", 0),
            output_scroll_id("s", "t2", 0)
        );
        assert_ne!(
            output_scroll_id("s", "t1", 0),
            output_scroll_id("other", "t1", 0)
        );
        assert_ne!(
            output_scroll_id("s", "t1", 0),
            output_scroll_id("s", "t1", 1)
        );
    }

    /// egui 0.29 pulls a stuck-to-the-end ScrollArea back down after a
    /// programmatic offset unless sticking is off for that frame. This runs
    /// the transcript's scroll setup headless: a jump up from the end must
    /// stay, and later frames (sticking on again) must not undo it.
    #[test]
    fn a_round_jump_from_the_end_stays() {
        let ctx = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                pos2(0.0, 0.0),
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let frame = |jump_to: Option<f32>, rows: usize| -> f32 {
            let mut offset = 0.0;
            let _ = ctx.run(input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut area = egui::ScrollArea::vertical()
                        .id_salt("t")
                        .auto_shrink([false, false])
                        .stick_to_bottom(jump_to.is_none());
                    if let Some(y) = jump_to {
                        area = area.vertical_scroll_offset(y);
                    }
                    offset = area
                        .show(ui, |ui| {
                            for i in 0..rows {
                                ui.label(format!("Zeile {i}"));
                            }
                        })
                        .state
                        .offset
                        .y;
                });
            });
            offset
        };
        // settle at the end of a long transcript
        let mut end = 0.0;
        for _ in 0..3 {
            end = frame(None, 200);
        }
        assert!(end > 1000.0, "stuck to the end: {end}");
        assert_eq!(frame(Some(500.0), 200), 500.0);
        // following frames, sticking requested again, new rows arriving
        for _ in 0..3 {
            assert_eq!(frame(None, 201), 500.0, "pulled back to the end");
        }
    }

    /// Round navigation (Handoff Ideenliste 4): the jump goes to the
    /// nearest round top above/below the current scroll position, never to
    /// the round the viewport already shows.
    #[test]
    fn round_jump_target_picks_the_nearest_round() {
        let tops = [100.0, 800.0, 1500.0, 2400.0];
        // between rounds 1 and 2
        let max = f32::INFINITY;
        assert_eq!(round_jump_target(&tops, 900.0, max, true), Some(800.0));
        assert_eq!(round_jump_target(&tops, 900.0, max, false), Some(1500.0));
        // right on a round's top: the dead zone keeps it as "current"
        assert_eq!(round_jump_target(&tops, 800.0, max, true), Some(100.0));
        assert_eq!(round_jump_target(&tops, 800.0, max, false), Some(1500.0));
        // at the very top/bottom there is nothing further
        assert_eq!(round_jump_target(&tops, 50.0, max, true), None);
        assert_eq!(round_jump_target(&tops, 2500.0, max, false), None);
        assert_eq!(round_jump_target(&[], 100.0, max, true), None);
        // at the end (offset 2000 = max): the last round starts inside the
        // viewport, so there is no next round, and up goes to round 3
        assert_eq!(round_jump_target(&tops, 2000.0, 2000.0, false), None);
        assert_eq!(round_jump_target(&tops, 2000.0, 2000.0, true), Some(1500.0));
    }

    fn entry(status: PlanStatus) -> PlanEntry {
        PlanEntry {
            content: "x".to_string(),
            priority: PlanPriority::High,
            status,
        }
    }

    #[test]
    fn shorten_collapses_whitespace_and_cuts_long_text() {
        assert_eq!(shorten("  Hallo\n\nWelt  ", 90), "Hallo Welt");
        assert_eq!(shorten("abcdef", 4), "abc…");
        assert_eq!(shorten("abcd", 4), "abcd");
        assert_eq!(shorten("äöüß", 3), "äö…", "counts characters, not bytes");
    }

    #[test]
    fn plan_progress_counts_completed_entries() {
        let plan = vec![
            entry(PlanStatus::Completed),
            entry(PlanStatus::InProgress),
            entry(PlanStatus::Pending),
        ];
        assert_eq!(plan_progress(&plan), (1, 3));
        assert_eq!(plan_progress(&[]), (0, 0));
    }
}
