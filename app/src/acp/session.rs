//! Pure, IO-free view state for one ACP session: `SessionView` is built
//! solely from `session/update` notifications (plus the prompt result and
//! permission requests) and contains everything the egui UI draws — user
//! bubbles, answer text, collapsible thoughts, grouped tool calls, the plan
//! checklist, file chips with diff stats, permission cards and the context
//! usage. No I/O, no threads, no logging; UI strings are German.

use crate::acp::protocol::*;
use crate::i18n::{fill, tr};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// One renderable block of the session transcript.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    User {
        message_id: Option<String>,
        text: String,
    },
    Agent {
        message_id: Option<String>,
        text: String,
    },
    Thought {
        message_id: Option<String>,
        text: String,
    },
    /// Reference into `SessionView::tool_calls` (by toolCallId).
    Tool(String),
    /// Position of the plan checklist within the current turn (the entries
    /// themselves live in `SessionView::plan`, always the latest).
    Plan,
}

impl Item {
    fn message_id(&self) -> Option<&str> {
        match self {
            Item::User { message_id, .. }
            | Item::Agent { message_id, .. }
            | Item::Thought { message_id, .. } => message_id.as_deref(),
            Item::Tool(_) | Item::Plan => None,
        }
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Item::User { text, .. } | Item::Agent { text, .. } | Item::Thought { text, .. } => {
                Some(text)
            }
            Item::Tool(_) | Item::Plan => None,
        }
    }

    /// Same chunk kind (User/Agent/Thought); Tool and Plan never merge.
    fn same_kind(&self, other: &Item) -> bool {
        matches!(
            (self, other),
            (Item::User { .. }, Item::User { .. })
                | (Item::Agent { .. }, Item::Agent { .. })
                | (Item::Thought { .. }, Item::Thought { .. })
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnState {
    Idle,
    Running,
    Cancelling,
}

#[derive(Debug, Clone)]
pub struct PendingPermission {
    pub request_id: Value,
    pub request: PermissionRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffStat {
    pub added: usize,
    pub removed: usize,
}

/// Lifecycle of one subagent as far as the parent session can observe it
/// (docs/acp.md, "Subagents").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentState {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// One subagent started by the parent session, derived from its spawn tool
/// call and the parent's `self.wait` call.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentInfo {
    pub tool_call_id: String,
    pub name: String,
    pub agent: String,
    pub task: String,
    pub state: SubagentState,
    pub child_session_id: Option<String>,
    pub turns: Option<u64>,
}

/// True for the tool call that starts a subagent (`agents.spawn`).
pub fn is_subagent(tc: &ToolCall) -> bool {
    let Some(meta) = tc.meta.as_ref() else {
        return false;
    };
    meta.get("effect_kind").and_then(Value::as_str) == Some("subagent")
        || meta.get("tool_name").and_then(Value::as_str) == Some("agents.spawn")
}

/// `_meta` string field, falling back to the same field of `rawInput`.
fn meta_or_input_str(tc: &ToolCall, key: &str) -> Option<String> {
    let from_meta = tc
        .meta
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(Value::as_str);
    let from_input = tc
        .raw_input
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(Value::as_str);
    from_meta.or(from_input).map(str::to_string)
}

/// True for the parent's wait call that completes once the child delivered
/// its result.
fn is_wait_call(tc: &ToolCall) -> bool {
    tc.meta
        .as_ref()
        .and_then(|m| m.get("tool_name"))
        .and_then(Value::as_str)
        == Some("self.wait")
}

/// The whole renderable state of one session. Build it with `new`, feed it
/// `apply_update` for every `session/update`, `add_permission` for every
/// `session/request_permission`, and `begin_turn`/`finish_turn` around a
/// `session/prompt`. `Clone` exists for `session/fork`: the fork keeps the
/// transcript Vibe already has, so the client shows its copy of it.
#[derive(Clone)]
pub struct SessionView {
    pub session_id: String,
    pub cwd: PathBuf,
    pub title: Option<String>,
    pub items: Vec<Item>,
    pub tool_calls: HashMap<String, ToolCall>,
    pub plan: Vec<PlanEntry>,
    pub modes: Option<ModeState>,
    pub config_options: Vec<ConfigOption>,
    pub commands: Vec<AvailableCommand>,
    /// Latest usage_update (context window usage). usage_update is always
    /// authoritative; after a model switch the window part may be replaced
    /// from Vibe's config until the next one arrives (set_model_window).
    pub usage: Option<UsageUpdate>,
    pub last_turn_usage: Option<TurnUsage>,
    pub last_stop_reason: Option<StopReason>,
    pub permissions: Vec<PendingPermission>,
    /// Painted frames per pending permission, keyed by the request id (its
    /// compact JSON text). The number-key lockout on the permission cards
    /// measures in painted frames, not wall-clock time (N14): a UI stall at
    /// the moment the card appears would otherwise expire the grace before
    /// the user ever saw the card. Cleared wherever the permissions are.
    pub permission_frames: HashMap<String, u32>,
    /// Subagent spawn ids frozen as Cancelled: the ids of the subagents
    /// that were still unfinished when a turn ended cancelled (N5). A
    /// global `last_stop_reason` cannot hold this state — the next
    /// `begin_turn` clears it and would show long-dead subagents as
    /// running again. Reset together with the transcript (rewind/load);
    /// a fork keeps it, like the copied transcript it belongs to.
    pub cancelled_tools: HashSet<String>,
    pub turn: TurnState,
    /// Index in `items` where the latest turn of this run began
    /// (`begin_turn`): what it added lies at or after it. `None` without a
    /// turn in this run (a loaded history), or once a rewind cut it away.
    pub turn_start: Option<usize>,
    /// How many of the oldest transcript items the retention bound dropped
    /// ([`Self::trim_front`], audit N1). The transcript shows the count as
    /// its first line; the authoritative history stays with Vibe, a
    /// rewind restores from there.
    pub trimmed_front: u32,
}

impl SessionView {
    /// A fork of this session under a new id: the transcript stays, the
    /// turn is idle, pending permissions belong to the source and do
    /// not carry over (`session/fork` does not replay them).
    pub fn forked(&self, session_id: impl Into<String>) -> Self {
        let mut fork = self.clone();
        fork.session_id = session_id.into();
        fork.turn = TurnState::Idle;
        fork.permissions = Vec::new();
        // frame counters belong to the fork's own pending permissions —
        // it starts without any
        fork.permission_frames = HashMap::new();
        // the copied ids belong to the source session: Vibe gives the fork
        // its own entries, so rewind/fork points must not use them
        for item in &mut fork.items {
            if let Item::User { message_id, .. } = item {
                *message_id = None;
            }
        }
        fork
    }

    /// Drop everything from the user message `message_id` on (rewind).
    /// Returns false when no such message is in the transcript.
    pub fn rewind_to_message(&mut self, message_id: &str) -> bool {
        let Some(pos) = self
            .items
            .iter()
            .position(|i| matches!(i, Item::User { message_id: Some(m), .. } if m == message_id))
        else {
            return false;
        };
        self.cut_items(pos);
        self.permissions.clear();
        self.permission_frames.clear();
        self.last_stop_reason = None;
        true
    }

    /// Keep the transcript only up to the end of the turn of the user
    /// message `message_id` (the fork point of `session/fork`).
    pub fn keep_through_turn_of(&mut self, message_id: &str) {
        let Some(pos) = self
            .items
            .iter()
            .position(|i| matches!(i, Item::User { message_id: Some(m), .. } if m == message_id))
        else {
            return;
        };
        let end = self.items[pos + 1..]
            .iter()
            .position(|i| matches!(i, Item::User { .. }))
            .map(|n| pos + 1 + n);
        if let Some(end) = end {
            self.cut_items(end);
        }
    }

    /// Truncate the transcript and forget what only the cut part referenced.
    fn cut_items(&mut self, at: usize) {
        self.items.truncate(at);
        let keep: std::collections::HashSet<&String> = self
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Tool(id) => Some(id),
                _ => None,
            })
            .collect();
        self.tool_calls.retain(|k, _| keep.contains(k));
        // frozen subagent cancellations only mean something for tools
        // that are still part of the transcript
        self.cancelled_tools
            .retain(|id| self.tool_calls.contains_key(id));
        if !self.items.iter().any(|i| matches!(i, Item::Plan)) {
            self.plan.clear();
        }
        // the latest turn was cut away: no part of the rest belongs to it
        if self.turn_start.is_some_and(|start| start >= at) {
            self.turn_start = None;
        }
    }

    pub fn new(session_id: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            session_id: session_id.into(),
            cwd: cwd.into(),
            title: None,
            items: Vec::new(),
            tool_calls: HashMap::new(),
            plan: Vec::new(),
            modes: None,
            config_options: Vec::new(),
            commands: Vec::new(),
            usage: None,
            last_turn_usage: None,
            last_stop_reason: None,
            permissions: Vec::new(),
            permission_frames: HashMap::new(),
            cancelled_tools: HashSet::new(),
            turn: TurnState::Idle,
            turn_start: None,
            trimmed_front: 0,
        }
    }

    /// The rounds of this transcript with the folding decision applied
    /// (`fold_rounds`).
    pub fn folded_rounds(&self) -> Vec<Round> {
        fold_rounds(
            &self.items,
            &self.turn,
            self.turn_start,
            self.last_stop_reason.as_ref(),
        )
    }

    /// Modes and config options from the `session/new` or `session/load`
    /// response.
    pub fn set_session_info(
        &mut self,
        modes: Option<ModeState>,
        config_options: Vec<ConfigOption>,
    ) {
        self.modes = modes;
        self.config_options = config_options;
    }

    /// Clear the transcript before a `session/load` replays the history.
    pub fn reset_items(&mut self) {
        self.items.clear();
        self.tool_calls.clear();
        self.plan.clear();
        self.permissions.clear();
        self.permission_frames.clear();
        self.cancelled_tools.clear();
        self.turn_start = None;
        self.trimmed_front = 0;
    }

    pub fn apply_update(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::UserMessageChunk(c) => self.apply_chunk(c, ChunkKind::User),
            SessionUpdate::AgentMessageChunk(c) => self.apply_chunk(c, ChunkKind::Agent),
            SessionUpdate::AgentThoughtChunk(c) => self.apply_chunk(c, ChunkKind::Thought),
            SessionUpdate::ToolCall(tc) => {
                let id = tc.tool_call_id.clone();
                // Replay/duplicate: replace the entry, no second Item::Tool.
                // The bounded copy caps the raws' size (audit N1).
                let is_new = !self.tool_calls.contains_key(&id);
                self.tool_calls.insert(id.clone(), tc.bounded_clone());
                if is_new {
                    self.items.push(Item::Tool(id));
                }
            }
            SessionUpdate::ToolCallUpdate(u) => {
                let id = u.tool_call_id.clone();
                match self.tool_calls.get_mut(&id) {
                    Some(tc) => tc.apply(u),
                    None => {
                        let tc = tool_call_from_update(u).bounded_clone();
                        self.tool_calls.insert(id.clone(), tc);
                        self.items.push(Item::Tool(id));
                    }
                }
            }
            SessionUpdate::Plan(p) => {
                self.plan = p.entries.clone();
                let plan_since_user = self
                    .items
                    .iter()
                    .rev()
                    .take_while(|i| !matches!(i, Item::User { .. }))
                    .any(|i| matches!(i, Item::Plan));
                if !plan_since_user {
                    self.items.push(Item::Plan);
                }
            }
            SessionUpdate::AvailableCommandsUpdate(c) => {
                self.commands = c.available_commands.clone();
            }
            SessionUpdate::CurrentModeUpdate(m) => {
                if let Some(modes) = &mut self.modes {
                    modes.current_mode_id = m.current_mode_id.clone();
                }
                for opt in &mut self.config_options {
                    if let ConfigOption::Select {
                        id, current_value, ..
                    } = opt
                    {
                        if id == "mode" {
                            *current_value = m.current_mode_id.clone();
                        }
                    }
                }
            }
            SessionUpdate::ConfigOptionUpdate(c) => {
                self.config_options = c.config_options.clone();
            }
            SessionUpdate::SessionInfoUpdate(info) => {
                if let Some(title) = &info.title {
                    self.title = Some(title.clone());
                }
            }
            SessionUpdate::UsageUpdate(u) => {
                self.usage = Some(u.clone());
            }
            SessionUpdate::Unknown => {}
        }
        self.trim_front();
    }

    /// Retention bound of one session's transcript (audit N1): the child
    /// drives every byte of it, and its memory needs a bound. This one
    /// counts ITEMS: past this many the oldest leave (the raw payloads
    /// of each tool call are bounded separately, in `ToolCall`). What
    /// it does not bound: the text of a single item — chunks of one
    /// message id merge into one growing item — and a tool's `content`
    /// list. Two thousand remaining items keep whole days of normal
    /// work, and a trim only ever removes display memory — Vibe keeps
    /// the authoritative history, a rewind restores from there.
    const MAX_RETAINED_ITEMS: usize = 4_000;
    /// How many items survive a trim.
    const RETAINED_KEEP_ITEMS: usize = 2_000;

    /// Drop the oldest items once the transcript outgrew the retention
    /// bound. The same forgetting rules as `cut_items` apply (tools still
    /// referenced stay, plan entries without their item go), `turn_start`
    /// shifts with the items, and the count feeds the transcript's hint.
    /// One visible cost: the folding keys of pre-trim rounds change, so
    /// their fold state resets — once, when the trim happened.
    fn trim_front(&mut self) {
        if self.items.len() <= Self::MAX_RETAINED_ITEMS {
            return;
        }
        let remove = self.items.len() - Self::RETAINED_KEEP_ITEMS;
        self.items.drain(..remove);
        self.trimmed_front = self.trimmed_front.saturating_add(remove as u32);
        let keep: std::collections::HashSet<&String> = self
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Tool(id) => Some(id),
                _ => None,
            })
            .collect();
        self.tool_calls.retain(|k, _| keep.contains(k));
        self.cancelled_tools
            .retain(|id| self.tool_calls.contains_key(id));
        if !self.items.iter().any(|i| matches!(i, Item::Plan)) {
            self.plan.clear();
        }
        self.turn_start = self.turn_start.map(|s| s.saturating_sub(remove));
    }

    /// The conversation as Markdown (for "Verlauf kopieren"): user and agent
    /// text plus one line per tool call. Thoughts stay out.
    pub fn transcript_markdown(&self, clean: impl Fn(&str) -> &str) -> String {
        let mut out = String::new();
        for item in &self.items {
            match item {
                Item::User { text, .. } => {
                    out.push_str("## Du\n\n");
                    out.push_str(clean(text).trim());
                    out.push_str("\n\n");
                }
                Item::Agent { text, .. } => {
                    out.push_str("## Vibe\n\n");
                    out.push_str(text.trim());
                    out.push_str("\n\n");
                }
                Item::Tool(id) => {
                    if let Some(tc) = self.tool_calls.get(id) {
                        out.push_str(&format!("- `{}`\n\n", tc.title));
                    }
                }
                Item::Thought { .. } | Item::Plan => {}
            }
        }
        out.trim_end().to_string()
    }

    /// A `session/prompt` was sent. The user text is NOT inserted locally:
    /// Vibe echoes it back as a `user_message_chunk`.
    pub fn begin_turn(&mut self) {
        self.turn = TurnState::Running;
        self.last_stop_reason = None;
        self.turn_start = Some(self.items.len());
    }

    /// `session/cancel` was sent. Returns the request ids of all open
    /// permission requests (the UI answers them with "cancelled") and
    /// clears the list.
    pub fn request_cancel(&mut self) -> Vec<Value> {
        if self.turn == TurnState::Running {
            self.turn = TurnState::Cancelling;
        }
        self.permission_frames.clear();
        self.permissions.drain(..).map(|p| p.request_id).collect()
    }

    /// The `session/prompt` request resolved.
    pub fn finish_turn(&mut self, result: &PromptResult) {
        self.turn = TurnState::Idle;
        self.last_stop_reason = Some(result.stop_reason.clone());
        self.last_turn_usage = result.usage.clone();
        self.permissions.clear();
        self.permission_frames.clear();
        if result.stop_reason == StopReason::Cancelled {
            // freeze what the cancel hit, before `begin_turn` clears the
            // stop reason (N5)
            self.cancel_unfinished_subagents();
        }
    }

    /// Freeze the subagents that were still unfinished when the turn ended
    /// cancelled (N5): `subagents()` shows them as Cancelled from here on,
    /// so the next `begin_turn` — which clears `last_stop_reason` — cannot
    /// show long-dead subagents as running again. Failed spawns keep their
    /// own state, and a subagent whose `self.wait` already completed has
    /// delivered its result: both stay out of the freeze. The freeze is
    /// bounded to the cancelled turn's own items: a spawn an earlier turn
    /// left unfinished is not rewritten by a later turn's stop (final
    /// audit E) — it keeps the state it already had.
    fn cancel_unfinished_subagents(&mut self) {
        let mut frozen = Vec::new();
        for (pos, item) in self
            .items
            .iter()
            .enumerate()
            .skip(self.turn_start.unwrap_or(0))
        {
            let Item::Tool(id) = item else {
                continue;
            };
            let Some(tc) = self.tool_calls.get(id) else {
                continue;
            };
            if !is_subagent(tc) || tc.status == ToolStatus::Failed {
                continue;
            }
            if !self.waited_for(pos) {
                frozen.push(tc.tool_call_id.clone());
            }
        }
        for id in frozen {
            self.cancelled_tools.insert(id);
        }
    }

    /// Whether the parent's `self.wait` for the tool call at `items[pos]`
    /// already completed (so the subagent delivered its result).
    fn waited_for(&self, pos: usize) -> bool {
        self.items[pos + 1..].iter().any(|later| {
            let Item::Tool(later_id) = later else {
                return false;
            };
            self.tool_calls.get(later_id).is_some_and(|later_tc| {
                is_wait_call(later_tc) && later_tc.status == ToolStatus::Completed
            })
        })
    }

    /// The model was switched (ConfigOptionsChanged). Vibe sends no
    /// usage_update with a switch, so the window shown would stay the old
    /// model's: pass the exact new window in (sourced from Vibe's own
    /// config); `None` means nothing exact is known and the stale values
    /// are cleared — the panel shows "–" until the next usage_update,
    /// which is authoritative for `used` and `size` again. Without a
    /// measured usage there is no `used` to pair with a size, so a known
    /// window alone invents nothing.
    pub fn set_model_window(&mut self, window: Option<u64>) {
        let Some(used) = self.usage.as_ref().map(|u| u.used) else {
            return;
        };
        self.usage = window.map(|size| UsageUpdate {
            used,
            size,
            cost: None,
        });
    }

    pub fn add_permission(&mut self, request_id: Value, request: PermissionRequest) {
        self.permissions.push(PendingPermission {
            request_id,
            request,
        });
    }

    pub fn take_permission(&mut self, request_id: &Value) -> Option<PendingPermission> {
        let pos = self
            .permissions
            .iter()
            .position(|p| &p.request_id == request_id)?;
        Some(self.permissions.remove(pos))
    }

    /// Count one more painted frame for every pending permission card and
    /// forget the counters of requests that are no longer pending. Called
    /// once per frame by the permission cards; the number-key lockout
    /// reads the result through `frames_shown` (N14).
    pub fn paint_permission_frames(&mut self) {
        for p in &self.permissions {
            *self
                .permission_frames
                .entry(p.request_id.to_string())
                .or_insert(0) += 1;
        }
        let live: Vec<String> = self
            .permissions
            .iter()
            .map(|p| p.request_id.to_string())
            .collect();
        self.permission_frames.retain(|id, _| live.contains(id));
    }

    /// How many frames the pending permission card of `request_id` has been
    /// painted in (0 without such a pending request).
    pub fn frames_shown(&self, request_id: &Value) -> u32 {
        self.permission_frames
            .get(&request_id.to_string())
            .copied()
            .unwrap_or(0)
    }

    /// Join a permission request with the tool call it refers to (Vibe sends
    /// only the toolCallId inside the request).
    pub fn tool_for_permission(&self, p: &PendingPermission) -> Option<&ToolCall> {
        self.tool_calls.get(&p.request.tool_call.tool_call_id)
    }

    /// Current value of a select config option (e.g. "mode", "model").
    pub fn current_config_value(&self, config_id: &str) -> Option<&str> {
        self.config_options.iter().find_map(|o| match o {
            ConfigOption::Select {
                id, current_value, ..
            } if id == config_id => Some(current_value.as_str()),
            _ => None,
        })
    }

    /// Context window fill level (`used / size`), clamped to 0..=1; `None`
    /// without usage info or when the window size is 0.
    pub fn context_fraction(&self) -> Option<f32> {
        let u = self.usage.as_ref()?;
        if u.size == 0 {
            return None;
        }
        Some((u.used as f32 / u.size as f32).clamp(0.0, 1.0))
    }

    /// All subagents of this session, in the order of their `Item::Tool`
    /// entries. The state follows the real flow (docs/acp.md): a failed spawn
    /// is `Failed`, a pending/running spawn is `Running`, a completed spawn is
    /// `Done` once a later `self.wait` call completed or the turn ended
    /// without a cancel, and `Cancelled` for the subagents that were still
    /// unfinished when a turn ended cancelled — frozen in
    /// `cancelled_tools` at that moment, so it survives the next
    /// `begin_turn` (N5).
    pub fn subagents(&self) -> Vec<SubagentInfo> {
        let mut out = Vec::new();
        for (pos, item) in self.items.iter().enumerate() {
            let Item::Tool(id) = item else {
                continue;
            };
            let Some(tc) = self.tool_calls.get(id) else {
                continue;
            };
            if !is_subagent(tc) {
                continue;
            }
            // a cancel of an earlier turn is frozen per subagent (N5): the
            // marking outlives `last_stop_reason`, which `begin_turn` of a
            // later turn clears
            let state = if self.cancelled_tools.contains(&tc.tool_call_id) {
                SubagentState::Cancelled
            } else {
                match tc.status {
                    ToolStatus::Failed => SubagentState::Failed,
                    ToolStatus::Pending | ToolStatus::InProgress => SubagentState::Running,
                    ToolStatus::Completed | ToolStatus::Other => {
                        let waited = self.waited_for(pos);
                        let turn_ended = self.turn == TurnState::Idle;
                        if waited || turn_ended {
                            SubagentState::Done
                        } else {
                            SubagentState::Running
                        }
                    }
                }
            };
            out.push(SubagentInfo {
                tool_call_id: tc.tool_call_id.clone(),
                name: tc.title.clone(),
                agent: meta_or_input_str(tc, "agent").unwrap_or_else(|| "generic".to_string()),
                task: meta_or_input_str(tc, "task").unwrap_or_default(),
                state,
                child_session_id: meta_or_input_str(tc, "child_session_id"),
                turns: tc
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("turn_count"))
                    .and_then(Value::as_u64),
            });
        }
        out
    }

    /// Append one streamed chunk: merges into the last item when it has the
    /// same kind AND (equal message_id OR the new message_id is None).
    fn apply_chunk(&mut self, chunk: &Chunk, kind: ChunkKind) {
        // Only text content is rendered; other content blocks are ignored.
        let Some(text) = chunk.content.as_text() else {
            return;
        };
        let new_item = match kind {
            ChunkKind::User => Item::User {
                message_id: chunk.message_id.clone(),
                text: String::new(),
            },
            ChunkKind::Agent => Item::Agent {
                message_id: chunk.message_id.clone(),
                text: String::new(),
            },
            ChunkKind::Thought => Item::Thought {
                message_id: chunk.message_id.clone(),
                text: String::new(),
            },
        };
        let append = match self.items.last() {
            Some(last) => {
                new_item.same_kind(last)
                    && (last.message_id() == chunk.message_id.as_deref()
                        || chunk.message_id.is_none())
            }
            None => false,
        };
        if append {
            if let Some(last) = self.items.last_mut() {
                if let Some(t) = last.text_mut() {
                    t.push_str(text);
                }
            }
        } else {
            let mut item = new_item;
            if let Some(t) = item.text_mut() {
                t.push_str(text);
            }
            self.items.push(item);
        }
    }
}

enum ChunkKind {
    User,
    Agent,
    Thought,
}

/// Build a `ToolCall` from a `tool_call_update` that arrived before any
/// `tool_call` with the same id (missing fields get defaults / empty title).
fn tool_call_from_update(u: &ToolCallUpdate) -> ToolCall {
    ToolCall {
        tool_call_id: u.tool_call_id.clone(),
        title: u.title.clone().unwrap_or_default(),
        kind: u.kind.clone().unwrap_or_default(),
        status: u.status.clone().unwrap_or_default(),
        content: u.content.clone().unwrap_or_default(),
        locations: u.locations.clone().unwrap_or_default(),
        raw_input: u.raw_input.clone(),
        raw_output: u.raw_output.clone(),
        meta: u.meta.clone(),
    }
}

/// `diff_stat` for per-frame callers (tool group summaries, panel rows):
/// cached by (old, new) hash.
pub fn diff_stat_cached(old: Option<&str>, new: &str) -> DiffStat {
    static CACHE: std::sync::OnceLock<crate::memo::Memo<DiffStat>> = std::sync::OnceLock::new();
    (*CACHE
        .get_or_init(|| crate::memo::Memo::new(2048))
        .get_or_insert(crate::memo::hash_pair(old, new), || diff_stat(old, new)))
    .clone()
}

/// Line-based multiset diff: lines of `new` not present (with multiplicity)
/// in `old` count as added, and vice versa. `old == None` means a new file:
/// every line of `new` is added. Empty text is 0 lines.
pub fn diff_stat(old: Option<&str>, new: &str) -> DiffStat {
    fn line_counts(text: &str) -> HashMap<&str, usize> {
        let mut counts = HashMap::new();
        for line in text.lines() {
            *counts.entry(line).or_insert(0) += 1;
        }
        counts
    }

    let new_counts = line_counts(new);
    let mut added = 0;
    let mut removed = 0;
    match old {
        None => {
            added = new.lines().count();
        }
        Some(old) => {
            let old_counts = line_counts(old);
            for (line, n) in &new_counts {
                let o = old_counts.get(line).copied().unwrap_or(0);
                if *n > o {
                    added += n - o;
                }
            }
            for (line, o) in &old_counts {
                let n = new_counts.get(line).copied().unwrap_or(0);
                if *o > n {
                    removed += o - n;
                }
            }
        }
    }
    DiffStat { added, removed }
}

/// One-line summary of a run of tool calls, e.g. "Reads 3 files, runs 2
/// commands". Subagent spawns get their own part ("starts 1 subagent") in
/// front of the tool part. Empty input yields "".
pub fn tool_group_summary(calls: &[&ToolCall]) -> String {
    let mut read = 0usize;
    let mut search = 0usize;
    let mut edit = 0usize;
    let mut execute = 0usize;
    let mut fetch = 0usize;
    let mut other = 0usize;
    let mut subagents = 0usize;
    for call in calls {
        if is_subagent(call) {
            subagents += 1;
            continue;
        }
        match call.kind {
            ToolKind::Read => read += 1,
            ToolKind::Search => search += 1,
            ToolKind::Edit | ToolKind::Delete | ToolKind::Move => edit += 1,
            ToolKind::Execute => execute += 1,
            ToolKind::Fetch => fetch += 1,
            ToolKind::Think | ToolKind::SwitchMode | ToolKind::Other => other += 1,
        }
    }
    // (count, text for one, text for several with {n})
    let counts: [(usize, &'static str, &'static str); 7] = [
        (subagents, "starts 1 subagent", "starts {n} subagents"),
        (read, "reads 1 file", "reads {n} files"),
        (search, "searches once", "searches {n} times"),
        (edit, "edits 1 file", "edits {n} files"),
        (execute, "runs 1 command", "runs {n} commands"),
        (fetch, "fetches 1 web page", "fetches {n} web pages"),
        (other, "uses 1 tool", "uses {n} tools"),
    ];
    let parts: Vec<String> = counts
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| {
            if n == 1 {
                tr(one).to_string()
            } else {
                fill(tr(many), &[("n", &n)])
            }
        })
        .collect();
    let mut summary = parts.join(", ");
    let mut chars = summary.chars();
    if let Some(first) = chars.next() {
        summary = first.to_uppercase().collect::<String>() + chars.as_str();
    }
    summary
}

/// Consecutive tool calls with the same `title` and the same `raw_input`
/// that count as a loop (the second repeat is still plausible, the third is
/// not).
pub const LOOP_THRESHOLD: usize = 3;

/// The tail of a detected loop: the repeated call and how often it ran in
/// a row.
#[derive(Debug, Clone, PartialEq)]
pub struct RepeatedCall {
    pub title: String,
    pub raw_input: Option<Value>,
    pub count: usize,
}

/// The loop at the end of the current turn, if any: counts the trailing
/// `Item::Tool` entries since the last `Item::User` that share `title` AND
/// `raw_input` (ACP gives every repeat its own toolCallId, so the ids do
/// not help). A different call, or any user message, ends the chain; a
/// call whose tool call is unknown ends it too. `None` below
/// `LOOP_THRESHOLD`.
pub fn repeated_tool_call(
    items: &[Item],
    tool_calls: &HashMap<String, ToolCall>,
) -> Option<RepeatedCall> {
    let mut chain: Option<RepeatedCall> = None;
    for item in items.iter().rev() {
        if matches!(item, Item::User { .. }) {
            break;
        }
        let Item::Tool(id) = item else {
            continue;
        };
        let Some(tc) = tool_calls.get(id) else {
            break;
        };
        match &mut chain {
            None => {
                chain = Some(RepeatedCall {
                    title: tc.title.clone(),
                    raw_input: tc.raw_input.clone(),
                    count: 1,
                });
            }
            Some(rep) if rep.title == tc.title && rep.raw_input == tc.raw_input => {
                rep.count += 1;
            }
            Some(_) => break,
        }
    }
    chain.filter(|rep| rep.count >= LOOP_THRESHOLD)
}

/// Ranges of consecutive `Item::Tool` entries in `items` (for the collapsed
/// tool group in the UI), e.g. `[User, Tool, Tool, Agent, Tool]` ->
/// `[1..3, 4..5]`.
pub fn tool_groups(items: &[Item]) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut start: Option<usize> = None;
    for (i, item) in items.iter().enumerate() {
        if matches!(item, Item::Tool(_)) {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            groups.push(s..i);
        }
    }
    if let Some(s) = start {
        groups.push(s..items.len());
    }
    groups
}

/// One round of the conversation: everything from an `Item::User` message up
/// to (not including) the next one, or from index 0 when the transcript does
/// not start with a user message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// Range of `items` that belongs to this round.
    pub range: std::ops::Range<usize>,
    /// The user message starting the round, if it starts with one.
    pub user_message_id: Option<String>,
    /// The last `Item::Agent` of the round: the answer that stays visible
    /// when the round is folded. `None` when the agent did not answer.
    pub answer: Option<usize>,
    /// Number of tool calls (`Item::Tool`) in the round.
    pub tool_calls: usize,
    /// The round is folded: everything but `answer` is hidden behind a
    /// summary row.
    pub collapsed: bool,
}

impl Round {
    /// `items` up to `answer`, without the answer itself: what the summary
    /// row stands for while the round is folded (empty without an answer).
    pub fn summary_range(&self) -> std::ops::Range<usize> {
        match self.answer {
            Some(a) => self.range.start..a,
            None => self.range.end..self.range.end,
        }
    }

    /// The items after the answer (usually nothing).
    #[cfg(test)]
    pub fn tail_range(&self) -> std::ops::Range<usize> {
        match self.answer {
            Some(a) => a + 1..self.range.end,
            None => self.range.end..self.range.end,
        }
    }

    /// Whether the summary row is drawn at all.
    pub fn shows_summary(&self) -> bool {
        self.collapsed && !self.summary_range().is_empty()
    }

    /// Is `index` drawn as the visible answer of a folded round?
    #[cfg(test)]
    pub fn shows_answer_only(&self, index: usize) -> bool {
        self.collapsed && self.answer == Some(index)
    }
}

/// Split `items` into rounds (see `Round`).
pub fn rounds(items: &[Item]) -> Vec<Round> {
    let mut starts: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| matches!(i, Item::User { .. }))
        .map(|(pos, _)| pos)
        .collect();
    if starts.first() != Some(&0) && !items.is_empty() {
        starts.insert(0, 0);
    }
    let mut out = Vec::with_capacity(starts.len());
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(items.len());
        let range = start..end;
        let user_message_id = match &items[start] {
            Item::User { message_id, .. } => message_id.clone(),
            _ => None,
        };
        let answer = items[range.clone()]
            .iter()
            .rposition(|i| matches!(i, Item::Agent { .. }))
            .map(|p| start + p);
        let tool_calls = items[range.clone()]
            .iter()
            .filter(|i| matches!(i, Item::Tool(_)))
            .count();
        out.push(Round {
            range,
            user_message_id,
            answer,
            tool_calls,
            collapsed: false,
        });
    }
    out
}

/// Rounds of a transcript with the folding decision applied: a finished
/// round folds to its last agent answer when the agent actually answered
/// and something else would be hidden. The rounds of the latest turn — those
/// reaching past `turn_start`, the index where it began (`None`: the last
/// round) — stay fully visible while it runs, and when it ended in an error
/// (`stop`). Older rounds stay folded meanwhile, so sending a message does
/// not unfold the history above it.
pub fn fold_rounds(
    items: &[Item],
    turn: &TurnState,
    turn_start: Option<usize>,
    stop: Option<&StopReason>,
) -> Vec<Round> {
    let running = *turn != TurnState::Idle;
    let failed = stop.is_some_and(|r| {
        !matches!(
            r,
            StopReason::EndTurn | StopReason::MaxTokens | StopReason::MaxTurnRequests
        )
    });
    let mut out = rounds(items);
    let last = out.len().checked_sub(1);
    for (i, round) in out.iter_mut().enumerate() {
        // part of the latest turn: what it added lies at or after its start
        // (its own round once Vibe echoed the prompt, else the round it
        // continues)
        let latest = match turn_start {
            Some(start) => round.range.end > start,
            None => Some(i) == last,
        };
        let keep_open = latest && (running || failed);
        let has_other = round.range.len() > usize::from(round.answer.is_some());
        round.collapsed = !keep_open && round.answer.is_some() && has_other;
    }
    out
}

/// Key of a round in the UI's `open_groups` map: a folded round is expanded
/// while its key is present. The user message id keeps the key stable while
/// the round grows; rounds without one fall back to their start index.
pub fn round_key(session_id: &str, round: &Round) -> String {
    match &round.user_message_id {
        Some(id) => format!("{session_id}#round#{id}"),
        None => format!("{session_id}#round@{}", round.range.start),
    }
}

// ---------------------------------------------------------------------------
// tests (offline, against the real wire transcript)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROMPT_SESSION: &str = include_str!("../../tests/fixtures/acp/prompt_session.jsonl");
    const SUBAGENT_SESSION: &str = include_str!("../../tests/fixtures/acp/subagent_session.jsonl");

    /// All `a2c` `session/update` notifications of the fixture, split at the
    /// marker line into (live, replay).
    fn session_updates(fixture: &str) -> (Vec<SessionNotification>, Vec<SessionNotification>) {
        let mut live = Vec::new();
        let mut replay = Vec::new();
        let mut seen_marker = false;
        for line in fixture.lines() {
            let v: Value = serde_json::from_str(line).expect("fixture line is JSON");
            if v["dir"] == "marker" {
                seen_marker = true;
                continue;
            }
            if v["dir"] == "a2c" && v["msg"]["method"] == "session/update" {
                let n: SessionNotification =
                    serde_json::from_value(v["msg"]["params"].clone()).expect("update parses");
                if seen_marker {
                    replay.push(n);
                } else {
                    live.push(n);
                }
            }
        }
        (live, replay)
    }

    fn view_with(updates: &[SessionNotification]) -> SessionView {
        let mut view = SessionView::new("sid", "/tmp/acp-fixture/ws");
        for n in updates {
            view.apply_update(&n.update);
        }
        view
    }

    fn live_view() -> SessionView {
        let (live, _) = session_updates(PROMPT_SESSION);
        view_with(&live)
    }

    fn agent_text(view: &SessionView) -> String {
        view.items
            .iter()
            .filter_map(|i| match i {
                Item::Agent { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn tool_call(kind: ToolKind, id: &str) -> ToolCall {
        serde_json::from_value(json!({
            "toolCallId": id,
            "kind": format!("{kind:?}").to_lowercase(),
        }))
        .unwrap()
    }

    #[test]
    fn transcript_markdown_keeps_the_conversation_and_drops_thoughts() {
        let mut v = SessionView::new("s", "/w");
        v.apply_update(&SessionUpdate::UserMessageChunk(
            serde_json::from_value(
                json!({"content": {"type": "text", "text": "Frage"}, "messageId": "m1"}),
            )
            .unwrap(),
        ));
        v.apply_update(&SessionUpdate::AgentThoughtChunk(
            serde_json::from_value(
                json!({"content": {"type": "text", "text": "geheim"}, "messageId": "r1"}),
            )
            .unwrap(),
        ));
        v.apply_update(&SessionUpdate::AgentMessageChunk(
            serde_json::from_value(
                json!({"content": {"type": "text", "text": "Antwort"}, "messageId": "a1"}),
            )
            .unwrap(),
        ));
        let md = v.transcript_markdown(|t| t);
        assert!(md.starts_with("## Du\n\nFrage"));
        assert!(md.contains("## Vibe\n\nAntwort"));
        assert!(!md.contains("geheim"));
    }

    #[test]
    fn rewind_cuts_from_the_message_on_and_forgets_its_tool_calls() {
        let mut v = SessionView::new("s", "/w");
        v.apply_update(&SessionUpdate::UserMessageChunk(
            serde_json::from_value(
                serde_json::json!({"content": {"type": "text", "text": "a"}, "messageId": "m1"}),
            )
            .unwrap(),
        ));
        v.apply_update(&SessionUpdate::ToolCall(tool_call(ToolKind::Read, "t1")));
        v.apply_update(&SessionUpdate::UserMessageChunk(
            serde_json::from_value(
                serde_json::json!({"content": {"type": "text", "text": "b"}, "messageId": "m2"}),
            )
            .unwrap(),
        ));
        v.apply_update(&SessionUpdate::ToolCall(tool_call(ToolKind::Edit, "t2")));
        assert!(v.rewind_to_message("m2"));
        assert_eq!(v.items.len(), 2);
        assert!(v.tool_calls.contains_key("t1"));
        assert!(!v.tool_calls.contains_key("t2"));
        assert!(
            !v.rewind_to_message("unknown"),
            "unknown id changes nothing"
        );
        assert_eq!(v.items.len(), 2);
    }

    #[test]
    fn fork_copy_has_no_source_message_ids_and_can_stop_after_a_turn() {
        let mut v = SessionView::new("s", "/w");
        for (id, text) in [("m1", "a"), ("m2", "b")] {
            v.apply_update(&SessionUpdate::UserMessageChunk(
                serde_json::from_value(
                    serde_json::json!({"content": {"type": "text", "text": text}, "messageId": id}),
                )
                .unwrap(),
            ));
            v.apply_update(&SessionUpdate::AgentMessageChunk(serde_json::from_value(
                serde_json::json!({"content": {"type": "text", "text": "r"}, "messageId": format!("a-{id}")}),
            ).unwrap()));
        }
        let mut cut = v.clone();
        cut.keep_through_turn_of("m1");
        assert_eq!(cut.items.len(), 2, "user message plus its answer");
        let fork = v.forked("f");
        assert!(fork.items.iter().all(|i| !matches!(
            i,
            Item::User {
                message_id: Some(_),
                ..
            }
        )));
    }

    #[test]
    fn live_session_shape() {
        let view = live_view();

        // exactly one user bubble, at the start
        assert!(matches!(&view.items[0], Item::User { .. }));
        assert_eq!(
            view.items
                .iter()
                .filter(|i| matches!(i, Item::User { .. }))
                .count(),
            1
        );
        // at least one thought and one agent text
        assert!(view.items.iter().any(|i| matches!(i, Item::Thought { .. })));
        assert!(view.items.iter().any(|i| matches!(i, Item::Agent { .. })));
        // one Item::Tool per distinct toolCallId
        let tool_items = view
            .items
            .iter()
            .filter(|i| matches!(i, Item::Tool(_)))
            .count();
        assert_eq!(tool_items, view.tool_calls.len());
        assert!(tool_items > 0);
        // exactly one plan marker
        assert_eq!(
            view.items
                .iter()
                .filter(|i| matches!(i, Item::Plan))
                .count(),
            1
        );
        // plan fully completed at the end, all tool calls completed
        assert!(!view.plan.is_empty());
        assert!(view.plan.iter().all(|e| e.status == PlanStatus::Completed));
        assert!(view
            .tool_calls
            .values()
            .all(|tc| tc.status == ToolStatus::Completed));
        // usage, title, commands
        assert_eq!(view.usage.as_ref().map(|u| u.size), Some(800_000));
        assert!(view.title.is_some());
        assert!(!view.commands.is_empty());
    }

    #[test]
    fn replay_into_fresh_view_matches_live() {
        let live = live_view();
        let (_, replay) = session_updates(PROMPT_SESSION);
        let replayed = view_with(&replay);

        // same toolCallIds with the same final status
        assert_eq!(live.tool_calls.len(), replayed.tool_calls.len());
        for (id, tc) in &live.tool_calls {
            let r = replayed
                .tool_calls
                .get(id)
                .expect("replay has the same toolCallId");
            assert_eq!(r.status, tc.status, "status differs for {id}");
        }
        // same plan
        assert_eq!(live.plan, replayed.plan);
        // same concatenated agent text (replay delivers it in one piece)
        assert_eq!(agent_text(&live), agent_text(&replayed));
        assert!(!agent_text(&replayed).is_empty());
    }

    #[test]
    fn chunk_merging_rules() {
        let chunk = |kind: &str, id: Option<&str>, text: &str| {
            let mut v = json!({
                "sessionUpdate": kind,
                "content": {"type": "text", "text": text},
            });
            if let Some(id) = id {
                v["messageId"] = json!(id);
            }
            serde_json::from_value::<SessionUpdate>(v).unwrap()
        };
        let mut view = SessionView::new("s", "/tmp");
        // same message id appends
        view.apply_update(&chunk("agent_message_chunk", Some("a"), "Hel"));
        view.apply_update(&chunk("agent_message_chunk", Some("a"), "lo"));
        // different message id starts a new item
        view.apply_update(&chunk("agent_message_chunk", Some("b"), "!"));
        // None appends to the last same-kind item
        view.apply_update(&chunk("agent_message_chunk", None, "?"));
        // a thought in between separates: same id "b" after it is a new item
        view.apply_update(&chunk("agent_thought_chunk", Some("t"), "hmm"));
        view.apply_update(&chunk("agent_message_chunk", Some("b"), "again"));
        // non-text content is ignored entirely
        let img = serde_json::from_value::<SessionUpdate>(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "image", "data": "AAA", "mimeType": "image/png"},
            "messageId": "a",
        }))
        .unwrap();
        view.apply_update(&img);

        let items: Vec<&Item> = view.items.iter().collect();
        assert_eq!(items.len(), 4);
        assert_eq!(
            items[0],
            &Item::Agent {
                message_id: Some("a".to_string()),
                text: "Hello".to_string()
            }
        );
        assert_eq!(
            items[1],
            &Item::Agent {
                message_id: Some("b".to_string()),
                text: "!?".to_string()
            }
        );
        assert!(matches!(&items[2], Item::Thought { text, .. } if text == "hmm"));
        assert_eq!(
            items[3],
            &Item::Agent {
                message_id: Some("b".to_string()),
                text: "again".to_string()
            }
        );
    }

    #[test]
    fn tool_call_update_for_unknown_id_creates_entry_and_item() {
        let mut view = SessionView::new("s", "/tmp");
        let upd: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "t-new",
            "status": "in_progress",
        }))
        .unwrap();
        view.apply_update(&upd);
        assert!(view.tool_calls.contains_key("t-new"));
        let tc = &view.tool_calls["t-new"];
        assert_eq!(tc.status, ToolStatus::InProgress);
        assert_eq!(tc.title, "");
        assert_eq!(view.items, vec![Item::Tool("t-new".to_string())]);

        // a later tool_call with the same id replaces without a second item
        let tc2: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "t-new",
            "title": "Bash",
            "kind": "execute",
            "status": "completed",
        }))
        .unwrap();
        view.apply_update(&tc2);
        assert_eq!(view.items.len(), 1);
        assert_eq!(view.tool_calls["t-new"].title, "Bash");
        assert_eq!(view.tool_calls["t-new"].status, ToolStatus::Completed);
    }

    #[test]
    fn diff_stat_cases() {
        // new file: all lines of `new` are added
        assert_eq!(
            diff_stat(None, "a\nb"),
            DiffStat {
                added: 2,
                removed: 0
            }
        );
        // one line changed
        assert_eq!(
            diff_stat(Some("a\nb\nc"), "a\nX\nc"),
            DiffStat {
                added: 1,
                removed: 1
            }
        );
        // duplicates counted with multiplicity
        assert_eq!(
            diff_stat(Some("a\na\nb"), "a\nb"),
            DiffStat {
                added: 0,
                removed: 1
            }
        );
        assert_eq!(
            diff_stat(Some("a"), "a\na"),
            DiffStat {
                added: 1,
                removed: 0
            }
        );
        // empty texts are 0 lines
        assert_eq!(
            diff_stat(Some(""), ""),
            DiffStat {
                added: 0,
                removed: 0
            }
        );
        assert_eq!(
            diff_stat(None, ""),
            DiffStat {
                added: 0,
                removed: 0
            }
        );
    }

    #[test]
    fn tool_group_summaries_have_german_texts() {
        // the summary texts come from a table, which the catalog scan in
        // i18n.rs cannot see: check them here
        use crate::i18n::{tr_in, Lang};
        for en in [
            "starts 1 subagent",
            "starts {n} subagents",
            "reads 1 file",
            "reads {n} files",
            "searches once",
            "searches {n} times",
            "edits 1 file",
            "edits {n} files",
            "runs 1 command",
            "runs {n} commands",
            "fetches 1 web page",
            "fetches {n} web pages",
            "uses 1 tool",
            "uses {n} tools",
        ] {
            assert_ne!(tr_in(Lang::De, en), en, "no German text for {en:?}");
        }
        assert_eq!(tr_in(Lang::De, "reads {n} files"), "liest {n} Dateien");
    }

    #[test]
    fn tool_group_summary_cases() {
        let calls = |kinds: &[ToolKind]| -> Vec<ToolCall> {
            kinds
                .iter()
                .enumerate()
                .map(|(i, k)| tool_call(k.clone(), &format!("t{i}")))
                .collect()
        };
        fn refs(v: &[ToolCall]) -> Vec<&ToolCall> {
            v.iter().collect()
        }

        let c = calls(&[
            ToolKind::Read,
            ToolKind::Read,
            ToolKind::Read,
            ToolKind::Execute,
            ToolKind::Execute,
        ]);
        assert_eq!(
            tool_group_summary(&refs(&c)),
            "Reads 3 files, runs 2 commands"
        );
        // singular forms
        assert_eq!(
            tool_group_summary(&refs(&calls(&[ToolKind::Read]))),
            "Reads 1 file"
        );
        assert_eq!(
            tool_group_summary(&refs(&calls(&[ToolKind::Execute]))),
            "Runs 1 command"
        );
        assert_eq!(
            tool_group_summary(&refs(&calls(&[ToolKind::Think]))),
            "Uses 1 tool"
        );
        // remaining groups
        assert_eq!(
            tool_group_summary(&refs(&calls(&[ToolKind::Search, ToolKind::Search]))),
            "Searches 2 times"
        );
        assert_eq!(
            tool_group_summary(&refs(&calls(&[
                ToolKind::Edit,
                ToolKind::Delete,
                ToolKind::Move
            ]))),
            "Edits 3 files"
        );
        assert_eq!(
            tool_group_summary(&refs(&calls(&[ToolKind::Fetch]))),
            "Fetches 1 web page"
        );
        assert_eq!(
            tool_group_summary(&refs(&calls(&[ToolKind::SwitchMode, ToolKind::Other]))),
            "Uses 2 tools"
        );
        // empty
        assert_eq!(tool_group_summary(&[]), "");
    }

    #[test]
    fn tool_groups_ranges() {
        let items = vec![
            Item::User {
                message_id: None,
                text: "u".to_string(),
            },
            Item::Tool("a".to_string()),
            Item::Tool("b".to_string()),
            Item::Agent {
                message_id: None,
                text: "x".to_string(),
            },
            Item::Tool("c".to_string()),
        ];
        assert_eq!(tool_groups(&items), vec![1..3, 4..5]);
        assert!(tool_groups(&items[..1]).is_empty());
        let trailing = vec![Item::Tool("a".to_string()), Item::Tool("b".to_string())];
        assert_eq!(tool_groups(&trailing), vec![0..2]);
    }

    /// A `tool_call` with a title and raw input (status completed).
    fn call_update(id: &str, title: &str, input: Value) -> SessionUpdate {
        serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": title,
            "kind": "execute",
            "status": "completed",
            "rawInput": input,
        }))
        .unwrap()
    }

    fn user_update(text: &str) -> SessionUpdate {
        serde_json::from_value(json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": text},
            "messageId": "m1",
        }))
        .unwrap()
    }

    #[test]
    fn loop_guard_needs_three_identical_calls() {
        let mut v = SessionView::new("s", "/tmp");
        v.begin_turn();
        v.apply_update(&user_update("Frage"));
        assert_eq!(repeated_tool_call(&v.items, &v.tool_calls), None);

        v.apply_update(&call_update("t1", "Bash", json!({"command": "ls"})));
        assert_eq!(repeated_tool_call(&v.items, &v.tool_calls), None);

        v.apply_update(&call_update("t2", "Bash", json!({"command": "ls"})));
        assert_eq!(
            repeated_tool_call(&v.items, &v.tool_calls),
            None,
            "two identical calls are still plausible"
        );

        v.apply_update(&call_update("t3", "Bash", json!({"command": "ls"})));
        assert_eq!(
            repeated_tool_call(&v.items, &v.tool_calls),
            Some(RepeatedCall {
                title: "Bash".to_string(),
                raw_input: Some(json!({"command": "ls"})),
                count: 3,
            })
        );
    }

    #[test]
    fn loop_guard_chain_breaks_on_a_different_call() {
        let mut v = SessionView::new("s", "/tmp");
        v.begin_turn();
        v.apply_update(&user_update("Frage"));
        for (id, title, input) in [
            ("t1", "Bash", json!({"command": "ls"})),
            ("t2", "Bash", json!({"command": "ls"})),
            ("t3", "Bash", json!({"command": "pwd"})),
        ] {
            v.apply_update(&call_update(id, title, input));
        }
        assert_eq!(
            repeated_tool_call(&v.items, &v.tool_calls),
            None,
            "a different raw_input ends the chain"
        );

        let mut v = SessionView::new("s", "/tmp");
        v.begin_turn();
        v.apply_update(&user_update("Frage"));
        for (id, title) in [("t1", "Bash"), ("t2", "Bash"), ("t3", "Read")] {
            v.apply_update(&call_update(id, title, json!({"command": "ls"})));
        }
        assert_eq!(
            repeated_tool_call(&v.items, &v.tool_calls),
            None,
            "a different title ends the chain"
        );

        // three identical calls after the differing one are a loop again
        v.apply_update(&call_update("t4", "Read", json!({"command": "ls"})));
        v.apply_update(&call_update("t5", "Read", json!({"command": "ls"})));
        let rep = repeated_tool_call(&v.items, &v.tool_calls).expect("loop detected");
        assert_eq!(rep.title, "Read");
        assert_eq!(rep.count, 3);
    }

    #[test]
    fn loop_guard_counts_only_since_the_last_user_message() {
        let mut v = SessionView::new("s", "/tmp");
        v.begin_turn();
        v.apply_update(&user_update("erste Frage"));
        for id in ["t1", "t2"] {
            v.apply_update(&call_update(id, "Bash", json!({"command": "ls"})));
        }
        // a new user turn resets the count: one repeat plus the old two is
        // not a chain of three
        v.apply_update(&user_update("zweite Frage"));
        v.apply_update(&call_update("t3", "Bash", json!({"command": "ls"})));
        assert_eq!(repeated_tool_call(&v.items, &v.tool_calls), None);
        v.apply_update(&call_update("t4", "Bash", json!({"command": "ls"})));
        assert_eq!(repeated_tool_call(&v.items, &v.tool_calls), None);
        v.apply_update(&call_update("t5", "Bash", json!({"command": "ls"})));
        assert_eq!(
            repeated_tool_call(&v.items, &v.tool_calls).map(|r| r.count),
            Some(3)
        );
    }

    #[test]
    fn cancel_and_finish_turn() {
        let mut view = SessionView::new("s", "/tmp");
        view.begin_turn();
        assert_eq!(view.turn, TurnState::Running);
        assert_eq!(view.last_stop_reason, None);

        let request: PermissionRequest = serde_json::from_value(json!({
            "sessionId": "s",
            "toolCall": {"toolCallId": "t1"},
            "options": [{"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"}],
        }))
        .unwrap();
        view.add_permission(json!(0), request);
        // join with an earlier tool call
        let tc: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "t1",
            "title": "Writing hallo.txt",
            "kind": "edit",
        }))
        .unwrap();
        view.apply_update(&tc);
        assert_eq!(
            view.tool_for_permission(&view.permissions[0])
                .map(|t| t.title.as_str()),
            Some("Writing hallo.txt")
        );

        let ids = view.request_cancel();
        assert_eq!(ids, vec![json!(0)]);
        assert!(view.permissions.is_empty());
        assert_eq!(view.turn, TurnState::Cancelling);

        let result: PromptResult = serde_json::from_value(json!({
            "stopReason": "cancelled",
            "usage": {"totalTokens": 5, "inputTokens": 3, "outputTokens": 2},
        }))
        .unwrap();
        view.finish_turn(&result);
        assert_eq!(view.turn, TurnState::Idle);
        assert_eq!(view.last_stop_reason, Some(StopReason::Cancelled));
        assert!(view.last_turn_usage.is_some());
        assert!(view.permissions.is_empty());

        // take_permission removes exactly the matching request
        let r2: PermissionRequest = serde_json::from_value(json!({
            "sessionId": "s", "toolCall": {"toolCallId": "t1"}
        }))
        .unwrap();
        view.add_permission(json!(7), r2);
        assert!(view.take_permission(&json!(8)).is_none());
        assert!(view.take_permission(&json!(7)).is_some());
        assert!(view.permissions.is_empty());
    }

    /// A `usage_update` notification as the real vibe-acp sends it.
    fn usage_update(used: u64, size: u64) -> SessionUpdate {
        serde_json::from_value(json!({
            "sessionUpdate": "usage_update",
            "used": used,
            "size": size,
        }))
        .unwrap()
    }

    #[test]
    fn context_fraction_clamping() {
        let mut view = SessionView::new("s", "/tmp");
        assert_eq!(view.context_fraction(), None);

        let usage = |used: u64, size: u64| {
            serde_json::from_value::<SessionUpdate>(json!({
                "sessionUpdate": "usage_update",
                "used": used,
                "size": size,
            }))
            .unwrap()
        };
        view.apply_update(&usage(0, 0));
        assert_eq!(view.context_fraction(), None, "size 0 -> None");
        view.apply_update(&usage(400, 800));
        assert_eq!(view.context_fraction(), Some(0.5));
        view.apply_update(&usage(900, 800));
        assert_eq!(view.context_fraction(), Some(1.0), "clamped to 1.0");
    }

    fn user(id: &str, text: &str) -> Item {
        Item::User {
            message_id: Some(id.to_string()),
            text: text.to_string(),
        }
    }

    fn agent(id: &str, text: &str) -> Item {
        Item::Agent {
            message_id: Some(id.to_string()),
            text: text.to_string(),
        }
    }

    fn thought(id: &str, text: &str) -> Item {
        Item::Thought {
            message_id: Some(id.to_string()),
            text: text.to_string(),
        }
    }

    /// One round: user message, thought, two tool calls, the answer.
    fn finished_round() -> Vec<Item> {
        vec![
            user("m1", "Frage"),
            thought("t1", "denkt"),
            Item::Tool("c1".to_string()),
            Item::Tool("c2".to_string()),
            agent("a1", "Antwort"),
        ]
    }

    #[test]
    fn rounds_split_at_user_messages() {
        let items = vec![
            user("m1", "a"),
            Item::Tool("c1".to_string()),
            agent("a1", "x"),
            user("m2", "b"),
            agent("a2", "y"),
            agent("a3", "z"),
        ];
        let rs = rounds(&items);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].range, 0..3);
        assert_eq!(rs[0].user_message_id.as_deref(), Some("m1"));
        assert_eq!(rs[0].answer, Some(2), "last agent answer of the round");
        assert_eq!(rs[0].tool_calls, 1);
        assert_eq!(rs[1].range, 3..6);
        assert_eq!(rs[1].answer, Some(5), "two answers: the last one stays");
        assert_eq!(rs[1].tool_calls, 0);
        assert!(rounds(&[]).is_empty());
    }

    #[test]
    fn finished_round_folds_to_its_last_answer() {
        let items = finished_round();
        let rs = fold_rounds(&items, &TurnState::Idle, None, Some(&StopReason::EndTurn));
        assert_eq!(rs.len(), 1);
        let r = &rs[0];
        assert!(r.collapsed);
        assert_eq!(r.answer, Some(4));
        assert_eq!(r.summary_range(), 0..4, "user, thought and tools fold");
        assert!(!r.summary_range().is_empty());
        assert!(r.shows_summary());
        assert!(r.shows_answer_only(4));
        assert!(!r.shows_answer_only(0));
        assert_eq!(r.tail_range(), 5..5);
        // no stop reason recorded (e.g. a replayed history): still finished
        let rs = fold_rounds(&items, &TurnState::Idle, None, None);
        assert!(rs[0].collapsed);
    }

    #[test]
    fn running_round_never_folds() {
        let items = finished_round();
        for turn in [TurnState::Running, TurnState::Cancelling] {
            let rs = fold_rounds(&items, &turn, None, None);
            assert!(!rs[0].collapsed, "{turn:?} keeps the round open");
            assert!(!rs[0].shows_summary());
            assert!(!rs[0].shows_answer_only(4));
        }
    }

    #[test]
    fn round_without_an_answer_does_not_fold() {
        let items = vec![
            user("m1", "Frage"),
            Item::Tool("c1".to_string()),
            thought("t1", "denkt"),
        ];
        let rs = fold_rounds(&items, &TurnState::Idle, None, Some(&StopReason::EndTurn));
        assert_eq!(rs[0].answer, None);
        assert!(!rs[0].collapsed);
        assert!(!rs[0].shows_summary());
        // an answer that is the whole round hides nothing: no summary row
        let only_answer = vec![user("m1", "Frage"), agent("a1", "Antwort")];
        let rs = fold_rounds(&only_answer, &TurnState::Idle, None, None);
        assert!(rs[0].collapsed);
        assert_eq!(rs[0].summary_range(), 0..1, "only the user message");
        assert!(rs[0].shows_summary());
        assert!(rs[0].shows_answer_only(1));
        // an answer alone is no round to fold: nothing would be hidden
        let answer_only = vec![agent("a1", "Antwort")];
        let rs = fold_rounds(&answer_only, &TurnState::Idle, None, None);
        assert!(!rs[0].collapsed);
        assert!(!rs[0].shows_summary());
    }

    #[test]
    fn error_end_does_not_fold() {
        let items = finished_round();
        for stop in [
            StopReason::Cancelled,
            StopReason::Refusal,
            StopReason::Other,
        ] {
            let rs = fold_rounds(&items, &TurnState::Idle, None, Some(&stop));
            assert!(!rs[0].collapsed, "{stop:?} keeps everything visible");
        }
        // a limit is a real end of the turn, not an error
        for stop in [StopReason::MaxTokens, StopReason::MaxTurnRequests] {
            let rs = fold_rounds(&items, &TurnState::Idle, None, Some(&stop));
            assert!(rs[0].collapsed, "{stop:?} still folds");
        }
    }

    #[test]
    fn several_rounds_fold_independently_and_keep_their_keys() {
        let mut items = finished_round();
        items.push(user("m2", "zweite Frage"));
        items.push(Item::Tool("c3".to_string()));
        items.push(agent("a2", "zweite Antwort"));
        items.push(user("m3", "laufende Frage"));
        items.push(Item::Tool("c4".to_string()));
        items.push(agent("a3", "dritte Antwort"));
        let rs = fold_rounds(&items, &TurnState::Idle, None, Some(&StopReason::EndTurn));
        assert_eq!(rs.len(), 3);
        assert!(rs[0].collapsed && rs[1].collapsed && rs[2].collapsed);
        assert_eq!(rs[1].range, 5..8);
        assert_eq!(rs[1].answer, Some(7));
        assert_eq!(rs[1].summary_range(), 5..7);
        assert_eq!(rs[2].range, 8..11);
        assert_eq!(rs[2].summary_range(), 8..10);
        // the running turn (last round) stays open, the finished rounds
        // above it stay folded (was: a running turn unfolded every round)
        let rs = fold_rounds(&items, &TurnState::Running, None, None);
        assert!(rs[0].collapsed && rs[1].collapsed);
        assert!(!rs[2].collapsed, "the running round is fully visible");
        // an aborted turn (error stop) likewise only keeps its own round
        let rs = fold_rounds(&items, &TurnState::Idle, None, Some(&StopReason::Cancelled));
        assert!(
            rs[0].collapsed && rs[1].collapsed,
            "finished rounds stay folded after a later turn was aborted"
        );
        assert!(!rs[2].collapsed, "the aborted round stays visible");
        // keys: the message id keeps a folded round's state stable
        assert_eq!(round_key("s", &rs[0]), "s#round#m1");
        assert_eq!(round_key("s", &rs[2]), "s#round#m3");
        let no_id = vec![Item::User {
            message_id: None,
            text: "x".to_string(),
        }];
        assert_eq!(round_key("s", &rounds(&no_id)[0]), "s#round@0");
    }

    /// Sending a message must not unfold the history above it: between
    /// `begin_turn` and Vibe's echo of the prompt nothing of the new turn
    /// exists, so every finished round stays folded; the echo opens the new
    /// round only. An error end keeps just the latest turn open, and a rewind
    /// that cuts the latest turn forgets where it began.
    #[test]
    fn only_the_latest_turns_rounds_unfold() {
        let mut v = SessionView::new("s", "/tmp");
        v.items = finished_round(); // 0..5
        v.items.extend([
            user("m2", "zweite Frage"),
            Item::Tool("c3".to_string()),
            agent("a2", "zweite Antwort"),
        ]); // 5..8
        assert_eq!(v.turn_start, None);
        assert!(v.folded_rounds().iter().all(|r| r.collapsed), "history");

        // sent, no echo yet: nothing unfolds
        v.begin_turn();
        assert_eq!(v.turn_start, Some(8));
        assert!(v.folded_rounds().iter().all(|r| r.collapsed));
        // the echo starts the running round: only that one is open
        v.items.extend([
            user("m3", "dritte Frage"),
            Item::Tool("c4".to_string()),
            agent("a3", "Zwischenstand"),
        ]); // 8..11
        let rs = v.folded_rounds();
        assert_eq!(rs.len(), 3);
        assert!(
            rs[0].collapsed && rs[1].collapsed,
            "the history stays folded"
        );
        assert!(!rs[2].collapsed);
        // finished: it folds like the others
        v.finish_turn(&end_turn());
        assert!(v.folded_rounds().iter().all(|r| r.collapsed));

        // cancelled: the latest turn stays visible, the history folded
        v.begin_turn();
        v.items.extend([
            user("m4", "vierte Frage"),
            Item::Tool("c5".to_string()),
            agent("a4", "halb"),
        ]); // 11..14
        v.finish_turn(&cancelled_turn());
        let rs = v.folded_rounds();
        assert!(rs[..3].iter().all(|r| r.collapsed));
        assert!(!rs[3].collapsed, "an error end keeps its round open");

        // rewound to before that turn: it is gone, so is its start
        assert!(v.rewind_to_message("m4"));
        assert_eq!(v.turn_start, None);
        assert!(v.folded_rounds().iter().all(|r| r.collapsed));

        // a turn whose prompt Vibe does not echo continues the last round:
        // that round is open while the turn runs
        let mut w = SessionView::new("s", "/tmp");
        w.items = finished_round();
        w.begin_turn();
        assert!(w.folded_rounds()[0].collapsed, "nothing new yet");
        w.items.push(agent("a9", "Ausgabe ohne Echo"));
        assert!(!w.folded_rounds()[0].collapsed);
        // a reload replays a history without a turn of this run
        w.reset_items();
        assert_eq!(w.turn_start, None);
    }

    #[test]
    fn model_window_replaces_the_size_keeps_the_measured_used() {
        let mut view = SessionView::new("s", "/tmp");
        view.apply_update(&usage_update(50_000, 800_000));
        view.set_model_window(Some(1_048_576));
        let u = view.usage.as_ref().expect("usage stays measurable");
        assert_eq!(u.used, 50_000);
        assert_eq!(u.size, 1_048_576);
        assert!(u.cost.is_none());
    }

    #[test]
    fn model_window_without_exact_value_shows_the_dash() {
        let mut view = SessionView::new("s", "/tmp");
        view.apply_update(&usage_update(50_000, 800_000));
        view.set_model_window(None);
        assert!(view.usage.is_none(), "stale size is cleared, not guessed");
        // the next usage_update is authoritative again
        view.apply_update(&usage_update(120_000, 1_048_576));
        let u = view.usage.as_ref().expect("usage_update restores it");
        assert_eq!(u.used, 120_000);
        assert_eq!(u.size, 1_048_576);
    }

    #[test]
    fn model_window_without_measured_usage_invents_nothing() {
        let mut view = SessionView::new("s", "/tmp");
        assert!(view.usage.is_none());
        view.set_model_window(Some(800_000));
        assert!(
            view.usage.is_none(),
            "no measured used yet: no 0-token fake is shown"
        );
    }

    #[test]
    fn mode_and_config_updates() {
        let mut view = SessionView::new("s", "/tmp");
        let modes: ModeState = serde_json::from_value(json!({
            "currentModeId": "accept-edits",
            "availableModes": [{"id": "ask", "name": "Ask"}],
        }))
        .unwrap();
        let config: Vec<ConfigOption> = serde_json::from_value(json!([
            {"type": "select", "id": "mode", "currentValue": "accept-edits",
             "options": [{"value": "ask"}, {"value": "accept-edits"}]},
            {"type": "select", "id": "model", "currentValue": "mistral-large-4-0-3", "options": []},
            {"type": "boolean", "id": "verbose", "currentValue": true},
        ]))
        .unwrap();
        view.set_session_info(Some(modes), config);
        assert_eq!(view.current_config_value("mode"), Some("accept-edits"));
        assert_eq!(
            view.current_config_value("model"),
            Some("mistral-large-4-0-3")
        );
        assert_eq!(
            view.current_config_value("verbose"),
            None,
            "boolean has no str value"
        );
        assert_eq!(view.current_config_value("nope"), None);

        let mode_upd: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "current_mode_update",
            "currentModeId": "ask",
        }))
        .unwrap();
        view.apply_update(&mode_upd);
        assert_eq!(
            view.modes.as_ref().map(|m| m.current_mode_id.as_str()),
            Some("ask")
        );
        assert_eq!(view.current_config_value("mode"), Some("ask"));

        // config_option_update replaces the whole list
        let cfg_upd: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [
                {"type": "select", "id": "mode", "currentValue": "ask", "options": []},
            ],
        }))
        .unwrap();
        view.apply_update(&cfg_upd);
        assert_eq!(view.config_options.len(), 1);
        assert_eq!(view.current_config_value("model"), None);

        // reset_items clears transcript state for a session/load replay
        view.apply_update(
            &serde_json::from_value::<SessionUpdate>(json!({
                "sessionUpdate": "plan",
                "entries": [{"content": "x", "priority": "high", "status": "pending"}],
            }))
            .unwrap(),
        );
        assert!(!view.items.is_empty());
        view.reset_items();
        assert!(view.items.is_empty());
        assert!(view.tool_calls.is_empty());
        assert!(view.plan.is_empty());
        assert!(view.permissions.is_empty());
    }

    /// The live `session/update`s of the subagent fixture, in order.
    fn subagent_updates() -> Vec<SessionUpdate> {
        SUBAGENT_SESSION
            .lines()
            .filter_map(|line| {
                let v: Value = serde_json::from_str(line).ok()?;
                if v["dir"] != "a2c" || v["msg"]["method"] != "session/update" {
                    return None;
                }
                serde_json::from_value(v["msg"]["params"]["update"].clone()).ok()
            })
            .collect()
    }

    fn end_turn() -> PromptResult {
        serde_json::from_value(json!({"stopReason": "end_turn"})).unwrap()
    }

    fn cancelled_turn() -> PromptResult {
        serde_json::from_value(json!({"stopReason": "cancelled"})).unwrap()
    }

    /// A spawn tool call with the given status and optional `_meta` extras.
    fn spawn_update(status: &str, extra_meta: Value) -> SessionUpdate {
        let mut meta = json!({
            "tool_name": "agents.spawn",
            "effect_kind": "subagent",
            "agent": "generic",
            "task": "Lies README.md",
        });
        if let (Some(meta), Some(extra)) = (meta.as_object_mut(), extra_meta.as_object()) {
            for (k, v) in extra {
                meta.insert(k.clone(), v.clone());
            }
        }
        serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "spawn-1",
            "title": "Starting reader",
            "kind": "think",
            "status": status,
            "rawInput": {"task": "Lies README.md", "agent": "generic"},
            "_meta": meta,
        }))
        .unwrap()
    }

    fn wait_update(status: &str) -> SessionUpdate {
        serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "wait-1",
            "title": "Waiting for input",
            "kind": "other",
            "status": status,
            "_meta": {"tool_name": "self.wait", "effect_kind": "tool"},
        }))
        .unwrap()
    }

    #[test]
    fn subagent_from_fixture_is_done_after_end_turn() {
        let mut view = SessionView::new("sid", "/tmp/acp-fixture/ws");
        view.begin_turn();
        for update in subagent_updates() {
            view.apply_update(&update);
        }
        view.finish_turn(&end_turn());

        let subs = view.subagents();
        assert_eq!(subs.len(), 1, "exactly one subagent");
        let s = &subs[0];
        assert_eq!(s.name, "Starting reader");
        assert_eq!(s.agent, "generic");
        assert!(s.task.starts_with("Lies README.md"), "task = {}", s.task);
        assert!(s
            .child_session_id
            .as_deref()
            .is_some_and(|id| id.starts_with("child-")));
        assert_eq!(s.state, SubagentState::Done);
        assert_eq!(s.turns, None);
        assert!(!s.tool_call_id.is_empty());
    }

    #[test]
    fn subagent_running_right_after_spawn() {
        let mut view = SessionView::new("s", "/tmp");
        view.begin_turn();
        view.apply_update(&spawn_update("in_progress", json!({})));
        let subs = view.subagents();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].state, SubagentState::Running);
        assert_eq!(subs[0].child_session_id, None);

        // spawn completed, but the parent has not waited yet and the turn runs
        view.apply_update(&spawn_update(
            "completed",
            json!({"child_session_id": "child-abc", "turn_count": 2}),
        ));
        let subs = view.subagents();
        assert_eq!(subs[0].state, SubagentState::Running);
        assert_eq!(subs[0].child_session_id.as_deref(), Some("child-abc"));
        assert_eq!(subs[0].turns, Some(2));

        // the parent's wait call completed -> the child delivered its result
        view.apply_update(&wait_update("completed"));
        assert_eq!(view.subagents()[0].state, SubagentState::Done);
    }

    #[test]
    fn subagent_failed_spawn() {
        let mut view = SessionView::new("s", "/tmp");
        view.begin_turn();
        view.apply_update(&spawn_update("failed", json!({})));
        assert_eq!(view.subagents()[0].state, SubagentState::Failed);
        // a failed spawn stays failed even after the turn ended
        view.finish_turn(&end_turn());
        assert_eq!(view.subagents()[0].state, SubagentState::Failed);
    }

    /// N14 counters: every painted frame adds one per pending card, and an
    /// answered request loses its counter on the next paint (no stale
    /// counters).
    #[test]
    fn permission_frame_counters_count_paints_and_forget_left_requests() {
        let mut v = SessionView::new("s", "/tmp");
        let req: PermissionRequest = serde_json::from_value(json!({
            "sessionId": "s",
            "toolCallId": "t1",
            "toolCall": {"toolCallId": "t1", "kind": "execute", "status": "pending"},
            "options": []
        }))
        .unwrap();
        let rid = Value::from("req-1");
        assert_eq!(v.frames_shown(&rid), 0, "unknown request: no frames");
        v.add_permission(rid.clone(), req);
        assert_eq!(v.frames_shown(&rid), 0, "arrival alone paints nothing yet");
        v.paint_permission_frames();
        v.paint_permission_frames();
        assert_eq!(v.frames_shown(&rid), 2, "two painted frames");
        v.paint_permission_frames();
        assert_eq!(v.frames_shown(&rid), 3, "grace reached after three");
        v.take_permission(&rid).expect("the request is pending");
        v.paint_permission_frames();
        assert_eq!(v.frames_shown(&rid), 0, "the counter dies with the request");
    }

    #[test]
    fn subagent_cancelled_turn() {
        let mut view = SessionView::new("s", "/tmp");
        view.begin_turn();
        view.apply_update(&spawn_update("completed", json!({})));
        assert_eq!(view.subagents()[0].state, SubagentState::Running);
        view.finish_turn(&cancelled_turn());
        assert_eq!(view.subagents()[0].state, SubagentState::Cancelled);
    }

    /// N5: the Cancelled state is frozen per subagent, not re-derived from
    /// the global turn state — a later `begin_turn` (and even its
    /// `finish_turn`) must not bring the long-dead subagent back to Running.
    #[test]
    fn cancelled_subagent_stays_cancelled_over_later_turns() {
        let mut view = SessionView::new("s", "/tmp");
        view.begin_turn();
        view.apply_update(&spawn_update("in_progress", json!({})));
        assert_eq!(view.subagents()[0].state, SubagentState::Running);
        view.finish_turn(&cancelled_turn());
        assert_eq!(view.subagents()[0].state, SubagentState::Cancelled);

        // the next turn runs and ends normally
        view.begin_turn();
        assert_eq!(
            view.subagents()[0].state,
            SubagentState::Cancelled,
            "begin_turn alone must not revive the subagent"
        );
        view.apply_update(&spawn_update("completed", json!({})));
        view.finish_turn(&end_turn());
        assert_eq!(
            view.subagents()[0].state,
            SubagentState::Cancelled,
            "the frozen marking outlives the later turn"
        );
    }

    /// N5 boundary: a subagent whose `self.wait` already completed before
    /// the cancel has delivered its result — it is Done, not Cancelled. A
    /// failed spawn keeps its own state as well.
    #[test]
    fn waited_and_failed_subagents_do_not_freeze_on_cancel() {
        let mut view = SessionView::new("s", "/tmp");
        view.begin_turn();
        view.apply_update(&spawn_update("completed", json!({})));
        view.apply_update(&wait_update("completed"));
        assert_eq!(view.subagents()[0].state, SubagentState::Done);
        view.finish_turn(&cancelled_turn());
        assert_eq!(view.subagents()[0].state, SubagentState::Done);

        // a failed spawn stays Failed, cancelled turn or not
        let mut view = SessionView::new("s2", "/tmp");
        view.begin_turn();
        view.apply_update(&spawn_update("failed", json!({})));
        assert_eq!(view.subagents()[0].state, SubagentState::Failed);
        view.finish_turn(&cancelled_turn());
        assert_eq!(view.subagents()[0].state, SubagentState::Failed);
    }

    /// Final audit E: the freeze is bounded to the cancelled turn. A
    /// spawn the PREVIOUS turn left unfinished keeps its own state when
    /// a LATER turn stops cancelled — the later cancel must not rewrite
    /// an item it never ran.
    #[test]
    fn unfinished_subagent_of_an_earlier_turn_survives_a_later_cancel() {
        let mut view = SessionView::new("s", "/tmp");
        view.begin_turn();
        view.apply_update(&spawn_update("in_progress", json!({})));
        // the earlier turn ends normally, its spawn unfinished
        view.finish_turn(&end_turn());
        assert_eq!(view.subagents()[0].state, SubagentState::Running);
        // a later turn runs and is cancelled
        view.begin_turn();
        view.finish_turn(&cancelled_turn());
        assert_eq!(
            view.subagents()[0].state,
            SubagentState::Running,
            "the cancel of turn 2 does not freeze turn 1's spawn"
        );
    }

    #[test]
    fn subagent_summary_counts_separately() {
        let spawn: ToolCall = serde_json::from_value(json!({
            "toolCallId": "spawn-1",
            "title": "Starting reader",
            "kind": "think",
            "_meta": {"tool_name": "agents.spawn", "effect_kind": "subagent"},
        }))
        .unwrap();
        let read = tool_call(ToolKind::Read, "read-1");
        assert!(is_subagent(&spawn));
        assert!(!is_subagent(&read));
        assert_eq!(
            tool_group_summary(&[&read, &spawn]),
            "Starts 1 subagent, reads 1 file"
        );
        assert_eq!(tool_group_summary(&[&spawn]), "Starts 1 subagent");
        let spawn2: ToolCall = serde_json::from_value(json!({
            "toolCallId": "spawn-2",
            "kind": "think",
            "_meta": {"tool_name": "agents.spawn"},
        }))
        .unwrap();
        assert_eq!(tool_group_summary(&[&spawn, &spawn2]), "Starts 2 subagents");
    }

    /// The retention bound (audit N1): past it the oldest items leave,
    /// tools that went with them are forgotten like `cut_items` forgets,
    /// the turn index shifts with the items, and the dropped count adds
    /// up for the transcript hint.
    #[test]
    fn retention_trims_the_oldest_items_and_shifts_bookkeeping() {
        let mut v = SessionView::new("s", "/w");
        v.items.push(Item::Tool("dropped".to_string()));
        for i in 0..4_500 {
            v.items.push(user(&format!("m{i}"), "Frage"));
        }
        v.items.push(Item::Tool("kept".to_string()));
        v.tool_calls.insert(
            "dropped".to_string(),
            serde_json::from_value(json!({"toolCallId": "dropped"})).unwrap(),
        );
        v.tool_calls.insert(
            "kept".to_string(),
            serde_json::from_value(json!({"toolCallId": "kept"})).unwrap(),
        );
        v.cancelled_tools.insert("dropped".to_string());
        v.turn_start = Some(3_000);
        v.trim_front();
        // 4502 items: 2502 leave, the last 2000 stay
        assert_eq!(v.items.len(), 2_000);
        assert_eq!(v.trimmed_front, 2_502);
        assert!(!v.tool_calls.contains_key("dropped"));
        assert!(!v.cancelled_tools.contains("dropped"));
        assert!(v.tool_calls.contains_key("kept"));
        assert_eq!(v.turn_start, Some(498));
        // under the bound nothing happens, the count stays where it was
        v.items.clear();
        v.trim_front();
        assert_eq!(v.trimmed_front, 2_502);
        // a `session/load` resets hint and count with the transcript
        v.trimmed_front = 7;
        v.reset_items();
        assert_eq!(v.trimmed_front, 0);
    }
}
