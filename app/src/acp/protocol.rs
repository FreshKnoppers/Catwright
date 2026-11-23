//! Typed ACP messages (schema verified against Vibe 3.1.0, see docs/acp.md).
//!
//! Parsing is deliberately tolerant: unknown fields are ignored, optional
//! fields may be missing, and every tagged enum has an `Unknown`/`Other`
//! catch-all so an unknown tag never fails the whole message.

use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct InitializeResult {
    pub protocol_version: u32,
    pub agent_capabilities: AgentCapabilities,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
    #[serde(default)]
    pub agent_info: Option<Implementation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct AuthMethod {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct AgentCapabilities {
    #[serde(default)]
    pub load_session: bool,
    #[serde(default)]
    pub prompt_capabilities: PromptCapabilities,
    #[serde(default)]
    pub session_capabilities: SessionCapabilities,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct PromptCapabilities {
    #[serde(default)]
    pub image: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub embedded_context: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SessionCapabilities {
    #[serde(default)]
    pub list: Option<Value>,
    #[serde(default)]
    pub fork: Option<Value>,
    #[serde(default)]
    pub close: Option<Value>,
}

// ---------------------------------------------------------------------------
// sessions: new / load / list
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResult {
    pub session_id: String,
    #[serde(default)]
    pub modes: Option<ModeState>,
    #[serde(default)]
    pub config_options: Vec<ConfigOption>,
    /// Vibe's `_meta` (`workspace_trust`); see `workspace_trust_of`.
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionResult {
    #[serde(default)]
    pub modes: Option<ModeState>,
    #[serde(default)]
    pub config_options: Vec<ConfigOption>,
    /// Vibe's `_meta` (`workspace_trust`); see `workspace_trust_of`.
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeState {
    pub current_mode_id: String,
    #[serde(default)]
    pub available_modes: Vec<SessionMode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[allow(dead_code)]
pub enum ConfigOption {
    Select {
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        category: Option<String>,
        #[serde(default, deserialize_with = "string_or_null")]
        current_value: String,
        #[serde(default)]
        options: Vec<ConfigSelectOption>,
    },
    Boolean {
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        category: Option<String>,
        // Vibe nulls `currentValue` the same way it does for select
        // options; null must not discard the whole option. Data model:
        // null behaves like a missing key, i.e. `false` (N13).
        #[serde(default, deserialize_with = "bool_or_null")]
        current_value: bool,
    },
    #[serde(other)]
    Unknown,
}

/// `currentValue` may be `null` in Vibe's answers; it reads as "" then.
fn string_or_null<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// The boolean counterpart of `string_or_null`: a nulled `currentValue`
/// reads as `false` (null behaves exactly like a missing key, N13);
/// anything that is neither a bool nor null still fails loudly.
fn bool_or_null<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or(false))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSelectOption {
    pub value: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsResult {
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SessionInfo {
    pub session_id: String,
    pub cwd: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub additional_directories: Vec<String>,
}

// ---------------------------------------------------------------------------
// prompt result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResult {
    pub stop_reason: StopReason,
    #[serde(default)]
    pub usage: Option<TurnUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct TurnUsage {
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub thought_tokens: Option<u64>,
    #[serde(default)]
    pub cached_read_tokens: Option<u64>,
    #[serde(default)]
    pub cached_write_tokens: Option<u64>,
}

// ---------------------------------------------------------------------------
// session/update notifications
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNotification {
    pub session_id: String,
    pub update: SessionUpdate,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "sessionUpdate",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SessionUpdate {
    UserMessageChunk(Chunk),
    AgentMessageChunk(Chunk),
    AgentThoughtChunk(Chunk),
    ToolCall(ToolCall),
    ToolCallUpdate(ToolCallUpdate),
    Plan(Plan),
    AvailableCommandsUpdate(AvailableCommands),
    CurrentModeUpdate(CurrentMode),
    ConfigOptionUpdate(ConfigOptions),
    SessionInfoUpdate(SessionInfoUpd),
    UsageUpdate(UsageUpdate),
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub content: ContentBlock,
    #[serde(default)]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[allow(dead_code)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        data: String,
        mime_type: String,
        uri: Option<String>,
    },
    ResourceLink {
        uri: String,
        name: String,
        mime_type: Option<String>,
        title: Option<String>,
    },
    Resource {
        resource: EmbeddedResource,
    },
    #[serde(other)]
    Unknown,
}

impl ContentBlock {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct EmbeddedResource {
    pub uri: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub blob: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

// ---------------------------------------------------------------------------
// tool calls
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub tool_call_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub kind: ToolKind,
    #[serde(default)]
    pub status: ToolStatus,
    #[serde(default)]
    pub content: Vec<ToolCallContent>,
    #[serde(default)]
    pub locations: Vec<Location>,
    #[serde(default)]
    pub raw_input: Option<Value>,
    #[serde(default)]
    pub raw_output: Option<Value>,
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
    /// Client-side arrival of the first update for this call (epoch
    /// seconds) — not from the wire; basis of the per-round duration view.
    #[serde(skip)]
    pub first_seen: u64,
    /// Client-side arrival of the last update (epoch seconds).
    #[serde(skip)]
    pub last_update: u64,
    /// Cached retained size in bytes (recomputed in `apply`/`bounded_clone`),
    /// so the per-update `trim_front` budget does not re-serialize every
    /// retained tool call on every streamed chunk.
    #[serde(skip)]
    pub size_hint: usize,
}

impl ToolCall {
    /// Apply a `tool_call_update`: only fields present in the update change;
    /// `content`/`locations` replace the whole list when present.
    pub fn apply(&mut self, upd: &ToolCallUpdate) {
        if let Some(title) = &upd.title {
            self.title = bounded_text(title, MAX_TOOL_TITLE_BYTES);
        }
        if let Some(kind) = &upd.kind {
            self.kind = kind.clone();
        }
        if let Some(status) = &upd.status {
            self.status = status.clone();
        }
        if let Some(content) = &upd.content {
            self.content = bounded_content(content);
        }
        if let Some(locations) = &upd.locations {
            self.locations = bounded_locations(locations);
        }
        // serde maps json `null` to None, so `is_some` is presence; the
        // takeover itself caps the value's size (see bounded_raw)
        if let Some(raw) = &upd.raw_input {
            self.raw_input = Some(bounded_raw(raw));
        }
        if let Some(raw) = &upd.raw_output {
            self.raw_output = Some(bounded_raw(raw));
        }
        if upd.meta.is_some() {
            self.meta = upd.meta.as_ref().map(bounded_raw).or(upd.meta.clone());
        }
        self.size_hint = self.compute_size_hint();
    }

    /// A copy whose raw input/output obey the retention bound: the first
    /// takeover goes through this cap too, not only updates of an entry.
    pub fn bounded_clone(&self) -> ToolCall {
        let mut tc = ToolCall {
            raw_input: self.raw_input.as_ref().map(bounded_raw),
            raw_output: self.raw_output.as_ref().map(bounded_raw),
            tool_call_id: self.tool_call_id.clone(),
            title: bounded_text(&self.title, MAX_TOOL_TITLE_BYTES),
            kind: self.kind.clone(),
            status: self.status.clone(),
            content: bounded_content(&self.content),
            locations: bounded_locations(&self.locations),
            meta: self.meta.as_ref().map(bounded_raw),
            first_seen: self.first_seen,
            last_update: self.last_update,
            size_hint: 0,
        };
        tc.size_hint = tc.compute_size_hint();
        tc
    }

    /// Recompute the cached retained size (called when fields change, not on
    /// every read): the flood bound in `SessionView::trim_front` runs per
    /// streamed chunk and must not re-serialize every retained entry then.
    fn compute_size_hint(&self) -> usize {
        let raw = |v: &Option<Value>| {
            v.as_ref()
                .map(|v| serde_json::to_string(v).map(|s| s.len()).unwrap_or(0))
                .unwrap_or(0)
        };
        self.title.len()
            + 8 // kind: small enum, flat estimate
            + self.tool_call_id.len()
            + raw(&self.raw_input)
            + raw(&self.raw_output)
            + raw(&self.meta)
            + self
                .locations
                .iter()
                .map(|l| l.path.len() + 8)
                .sum::<usize>()
            + self.content.iter().map(content_entry_bytes).sum::<usize>()
    }

    /// The cached retained size of this entry in bytes (see
    /// [`Self::compute_size_hint`]).
    pub fn approx_bytes(&self) -> usize {
        self.size_hint
    }

    /// Client-side duration in seconds: last update minus first arrival; a
    /// call that is not finished measures up to `now`. `None` when the call
    /// was not watched live (replayed history): no honest duration exists.
    pub fn duration_secs(&self, now: u64) -> Option<u64> {
        if self.first_seen == 0 {
            return None;
        }
        let end = match self.status {
            ToolStatus::Completed | ToolStatus::Failed => self.last_update,
            _ => now,
        };
        Some(end.saturating_sub(self.first_seen))
    }
}

/// How large a retained tool raw value may be (audit N1): raws are display
/// data for the cards, and a child streaming giant payloads through a
/// single tool call must not grow its entry freely.
const MAX_TOOL_RAW_BYTES: usize = 256 * 1024;

/// Retention bounds for the rest of a tool call's display data (flood
/// hardening): the agent drives every byte, so title, content entries,
/// locations and meta all get caps too. Diff texts are truncated in place
/// (the diff view still works on the rest); an oversized entry of another
/// kind becomes a short marker text.
const MAX_TOOL_TITLE_BYTES: usize = 4 * 1024;
const MAX_TOOL_CONTENT_ENTRIES: usize = 64;
const MAX_TOOL_CONTENT_ENTRY_BYTES: usize = 256 * 1024;
const MAX_TOOL_LOCATIONS: usize = 64;
const MAX_TOOL_PATH_BYTES: usize = 4 * 1024;

/// A raw value within the bound is taken as is; a larger one becomes a
/// short marker object that says so. The marker keeps the JSON shape the
/// cards render, with the content gone.
fn bounded_raw(v: &Value) -> Value {
    match serde_json::to_string(v) {
        Ok(s) if s.len() <= MAX_TOOL_RAW_BYTES => v.clone(),
        _ => json!({"trimmed": true, "reason": "larger than the retention bound"}),
    }
}

/// `s` cut at a char boundary to at most `max` bytes, with a marker appended.
fn bounded_text(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push_str("… [truncated]");
    out
}

/// Approximate size of one content entry in bytes (the enums are
/// Deserialize-only, so the strings are summed by hand).
fn content_entry_bytes(c: &ToolCallContent) -> usize {
    match c {
        ToolCallContent::Content { content } => match content {
            ContentBlock::Text { text } => text.len(),
            ContentBlock::Image {
                data,
                mime_type,
                uri,
            } => data.len() + mime_type.len() + uri.as_ref().map(|u| u.len()).unwrap_or(0),
            ContentBlock::ResourceLink {
                uri,
                name,
                mime_type,
                title,
            } => {
                uri.len()
                    + name.len()
                    + mime_type.as_ref().map(|m| m.len()).unwrap_or(0)
                    + title.as_ref().map(|t| t.len()).unwrap_or(0)
            }
            ContentBlock::Resource { resource } => {
                resource.uri.len()
                    + resource.text.as_ref().map(|t| t.len()).unwrap_or(0)
                    + resource.blob.as_ref().map(|b| b.len()).unwrap_or(0)
                    + resource.mime_type.as_ref().map(|m| m.len()).unwrap_or(0)
            }
            ContentBlock::Unknown => 0,
        },
        ToolCallContent::Diff {
            path,
            old_text,
            new_text,
        } => path.len() + new_text.len() + old_text.as_ref().map(|t| t.len()).unwrap_or(0),
        ToolCallContent::Terminal { terminal_id } => terminal_id.len(),
        ToolCallContent::Unknown => 0,
    }
}

/// One tool call's `content` list within the retention bounds: at most
/// [`MAX_TOOL_CONTENT_ENTRIES`] entries, each within
/// [`MAX_TOOL_CONTENT_ENTRY_BYTES`] (diffs keep their shape, truncated).
fn bounded_content(content: &[ToolCallContent]) -> Vec<ToolCallContent> {
    content
        .iter()
        .take(MAX_TOOL_CONTENT_ENTRIES)
        .map(|c| match c {
            ToolCallContent::Diff {
                path,
                old_text,
                new_text,
            } => ToolCallContent::Diff {
                path: bounded_text(path, MAX_TOOL_PATH_BYTES),
                old_text: old_text
                    .as_deref()
                    .map(|t| bounded_text(t, MAX_TOOL_CONTENT_ENTRY_BYTES)),
                new_text: bounded_text(new_text, MAX_TOOL_CONTENT_ENTRY_BYTES),
            },
            other if content_entry_bytes(other) <= MAX_TOOL_CONTENT_ENTRY_BYTES => other.clone(),
            _ => ToolCallContent::Content {
                content: ContentBlock::Text {
                    text: format!(
                        "… [truncated: larger than {MAX_TOOL_CONTENT_ENTRY_BYTES} bytes]"
                    ),
                },
            },
        })
        .collect()
}

/// Most options a permission request keeps (Vibe sends at most six; the
/// card shows this many).
pub const MAX_PERMISSION_OPTIONS: usize = 8;
/// Longest option id kept: the id is sent back verbatim, so it is never
/// cut — an option with a longer id is dropped (Vibe's ids are a few
/// words).
const MAX_OPTION_ID_BYTES: usize = 256;
/// Longest option label kept.
const MAX_OPTION_NAME_BYTES: usize = 256;

impl PermissionRequest {
    /// The request within byte bounds (audit M4): it is kept until the
    /// user answers and its card is laid out every frame, while a wire line
    /// may be 64 MiB. The tool call gets the retention bounds of a
    /// `tool_call` (Vibe sends only its id), at most
    /// [`MAX_PERMISSION_OPTIONS`] options are kept, labels are cut, and an
    /// option whose id is too long to be real is dropped (an id is never
    /// cut: it is what the answer sends).
    pub fn bounded(self) -> PermissionRequest {
        let tc = self.tool_call;
        PermissionRequest {
            session_id: bounded_text(&self.session_id, MAX_TOOL_PATH_BYTES),
            tool_call: ToolCallUpdate {
                tool_call_id: bounded_text(&tc.tool_call_id, MAX_TOOL_PATH_BYTES),
                title: tc.title.map(|t| bounded_text(&t, MAX_TOOL_TITLE_BYTES)),
                kind: tc.kind,
                status: tc.status,
                content: tc.content.map(|c| bounded_content(&c)),
                locations: tc.locations.map(|l| bounded_locations(&l)),
                raw_input: tc.raw_input.as_ref().map(bounded_raw),
                raw_output: tc.raw_output.as_ref().map(bounded_raw),
                meta: tc.meta.as_ref().map(bounded_raw),
            },
            options: self
                .options
                .into_iter()
                .filter(|o| o.option_id.len() <= MAX_OPTION_ID_BYTES)
                .take(MAX_PERMISSION_OPTIONS)
                .map(|o| PermissionOption {
                    name: bounded_text(&o.name, MAX_OPTION_NAME_BYTES),
                    ..o
                })
                .collect(),
        }
    }
}

/// Most entries a plan keeps (the agent drives the list; the latest stay).
pub const MAX_PLAN_ENTRIES: usize = 256;
/// Longest plan entry kept: a plan is shown in the transcript and the
/// panel every frame (audit M4).
const MAX_PLAN_ENTRY_BYTES: usize = 4 * 1024;

/// The latest [`MAX_PLAN_ENTRIES`] entries of `entries`, each cut to
/// [`MAX_PLAN_ENTRY_BYTES`].
pub fn bounded_plan(entries: &[PlanEntry]) -> Vec<PlanEntry> {
    let skip = entries.len().saturating_sub(MAX_PLAN_ENTRIES);
    entries[skip..]
        .iter()
        .map(|e| PlanEntry {
            content: bounded_text(&e.content, MAX_PLAN_ENTRY_BYTES),
            ..e.clone()
        })
        .collect()
}

/// Most slash commands kept (Vibe sends its built-ins plus one per skill).
const MAX_COMMANDS: usize = 256;
/// Longest command name kept: the name goes into the message when picked,
/// so a longer one is dropped, not cut.
const MAX_COMMAND_NAME_BYTES: usize = 128;
/// Longest command description and input hint kept.
const MAX_COMMAND_TEXT_BYTES: usize = 1024;

/// The slash commands within bounds (audit M4): the list is copied for the
/// `/` popup and the palette while they are open.
pub fn bounded_commands(commands: &[AvailableCommand]) -> Vec<AvailableCommand> {
    commands
        .iter()
        .filter(|c| c.name.len() <= MAX_COMMAND_NAME_BYTES)
        .take(MAX_COMMANDS)
        .map(|c| AvailableCommand {
            name: c.name.clone(),
            description: bounded_text(&c.description, MAX_COMMAND_TEXT_BYTES),
            input: c.input.as_ref().map(|i| CommandInput {
                hint: i
                    .hint
                    .as_deref()
                    .map(|h| bounded_text(h, MAX_COMMAND_TEXT_BYTES)),
            }),
        })
        .collect()
}

fn bounded_locations(locations: &[Location]) -> Vec<Location> {
    locations
        .iter()
        .take(MAX_TOOL_LOCATIONS)
        .map(|l| Location {
            path: bounded_text(&l.path, MAX_TOOL_PATH_BYTES),
            line: l.line,
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallUpdate {
    pub tool_call_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub kind: Option<ToolKind>,
    #[serde(default)]
    pub status: Option<ToolStatus>,
    #[serde(default)]
    pub content: Option<Vec<ToolCallContent>>,
    #[serde(default)]
    pub locations: Option<Vec<Location>>,
    #[serde(default)]
    pub raw_input: Option<Value>,
    #[serde(default)]
    pub raw_output: Option<Value>,
    #[serde(rename = "_meta", default)]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[allow(dead_code)]
pub enum ToolCallContent {
    Content {
        content: ContentBlock,
    },
    Diff {
        path: String,
        old_text: Option<String>,
        new_text: String,
    },
    Terminal {
        terminal_id: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Location {
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
}

// ---------------------------------------------------------------------------
// plan / commands / mode / config / info / usage updates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub entries: Vec<PlanEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PlanEntry {
    pub content: String,
    #[serde(default)]
    pub priority: PlanPriority,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPriority {
    High,
    Medium,
    Low,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableCommands {
    #[serde(default)]
    pub available_commands: Vec<AvailableCommand>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct AvailableCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input: Option<CommandInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct CommandInput {
    #[serde(default)]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentMode {
    pub current_mode_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOptions {
    #[serde(default)]
    pub config_options: Vec<ConfigOption>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SessionInfoUpd {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageUpdate {
    pub used: u64,
    pub size: u64,
    #[serde(default)]
    pub cost: Option<Cost>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
}

// ---------------------------------------------------------------------------
// session/request_permission (agent -> client request)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub session_id: String,
    pub tool_call: ToolCallUpdate,
    #[serde(default)]
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: PermissionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    #[default]
    #[serde(other)]
    Other,
}

// ---------------------------------------------------------------------------
// outgoing params
// ---------------------------------------------------------------------------

/// One block of a `session/prompt` prompt array.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptBlock {
    Text(String),
    Image {
        data_base64: String,
        mime_type: String,
    },
    ResourceLink {
        uri: String,
        name: String,
    },
}

impl PromptBlock {
    fn to_block(&self) -> Value {
        match self {
            PromptBlock::Text(text) => json!({ "type": "text", "text": text }),
            PromptBlock::Image {
                data_base64,
                mime_type,
            } => json!({ "type": "image", "data": data_base64, "mimeType": mime_type }),
            PromptBlock::ResourceLink { uri, name } => {
                json!({ "type": "resource_link", "uri": uri, "name": name })
            }
        }
    }
}

/// We do not implement the fs/terminal client methods, so those
/// capabilities stay off (Vibe then runs tools itself). The `elicitation`
/// form capability is on: it unlocks Vibe's `ask_user_question` tool, whose
/// `elicitation/create` requests render as a question card.
pub fn initialize_params(client_name: &str, client_version: &str) -> Value {
    json!({
        "protocolVersion": 1,
        "clientCapabilities": {
            "fs": { "readTextFile": false, "writeTextFile": false },
            "terminal": false,
            "elicitation": { "form": {} },
        },
        "clientInfo": { "name": client_name, "version": client_version },
    })
}

pub fn new_session_params(cwd: &Path) -> Value {
    json!({
        "cwd": cwd.to_string_lossy(),
        "mcpServers": [],
    })
}

pub fn load_session_params(cwd: &Path, session_id: &str) -> Value {
    json!({
        "cwd": cwd.to_string_lossy(),
        "sessionId": session_id,
        "mcpServers": [],
    })
}

/// `session/resume` for a session that is already open in this vibe-acp
/// process. Verified against Vibe 3.1.2: Vibe implements it without
/// advertising it in `sessionCapabilities`; on an open session it only
/// re-sends `usage_update` (the same params as `session/load`). The harness
/// uses it after a model switch, which itself carries no usage_update.
pub fn resume_session_params(cwd: &Path, session_id: &str) -> Value {
    load_session_params(cwd, session_id)
}

/// `session/fork` (ACP v1, verified against Vibe 3.1.0): a new session with
/// the context of an existing one; the response has `session/new`'s shape.
/// With `message_id` (a user message's `messageId`, passed in `_meta`) the
/// fork ends after that message's turn; without it the whole history forks.
pub fn fork_session_params(cwd: &Path, session_id: &str, message_id: Option<&str>) -> Value {
    let mut params = json!({
        "sessionId": session_id,
        "cwd": cwd.to_string_lossy(),
        "mcpServers": [],
    });
    if let Some(message_id) = message_id {
        params["_meta"] = json!({ "messageId": message_id });
    }
    params
}

/// Only fields that are set are sent.
pub fn list_sessions_params(cwd: Option<&Path>, cursor: Option<&str>) -> Value {
    let mut p = serde_json::Map::new();
    if let Some(cwd) = cwd {
        p.insert(
            "cwd".to_string(),
            Value::String(cwd.to_string_lossy().into_owned()),
        );
    }
    if let Some(cursor) = cursor {
        p.insert("cursor".to_string(), Value::String(cursor.to_string()));
    }
    Value::Object(p)
}

pub fn prompt_params(session_id: &str, blocks: &[PromptBlock]) -> Value {
    json!({
        "sessionId": session_id,
        "prompt": blocks.iter().map(PromptBlock::to_block).collect::<Vec<_>>(),
    })
}

pub fn session_id_params(session_id: &str) -> Value {
    json!({ "sessionId": session_id })
}

pub fn set_mode_params(session_id: &str, mode_id: &str) -> Value {
    json!({ "sessionId": session_id, "modeId": mode_id })
}

pub fn set_config_option_params(session_id: &str, config_id: &str, value: &str) -> Value {
    json!({ "sessionId": session_id, "configId": config_id, "value": value })
}

/// Response body for `elicitation/create`: `accept` with the form content,
/// or `decline` / `cancel` without one.
pub fn elicitation_response(action: &str, content: Option<Value>) -> Value {
    match content {
        Some(content) => json!({ "action": action, "content": content }),
        None => json!({ "action": action }),
    }
}

/// Response body for `session/request_permission`.
pub fn permission_outcome(option_id: Option<&str>) -> Value {
    match option_id {
        Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
        None => json!({ "outcome": { "outcome": "cancelled" } }),
    }
}

// ---------------------------------------------------------------------------
// tests (offline, against the real wire transcripts)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit M4: plan entries and slash commands are cut at intake; a
    /// command whose name is too long is dropped (its name would be sent).
    #[test]
    fn plans_and_commands_are_byte_bounded() {
        let entries: Vec<PlanEntry> = (0..300)
            .map(|i| PlanEntry {
                content: format!("{i}:{}", "p".repeat(10_000)),
                priority: PlanPriority::Medium,
                status: PlanStatus::Pending,
            })
            .collect();
        let plan = bounded_plan(&entries);
        assert_eq!(plan.len(), MAX_PLAN_ENTRIES);
        assert!(plan[0].content.starts_with("44:"), "the latest stay");
        assert!(plan
            .iter()
            .all(|e| e.content.len() <= MAX_PLAN_ENTRY_BYTES + 20));

        let mut commands: Vec<AvailableCommand> = (0..300)
            .map(|i| AvailableCommand {
                name: format!("c{i}"),
                description: "d".repeat(100_000),
                input: Some(CommandInput {
                    hint: Some("h".repeat(100_000)),
                }),
            })
            .collect();
        commands.insert(
            0,
            AvailableCommand {
                name: "n".repeat(10_000),
                description: String::new(),
                input: None,
            },
        );
        let kept = bounded_commands(&commands);
        assert_eq!(kept.len(), MAX_COMMANDS);
        assert_eq!(kept[0].name, "c0", "the over-long name is dropped");
        assert!(kept
            .iter()
            .all(|c| c.description.len() <= MAX_COMMAND_TEXT_BYTES + 20));
        assert!(kept.iter().all(|c| c
            .input
            .as_ref()
            .and_then(|i| i.hint.as_ref())
            .is_none_or(|h| h.len() <= MAX_COMMAND_TEXT_BYTES + 20)));
    }

    #[test]
    fn fork_params_use_session_id_and_optional_fork_point() {
        let plain = fork_session_params(Path::new("/w"), "s1", None);
        assert_eq!(plain["sessionId"], json!("s1"));
        assert_eq!(plain["cwd"], json!("/w"));
        assert!(plain.get("forkSessionId").is_none());
        assert!(plain.get("_meta").is_none());
        let at = fork_session_params(Path::new("/w"), "s1", Some("m7"));
        assert_eq!(at["_meta"]["messageId"], json!("m7"));
    }

    #[test]
    fn elicitation_capability_and_responses() {
        let init = initialize_params("catwright", "0.1.0");
        assert_eq!(init["clientCapabilities"]["elicitation"]["form"], json!({}));
        assert_eq!(
            elicitation_response("accept", Some(json!({"q0": "CSV"}))),
            json!({"action": "accept", "content": {"q0": "CSV"}})
        );
        assert_eq!(
            elicitation_response("cancel", None),
            json!({"action": "cancel"})
        );
    }

    #[test]
    fn session_results_keep_vibes_meta() {
        let r: NewSessionResult = serde_json::from_value(json!({
            "sessionId": "s",
            "_meta": {"workspace_trust": {"status": "untrusted", "details": null}}
        }))
        .unwrap();
        assert_eq!(
            r.meta.unwrap()["workspace_trust"]["status"],
            json!("untrusted")
        );
    }

    /// Bug 3 (Handoff 2026-11-09): inner fields Vibe sometimes omits or
    /// nulls must not discard the whole message.
    #[test]
    fn partially_worded_answers_still_parse() {
        let usage: TurnUsage = serde_json::from_value(json!({"totalTokens": 12})).unwrap();
        assert_eq!(usage.total_tokens, 12);
        assert_eq!(usage.input_tokens, 0);
        let options: ConfigOptions = serde_json::from_value(json!({
            "configOptions": [
                {"id": "model", "type": "select", "currentValue": null,
                 "options": [{"value": "m1", "name": "M1"}]}
            ]
        }))
        .unwrap();
        match &options.config_options[0] {
            ConfigOption::Select { current_value, .. } => assert_eq!(current_value, ""),
            other => panic!("expected select, got {other:?}"),
        }
        let plan: Plan = serde_json::from_value(json!({
            "entries": [{"content": "Schritt", "status": "pending"}]
        }))
        .unwrap();
        assert_eq!(plan.entries[0].priority, PlanPriority::Other);
    }

    /// N13: a boolean option with `currentValue: null` parses, reading the
    /// null as `false` (the same tolerance Select already had); a real
    /// value keeps its meaning.
    #[test]
    fn boolean_config_option_tolerates_a_null_current_value() {
        let c: ConfigOption = serde_json::from_value(json!({
            "type": "boolean", "id": "auto", "currentValue": null
        }))
        .unwrap();
        match c {
            ConfigOption::Boolean { current_value, .. } => {
                assert!(!current_value, "null reads as false")
            }
            other => panic!("expected boolean, got {other:?}"),
        }
        let c: ConfigOption = serde_json::from_value(json!({
            "type": "boolean", "id": "auto", "currentValue": true
        }))
        .unwrap();
        match c {
            ConfigOption::Boolean { current_value, .. } => assert!(current_value),
            other => panic!("expected boolean, got {other:?}"),
        }
        // a missing key behaves like null
        let c: ConfigOption = serde_json::from_value(json!({
            "type": "boolean", "id": "auto"
        }))
        .unwrap();
        match c {
            ConfigOption::Boolean { current_value, .. } => assert!(!current_value),
            other => panic!("expected boolean, got {other:?}"),
        }
    }
    use crate::acp::rpc::{parse_incoming, Incoming};
    use std::collections::HashMap;

    const HANDSHAKE: &str = include_str!("../../tests/fixtures/acp/handshake.jsonl");
    const PROMPT_SESSION: &str = include_str!("../../tests/fixtures/acp/prompt_session.jsonl");

    /// All `a2c` messages of a fixture, in order.
    fn a2c_msgs(fixture: &str) -> Vec<Value> {
        fixture
            .lines()
            .filter_map(|line| {
                let v: Value = serde_json::from_str(line).ok()?;
                if v["dir"] == "a2c" {
                    Some(v["msg"].clone())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn every_fixture_a2c_line_parses_as_rpc() {
        for fixture in [HANDSHAKE, PROMPT_SESSION] {
            for msg in a2c_msgs(fixture) {
                let line = serde_json::to_string(&msg).unwrap();
                let parsed = parse_incoming(&line);
                assert!(
                    parsed.is_ok(),
                    "a2c line must parse, method = {}",
                    msg["method"].as_str().unwrap_or("<response>")
                );
            }
        }
    }

    #[test]
    fn every_fixture_session_update_is_a_known_variant() {
        for fixture in [HANDSHAKE, PROMPT_SESSION] {
            for msg in a2c_msgs(fixture) {
                if msg["method"] != "session/update" {
                    continue;
                }
                let n: SessionNotification = serde_json::from_value(msg["params"].clone()).unwrap();
                assert!(
                    !matches!(n.update, SessionUpdate::Unknown),
                    "update kind must be known: {}",
                    msg["params"]["update"]["sessionUpdate"]
                );
            }
        }
    }

    #[test]
    fn handshake_initialize_result() {
        let msgs = a2c_msgs(HANDSHAKE);
        let init = msgs
            .iter()
            .find(|m| m["id"] == 1 && m.get("result").is_some())
            .expect("initialize response");
        let r: InitializeResult = serde_json::from_value(init["result"].clone()).unwrap();
        assert_eq!(r.protocol_version, 1);
        assert!(r.agent_capabilities.load_session);
        assert!(r.agent_capabilities.session_capabilities.list.is_some());
        assert!(r.agent_capabilities.session_capabilities.close.is_some());
        assert!(
            r.agent_capabilities.prompt_capabilities.image,
            "Vibe takes PNG blocks"
        );
        assert_eq!(r.auth_methods.len(), 1);
        assert_eq!(r.auth_methods[0].id, "browser-auth");
        let info = r.agent_info.unwrap();
        assert_eq!(info.name, "@mistralai/mistral-vibe");
        assert_eq!(info.version, "3.1.0");
    }

    #[test]
    fn handshake_session_new_modes_and_config_options() {
        let msgs = a2c_msgs(HANDSHAKE);
        let new = msgs
            .iter()
            .find(|m| m["id"] == 2 && m.get("result").is_some())
            .expect("session/new response");
        let r: NewSessionResult = serde_json::from_value(new["result"].clone()).unwrap();
        assert!(!r.session_id.is_empty());
        let modes = r.modes.expect("modes present");
        assert_eq!(modes.current_mode_id, "accept-edits");
        assert_eq!(modes.available_modes.len(), 5);
        assert!(modes.available_modes.iter().any(|m| m.id == "ask"));
        fn id(o: &ConfigOption) -> Option<&str> {
            match o {
                ConfigOption::Select { id, .. } | ConfigOption::Boolean { id, .. } => {
                    Some(id.as_str())
                }
                ConfigOption::Unknown => None,
            }
        }
        let ids: Vec<Option<&str>> = r.config_options.iter().map(id).collect();
        for want in ["mode", "model", "thinking"] {
            assert!(ids.contains(&Some(want)), "config option {want} missing");
        }
    }

    #[test]
    fn prompt_session_permission_plan_usage_and_result() {
        let msgs = a2c_msgs(PROMPT_SESSION);
        let mut tool_calls: HashMap<String, ToolCall> = HashMap::new();
        let mut permission: Option<(Value, PermissionRequest)> = None;
        let mut plans: Vec<Plan> = Vec::new();
        let mut prompt_result: Option<PromptResult> = None;
        let mut usage_sizes: Vec<u64> = Vec::new();
        for msg in &msgs {
            let line = serde_json::to_string(msg).unwrap();
            match parse_incoming(&line).unwrap() {
                Incoming::Request { id, method, params }
                    if method == "session/request_permission" =>
                {
                    assert!(
                        permission.is_none(),
                        "exactly one permission request expected"
                    );
                    let req = serde_json::from_value(params).unwrap();
                    permission = Some((id, req));
                }
                Incoming::Notification { method, params } if method == "session/update" => {
                    let n: SessionNotification = serde_json::from_value(params).unwrap();
                    match n.update {
                        SessionUpdate::ToolCall(tc) => {
                            tool_calls.insert(tc.tool_call_id.clone(), tc);
                        }
                        SessionUpdate::ToolCallUpdate(upd) => {
                            if let Some(tc) = tool_calls.get_mut(&upd.tool_call_id) {
                                tc.apply(&upd);
                            }
                        }
                        SessionUpdate::Plan(p) => plans.push(p),
                        SessionUpdate::UsageUpdate(u) => usage_sizes.push(u.size),
                        _ => {}
                    }
                }
                Incoming::Response {
                    id: 4,
                    result: Ok(v),
                } => {
                    prompt_result = serde_json::from_value(v).ok();
                }
                _ => {}
            }
        }

        // exactly one permission request with 4 options, id 0
        let (req_id, req) = permission.expect("one session/request_permission");
        assert_eq!(req_id, json!(0));
        assert_eq!(req.options.len(), 4);
        assert_eq!(req.options[0].option_id, "allow_once");
        assert_eq!(req.options[0].kind, PermissionKind::AllowOnce);
        assert_eq!(req.options[3].kind, PermissionKind::RejectOnce);

        // its toolCallId belongs to an earlier tool_call containing a Diff
        let tc = tool_calls
            .get(&req.tool_call.tool_call_id)
            .expect("permission toolCallId must match an earlier tool_call");
        let has_diff = tc.content.iter().any(
            |c| matches!(c, ToolCallContent::Diff { new_text, .. } if new_text == "Hallo Welt"),
        );
        assert!(has_diff, "tool call must contain the write_file diff");

        // at least one plan with exactly 2 entries
        assert!(plans.iter().any(|p| p.entries.len() == 2));
        assert!(plans
            .iter()
            .any(|p| p.entries.iter().any(|e| e.status == PlanStatus::Completed)));

        // session/prompt resolved with end_turn and the observed usage
        let pr = prompt_result.expect("session/prompt result");
        assert_eq!(pr.stop_reason, StopReason::EndTurn);
        let usage = pr.usage.expect("usage present");
        assert_eq!(usage.output_tokens, 898);
        assert_eq!(usage.total_tokens, 47147);
        assert_eq!(usage.cached_read_tokens, Some(38_144));

        // a usage_update with size == 800000 (context window)
        assert!(usage_sizes.contains(&800_000));
    }

    #[test]
    fn prompt_session_session_list_entry() {
        let msgs = a2c_msgs(PROMPT_SESSION);
        let list = msgs
            .iter()
            .find(|m| m["id"] == 5 && m.get("result").is_some())
            .expect("session/list response");
        let r: ListSessionsResult = serde_json::from_value(list["result"].clone()).unwrap();
        assert_eq!(r.sessions.len(), 1);
        assert_eq!(r.sessions[0].cwd, "/tmp/acp-fixture/ws");
        assert!(r.sessions[0].title.is_some());
        assert!(r.sessions[0].updated_at.is_some());
        assert!(r.next_cursor.is_none());
    }

    #[test]
    fn unknown_tags_are_tolerated() {
        let u: SessionUpdate =
            serde_json::from_value(json!({"sessionUpdate": "plan_update", "plan": {}})).unwrap();
        assert!(matches!(u, SessionUpdate::Unknown));

        let b: ContentBlock =
            serde_json::from_value(json!({"type": "audio", "data": "", "mimeType": "audio/wav"}))
                .unwrap();
        assert!(matches!(b, ContentBlock::Unknown));

        let k: ToolKind = serde_json::from_value(json!("weird")).unwrap();
        assert_eq!(k, ToolKind::Other);
        let s: ToolStatus = serde_json::from_value(json!("queued")).unwrap();
        assert_eq!(s, ToolStatus::Other);
        let p: PermissionKind = serde_json::from_value(json!("maybe")).unwrap();
        assert_eq!(p, PermissionKind::Other);
        let c: ConfigOption = serde_json::from_value(json!({"type": "slider", "id": "x"})).unwrap();
        assert!(matches!(c, ConfigOption::Unknown));
        let r: StopReason = serde_json::from_value(json!("weird_stop")).unwrap();
        assert_eq!(r, StopReason::Other);
    }

    #[test]
    fn content_block_as_text() {
        let b: ContentBlock =
            serde_json::from_value(json!({"type": "text", "text": "hi"})).unwrap();
        assert_eq!(b.as_text(), Some("hi"));
        let img: ContentBlock = serde_json::from_value(
            json!({"type": "image", "data": "AAA", "mimeType": "image/png"}),
        )
        .unwrap();
        assert_eq!(img.as_text(), None);
    }

    #[test]
    fn tool_call_apply_only_present_fields() {
        let mut tc = ToolCall {
            tool_call_id: "t1".to_string(),
            title: "alt".to_string(),
            kind: ToolKind::Read,
            status: ToolStatus::Pending,
            content: vec![ToolCallContent::Terminal {
                terminal_id: "term-1".to_string(),
            }],
            locations: vec![Location {
                path: "/a".to_string(),
                line: Some(3),
            }],
            raw_input: None,
            raw_output: None,
            meta: None,
            first_seen: 0,
            last_update: 0,
            size_hint: 0,
        };
        // only status present: everything else stays
        let upd: ToolCallUpdate =
            serde_json::from_value(json!({"toolCallId": "t1", "status": "completed"})).unwrap();
        tc.apply(&upd);
        assert_eq!(tc.status, ToolStatus::Completed);
        assert_eq!(tc.title, "alt");
        assert_eq!(tc.kind, ToolKind::Read);
        assert_eq!(tc.content.len(), 1);
        assert_eq!(tc.locations.len(), 1);
        assert!(tc.raw_input.is_none());

        // content present: the list is replaced
        let upd2: ToolCallUpdate = serde_json::from_value(
            json!({"toolCallId": "t1", "content": [{"type": "diff", "path": "/p", "newText": "n"}]}),
        )
        .unwrap();
        tc.apply(&upd2);
        assert_eq!(tc.content.len(), 1);
        assert!(
            matches!(&tc.content[0], ToolCallContent::Diff { new_text, old_text, .. }
                if new_text == "n" && old_text.is_none())
        );
        assert_eq!(
            tc.status,
            ToolStatus::Completed,
            "status untouched by second update"
        );

        // raw_output present is taken over (JSON null maps to None for Option)
        let upd3: ToolCallUpdate =
            serde_json::from_value(json!({"toolCallId": "t1", "rawOutput": {"stdout": "x"}}))
                .unwrap();
        tc.apply(&upd3);
        assert_eq!(tc.raw_output, Some(json!({"stdout": "x"})));
    }

    #[test]
    fn outgoing_params_builders() {
        let init = initialize_params("catwright", "0.1.0");
        assert_eq!(init["protocolVersion"], json!(1));
        assert_eq!(
            init["clientCapabilities"]["fs"]["readTextFile"],
            json!(false)
        );
        assert_eq!(init["clientCapabilities"]["terminal"], json!(false));
        assert_eq!(init["clientInfo"]["name"], json!("catwright"));

        let p = Path::new("/tmp/ws");
        let new = new_session_params(p);
        assert_eq!(new["cwd"], json!("/tmp/ws"));
        assert_eq!(new["mcpServers"], json!([]));
        let load = load_session_params(p, "sid");
        assert_eq!(load["sessionId"], json!("sid"));
        assert_eq!(load["mcpServers"], json!([]));

        assert_eq!(list_sessions_params(None, None), json!({}));
        assert_eq!(
            list_sessions_params(Some(p), Some("cur")),
            json!({"cwd": "/tmp/ws", "cursor": "cur"})
        );
        assert_eq!(
            list_sessions_params(Some(p), None),
            json!({"cwd": "/tmp/ws"})
        );

        let blocks = vec![
            PromptBlock::Text("Hallo".to_string()),
            PromptBlock::Image {
                data_base64: "AAA".to_string(),
                mime_type: "image/png".to_string(),
            },
            PromptBlock::ResourceLink {
                uri: "file:///x".to_string(),
                name: "x".to_string(),
            },
        ];
        let pr = prompt_params("sid", &blocks);
        assert_eq!(pr["sessionId"], json!("sid"));
        assert_eq!(pr["prompt"][0], json!({"type": "text", "text": "Hallo"}));
        assert_eq!(
            pr["prompt"][1],
            json!({"type": "image", "data": "AAA", "mimeType": "image/png"})
        );
        assert_eq!(
            pr["prompt"][2],
            json!({"type": "resource_link", "uri": "file:///x", "name": "x"})
        );

        assert_eq!(session_id_params("s"), json!({"sessionId": "s"}));
        assert_eq!(
            set_mode_params("s", "ask"),
            json!({"sessionId": "s", "modeId": "ask"})
        );
        assert_eq!(
            set_config_option_params("s", "model", "mistral-medium-3.5"),
            json!({"sessionId": "s", "configId": "model", "value": "mistral-medium-3.5"})
        );
        assert_eq!(
            permission_outcome(Some("allow_once")),
            json!({"outcome": {"outcome": "selected", "optionId": "allow_once"}})
        );
        assert_eq!(
            permission_outcome(None),
            json!({"outcome": {"outcome": "cancelled"}})
        );
    }

    /// The retention bound of raw values (audit N1): only oversized raws
    /// become the marker object; in-bound and absent ones are untouched.
    #[test]
    fn oversized_raw_values_become_markers() {
        let huge = "x".repeat(MAX_TOOL_RAW_BYTES + 1);
        let mut tc = ToolCall {
            tool_call_id: "t1".to_string(),
            title: "T".to_string(),
            kind: ToolKind::Other,
            status: ToolStatus::Completed,
            content: Vec::new(),
            locations: Vec::new(),
            raw_input: None,
            raw_output: None,
            meta: None,
            first_seen: 0,
            last_update: 0,
            size_hint: 0,
        };
        // an update's oversized output is capped by the marker object
        let upd: ToolCallUpdate =
            serde_json::from_value(json!({"toolCallId": "t1", "rawOutput": {"blob": huge}}))
                .unwrap();
        tc.apply(&upd);
        assert_eq!(
            tc.raw_output,
            Some(json!({"trimmed": true, "reason": "larger than the retention bound"}))
        );
        // the first takeover is capped too: an oversized input through the
        // bounded copy comes back as the same marker
        tc.raw_input = Some(json!({"blob": huge}));
        let bounded = tc.bounded_clone();
        assert_eq!(bounded.raw_input, tc.raw_output);
        // an in-bound raw is kept as is, an absent one stays absent
        tc.raw_input = Some(json!({"small": 1}));
        let kept = tc.bounded_clone();
        assert_eq!(kept.raw_input, Some(json!({"small": 1})));
        assert_eq!(kept.raw_output, tc.raw_output);
    }
}
