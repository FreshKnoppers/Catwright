//! egui rendering of transcripts: markdown blocks and the generative-UI DSL
//! (```ui fenced JSON blocks render as native widgets).

use crate::i18n::{self, tr, trf};
use crate::icons::{self, Icon};
use crate::markdown::{MdBlock, MdInline};
use crate::model::{parse_ui_blocks, UiEvent, UiNode};
use crate::script::{ActionOutcome, ScriptTool};
use crate::theme;
use crate::widgets::{line_galley, paint_line};
use egui::{Color32, RichText};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub const ACCENT: Color32 = crate::theme::ORANGE;
pub const CODE_BG: Color32 = crate::theme::CARD;
pub const TEXT: Color32 = crate::theme::TEXT;
pub const DIM: Color32 = crate::theme::DIM;
pub const LINK: Color32 = crate::theme::AMBER;
/// Inline code spans: a calm warm tint on a quiet box. The box shows where
/// the code starts and ends (a trailing dot is not part of a command), so
/// the colour can stay calm and a function name is never the loudest word
/// of a sentence.
const CODE_TEXT: Color32 = Color32::from_rgb(0xe3, 0xd4, 0xc2);
const CODE_SPAN_BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x2b);
/// File chips keep the warmer tint: they are the clickable ones.
const CHIP_TEXT: Color32 = Color32::from_rgb(0xff, 0xd9, 0xa0);

/// True only for link schemes we are willing to open from a model answer:
/// http://, https:// and mailto: (case-insensitive, after trim). Everything
/// else (javascript:, file:, data:, …) is rendered as plain text.
pub fn is_safe_link(url: &str) -> bool {
    let u = url.trim().to_lowercase();
    u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")
}

/// Mutable render state shared while painting one message.
pub struct RenderCtx<'a> {
    pub ui_events: &'a mut Vec<UiEvent>,
    /// Id of the chat/session whose transcript is painted (`owner` of every
    /// per-interaction state key, see `field_key`): the session id in the
    /// harness world, the chat id in the API-chat world. One view's values
    /// never surface in another (Handoff 2.5).
    pub owner: &'a str,
    /// Input state of the UI blocks, keyed by `field_key`
    /// ("<owner>#<msg_idx>#<block>#<name>").
    pub form_values: &'a mut HashMap<String, String>,
    /// Nodes of the ```ui block currently being painted: `computed` and a
    /// `form` submit read the inputs of their own block from here.
    /// (owned copy: ui blocks are small, and a borrowed slice would tie the
    /// context's lifetime to every temporary parse result)
    pub block_nodes: Vec<UiNode>,
    /// Persistent ```ui-js tools of the view, keyed by `script_key`, each
    /// carrying its own boa context (`owner`-prefixed, Handoff 2.5).
    pub scripts: Option<&'a mut HashMap<String, ScriptTool>>,
    /// Set while the frame paints a slider the pointer is dragging. The
    /// enclosing ui-js block reads it after painting itself to throttle
    /// its re-evaluations (Handoff 2.7); `render_script_tool` resets it
    /// first, so it always speaks for the block just painted.
    pub slider_dragging: bool,
    /// Running count of ```ui / ```ui-js blocks already painted in this
    /// message: every block draws with its own number, so the per-block
    /// state keys (`field_key`, `script_key`) never collide (review
    /// N17). Top-level blocks count from 0; a run nested inside a ui
    /// `text` node counts in its own space past 1_000_000 (final audit
    /// G), so nested content cannot shift the outer numbers — restored
    /// after the nested run, the counter resumes as if the host block
    /// had contained no ui children.
    pub block_offset: usize,
    /// Salt separating scroll-state id paths of nested markdown blocks
    /// from the top-level blocks of the same message (review N16): 0 for
    /// a message's own blocks, the text hash under a ui `text` node, the
    /// file path in the artifacts preview.
    pub scroll_salt: u64,
    /// How many `text` nodes deep the current paint is (a `text` node holds
    /// markdown, which can hold ```ui blocks again). Bounded by
    /// `MAX_TEXT_DEPTH`; a ```ui-js block below depth 0 never runs, so a
    /// script cannot spawn itself (a tool printing its own source as a new
    /// ```ui-js block re-ran until the stack overflowed).
    pub depth: u8,
    /// File chips while painting an agent answer: path-like words and inline
    /// code that name an existing file of the session's workspace render as
    /// clickable chips (click opens the preview pane). `None` outside the
    /// transcript (artifact previews, the chat view).
    pub file_chips: Option<FileChips<'a>>,
}

/// File-chip state of one agent message: the workspace to resolve against
/// and where clicked paths go.
pub struct FileChips<'a> {
    pub cwd: &'a Path,
    /// Absolute paths of clicked chips; the caller drains this after the
    /// message is painted and opens each in the preview pane.
    pub open: &'a mut Vec<String>,
}

/// Deepest nesting of `text` nodes that is still rendered as markdown.
const MAX_TEXT_DEPTH: u8 = 4;

impl<'a> RenderCtx<'a> {
    pub fn new(
        ui_events: &'a mut Vec<UiEvent>,
        form_values: &'a mut HashMap<String, String>,
        owner: &'a str,
    ) -> Self {
        Self {
            ui_events,
            owner,
            form_values,
            block_nodes: Vec::new(),
            scripts: None,
            slider_dragging: false,
            block_offset: 0,
            scroll_salt: 0,
            depth: 0,
            file_chips: None,
        }
    }

    /// Enable ```ui-js tools, keeping their state in `scripts`.
    pub fn with_scripts(mut self, scripts: &'a mut HashMap<String, ScriptTool>) -> Self {
        self.scripts = Some(scripts);
        self
    }
}

/// Paint one ```ui block: its nodes become the block context, so `computed`
/// sees the sliders/fields of the same block.
pub fn render_ui_block(
    ui: &mut egui::Ui,
    nodes: &[UiNode],
    msg_idx: usize,
    block: usize,
    ctx: &mut RenderCtx,
) {
    let outer = std::mem::replace(&mut ctx.block_nodes, nodes.to_vec());
    for n in nodes {
        render_ui_node(ui, n, msg_idx, block, ctx);
    }
    ctx.block_nodes = outer;
}

/// Key of one input inside a UI block: the owning chat/session id first,
/// then message, block index and field name —
/// `"<owner>#<msg_idx>#<block>#<name>"`. The owner prefix keeps the UI
/// state of one chat/session out of another at the same transcript position
/// and makes a dropped owner's entries removable wholesale (Handoff 2.5).
/// Pure, so the rule is unit-tested (`keys_are_owner_scoped`).
pub fn field_key(owner: &str, msg_idx: usize, block: usize, name: &str) -> String {
    format!("{owner}#{msg_idx}#{block}#{name}")
}

/// A ui button/form is armed by its first click; the send needs a second
/// click within this window. The action text is agent-chosen, so one
/// accidental click must not send it in the user's name.
const ARM_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);
/// … and not before this minimum gap after arming, so a *double*-click
/// (two `clicked()` events within egui's double-click time) cannot send
/// either — an injected "double-click to continue" label must not pass.
const MIN_ARM_GAP: std::time::Duration = std::time::Duration::from_millis(400);

/// Armed ui buttons, by widget id. Bounded: a wholesale clear at the cap
/// (the ids are per-frame stable, so old entries expire on their next
/// check anyway).
fn armed_buttons() -> &'static std::sync::Mutex<HashMap<egui::Id, std::time::Instant>> {
    static ARMED: std::sync::OnceLock<std::sync::Mutex<HashMap<egui::Id, std::time::Instant>>> =
        std::sync::OnceLock::new();
    ARMED.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Is `id` armed AND past the minimum gap (so the second click counts)?
/// An expired arm is dropped here.
fn button_armed(id: egui::Id) -> bool {
    let mut m = armed_buttons().lock().unwrap();
    match m.get(&id) {
        Some(t) if t.elapsed() <= ARM_WINDOW => t.elapsed() >= MIN_ARM_GAP,
        _ => {
            m.remove(&id);
            false
        }
    }
}

fn arm_button(id: egui::Id) {
    let mut m = armed_buttons().lock().unwrap();
    if m.len() > 1024 {
        m.clear();
    }
    m.insert(id, std::time::Instant::now());
}

fn disarm_button(id: egui::Id) {
    armed_buttons().lock().unwrap().remove(&id);
}

/// The shared prefix of every per-interaction key of `owner`:
/// `"<owner>#"` (see `field_key` / `script_key`).
pub fn owner_key_prefix(owner: &str) -> String {
    format!("{owner}#")
}

/// Drop every entry that belongs to `owner` from a per-interaction state
/// map (its keys start with `owner_key_prefix`). Called when a session /
/// chat goes away, so its form inputs and ui-js tools (their boa contexts)
/// cannot leak into another view or grow the map without bound
/// (Handoff 2.5).
pub fn drop_owner_entries<V>(map: &mut HashMap<String, V>, owner: &str) {
    let prefix = owner_key_prefix(owner);
    map.retain(|k, _| !k.starts_with(&prefix));
}

/// The values of all slider/number/select inputs of one block, as numbers
/// (selects contribute their numeric option, everything else 0).
/// All nodes of a block, including those nested in `card`/`columns`
/// (inputs inside a card must still feed the block's formulas).
fn flatten_nodes(nodes: &[UiNode]) -> Vec<&UiNode> {
    let mut out = Vec::new();
    for n in nodes {
        out.push(n);
        if let UiNode::Card { children, .. } | UiNode::Columns { children } = n {
            out.extend(flatten_nodes(children));
        }
    }
    out
}

fn block_vars(
    ctx: &RenderCtx,
    msg_idx: usize,
    block: usize,
    nodes: &[UiNode],
) -> HashMap<String, f64> {
    let mut vars = HashMap::new();
    for n in flatten_nodes(nodes) {
        let (name, value) = match n {
            UiNode::Slider { name, value, .. } => {
                let v = ctx
                    .form_values
                    .get(&field_key(ctx.owner, msg_idx, block, name))
                    .and_then(|s| s.parse::<f64>().ok())
                    .or(*value)
                    .unwrap_or(0.0);
                (name, v)
            }
            UiNode::Number { name, value, .. } => {
                let v = ctx
                    .form_values
                    .get(&field_key(ctx.owner, msg_idx, block, name))
                    .and_then(|s| s.parse::<f64>().ok())
                    .or(*value)
                    .unwrap_or(0.0);
                (name, v)
            }
            UiNode::Select {
                name,
                options,
                value,
                ..
            } => {
                let current = ctx
                    .form_values
                    .get(&field_key(ctx.owner, msg_idx, block, name))
                    .cloned()
                    .or_else(|| value.clone())
                    .unwrap_or_default();
                let v = current.trim().parse::<f64>().unwrap_or(0.0);
                let _ = options;
                (name, v)
            }
            UiNode::Toggle { name, value, .. } => {
                let on = ctx
                    .form_values
                    .get(&field_key(ctx.owner, msg_idx, block, name))
                    .map(|s| s == "true")
                    .or(*value)
                    .unwrap_or(false);
                (name, if on { 1.0 } else { 0.0 })
            }
            _ => continue,
        };
        vars.insert(name.clone(), value);
    }
    vars
}

/// The checkbox states of the block's checklists as one array of bools
/// under the name `checklist`, with the next form submit of the same
/// block — SKILL.md promises "their values are sent with the next form
/// submit of the same block" without naming a concrete shape, so the
/// block's states travel as one `true`/`false` array, consistent in both
/// submit paths (review M9). The value rule is the checklist painter's:
/// the stored `check:{i}` entry wins over the item's default. The second
/// return says whether the user changed any of them (the `changed_inputs`
/// gate); the whole array is sent, not just the toggled item, so a
/// consumer can read it as the checklist's state.
fn checklist_values(
    ctx: &RenderCtx,
    msg_idx: usize,
    block: usize,
    nodes: &[UiNode],
) -> Option<(serde_json::Value, bool)> {
    let mut states: Option<Vec<bool>> = None;
    let mut touched = false;
    for n in flatten_nodes(nodes) {
        let UiNode::Checklist { items } = n else {
            continue;
        };
        let out = states.get_or_insert_with(Vec::new);
        for (i, item) in items.iter().enumerate() {
            let key = field_key(ctx.owner, msg_idx, block, &format!("check:{i}"));
            match ctx.form_values.get(&key) {
                // a stored entry (whatever it says) marks the item as
                // the user's own change
                Some(s) => {
                    touched = true;
                    out.push(s == "true");
                }
                None => out.push(item.done),
            }
        }
    }
    states.map(|states| {
        (
            serde_json::Value::Array(states.into_iter().map(serde_json::Value::Bool).collect()),
            touched,
        )
    })
}

/// The inputs of one block as JSON values (for a form submit of that
/// block).
fn block_inputs(
    ctx: &RenderCtx,
    msg_idx: usize,
    block: usize,
    nodes: &[UiNode],
) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    for n in flatten_nodes(nodes) {
        match n {
            UiNode::Slider { name, value, .. } | UiNode::Number { name, value, .. } => {
                let v = ctx
                    .form_values
                    .get(&field_key(ctx.owner, msg_idx, block, name))
                    .and_then(|s| s.parse::<f64>().ok())
                    .or(*value)
                    .unwrap_or(0.0);
                out.push((name.clone(), serde_json::json!(v)));
            }
            UiNode::Select { name, value, .. } => {
                let v = ctx
                    .form_values
                    .get(&field_key(ctx.owner, msg_idx, block, name))
                    .cloned()
                    .or_else(|| value.clone())
                    .unwrap_or_default();
                out.push((name.clone(), serde_json::Value::String(v)));
            }
            UiNode::Toggle { name, value, .. } => {
                let on = ctx
                    .form_values
                    .get(&field_key(ctx.owner, msg_idx, block, name))
                    .map(|s| s == "true")
                    .or(*value)
                    .unwrap_or(false);
                out.push((name.clone(), serde_json::Value::Bool(on)));
            }
            UiNode::TextInput { name, value, .. } => {
                let v = ctx
                    .form_values
                    .get(&field_key(ctx.owner, msg_idx, block, name))
                    .cloned()
                    .or_else(|| value.clone())
                    .unwrap_or_default();
                out.push((name.clone(), serde_json::Value::String(v)));
            }
            _ => {}
        }
    }
    // the checklist state travels with the same submit (review M9)
    if let Some((value, _)) = checklist_values(ctx, msg_idx, block, nodes) {
        out.push(("checklist".to_string(), value));
    }
    out
}

/// Inputs the user actually changed in this block (present in form_values),
/// typed for a script tool's state: numbers, bools and strings. A touched
/// checklist contributes its full state array (`checklist`, same shape as
/// the form submit of the block).
fn changed_inputs(
    ctx: &RenderCtx,
    msg_idx: usize,
    block: usize,
    nodes: &[UiNode],
) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    for n in flatten_nodes(nodes) {
        let (name, numeric, boolean) = match n {
            UiNode::Slider { name, .. } | UiNode::Number { name, .. } => (name, true, false),
            UiNode::Toggle { name, .. } => (name, false, true),
            UiNode::Select { name, .. } | UiNode::TextInput { name, .. } => (name, false, false),
            _ => continue,
        };
        let Some(raw) = ctx
            .form_values
            .get(&field_key(ctx.owner, msg_idx, block, name))
        else {
            continue;
        };
        let value = if numeric {
            match raw.trim().replace(',', ".").parse::<f64>() {
                Ok(f) => serde_json::json!(f),
                Err(_) => continue, // half-typed number: keep the old state
            }
        } else if boolean {
            serde_json::Value::Bool(raw == "true")
        } else {
            serde_json::Value::String(raw.clone())
        };
        out.push((name.clone(), value));
    }
    if let Some((value, true)) = checklist_values(ctx, msg_idx, block, nodes) {
        out.push(("checklist".to_string(), value));
    }
    out
}

/// Stable cache key for a ui-js tool: the owning chat/session id, the
/// position in the view and a hash of the code (`owner`-prefixed like
/// `field_key`, so one owner's tools are scoped and droppable, Handoff 2.5).
pub fn script_key(owner: &str, msg_idx: usize, block: usize, code: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut h);
    format!("{owner}#{msg_idx}#{block}#{:x}", h.finish())
}

/// A ```ui-js block: run the script tool (sandboxed boa), paint its nodes,
/// feed input changes back into its state, route button clicks to
/// `onAction` (local) or to the agent.
fn render_script_tool(
    ui: &mut egui::Ui,
    code: &str,
    closed: bool,
    msg_idx: usize,
    block: usize,
    ctx: &mut RenderCtx,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("■ TOOL")
                .monospace()
                .size(11.5)
                .color(crate::theme::ORANGE),
        );
        ui.label(
            RichText::new(tr("runs locally, without network"))
                .color(DIM)
                .small(),
        );
    });
    if ctx.depth > 0 {
        // a script inside another node's output (a ```ui-js fence in a
        // `text` node): shown, never run — no tool can start another one
        ui.label(
            RichText::new(tr("Nested tools do not run."))
                .color(DIM)
                .small(),
        );
        render_code_block(
            ui,
            egui::Id::new((ctx.owner, msg_idx, block, "ui-js-nested")),
            "ui-js",
            code,
        );
        return;
    }
    if !closed {
        // the answer is still streaming: never run half a script
        ui.label(RichText::new(tr("Loading tool…")).color(DIM).small());
        return;
    }
    let key = script_key(ctx.owner, msg_idx, block, code);
    /// How many ```ui-js tools the harness keeps alive at once (flood
    /// bound): each holds a source + state and every evaluation spawns a
    /// worker process, so an agent message with N blocks must not create
    /// N live tools. Over the cap the block renders as plain source.
    const MAX_UI_JS_TOOLS: usize = 64;
    // take the tool out of the map while painting (the paint needs &mut ctx)
    let tool = ctx.scripts.as_deref_mut().and_then(|m| {
        if let Some(t) = m.remove(&key) {
            Some(t)
        } else if m.len() < MAX_UI_JS_TOOLS {
            Some(ScriptTool::new(code))
        } else {
            None
        }
    });
    let Some(mut tool) = tool else {
        render_code_block(
            ui,
            egui::Id::new((ctx.owner, msg_idx, block, "ui-js")),
            "ui-js",
            code,
        );
        return;
    };
    let result = tool.nodes().clone();
    // did the throttle skip a dirty evaluation? then come back when the
    // gap allows it, so a dragged slider's final value is never stale
    let throttled = tool.eval_pending();
    // the flag speaks for this block only: measure it from scratch
    ctx.slider_dragging = false;
    match result {
        Ok(nodes) => {
            if throttled {
                ui.ctx().request_repaint();
            }
            let before = ctx.ui_events.len();
            render_ui_block(ui, &nodes, msg_idx, block, ctx);
            // the ui-js block decides over this tool's next evaluation
            tool.set_dragging(ctx.slider_dragging);
            // clicks of this tool: onAction first, the agent only if unhandled
            let new_events: Vec<UiEvent> = ctx.ui_events.drain(before..).collect();
            let mut state_reset = false;
            for e in new_events {
                match e {
                    UiEvent::Button { action } => match tool.on_action(&action) {
                        ActionOutcome::Local => state_reset = true,
                        ActionOutcome::SendToAgent => {
                            ctx.ui_events.push(UiEvent::Button { action })
                        }
                    },
                    UiEvent::Form { submit, mut values } => {
                        if let serde_json::Value::Object(map) = &mut values {
                            map.insert("state".into(), tool.state().clone());
                        }
                        ctx.ui_events.push(UiEvent::Form { submit, values });
                    }
                }
            }
            if state_reset {
                // the script set a new state: inputs re-seed from it
                // (every key of this owner's block shares the prefix)
                let prefix = field_key(ctx.owner, msg_idx, block, "");
                ctx.form_values.retain(|k, _| !k.starts_with(&prefix));
            } else {
                let before_state = tool.state().clone();
                for (name, value) in changed_inputs(ctx, msg_idx, block, &nodes) {
                    tool.set_value(&name, value);
                }
                if tool.state() != &before_state {
                    ui.ctx().request_repaint();
                }
            }
            if state_reset {
                ui.ctx().request_repaint();
            }
        }
        Err(e) => {
            egui::Frame::default()
                .fill(crate::theme::CARD)
                .stroke(egui::Stroke::new(1.0_f32, crate::theme::RED))
                .rounding(egui::Rounding::same(4.0))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.label(RichText::new(&e).color(crate::theme::RED).size(13.0));
                    egui::CollapsingHeader::new(crate::theme::mono_label(tr("Source")))
                        .id_salt(("tool_src", msg_idx, block))
                        .show(ui, |ui| {
                            ui.label(RichText::new(code).monospace().size(12.0).color(TEXT));
                        });
                });
        }
    }
    if let Some(m) = ctx.scripts.as_deref_mut() {
        m.insert(key, tool);
    }
}

/// A slider's range in order: the model may send `min > max` (or a script
/// may compute it), and `f64::clamp` panics on that. Non-finite bounds fall
/// back to 0 so the slider always has a usable range.
fn slider_bounds(min: f64, max: f64) -> (f64, f64) {
    let min = if min.is_finite() { min } else { 0.0 };
    let max = if max.is_finite() { max } else { 0.0 };
    (min.min(max), min.max(max))
}

/// Narrowest column a `columns` node lays out side by side.
const MIN_COLUMN_WIDTH: f32 = 120.0;

/// How many children of a `columns` node share one row: all of them when
/// they fit at `MIN_COLUMN_WIDTH`, else as many as fit (at least one).
fn columns_per_row(children: usize, available: f32) -> usize {
    let fit = (available / MIN_COLUMN_WIDTH).floor().max(1.0) as usize;
    children.clamp(1, fit.max(1))
}

/// Angle where a pie chart starts: 12 o'clock.
const PIE_START: f32 = -std::f32::consts::FRAC_PI_2;

/// The slices of a pie chart that have a share: `(slice index, start
/// angle, sweep)`, clockwise from 12 o'clock. `fracs` are the slices'
/// shares of the whole (summing to 1).
fn pie_slices(fracs: &[f32]) -> Vec<(usize, f32, f32)> {
    let mut out = Vec::new();
    let mut angle = PIE_START;
    for (i, frac) in fracs.iter().enumerate() {
        let sweep = frac.max(0.0) * std::f32::consts::TAU;
        if sweep > 0.0 {
            out.push((i, angle, sweep));
            angle += sweep;
        }
    }
    out
}

/// The angles where one slice ends and the next begins (one separator line
/// each); none for a pie with fewer than two slices that have a share.
fn pie_boundaries(fracs: &[f32]) -> Vec<f32> {
    let shares: Vec<f32> = fracs.iter().copied().filter(|f| *f > 0.0).collect();
    if shares.len() < 2 {
        return Vec::new();
    }
    let mut angle = PIE_START;
    let mut out = vec![angle];
    for f in &shares[..shares.len() - 1] {
        angle += f * std::f32::consts::TAU;
        out.push(angle);
    }
    out
}

/// Fractional digits of a slider `step`, used to format the value it shows:
/// an integer step renders without decimals (`1` → "5", not "5.00"),
/// `0.5` → 1 digit, `0.05` → 2 digits; capped for steps with float noise.
fn step_decimals(step: f64) -> usize {
    let frac = format!("{step}")
        .split_once('.')
        .map_or(0, |(_, f)| f.len());
    frac.min(6)
}

/// Format a computed value: `decimals` digits (default 2) in the notation of
/// `lang` (English: thousands commas, decimal point; German, Spanish, Italian,
/// Portuguese: thousands dots, decimal comma; French: thousands no-break
/// spaces, so a number never wraps, decimal comma), plus an optional unit.
fn format_computed(
    lang: i18n::Lang,
    value: f64,
    decimals: Option<u8>,
    unit: Option<&str>,
) -> String {
    let d = decimals.unwrap_or(2).min(10) as usize;
    let s = format!("{value:.d$}");
    let (int_part, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s.as_str(), None),
    };
    let (group, point) = match lang {
        i18n::Lang::En => (',', '.'),
        i18n::Lang::De => ('.', ','),
        i18n::Lang::Fr => ('\u{a0}', ','),
        i18n::Lang::Es | i18n::Lang::It | i18n::Lang::Pt => ('.', ','),
    };
    let mut out = group_thousands(int_part, group);
    if let Some(f) = frac {
        out.push(point);
        out.push_str(f);
    }
    match unit {
        Some(u) if !u.trim().is_empty() => {
            out.push(' ');
            out.push_str(u);
        }
        _ => {}
    }
    out
}

/// Thousands separators (`sep`) for the integer part of a fixed-point
/// number: "22645" → "22.645", "-1234" → "-1.234", "999" stays "999".
/// Anything that is not plain digits ("NaN", "inf") passes through unchanged.
fn group_thousands(int_part: &str, sep: char) -> String {
    if int_part.is_empty() || !int_part.bytes().all(|b| b == b'-' || b.is_ascii_digit()) {
        return int_part.to_string();
    }
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", int_part),
    };
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    format!("{sign}{out}")
}

/// Colour of series `i` of a chart: `theme::CHART_SERIES` in its fixed
/// order (neighbouring series in clearly different hues).
fn series_color(i: usize) -> Color32 {
    crate::theme::CHART_SERIES[i % crate::theme::CHART_SERIES.len()]
}

/// The parsed nodes of a ```ui fence, cached by its content: the same memo
/// pattern as `markdown::parse_cached` — the frame asks for the same block
/// again and again, and parsing its json again is per-frame work for
/// nothing (review M10). The code string is the key; a full cache is
/// dropped wholesale (ui blocks are small, the next frame refills it).
fn parse_ui_blocks_cached(code: &str) -> std::sync::Arc<Vec<UiNode>> {
    static CACHE: std::sync::OnceLock<crate::memo::Memo<Vec<UiNode>>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| crate::memo::Memo::new(256))
        .get_or_insert(crate::memo::hash_str(code), || {
            parse_ui_blocks(&format!("```ui\n{code}\n```"))
        })
}

/// Scroll id of one markdown code block: the owning view, the message
/// index, the nesting salt and the block's position — never the content.
/// A content-derived id (the code's length) shared the scroll state
/// between equally long blocks and reset a streamed block's scroll with
/// every chunk (review N16). Pure, so the rule is unit-tested.
fn code_scroll_id(ctx: &RenderCtx, msg_idx: usize, block: usize) -> egui::Id {
    egui::Id::new((ctx.owner, msg_idx, ctx.scroll_salt, block))
}

pub fn render_blocks(ui: &mut egui::Ui, blocks: &[MdBlock], msg_idx: usize, ctx: &mut RenderCtx) {
    // The index of a ```ui block inside this message is `ctx.block_offset`:
    // a running counter at the top level and a per-host-block space
    // under ui `text` nodes, so every block keeps its own field/script
    // keys (N17, final audit G). It advances BEFORE the block's nodes
    // paint, so the block's own number is spent before its children.
    for (block_idx, b) in blocks.iter().enumerate() {
        match b {
            MdBlock::Heading(level, inl) => {
                let size = match level {
                    1 => 22.0,
                    2 => 19.0,
                    3 => 17.0,
                    _ => 15.5,
                };
                ui.add_space(8.0);
                paragraph(
                    ui,
                    inl,
                    InlineStyle {
                        size,
                        bold: true,
                        ..InlineStyle::BODY
                    },
                    ctx.file_chips.as_mut(),
                );
                ui.add_space(4.0);
            }
            MdBlock::Para(inl) => {
                paragraph(ui, inl, InlineStyle::BODY, ctx.file_chips.as_mut());
                ui.add_space(8.0);
            }
            MdBlock::Code { lang, code, closed } => {
                if lang.eq_ignore_ascii_case("ui-js") {
                    let block = ctx.block_offset;
                    ctx.block_offset += 1;
                    render_script_tool(ui, code, *closed, msg_idx, block, ctx);
                } else if lang.eq_ignore_ascii_case("ui") {
                    // generative UI block: render as native widgets, the
                    // json parsed once per content (review M10)
                    let nodes = parse_ui_blocks_cached(code);
                    let block = ctx.block_offset;
                    ctx.block_offset += 1;
                    if nodes.is_empty() {
                        render_code_block(ui, code_scroll_id(ctx, msg_idx, block_idx), lang, code);
                    } else {
                        render_ui_block(ui, &nodes, msg_idx, block, ctx);
                    }
                } else {
                    render_code_block(ui, code_scroll_id(ctx, msg_idx, block_idx), lang, code);
                }
                ui.add_space(8.0);
            }
            MdBlock::List(items) => {
                for item in items {
                    ui.horizontal_top(|ui| {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(14.0, InlineStyle::BODY.size * 1.4),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_center_size(
                                egui::pos2(rect.left() + 4.0, rect.center().y),
                                egui::vec2(5.0, 5.0),
                            ),
                            0.0,
                            ACCENT,
                        );
                        ui.vertical(|ui| {
                            paragraph(ui, item, InlineStyle::BODY, ctx.file_chips.as_mut())
                        });
                    });
                }
                ui.add_space(8.0);
            }
            MdBlock::Quote(inl) => {
                let resp = ui.horizontal_top(|ui| {
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            render_inline_styled(
                                ui,
                                inl,
                                InlineStyle {
                                    size: 14.0,
                                    color: DIM,
                                    ..InlineStyle::BODY
                                },
                                ctx.file_chips.as_mut(),
                            );
                        });
                    });
                });
                let r = resp.response.rect;
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(r.min, egui::vec2(3.0, r.height())),
                    0.0,
                    ACCENT,
                );
                ui.add_space(8.0);
            }
            MdBlock::Table { columns, rows } => {
                let id = egui::Id::new(("md_tbl", msg_idx, block_idx, columns.len(), rows.len()));
                render_table_grid(ui, id, columns, rows, ctx.file_chips.as_mut());
                ui.add_space(10.0);
            }
            MdBlock::Hr => {
                ui.separator();
                ui.add_space(8.0);
            }
        }
    }
}

pub(crate) fn render_code_block(ui: &mut egui::Ui, id: egui::Id, lang: &str, code: &str) {
    egui::Frame::default()
        .fill(CODE_BG)
        .rounding(egui::Rounding::same(4.0))
        .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER_SOFT))
        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // copy button first (right edge), then the language tag in what
            // is left, truncated: a long tag must never widen the block, or
            // the transcript column grows with it for every row below
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let copy =
                        crate::widgets::IconButton::new(crate::icons::Icon::Copy, tr("Copy"))
                            .side(22.0);
                    if ui.add(copy).clicked() {
                        ui.ctx().copy_text(code.to_string());
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(crate::theme::mono_label(if lang.is_empty() {
                                "code"
                            } else {
                                lang
                            }))
                            .truncate(),
                        );
                    });
                });
            });
            ui.add_space(2.0);
            egui::ScrollArea::horizontal()
                .id_salt(id)
                .auto_shrink([false, true])
                .max_width(ui.available_width())
                .show(ui, |ui| {
                    let mut job = egui::text::LayoutJob::default();
                    for (tok, text) in crate::syntax::tokenize(lang, code) {
                        let color = match tok {
                            crate::syntax::Tok::Keyword => crate::theme::BLUE,
                            crate::syntax::Tok::Str => crate::theme::GREEN,
                            crate::syntax::Tok::Comment => crate::theme::DIM,
                            crate::syntax::Tok::Num => crate::theme::AMBER,
                            crate::syntax::Tok::Plain => TEXT,
                        };
                        job.append(
                            text.as_str(),
                            0.0,
                            egui::TextFormat {
                                font_id: crate::widgets::mono(12.5),
                                color,
                                ..Default::default()
                            },
                        );
                    }
                    ui.add(egui::Label::new(job).extend());
                });
        });
}

/// Code block with line numbers (the preview pane): one row per line,
/// horizontal scroll instead of truncation.
pub(crate) fn render_code_block_lines(ui: &mut egui::Ui, id: egui::Id, lang: &str, code: &str) {
    egui::Frame::default()
        .fill(CODE_BG)
        .rounding(egui::Rounding::same(4.0))
        .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER_SOFT))
        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let copy =
                        crate::widgets::IconButton::new(crate::icons::Icon::Copy, tr("Copy"))
                            .side(22.0);
                    if ui.add(copy).clicked() {
                        ui.ctx().copy_text(code.to_string());
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(crate::theme::mono_label(if lang.is_empty() {
                                "code"
                            } else {
                                lang
                            }))
                            .truncate(),
                        );
                    });
                });
            });
            ui.add_space(2.0);
            egui::ScrollArea::horizontal()
                .id_salt(id)
                .auto_shrink([false, true])
                .max_width(ui.available_width())
                .show(ui, |ui| {
                    for (i, line) in code.lines().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!("{:>4}", i + 1))
                                        .monospace()
                                        .size(11.0)
                                        .color(crate::theme::DIM),
                                )
                                .extend(),
                            );
                            ui.add_space(8.0);
                            let mut job = egui::text::LayoutJob::default();
                            for (tok, text) in crate::syntax::tokenize(lang, line) {
                                let color = match tok {
                                    crate::syntax::Tok::Keyword => crate::theme::BLUE,
                                    crate::syntax::Tok::Str => crate::theme::GREEN,
                                    crate::syntax::Tok::Comment => crate::theme::DIM,
                                    crate::syntax::Tok::Num => crate::theme::AMBER,
                                    crate::syntax::Tok::Plain => TEXT,
                                };
                                job.append(
                                    text.as_str(),
                                    0.0,
                                    egui::TextFormat {
                                        font_id: crate::widgets::mono(12.5),
                                        color,
                                        ..Default::default()
                                    },
                                );
                            }
                            ui.add(egui::Label::new(job).extend());
                        });
                    }
                });
        });
}

/// Inner column widths for a table of the given natural widths. Everything
/// fits as measured when it can; otherwise narrow columns keep their width
/// and the wide ones share what is left (water filling), so no column is
/// squeezed to a few letters while another has room. When even the minima
/// do not fit, the table is still never wider than `available`: columns
/// shrink below the minimum and their cells wrap instead.
pub(crate) fn table_widths(
    natural: &[f32],
    available: f32,
    padding: f32,
    minimum: f32,
) -> Vec<f32> {
    let count = natural.len();
    if count == 0 {
        return Vec::new();
    }
    let full: Vec<f32> = natural.iter().map(|w| w.max(minimum)).collect();
    if full.iter().sum::<f32>() + padding * count as f32 <= available {
        return full;
    }
    // too tight for the minima: water filling without a floor, so the total
    // stays inside the space the panel has
    let inner = (available - padding * count as f32).max(0.0);
    let mut widths = vec![0.0_f32; count];
    let mut open: Vec<usize> = (0..count).collect();
    let mut left = inner;
    while !open.is_empty() {
        let share = left / open.len() as f32;
        let (fit, wide): (Vec<usize>, Vec<usize>) =
            open.iter().partition(|&&i| natural[i] <= share);
        if fit.is_empty() {
            for i in wide {
                widths[i] = share;
            }
            break;
        }
        for i in fit {
            widths[i] = natural[i];
            left -= natural[i];
        }
        open = wide;
    }
    widths
}

/// Paint one table: a header row in the small dim style of the generative-ui
/// `table` node and body rows at 14.0, with widths from the content (both
/// paths — `ui` JSON tables and markdown pipe tables — share this painter).
pub(crate) fn render_table_grid(
    ui: &mut egui::Ui,
    id: egui::Id,
    columns: &[Vec<MdInline>],
    rows: &[Vec<Vec<MdInline>>],
    mut chips: Option<&mut FileChips>,
) {
    const PADDING: f32 = 20.0;
    const MINIMUM: f32 = 48.0;
    /// Table caps (flood hardening): one table — a ```ui``` block, a
    /// markdown pipe table or a previewed CSV — is measured cell by cell
    /// and painted wholesale on every frame. A hostile message or a big
    /// artifact file must not put millions of cells in here.
    const MAX_TABLE_COLS: usize = 32;
    const MAX_TABLE_ROWS: usize = 200;
    let count = columns
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if count == 0 {
        return;
    }
    let cols = MAX_TABLE_COLS.min(count);
    let shown_rows = MAX_TABLE_ROWS.min(rows.len());
    let header_st = InlineStyle {
        size: 12.5,
        color: DIM,
        bold: true,
        ..InlineStyle::BODY
    };
    let body_st = InlineStyle {
        size: 14.0,
        color: TEXT,
        ..InlineStyle::BODY
    };
    // natural width of each column, measured on the unwrapped cell text
    let measure = |cell: &[MdInline], st: InlineStyle| {
        let mut job = egui::text::LayoutJob::default();
        append_inline(&mut job, cell, st);
        ui.fonts(|f| f.layout_job(job)).size().x.ceil() + 2.0
    };
    let mut natural = vec![0.0_f32; cols];
    for (i, cell) in columns.iter().take(cols).enumerate() {
        natural[i] = natural[i].max(measure(cell, header_st));
    }
    // measure only the rows that will be painted (and their first `cols`
    // cells): the loop above is the per-frame cost that must stay bounded
    for row in rows.iter().take(shown_rows) {
        for (i, cell) in row.iter().take(cols).enumerate() {
            natural[i] = natural[i].max(measure(cell, body_st));
        }
    }
    let widths = table_widths(&natural, ui.available_width() - 2.0, PADDING, MINIMUM);
    let total: f32 = widths.iter().map(|w| w + PADDING).sum();
    // columns of numbers align right, so digits of the same place line up
    let numeric: Vec<bool> = (0..cols)
        .map(|c| numeric_column(rows.iter().take(shown_rows).map(|r| r.get(c))))
        .collect();

    egui::Frame::none()
        .rounding(egui::Rounding::same(4.0))
        .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER_SOFT))
        .show(ui, |ui| {
            ui.set_max_width(total);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            let mut table_row =
                |ui: &mut egui::Ui, cells: &[Vec<MdInline>], st: InlineStyle, index: usize| {
                    let bg = ui.painter().add(egui::Shape::Noop);
                    let row = ui.horizontal_top(|ui| {
                        for (col, width) in widths.iter().enumerate() {
                            let frame = egui::Frame::none()
                                .inner_margin(egui::Margin::symmetric(PADDING / 2.0, 5.0));
                            ui.allocate_ui_with_layout(
                                egui::vec2(width + PADDING, 0.0),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| {
                                    ui.set_min_width(width + PADDING);
                                    frame.show(ui, |ui| {
                                        ui.set_width(*width);
                                        let Some(cell) = cells.get(col) else {
                                            return;
                                        };
                                        if numeric.get(col).copied().unwrap_or(false) {
                                            let mut job = egui::text::LayoutJob::default();
                                            append_inline(&mut job, cell, st);
                                            ui.with_layout(
                                                egui::Layout::top_down(egui::Align::RIGHT),
                                                |ui| ui.add(egui::Label::new(job)),
                                            );
                                        } else {
                                            render_table_cell(ui, cell, st, chips.as_deref_mut());
                                        }
                                    });
                                },
                            );
                        }
                    });
                    let rect = row.response.rect;
                    let fill = if index == 0 {
                        crate::theme::RAISED
                    } else {
                        crate::theme::CARD.gamma_multiply(if index.is_multiple_of(2) {
                            1.0
                        } else {
                            0.4
                        })
                    };
                    ui.painter()
                        .set(bg, egui::Shape::rect_filled(rect, 0.0, fill));
                };
            let _ = id;
            table_row(ui, columns.get(..cols).unwrap_or(columns), header_st, 0);
            for (i, row) in rows.iter().take(shown_rows).enumerate() {
                table_row(ui, row, body_st, i + 1);
            }
        });
    let more_rows = rows.len() - shown_rows;
    let more_cols = count - cols;
    if more_rows > 0 || more_cols > 0 {
        ui.add_space(2.0);
        ui.label(
            RichText::new(trf(
                "Table truncated: {rows} more rows, {cols} more columns not shown.",
                &[("rows", &more_rows), ("cols", &more_cols)],
            ))
            .size(12.0)
            .color(DIM),
        );
    }
}

/// One grid cell: a horizontal run of inline markdown, with zero item
/// spacing so several snippets look like one line of text.
/// A table cell that holds a number: "12", "-1.5", "1,234.5", "48.3 °C",
/// "57.4 %", "€3465.80". Dates ("2026-10-01") and versions ("1.2.3-rc")
/// are not numbers here.
fn is_numeric_cell(cell: &[MdInline]) -> bool {
    let [MdInline::Text(text)] = cell else {
        return false;
    };
    let mut t = text.trim();
    for prefix in ["+", "-", "−", "$", "€", "£"] {
        t = t.strip_prefix(prefix).unwrap_or(t).trim_start();
    }
    for suffix in ["%", "°C", "°F", "°", "€", "$", "£"] {
        t = t.strip_suffix(suffix).unwrap_or(t).trim_end();
    }
    let starts = t.chars().next().is_some_and(|c| c.is_ascii_digit());
    let ends = t.chars().next_back().is_some_and(|c| c.is_ascii_digit());
    starts
        && ends
        && t.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '\'' | '\u{2009}'))
}

/// True when a column holds numbers: at least one, and every other body
/// cell empty or a dash.
fn numeric_column<'a>(cells: impl Iterator<Item = Option<&'a Vec<MdInline>>>) -> bool {
    let mut any = false;
    for cell in cells.flatten() {
        let blank = match cell.as_slice() {
            [] => true,
            [MdInline::Text(t)] => matches!(t.trim(), "" | "-" | "–" | "—"),
            _ => false,
        };
        if blank {
            continue;
        }
        if !is_numeric_cell(cell) {
            return false;
        }
        any = true;
    }
    any
}

fn render_table_cell(
    ui: &mut egui::Ui,
    cell: &[MdInline],
    st: InlineStyle,
    chips: Option<&mut FileChips>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        render_inline_styled(ui, cell, st, chips);
    });
}

/// Text style of an inline run; bold/italic accumulate through nesting.
#[derive(Clone, Copy)]
struct InlineStyle {
    size: f32,
    color: Color32,
    bold: bool,
    italic: bool,
}

impl InlineStyle {
    const BODY: InlineStyle = InlineStyle {
        size: 15.0,
        color: TEXT,
        bold: false,
        italic: false,
    };

    fn apply(&self, text: RichText) -> RichText {
        let mut t = text.size(self.size);
        if self.bold {
            t = t.family(crate::fonts::bold());
        }
        if self.italic {
            t = t.italics();
        }
        t
    }
}

/// True when the run holds a link (links need real widgets).
fn has_link(inl: &[MdInline]) -> bool {
    inl.iter().any(|i| match i {
        MdInline::Link { .. } => true,
        MdInline::Bold(inner) | MdInline::Italic(inner) => has_link(inner),
        _ => false,
    })
}

/// Append an inline run to one layout job: bold, italics and code spans keep
/// a common baseline and line height.
fn append_inline(job: &mut egui::text::LayoutJob, inl: &[MdInline], st: InlineStyle) {
    for item in inl {
        match item {
            MdInline::Text(t) => {
                let family = if st.bold {
                    crate::fonts::bold()
                } else {
                    egui::FontFamily::Proportional
                };
                job.append(
                    t,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::new(st.size, family),
                        color: st.color,
                        italics: st.italic,
                        line_height: Some(st.size * 1.42),
                        ..Default::default()
                    },
                );
            }
            MdInline::Bold(inner) => append_inline(job, inner, InlineStyle { bold: true, ..st }),
            MdInline::Italic(inner) => {
                append_inline(job, inner, InlineStyle { italic: true, ..st })
            }
            MdInline::Code(c) => job.append(
                c,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(st.size - 1.5),
                    color: CODE_TEXT,
                    background: CODE_SPAN_BG,
                    line_height: Some(st.size * 1.42),
                    ..Default::default()
                },
            ),
            // has_link() routes these to widgets; here only table cells get
            // measured with them
            MdInline::Link { text, .. } => job.append(
                text,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(st.size),
                    color: LINK,
                    ..Default::default()
                },
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// File chips: path-like words in an answer that name an existing file of the
// session's workspace become clickable chips (click opens the preview pane)
// ---------------------------------------------------------------------------

/// How long a cached existence answer stays valid, so a file that appears or
/// disappears is picked up without a stat per token per frame.
const FILE_EXISTS_TTL: std::time::Duration = std::time::Duration::from_secs(3);

/// path -> (is a file, checked at). Existence only; file contents are never
/// read for chips.
fn file_exists_cached(p: &Path) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (bool, Instant)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut c = cache.lock().unwrap();
    if let Some((v, at)) = c.get(p) {
        if at.elapsed() < FILE_EXISTS_TTL {
            return *v;
        }
    }
    let v = std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false);
    if c.len() > 8192 {
        c.clear();
    }
    c.insert(p.to_path_buf(), (v, Instant::now()));
    v
}

/// (workspace, path) -> (lies outside the workspace, checked at), cached with
/// the same TTL as the existence probe above. Chips resolve every token of
/// every agent message on every frame, so the `canonicalize` inside
/// [`is_outside_workspace`] must run once per (workspace, path) per TTL,
/// not again on each frame.
pub(crate) fn outside_workspace_cached(cwd: &Path, p: &Path) -> bool {
    /// (workspace, path) -> (lies outside, checked at).
    type OutsideCache = Mutex<HashMap<(PathBuf, PathBuf), (bool, Instant)>>;
    static CACHE: OnceLock<OutsideCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut c = cache.lock().unwrap();
    let key = (cwd.to_path_buf(), p.to_path_buf());
    if let Some((v, at)) = c.get(&key) {
        if at.elapsed() < FILE_EXISTS_TTL {
            return *v;
        }
    }
    let v = is_outside_workspace(cwd, p);
    if c.len() > 8192 {
        c.clear();
    }
    c.insert(key, (v, Instant::now()));
    v
}

/// Canonical workspace root, cached (a syscall per token otherwise).
fn canonical_cwd(cwd: &Path) -> PathBuf {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut c = cache.lock().unwrap();
    if let Some(v) = c.get(cwd) {
        return v.clone();
    }
    let v = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    c.insert(cwd.to_path_buf(), v.clone());
    v
}

/// Lexical normalization (`.` and `..` resolved without touching the disk).
pub(crate) fn normalize_lexically(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when the absolute `path` resolves to a file outside the workspace
/// `cwd` — an absolute path elsewhere, a symlink, or a `..` that a linked
/// folder turns into another target. The candidate is canonicalized when it
/// exists (a missing file falls back to the lexical view). The syscalls are
/// not free, so callers that run per frame cache the result (see
/// [`outside_workspace_cached`]); click-triggered callers may use this
/// directly.
pub(crate) fn is_outside_workspace(cwd: &Path, path: &Path) -> bool {
    let root = canonical_cwd(cwd);
    let canon = path
        .canonicalize()
        .unwrap_or_else(|_| normalize_lexically(path));
    !(canon.starts_with(&root) || canon.starts_with(normalize_lexically(cwd)))
}

/// Resolve one token (an inline-code span or a path-like word) to an
/// existing file inside the workspace `cwd`. Absolute paths must live under
/// `cwd`; relative ones resolve against it. Only existence is checked
/// (cached) — contents are never read. Function names and symbols never
/// match: without a slash or a file extension the token is not a candidate.
pub fn resolve_workspace_file(cwd: &Path, token: &str) -> Option<PathBuf> {
    let t = chip_core(token);
    if t.is_empty() || t.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    let looks_like_path = t.contains('/') || {
        let name = t.rsplit('/').next().unwrap_or(t);
        match name.rsplit_once('.') {
            Some((stem, ext)) => {
                !stem.is_empty()
                    && (1..=5).contains(&ext.len())
                    && ext.chars().all(|c| c.is_ascii_alphanumeric())
            }
            None => false,
        }
    };
    if !looks_like_path {
        return None;
    }
    // relative tokens resolve against the canonical root; an absolute one
    // may be spelled through the workspace path as given (a symlinked
    // folder) or through its canonical form
    let root = canonical_cwd(cwd);
    let raw = Path::new(t);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    let abs = normalize_lexically(&joined);
    if !abs.starts_with(&root) && !abs.starts_with(normalize_lexically(cwd)) {
        return None;
    }
    // `metadata` follows symlinks, so a link inside the workspace can still
    // point outside (repo/x.txt -> /etc/passwd): the canonical containment
    // check (cached with the existence probe, see outside_workspace_cached)
    // rejects such candidates before they become chips.
    if file_exists_cached(&abs) && !outside_workspace_cached(cwd, &abs) {
        Some(abs)
    } else {
        None
    }
}

/// The path-like core of a token: surrounding quotes and brackets, list
/// punctuation and sentence-ending marks stripped (`(src/main.rs),` and
/// `analysis.py.` both give the file name).
fn chip_core(token: &str) -> &str {
    const WRAP: [char; 12] = ['`', '"', '\'', '(', ')', '[', ']', '{', '}', ',', ';', ':'];
    token
        .trim()
        .trim_matches(WRAP)
        .trim_end_matches(['.', '!', '?'])
        .trim_end_matches(['"', '\'', ')', ']', '}'])
}

/// True when any inline of the run resolves to a workspace file.
fn has_file_chip(inl: &[MdInline], chips: Option<&FileChips>) -> bool {
    let Some(chips) = chips else { return false };
    inl.iter().any(|item| match item {
        MdInline::Text(t) => t
            .split_whitespace()
            .any(|w| resolve_workspace_file(chips.cwd, w).is_some()),
        MdInline::Code(c) => resolve_workspace_file(chips.cwd, c).is_some(),
        MdInline::Bold(inner) | MdInline::Italic(inner) => has_file_chip(inner, Some(chips)),
        MdInline::Link { .. } => false,
    })
}

/// One file chip: file icon + the token as typed; the tooltip shows the
/// absolute path, a click reports the path to `open`.
fn file_chip(ui: &mut egui::Ui, token: &str, path: &Path, size: f32, open: &mut Vec<String>) {
    let g = line_galley(
        ui,
        token,
        egui::FontId::monospace(size - 2.0),
        CHIP_TEXT,
        220.0,
    );
    let h = size + 6.0;
    let w = g.size().x + 24.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    let fill = if resp.hovered() {
        theme::HOVER
    } else {
        CODE_BG
    };
    ui.painter().rect(
        rect,
        3.0,
        fill,
        egui::Stroke::new(1.0_f32, theme::BORDER_SOFT),
    );
    icons::paint(
        ui.painter(),
        Icon::File,
        egui::pos2(rect.left() + 6.0, rect.center().y - 4.5),
        0.9,
        theme::DIM,
    );
    paint_line(ui, rect.left() + 17.0, rect.center().y, g);
    let clicked = resp.clicked();
    resp.on_hover_text(path.to_string_lossy());
    if clicked {
        open.push(path.to_string_lossy().into_owned());
    }
}

/// One paragraph-like block. A single layout job wraps cleanly and selects
/// as one text; a run with links or file chips keeps the widget path (both
/// are clickable).
fn paragraph(ui: &mut egui::Ui, inl: &[MdInline], st: InlineStyle, chips: Option<&mut FileChips>) {
    if has_link(inl) || has_file_chip(inl, chips.as_deref()) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            render_inline_styled(ui, inl, st, chips);
        });
        return;
    }
    let mut job = egui::text::LayoutJob::default();
    append_inline(&mut job, inl, st);
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap());
}

fn render_inline_styled(
    ui: &mut egui::Ui,
    inl: &[MdInline],
    st: InlineStyle,
    mut chips: Option<&mut FileChips>,
) {
    for item in inl {
        match item {
            MdInline::Text(t) => {
                // keep soft newlines as line breaks inside the paragraph
                for (i, seg) in t.split('\n').enumerate() {
                    if i > 0 {
                        ui.end_row();
                    }
                    if seg.is_empty() {
                        continue;
                    }
                    let Some(ch) = chips.as_deref_mut() else {
                        ui.label(st.apply(RichText::new(seg).color(st.color)));
                        continue;
                    };
                    // path-like words that name a workspace file become chips
                    for piece in seg.split_inclusive(char::is_whitespace) {
                        if piece.chars().all(|c| c.is_whitespace()) {
                            ui.label(st.apply(RichText::new(piece).color(st.color)));
                            continue;
                        }
                        let core = chip_core(piece);
                        if let Some(path) = resolve_workspace_file(ch.cwd, core) {
                            // the punctuation and the space around the path
                            // stay text, so the sentence keeps its spacing
                            let at = piece.find(core).unwrap_or(0);
                            let (before, after) = (&piece[..at], &piece[at + core.len()..]);
                            if !before.is_empty() {
                                ui.label(st.apply(RichText::new(before).color(st.color)));
                            }
                            file_chip(ui, core, &path, st.size, ch.open);
                            if !after.is_empty() {
                                ui.label(st.apply(RichText::new(after).color(st.color)));
                            }
                        } else {
                            ui.label(st.apply(RichText::new(piece).color(st.color)));
                        }
                    }
                }
            }
            MdInline::Bold(inner) => {
                render_inline_styled(
                    ui,
                    inner,
                    InlineStyle { bold: true, ..st },
                    chips.as_deref_mut(),
                );
            }
            MdInline::Italic(inner) => {
                render_inline_styled(
                    ui,
                    inner,
                    InlineStyle { italic: true, ..st },
                    chips.as_deref_mut(),
                );
            }
            MdInline::Code(c) => {
                if let Some(ch) = chips.as_deref_mut() {
                    if let Some(path) = resolve_workspace_file(ch.cwd, c) {
                        file_chip(ui, chip_core(c), &path, st.size, ch.open);
                        continue;
                    }
                }
                ui.label(
                    RichText::new(c)
                        .monospace()
                        .size(st.size - 2.0)
                        .color(CODE_TEXT)
                        .background_color(CODE_SPAN_BG),
                );
            }
            MdInline::Link { text, url } => {
                if is_safe_link(url) {
                    let resp = ui.add(
                        egui::Label::new(st.apply(RichText::new(text).color(LINK).underline()))
                            .sense(egui::Sense::click()),
                    );
                    if resp.clicked() {
                        ui.ctx().output_mut(|o| {
                            o.open_url = Some(egui::OpenUrl::same_tab(url));
                        });
                    }
                    if resp.hovered() {
                        resp.on_hover_text(url);
                    }
                } else {
                    // unsafe scheme: plain, non-clickable text with an explanation
                    ui.label(st.apply(RichText::new(text).color(st.color)))
                        .on_hover_text(trf("Link blocked: {url}", &[("url", &url)]));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Generative-UI DSL rendering (egui-native analog of CopilotKit generative UI)
// ---------------------------------------------------------------------------

pub fn render_ui_node(
    ui: &mut egui::Ui,
    node: &UiNode,
    msg_idx: usize,
    block: usize,
    ctx: &mut RenderCtx,
) {
    match node {
        UiNode::Card { title, children } => {
            egui::Frame::default()
                .fill(CODE_BG)
                .rounding(egui::Rounding::same(4.0))
                .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER))
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    if let Some(t) = title {
                        block_title(ui, t);
                        ui.add_space(4.0);
                    }
                    for c in children {
                        render_ui_node(ui, c, msg_idx, block, ctx);
                    }
                });
            ui.add_space(6.0);
        }
        UiNode::Text { text } if ctx.depth >= MAX_TEXT_DEPTH => {
            // nested too deep: plain text, no further markdown or blocks
            ui.label(RichText::new(text).color(TEXT).size(14.0));
        }
        UiNode::Text { text } => {
            let blocks = crate::markdown::parse_cached(text);
            // Nested markdown: its blocks get a salted scroll-id path, so
            // their code blocks stay distinct from the outer message's
            // blocks (N16); the ui-block numbering inside runs in its own
            // space (final audit G), so a nested run cannot shift the
            // field/script keys of the outer blocks painted after it —
            // the outer counter resumes where it was. Host block +
            // 1_000_000 leaves a margin of 1_024 nested ui blocks per
            // host block, far past any node budget, and no outer message
            // paints enough ui blocks to reach the space.
            let outer_offset = ctx.block_offset;
            ctx.block_offset = 1_000_000 + block * 1_024;
            let outer_salt = std::mem::replace(&mut ctx.scroll_salt, crate::memo::hash_str(text));
            ctx.depth += 1;
            render_blocks(ui, &blocks, msg_idx, ctx);
            ctx.depth -= 1;
            ctx.block_offset = outer_offset;
            ctx.scroll_salt = outer_salt;
        }
        UiNode::Heading { level, text } => {
            let size = match level.unwrap_or(2) {
                1 => 20.0,
                2 => 17.0,
                _ => 15.0,
            };
            ui.label(
                RichText::new(text)
                    .color(TEXT)
                    .size(size)
                    .family(crate::fonts::bold()),
            );
            ui.add_space(2.0);
        }
        UiNode::Button { label, action } => {
            // The click sends `action`, not the label — both are
            // agent-controlled text. Two safeguards: the action is always
            // visible inline (not only on hover), and the first click only
            // arms the button — the send needs a second click within
            // ARM_WINDOW, so one accidental click on a prompt-injected
            // "OK" button sends nothing.
            let id = egui::Id::new(("ui_btn", ctx.owner, msg_idx, block, action));
            let armed = button_armed(id);
            let text = if armed {
                trf("Click again to send “{action}”", &[("action", action)])
            } else {
                label.clone()
            };
            let fill = if armed { crate::theme::AMBER } else { ACCENT };
            if ui
                .add(
                    egui::Button::new(RichText::new(text).color(crate::theme::ON_ACCENT))
                        .fill(fill)
                        .rounding(egui::Rounding::same(3.0)),
                )
                .clicked()
            {
                if armed {
                    disarm_button(id);
                    ctx.ui_events.push(UiEvent::Button {
                        action: action.clone(),
                    });
                } else {
                    arm_button(id);
                }
            }
            ui.label(
                RichText::new(trf("Sends: {action}", &[("action", action)]))
                    .monospace()
                    .size(11.0)
                    .color(DIM),
            );
            ui.add_space(4.0);
        }
        UiNode::Table { columns, rows } => {
            // plain-string cells of the ui DSL, painted by the shared table
            // grid (same path and look as markdown tables)
            let header: Vec<Vec<MdInline>> = columns
                .iter()
                .map(|c| vec![MdInline::Text(c.clone())])
                .collect();
            let cells: Vec<Vec<Vec<MdInline>>> = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| vec![MdInline::Text(cell.clone())])
                        .collect()
                })
                .collect();
            let id = egui::Id::new(("tbl", msg_idx, columns.len(), rows.len()));
            render_table_grid(ui, id, &header, &cells, None);
            ui.add_space(6.0);
        }
        UiNode::Form { submit, fields } => {
            egui::Frame::default()
                .fill(CODE_BG)
                .rounding(egui::Rounding::same(4.0))
                .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    for f in fields {
                        let key = field_key(ctx.owner, msg_idx, block, &f.name);
                        let entry = ctx
                            .form_values
                            .entry(key)
                            .or_insert_with(|| f.value.clone().unwrap_or_default());
                        ui.horizontal(|ui| {
                            ui.set_min_width(110.0);
                            ui.label(RichText::new(&f.label).color(DIM).size(13.5));
                            match f.kind.as_str() {
                                "checkbox" => {
                                    let mut b = entry == "true";
                                    if ui.checkbox(&mut b, "").changed() {
                                        *entry = b.to_string();
                                    }
                                }
                                _ => {
                                    ui.text_edit_singleline(entry);
                                }
                            }
                        });
                    }
                    ui.add_space(4.0);
                    // the submit label is agent-chosen and the values go to
                    // the agent (and with it the model provider and Vibe's
                    // history): say so inline, and arm the button like a
                    // plain one — a second click within ARM_WINDOW sends
                    let btn_id = egui::Id::new(("ui_form", ctx.owner, msg_idx, block, submit));
                    let armed = button_armed(btn_id);
                    let text = if armed {
                        trf("Click again to send “{submit}”", &[("submit", submit)])
                    } else {
                        submit.clone()
                    };
                    let fill = if armed { crate::theme::AMBER } else { ACCENT };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(text).color(crate::theme::ON_ACCENT))
                                .fill(fill)
                                .rounding(egui::Rounding::same(3.0)),
                        )
                        .clicked()
                    {
                        if armed {
                            disarm_button(btn_id);
                            let mut values = serde_json::Map::new();
                            for f in fields {
                                let key = field_key(ctx.owner, msg_idx, block, &f.name);
                                values.insert(
                                    f.name.clone(),
                                    serde_json::Value::String(
                                        ctx.form_values.get(&key).cloned().unwrap_or_default(),
                                    ),
                                );
                            }
                            // sliders, number fields and selects of the same block
                            // travel with the submit
                            for (name, value) in block_inputs(ctx, msg_idx, block, &ctx.block_nodes)
                            {
                                values.entry(name).or_insert(value);
                            }
                            ctx.ui_events.push(UiEvent::Form {
                                submit: submit.clone(),
                                values: serde_json::Value::Object(values),
                            });
                        } else {
                            arm_button(btn_id);
                        }
                    }
                    ui.label(
                        RichText::new(tr("On submit, the values above are sent to the agent."))
                            .size(11.0)
                            .color(DIM),
                    );
                });
            ui.add_space(6.0);
        }
        UiNode::BarChart { title, bars } => {
            egui::Frame::default()
                .fill(CODE_BG)
                .rounding(egui::Rounding::same(4.0))
                .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    if let Some(t) = title {
                        block_title(ui, t);
                        ui.add_space(4.0);
                    }
                    let max = bars
                        .iter()
                        .map(|b| b.value)
                        .fold(0.0_f64, f64::max)
                        .max(1.0);
                    let bar_area_w = (ui.available_width() - 140.0).max(40.0);
                    for b in bars {
                        ui.horizontal(|ui| {
                            ui.set_min_width(120.0);
                            ui.label(RichText::new(&b.label).color(TEXT).size(13.0));
                            let frac = (b.value / max) as f32;
                            let h = 14.0;
                            let rect = egui::Rect::from_min_size(
                                ui.cursor().min + egui::vec2(0.0, 2.0),
                                egui::vec2(bar_area_w, h),
                            );
                            ui.allocate_rect(rect, egui::Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                egui::Rounding::same(0.0),
                                crate::theme::RAISED,
                            );
                            let fill = egui::Rect::from_min_size(
                                rect.min,
                                egui::vec2((bar_area_w * frac).max(2.0), h),
                            );
                            ui.painter()
                                .rect_filled(fill, egui::Rounding::same(0.0), ACCENT);
                            ui.label(RichText::new(format!("{}", b.value)).color(DIM).small());
                        });
                    }
                });
            ui.add_space(6.0);
        }
        UiNode::Divider {} => {
            ui.separator();
            ui.add_space(4.0);
        }
        UiNode::Columns { children } => {
            // as many columns side by side as fit at a readable width; more
            // children wrap into further rows (egui cannot lay out columns
            // narrower than their spacing)
            let per_row = columns_per_row(children.len(), ui.available_width());
            for row in children.chunks(per_row) {
                ui.columns(per_row, |uis| {
                    for (i, c) in row.iter().enumerate() {
                        render_ui_node(&mut uis[i], c, msg_idx, block, ctx);
                    }
                });
            }
            ui.add_space(4.0);
        }
        UiNode::Slider {
            name,
            label,
            min,
            max,
            step,
            value,
        } => {
            let key = field_key(ctx.owner, msg_idx, block, name);
            // the bounds come from the model (or a script computing them):
            // `clamp` panics on min > max, so they are put in order first
            let (lo, hi) = slider_bounds(*min, *max);
            let mut current = ctx
                .form_values
                .get(&key)
                .and_then(|s| s.parse::<f64>().ok())
                .or(*value)
                .filter(|v| v.is_finite())
                .unwrap_or(lo)
                .clamp(lo, hi);
            ui.horizontal(|ui| {
                ui.set_min_width(110.0);
                ui.label(RichText::new(label).color(DIM).size(13.5));
                let mut slider = egui::Slider::new(&mut current, lo..=hi);
                if let Some(s) = step {
                    if *s > 0.0 {
                        // show as many decimals as the step has: "5" for a
                        // step of 1 instead of the pixel-speed default
                        // "5.00"; values snap to the step anyway
                        slider = slider.step_by(*s).fixed_decimals(step_decimals(*s));
                    }
                }
                let response = ui.add(slider);
                // An ongoing drag makes the enclosing ui-js block throttle
                // its re-evaluations (Handoff 2.7). |=, because a later,
                // idle slider of the same block must not clear the flag.
                ctx.slider_dragging |= response.dragged();
                if response.changed() {
                    ctx.form_values.insert(key.clone(), current.to_string());
                }
            });
            ui.add_space(2.0);
        }
        UiNode::Number { name, label, value } => {
            let key = field_key(ctx.owner, msg_idx, block, name);
            let mut text = ctx
                .form_values
                .get(&key)
                .cloned()
                .unwrap_or_else(|| value.map(|v| v.to_string()).unwrap_or_default());
            ui.horizontal(|ui| {
                ui.set_min_width(110.0);
                ui.label(RichText::new(label).color(DIM).size(13.5));
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(90.0)
                            .hint_text("0"),
                    )
                    .changed()
                {
                    ctx.form_values.insert(key.clone(), text.clone());
                }
            });
            ui.add_space(2.0);
        }
        UiNode::Toggle { name, label, value } => {
            let key = field_key(ctx.owner, msg_idx, block, name);
            let mut on = ctx
                .form_values
                .get(&key)
                .map(|s| s == "true")
                .or(*value)
                .unwrap_or(false);
            ui.horizontal(|ui| {
                ui.set_min_width(110.0);
                if ui
                    .checkbox(&mut on, RichText::new(label).color(DIM).size(13.5))
                    .changed()
                {
                    ctx.form_values.insert(key.clone(), on.to_string());
                }
            });
            ui.add_space(2.0);
        }
        UiNode::TextInput { name, label, value } => {
            let key = field_key(ctx.owner, msg_idx, block, name);
            let mut text = ctx
                .form_values
                .get(&key)
                .cloned()
                .or_else(|| value.clone())
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.set_min_width(110.0);
                ui.label(RichText::new(label).color(DIM).size(13.5));
                if ui
                    .add(egui::TextEdit::singleline(&mut text).desired_width(200.0))
                    .changed()
                {
                    ctx.form_values.insert(key.clone(), text.clone());
                }
            });
            ui.add_space(2.0);
        }
        UiNode::Select {
            name,
            label,
            options,
            value,
        } => {
            let key = field_key(ctx.owner, msg_idx, block, name);
            let current = ctx
                .form_values
                .get(&key)
                .cloned()
                .or_else(|| value.clone())
                .unwrap_or_else(|| options.first().cloned().unwrap_or_default());
            ui.horizontal(|ui| {
                ui.set_min_width(110.0);
                ui.label(RichText::new(label).color(DIM).size(13.5));
                let mut chosen = current.clone();
                egui::ComboBox::from_id_salt(("ui_select", msg_idx, block, name))
                    .selected_text(chosen.clone())
                    .show_ui(ui, |ui| {
                        for o in options {
                            ui.selectable_value(&mut chosen, o.clone(), o);
                        }
                    });
                if chosen != current {
                    ctx.form_values.insert(key.clone(), chosen);
                }
            });
            ui.add_space(2.0);
        }
        UiNode::Computed {
            label,
            expr,
            decimals,
            unit,
        } => {
            // the inputs of this block are the variables of the formula
            let vars = block_vars(ctx, msg_idx, block, &ctx.block_nodes);
            let result = crate::formula::eval(expr, &vars);
            ui.horizontal(|ui| {
                ui.set_min_width(110.0);
                ui.label(RichText::new(label).color(DIM).size(13.5));
                match result {
                    Ok(v) => ui.label(
                        RichText::new(format_computed(i18n::lang(), v, *decimals, unit.as_deref()))
                            .color(crate::theme::ORANGE_DIM)
                            .monospace()
                            .size(14.0)
                            .strong(),
                    ),
                    Err(e) => ui.label(RichText::new(e).color(DIM).small()),
                };
            });
            ui.add_space(2.0);
        }
        UiNode::LineChart {
            title,
            x_label,
            y_label,
            series,
        } => {
            render_line_chart(
                ui,
                title.as_deref(),
                x_label.as_deref(),
                y_label.as_deref(),
                series,
            );
            ui.add_space(6.0);
        }
        UiNode::PieChart { title, slices } => {
            render_pie_chart(ui, title.as_deref(), slices);
            ui.add_space(6.0);
        }
        UiNode::Progress { label, value } => {
            let frac = if value.is_nan() {
                0.0
            } else {
                value.clamp(0.0, 1.0) as f32
            };
            ui.horizontal(|ui| {
                ui.set_min_width(110.0);
                ui.label(RichText::new(label).color(DIM).size(13.5));
                let filled = crate::theme::meter_blocks(frac, 20);
                crate::theme::pixel_meter(ui, filled, 20, 10.0);
                ui.label(
                    RichText::new(format!("{:.0} %", frac * 100.0))
                        .color(DIM)
                        .monospace()
                        .size(12.0),
                );
            });
            ui.add_space(2.0);
        }
        UiNode::Checklist { items } => {
            for (i, item) in items.iter().enumerate() {
                let key = field_key(ctx.owner, msg_idx, block, &format!("check:{i}"));
                let mut done = ctx
                    .form_values
                    .get(&key)
                    .map(|s| s == "true")
                    .unwrap_or(item.done);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut done, "").changed() {
                        ctx.form_values.insert(key.clone(), done.to_string());
                    }
                    let text = RichText::new(&item.text).size(14.0);
                    ui.label(if done {
                        text.color(DIM).strikethrough()
                    } else {
                        text.color(TEXT)
                    });
                });
            }
            ui.add_space(4.0);
        }
        UiNode::Kv { rows } => {
            egui::Grid::new(egui::Id::new(("kv", msg_idx, block, rows.len())))
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for row in rows {
                        ui.label(RichText::new(&row[0]).color(DIM).size(13.0));
                        ui.label(RichText::new(&row[1]).color(TEXT).size(13.5));
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
        }
    }
}

/// Line chart: axes, up to 6 series in gradient colours, legend.
/// The title of a card or chart: one style for all of them, so siblings
/// on a dashboard read as equals (no orange: the accent is for actions).
fn block_title(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .color(TEXT)
            .size(13.5)
            .family(crate::fonts::bold()),
    );
}

/// Round axis ticks for `[min, max]`: a step of 1, 2 or 5 times a power
/// of ten that gives about `target` ticks, the first at or below `min`, the
/// last at or above `max` (a chart from -1.1 to 21.2 gets 0 / 10 / 20 …
/// instead of -1.1 / 10.05 / 21.2).
fn nice_ticks(min: f64, max: f64, target: usize) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite()) || max <= min || target < 2 {
        return vec![min, max];
    }
    let raw = (max - min) / (target - 1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag);
    // round to the step's decimals: 3 × 0.2 is 0.6000000000000001 in floats
    let scale = 10f64.powi((-step.log10().floor()).max(0.0) as i32 + 1);
    let snap = |v: f64| (v * scale).round() / scale;
    let first = (min / step).floor();
    let mut out = Vec::new();
    // bounded: at most target + 3 ticks for any input
    for k in 0..(target as u32 + 3) {
        let v = snap((first + k as f64) * step);
        out.push(v);
        if v >= max - step * 1e-9 {
            break;
        }
    }
    out
}

/// An axis value without float noise or trailing zeros.
fn tick_label(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

fn render_line_chart(
    ui: &mut egui::Ui,
    title: Option<&str>,
    x_label: Option<&str>,
    y_label: Option<&str>,
    series: &[crate::model::Series],
) {
    egui::Frame::default()
        .fill(CODE_BG)
        .rounding(egui::Rounding::same(4.0))
        .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER))
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            if let Some(t) = title {
                block_title(ui, t);
                ui.add_space(4.0);
            }
            let series = &series[..series.len().min(6)];
            let points: Vec<&[f64; 2]> = series.iter().flat_map(|s| s.points.iter()).collect();
            if points.is_empty() {
                ui.label(RichText::new(tr("No data.")).color(DIM).small());
                return;
            }
            let (mut x_min, mut x_max) = (f64::INFINITY, f64::NEG_INFINITY);
            let (mut y_min, mut y_max) = (f64::INFINITY, f64::NEG_INFINITY);
            for p in &points {
                x_min = x_min.min(p[0]);
                x_max = x_max.max(p[0]);
                y_min = y_min.min(p[1]);
                y_max = y_max.max(p[1]);
            }
            if x_max <= x_min {
                x_max = x_min + 1.0;
            }
            if y_max <= y_min {
                y_max = y_min + 1.0;
            }
            // the y range grows to round ticks; x keeps the data's range
            // and labels the round values inside it
            let y_ticks = nice_ticks(y_min, y_max, 6);
            y_min = y_ticks.first().copied().unwrap_or(y_min);
            y_max = y_ticks
                .last()
                .copied()
                .unwrap_or(y_max)
                .max(y_min + f64::EPSILON);
            let x_ticks: Vec<f64> = nice_ticks(x_min, x_max, 6)
                .into_iter()
                .filter(|x| *x >= x_min - 1e-9 && *x <= x_max + 1e-9)
                .collect();
            let w = ui.available_width().max(120.0);
            let h = 140.0;
            let (rect, _resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
            let plot = egui::Rect::from_min_max(
                rect.min + egui::vec2(34.0, 6.0),
                rect.max - egui::vec2(6.0, 18.0),
            );
            let painter = ui.painter();
            // axes
            painter.line_segment(
                [plot.left_bottom(), plot.right_bottom()],
                egui::Stroke::new(1.0_f32, crate::theme::BORDER),
            );
            painter.line_segment(
                [plot.left_bottom(), plot.left_top()],
                egui::Stroke::new(1.0_f32, crate::theme::BORDER),
            );
            // grid lines + y labels at the round ticks
            for (i, value) in y_ticks.iter().enumerate() {
                let frac = ((value - y_min) / (y_max - y_min)) as f32;
                let y = plot.bottom() - plot.height() * frac;
                if i > 0 {
                    painter.line_segment(
                        [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                        egui::Stroke::new(1.0_f32, crate::theme::RAISED),
                    );
                }
                painter.text(
                    egui::pos2(plot.left() - 4.0, y),
                    egui::Align2::RIGHT_CENTER,
                    tick_label(*value),
                    egui::FontId::monospace(10.5),
                    DIM,
                );
            }
            // x labels at the round values inside the data's range
            for (i, value) in x_ticks.iter().enumerate() {
                let frac = ((value - x_min) / (x_max - x_min)) as f32;
                let x = plot.left() + plot.width() * frac;
                let align = if i == 0 && frac < 0.05 {
                    egui::Align2::LEFT_TOP
                } else if frac > 0.95 {
                    egui::Align2::RIGHT_TOP
                } else {
                    egui::Align2::CENTER_TOP
                };
                painter.text(
                    egui::pos2(x, plot.bottom() + 3.0),
                    align,
                    tick_label(*value),
                    egui::FontId::monospace(10.5),
                    DIM,
                );
            }
            let to_screen = |p: &[f64; 2]| {
                let fx = ((p[0] - x_min) / (x_max - x_min)) as f32;
                let fy = ((p[1] - y_min) / (y_max - y_min)) as f32;
                egui::pos2(
                    plot.left() + plot.width() * fx,
                    plot.bottom() - plot.height() * fy,
                )
            };
            for (i, s) in series.iter().enumerate() {
                let color = series_color(i);
                let pts: Vec<egui::Pos2> = s.points.iter().map(to_screen).collect();
                for pair in pts.windows(2) {
                    painter.line_segment([pair[0], pair[1]], egui::Stroke::new(1.8_f32, color));
                }
                // point marks only while they stand apart: on a dense line
                // they merge into a beaded, fuzzy stroke
                let spacing = plot.width() / pts.len().max(1) as f32;
                let marks = if spacing >= 8.0 { pts.as_slice() } else { &[] };
                for p in marks {
                    painter.rect_filled(
                        egui::Rect::from_center_size(*p, egui::vec2(3.0, 3.0)),
                        egui::Rounding::same(0.0),
                        color,
                    );
                }
            }
            // legend
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                for (i, s) in series.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(r, egui::Rounding::same(0.0), series_color(i));
                    ui.label(RichText::new(&s.name).color(DIM).small());
                    ui.add_space(6.0);
                }
            });
            let mut axis = String::new();
            if let Some(x) = x_label {
                axis.push_str(x);
            }
            if let Some(y) = y_label {
                if !axis.is_empty() {
                    axis.push_str(" · ");
                }
                axis.push_str(y);
            }
            if !axis.is_empty() {
                ui.label(RichText::new(axis).color(DIM).small());
            }
        });
}

/// Pie chart: circle segments as polygons plus a legend with percentages.
fn render_pie_chart(ui: &mut egui::Ui, title: Option<&str>, slices: &[crate::model::Bar]) {
    egui::Frame::default()
        .fill(CODE_BG)
        .rounding(egui::Rounding::same(4.0))
        .stroke(egui::Stroke::new(1.0_f32, crate::theme::BORDER))
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            if let Some(t) = title {
                block_title(ui, t);
                ui.add_space(4.0);
            }
            let total: f64 = slices.iter().map(|s| s.value.max(0.0)).sum();
            if total <= 0.0 {
                ui.label(RichText::new(tr("No data.")).color(DIM).small());
                return;
            }
            ui.horizontal(|ui| {
                let size = 120.0;
                let (rect, _resp) =
                    ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                let center = rect.center();
                let radius = size / 2.0 - 2.0;
                let painter = ui.painter();
                let fracs: Vec<f32> = slices
                    .iter()
                    .map(|s| (s.value.max(0.0) / total) as f32)
                    .collect();
                let point = |a: f32| center + egui::vec2(a.cos(), a.sin()) * radius;
                // Each slice is one triangle fan from the centre, without
                // an outline and without feathering inside: a per-slice
                // stroke grew into a spike at a thin slice's sharp centre
                // angle, a slice over half the circle drawn as one
                // "convex" polygon frayed at the centre, and feathered
                // parts side by side left seams. Only the rim (a line in
                // the slice's colour) and the separators are smoothed.
                let mut fill = egui::Mesh::default();
                let mut rims = Vec::new();
                for (i, start, sweep) in pie_slices(&fracs) {
                    let color = series_color(i);
                    let steps = ((sweep / 0.05).ceil() as usize).max(2);
                    let rim: Vec<egui::Pos2> = (0..=steps)
                        .map(|k| point(start + sweep * (k as f32 / steps as f32)))
                        .collect();
                    let base = fill.vertices.len() as u32;
                    fill.colored_vertex(center, color);
                    for p in &rim {
                        fill.colored_vertex(*p, color);
                    }
                    for k in 0..steps as u32 {
                        fill.add_triangle(base, base + 1 + k, base + 2 + k);
                    }
                    rims.push(egui::Shape::line(rim, egui::Stroke::new(1.0_f32, color)));
                }
                painter.add(fill);
                painter.extend(rims);
                // the gaps between slices: straight radial lines, which
                // meet cleanly in the centre
                for a in pie_boundaries(&fracs) {
                    painter.line_segment([center, point(a)], egui::Stroke::new(1.5_f32, CODE_BG));
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    for (i, s) in slices.iter().enumerate() {
                        let pct = s.value.max(0.0) / total * 100.0;
                        ui.horizontal(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter()
                                .rect_filled(r, egui::Rounding::same(0.0), series_color(i));
                            ui.label(RichText::new(&s.label).color(TEXT).size(13.0));
                            ui.label(
                                RichText::new(i18n::decimal(format!("{pct:.1} %")))
                                    .color(DIM)
                                    .monospace()
                                    .size(11.5),
                            );
                        });
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    /// Paint one agent message through the real renderer for a few frames.
    fn paint_message(
        text: &str,
        scripts: &mut std::collections::HashMap<String, crate::script::ScriptTool>,
    ) {
        let ctx = egui::Context::default();
        crate::fonts::install_fallback_fonts(&ctx);
        let blocks = crate::markdown::parse(text);
        let mut events = Vec::new();
        let mut forms = std::collections::HashMap::new();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut rctx =
                        super::RenderCtx::new(&mut events, &mut forms, "s1").with_scripts(scripts);
                    super::render_blocks(ui, &blocks, 0, &mut rctx);
                });
            });
        }
    }

    /// A ```ui-js fence inside a `text` node (what a tool printing its own
    /// source produces) is shown, never run: no tool is created for it.
    /// Before, each run made the next one until the stack overflowed.
    #[test]
    fn nested_ui_js_never_runs() {
        let msg = concat!(
            "```ui\n",
            r#"{"type":"text","text":"\u0060\u0060\u0060ui-js\nfunction render(s){ return {type:'divider'}; }\n\u0060\u0060\u0060"}"#,
            "\n```\n"
        );
        let mut scripts = std::collections::HashMap::new();
        paint_message(msg, &mut scripts);
        assert!(scripts.is_empty(), "the nested tool must not run");
        // text nodes nested deeper than the limit still paint
        let mut deep = r#"{"type":"text","text":"unten"}"#.to_string();
        for _ in 0..8 {
            let inner =
                serde_json::to_string(&format!("\u{60}\u{60}\u{60}ui\n{deep}\n\u{60}\u{60}\u{60}"))
                    .unwrap();
            deep = format!(r#"{{"type":"text","text":{inner}}}"#);
        }
        paint_message(&format!("```ui\n{deep}\n```\n"), &mut scripts);
    }

    /// The full renderer with a slider whose bounds are reversed (the model
    /// or a script computed them): it used to panic in `f64::clamp`.
    #[test]
    fn a_reversed_slider_paints() {
        let msg = "```ui\n{\"type\":\"slider\",\"name\":\"n\",\"label\":\"N\",\"min\":10,\"max\":1}\n```\n";
        paint_message(msg, &mut std::collections::HashMap::new());
    }

    /// Pie slices follow each other around the full circle, empty slices
    /// take no room, and the separators sit exactly between slices that
    /// have a share.
    #[test]
    fn pie_slices_cover_the_circle_and_separators_sit_between_them() {
        use super::{pie_boundaries, pie_slices, PIE_START};
        use std::f32::consts::TAU;
        let fracs = [0.574, 0.15, 0.0, 0.088, 0.037, 0.068, 0.048, 0.035];
        let slices = pie_slices(&fracs);
        assert_eq!(slices.len(), 7, "no slice for an empty share");
        assert!(slices.iter().all(|(i, _, _)| *i != 2));
        let total: f32 = slices.iter().map(|(_, _, s)| s).sum();
        assert!((total - TAU).abs() < 1e-3, "{total}");
        for pair in slices.windows(2) {
            assert!(
                (pair[1].1 - (pair[0].1 + pair[0].2)).abs() < 1e-5,
                "gapless"
            );
        }
        assert!((slices[0].1 - PIE_START).abs() < 1e-6);
        // one separator per slice with a share, at the slices' starts
        let lines = pie_boundaries(&fracs);
        assert_eq!(lines.len(), 7);
        for (line, slice) in lines.iter().zip(&slices) {
            assert!((line - slice.1).abs() < 1e-5);
        }
        // a single slice is a full disc without any separator
        assert!(pie_boundaries(&[1.0, 0.0]).is_empty());
        assert_eq!(pie_slices(&[1.0]).len(), 1);
    }

    #[test]
    fn slider_bounds_are_ordered_and_finite() {
        use super::slider_bounds;
        assert_eq!(slider_bounds(1.0, 10.0), (1.0, 10.0));
        // reversed bounds from a model or script used to panic in clamp
        assert_eq!(slider_bounds(10.0, 1.0), (1.0, 10.0));
        assert_eq!(slider_bounds(0.0, -1.0), (-1.0, 0.0));
        assert_eq!(slider_bounds(5.0, 5.0), (5.0, 5.0));
        let (lo, hi) = slider_bounds(f64::NAN, f64::INFINITY);
        assert!(lo <= hi && lo.is_finite() && hi.is_finite());
        let (lo, hi) = slider_bounds(3.0, -3.0);
        assert_eq!(7.0_f64.clamp(lo, hi), 3.0);
    }

    #[test]
    fn columns_wrap_when_they_do_not_fit() {
        use super::columns_per_row;
        assert_eq!(columns_per_row(3, 800.0), 3);
        assert_eq!(columns_per_row(300, 800.0), 6);
        assert_eq!(columns_per_row(4, 100.0), 1);
        assert_eq!(columns_per_row(0, 800.0), 1);
    }

    #[test]
    fn nice_ticks_are_round_and_cover_the_range() {
        use super::{nice_ticks, tick_label};
        // the dashboard's outdoor temperatures
        assert_eq!(
            nice_ticks(-1.1, 21.2, 6),
            [-5.0, 0.0, 5.0, 10.0, 15.0, 20.0, 25.0]
        );
        // hours of a week
        assert_eq!(nice_ticks(0.0, 167.0, 6), [0.0, 50.0, 100.0, 150.0, 200.0]);
        // small values without float noise
        assert_eq!(nice_ticks(0.0, 1.0, 6), [0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        assert_eq!(tick_label(0.30000000000000004), "0.3");
        assert_eq!(tick_label(-0.0), "0");
        assert_eq!(tick_label(150.0), "150");
        // degenerate input stays bounded
        assert_eq!(nice_ticks(3.0, 3.0, 6), [3.0, 3.0]);
        assert!(nice_ticks(0.0, 1e12, 6).len() <= 9);
    }

    #[test]
    fn numeric_cells_and_columns() {
        use super::{is_numeric_cell, numeric_column};
        use crate::markdown::MdInline;
        let cell = |t: &str| vec![MdInline::Text(t.to_string())];
        for n in [
            "12",
            "-1.5",
            "1,234.5",
            "48.3 °C",
            "57.4 %",
            "€3465.80",
            "−2",
            "0.000",
        ] {
            assert!(is_numeric_cell(&cell(n)), "{n} is a number");
        }
        for t in [
            "2026-10-01",
            "1.2.3-rc",
            "Setup",
            "12 items",
            "",
            "v2",
            "1/2",
        ] {
            assert!(!is_numeric_cell(&cell(t)), "{t} is not a number");
        }
        assert!(!is_numeric_cell(&[MdInline::Code("12".to_string())]));
        let col = |cells: Vec<Vec<MdInline>>| numeric_column(cells.iter().map(Some));
        assert!(col(vec![cell("2.5"), cell("–"), cell("16.9")]));
        assert!(!col(vec![cell("2.5"), cell("n/a")]));
        assert!(!col(vec![cell("–"), cell("")]), "no number at all");
    }

    #[test]
    fn table_widths_keep_narrow_columns_and_share_the_rest() {
        use super::table_widths;
        // everything fits: natural widths (a minimum for tiny columns)
        assert_eq!(
            table_widths(&[100.0, 10.0], 300.0, 20.0, 48.0),
            vec![100.0, 48.0]
        );
        // too wide: the short column keeps its width, the long ones share
        let w = table_widths(&[60.0, 800.0, 800.0], 500.0, 20.0, 48.0);
        assert_eq!(w[0], 60.0, "short column is not squeezed");
        assert_eq!(w[1], w[2]);
        assert!(
            (w.iter().sum::<f32>() - (500.0 - 60.0)).abs() < 0.5,
            "{w:?}"
        );
        // when even the minima do not fit, the table still never gets
        // wider than the space: columns shrink under the minimum and wrap
        let w = table_widths(&[900.0; 12], 300.0, 20.0, 48.0);
        assert!(w.iter().all(|x| *x > 0.0), "{w:?}");
        assert!(w.iter().sum::<f32>() + 20.0 * 12.0 <= 300.0, "{w:?}");
        assert!(table_widths(&[], 300.0, 20.0, 48.0).is_empty());
    }

    #[test]
    fn inputs_nested_in_cards_and_columns_feed_formulas() {
        let nodes: Vec<crate::model::UiNode> = serde_json::from_str(
            r#"[{"type":"card","title":"T","children":[
                {"type":"slider","name":"a","label":"A","min":0,"max":10,"value":2},
                {"type":"columns","children":[{"type":"number","name":"b","label":"B","value":3}]}]},
               {"type":"computed","label":"S","expr":"a*b"}]"#,
        )
        .unwrap();
        let mut events = Vec::new();
        let mut values = std::collections::HashMap::new();
        let ctx = super::RenderCtx::new(&mut events, &mut values, "chat");
        let vars = super::block_vars(&ctx, 0, 0, &nodes);
        assert_eq!(vars.get("a"), Some(&2.0));
        assert_eq!(vars.get("b"), Some(&3.0));
        assert_eq!(crate::formula::eval("a*b", &vars), Ok(6.0));
    }

    use super::is_safe_link;
    use crate::model::{parse_ui_blocks, UiEvent, UiNode};

    #[test]
    fn ui_block_parses_card() {
        let text = "Hier ist eine Karte:\n```ui\n{\"type\":\"card\",\"title\":\"T\",\"children\":[{\"type\":\"text\",\"text\":\"Hallo\"},{\"type\":\"button\",\"label\":\"OK\",\"action\":\"ok\"}]}\n```\n";
        let nodes = parse_ui_blocks(text);
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            crate::model::UiNode::Card { title, children } => {
                assert_eq!(title.as_deref(), Some("T"));
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected card"),
        }
    }

    #[test]
    fn ui_block_parses_array() {
        let text = "```ui\n[{\"type\":\"text\",\"text\":\"a\"},{\"type\":\"divider\"}]\n```";
        let nodes = parse_ui_blocks(text);
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn safe_link_schemes_only() {
        assert!(is_safe_link("https://x.y"));
        assert!(is_safe_link("HTTPS://X.Y"));
        assert!(is_safe_link("http://x.y/path?q=1"));
        assert!(is_safe_link("mailto:a@b"));
        assert!(is_safe_link("  https://x.y  "));
        assert!(!is_safe_link("javascript:alert(1)"));
        assert!(!is_safe_link("JAVASCRIPT:alert(1)"));
        assert!(!is_safe_link("file:///etc/passwd"));
        assert!(!is_safe_link("data:text/html,<script>alert(1)</script>"));
        assert!(!is_safe_link("ftp://x.y"));
        assert!(!is_safe_link(""));
        assert!(!is_safe_link("x.y"));
    }

    #[test]
    fn computed_format_uses_german_decimal_comma() {
        use crate::i18n::Lang::De;
        assert_eq!(super::format_computed(De, 1234.5, None, None), "1.234,50");
        assert_eq!(super::format_computed(De, 2.0, Some(0), None), "2");
        assert_eq!(
            super::format_computed(De, 1.23456, Some(3), Some("€")),
            "1,235 €"
        );
        assert_eq!(super::format_computed(De, 7.0, Some(1), Some("  ")), "7,0");
        // German thousands dots (grouping is wanted, not just a comma)
        assert_eq!(
            super::format_computed(De, 22645.48, None, Some("€")),
            "22.645,48 €"
        );
        assert_eq!(
            super::format_computed(De, 1234567.891, Some(2), None),
            "1.234.567,89"
        );
        // values below 1000 keep their plain form, negatives get both markers
        assert_eq!(super::format_computed(De, 999.994, Some(2), None), "999,99");
        assert_eq!(super::format_computed(De, -1234.5, None, None), "-1.234,50");
        assert_eq!(
            super::format_computed(De, -5.25, Some(2), Some("€")),
            "-5,25 €"
        );
    }

    #[test]
    fn computed_format_follows_the_other_uis() {
        use crate::i18n::Lang::{Es, Fr, It, Pt};
        // French groups with a no-break space: the number never wraps
        assert_eq!(
            super::format_computed(Fr, 1234567.891, Some(2), None),
            "1\u{a0}234\u{a0}567,89"
        );
        for lang in [Es, It, Pt] {
            assert_eq!(super::format_computed(lang, 1234.5, None, None), "1.234,50");
        }
    }

    #[test]
    fn computed_format_follows_the_english_ui() {
        use crate::i18n::Lang::En;
        assert_eq!(super::format_computed(En, 1234.5, None, None), "1,234.50");
        assert_eq!(super::format_computed(En, 2.0, Some(0), None), "2");
        assert_eq!(
            super::format_computed(En, 252.4, Some(2), Some("€")),
            "252.40 €"
        );
        assert_eq!(
            super::format_computed(En, 1234567.891, Some(2), None),
            "1,234,567.89"
        );
        assert_eq!(super::format_computed(En, -1234.5, None, None), "-1,234.50");
        assert_eq!(super::format_computed(En, f64::NAN, Some(1), None), "NaN");
    }

    #[test]
    fn slider_decimals_follow_the_step() {
        assert_eq!(
            super::step_decimals(1.0),
            0,
            "integer step shows no decimals"
        );
        assert_eq!(super::step_decimals(2.0), 0);
        assert_eq!(super::step_decimals(0.5), 1);
        assert_eq!(super::step_decimals(0.05), 2);
        assert_eq!(super::step_decimals(0.25), 2);
        assert_eq!(super::step_decimals(0.001), 3);
        assert_eq!(
            super::step_decimals(0.3333333333333333),
            6,
            "steps with float noise stay sensible"
        );
    }

    #[test]
    fn block_vars_reads_inputs_of_the_block() {
        use crate::model::UiNode;
        use std::collections::HashMap;
        let nodes = vec![
            UiNode::Slider {
                name: "rate".into(),
                label: "Zins".into(),
                min: 0.0,
                max: 10.0,
                step: None,
                value: Some(2.0),
            },
            UiNode::Number {
                name: "years".into(),
                label: "Jahre".into(),
                value: Some(10.0),
            },
            UiNode::Select {
                name: "kind".into(),
                label: "Art".into(),
                options: vec!["1".into(), "2".into()],
                value: Some("2".into()),
            },
            UiNode::Select {
                name: "word".into(),
                label: "Wort".into(),
                options: vec!["ja".into()],
                value: Some("ja".into()),
            },
            UiNode::Text {
                text: "kein Eingabefeld".into(),
            },
        ];
        let mut events = Vec::new();
        let mut values = HashMap::new();
        let ctx = super::RenderCtx::new(&mut events, &mut values, "chat");
        let vars = super::block_vars(&ctx, 3, 1, &nodes);
        assert_eq!(vars.get("rate"), Some(&2.0));
        assert_eq!(vars.get("years"), Some(&10.0));
        assert_eq!(vars.get("kind"), Some(&2.0), "numeric option counts");
        assert_eq!(vars.get("word"), Some(&0.0), "non-numeric option is 0");
        assert_eq!(vars.len(), 4);

        // a stored value wins over the default
        let mut events = Vec::new();
        let mut values = HashMap::new();
        values.insert(super::field_key("chat", 3, 1, "rate"), "7.5".to_string());
        let ctx = super::RenderCtx::new(&mut events, &mut values, "chat");
        let vars = super::block_vars(&ctx, 3, 1, &nodes);
        assert_eq!(vars.get("rate"), Some(&7.5));
        // another block keeps its own state
        let vars = super::block_vars(&ctx, 3, 2, &nodes);
        assert_eq!(vars.get("rate"), Some(&2.0));
    }

    #[test]
    fn block_inputs_are_json_values_for_the_form_submit() {
        use crate::model::UiNode;
        use std::collections::HashMap;
        let nodes = vec![
            UiNode::Slider {
                name: "rate".into(),
                label: "Zins".into(),
                min: 0.0,
                max: 10.0,
                step: None,
                value: Some(2.5),
            },
            UiNode::Select {
                name: "kind".into(),
                label: "Art".into(),
                options: vec!["a".into()],
                value: Some("a".into()),
            },
        ];
        let mut events = Vec::new();
        let mut values = HashMap::new();
        let ctx = super::RenderCtx::new(&mut events, &mut values, "chat");
        let inputs = super::block_inputs(&ctx, 0, 0, &nodes);
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].0, "rate");
        assert_eq!(inputs[0].1, serde_json::json!(2.5));
        assert_eq!(inputs[1].1, serde_json::json!("a"));
    }

    #[test]
    fn keys_are_owner_scoped() {
        // the format: "<owner>#<msg_idx>#<block>#<name>"
        assert_eq!(super::field_key("ses-1", 2, 0, "rate"), "ses-1#2#0#rate");
        assert_eq!(super::owner_key_prefix("ses-1"), "ses-1#");
        // the same position in another owner is another key: one chat's
        // slider value never surfaces in the next (Handoff 2.5)
        assert_ne!(
            super::field_key("ses-1", 2, 0, "rate"),
            super::field_key("ses-2", 2, 0, "rate")
        );
        // an empty name yields the block's key prefix (script state reset)
        assert_eq!(super::field_key("ses-1", 2, 0, ""), "ses-1#2#0#");
        // script tools are owner-scoped the same way, and stay stable
        // within one owner (same code, same position)
        let code = "function render(s){return null}";
        assert_ne!(
            super::script_key("ses-1", 0, 1, code),
            super::script_key("ses-2", 0, 1, code)
        );
        assert_eq!(
            super::script_key("ses-1", 0, 1, code),
            super::script_key("ses-1", 0, 1, code)
        );
        assert!(super::script_key("ses-1", 0, 1, code).starts_with("ses-1#0#1#"));
    }

    #[test]
    fn drop_owner_entries_removes_only_that_owner() {
        let mut values: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        values.insert(super::field_key("a", 1, 0, "x"), "1".into());
        values.insert(super::field_key("a", 2, 1, "y"), "2".into());
        values.insert(super::field_key("b", 1, 0, "x"), "3".into());
        // an owner that only shares the first chars stays: the separator
        // decides, not the raw prefix
        values.insert("ab#1#0#x".to_string(), "4".into());
        super::drop_owner_entries(&mut values, "a");
        assert!(!values.contains_key(&super::field_key("a", 1, 0, "x")));
        assert!(!values.contains_key(&super::field_key("a", 2, 1, "y")));
        assert!(values.contains_key(&super::field_key("b", 1, 0, "x")));
        assert!(values.contains_key("ab#1#0#x"));
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn series_colors_come_from_the_theme_palettes() {
        for i in 0..6 {
            let c = super::series_color(i);
            assert!(
                crate::theme::GRADIENT.contains(&c) || crate::theme::PROJECT_PALETTE.contains(&c),
                "series {i} uses a theme colour"
            );
        }
    }

    #[test]
    fn neighbouring_series_never_use_neighbouring_gradient_steps() {
        let series = crate::theme::CHART_SERIES;
        // every palette colour exactly once
        for c in crate::theme::PROJECT_PALETTE {
            assert_eq!(series.iter().filter(|&&s| s == c).count(), 1);
        }
        let step = |c: egui::Color32| crate::theme::GRADIENT.iter().position(|&g| g == c);
        for i in 0..series.len() {
            let (a, b) = (super::series_color(i), super::series_color(i + 1));
            assert_ne!(a, b, "series {i} and {} differ", i + 1);
            if let (Some(x), Some(y)) = (step(a), step(b)) {
                assert!(
                    x.abs_diff(y) > 1,
                    "series {i} and {} are neighbouring gradient steps",
                    i + 1
                );
            }
        }
    }

    /// M10: a ```ui block's json parses once per content — the frame asks
    /// again and again, the cache hands the parsed nodes back.
    #[test]
    fn ui_block_parse_is_cached_by_content() {
        let code = r#"[{"type":"divider"},{"type":"text","text":"hi"}]"#;
        let a = super::parse_ui_blocks_cached(code);
        let b = super::parse_ui_blocks_cached(code);
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "the same block content parses once"
        );
        assert!(
            matches!(&(*a)[..], [UiNode::Divider {}, UiNode::Text { text }] if text == "hi"),
            "the cache returns exactly what a fresh parse would"
        );
        assert!(!std::sync::Arc::ptr_eq(
            &a,
            &super::parse_ui_blocks_cached(r#"[{"type":"hr"}]"#)
        ));
    }

    /// N16: a code block's scroll id comes from owner, message, nesting
    /// salt and block position — never from the content, so equally long
    /// blocks no longer share a scroll state and a streamed block keeps
    /// its scroll position chunk by chunk.
    #[test]
    fn code_scroll_ids_are_content_independent() {
        fn scroll_id(owner: &str, salt: u64) -> egui::Id {
            let mut events = Vec::new();
            let mut values = std::collections::HashMap::new();
            let mut ctx = super::RenderCtx::new(&mut events, &mut values, owner);
            ctx.scroll_salt = salt;
            super::code_scroll_id(&ctx, 0, 1)
        }
        assert_eq!(scroll_id("s1", 0), scroll_id("s1", 0));
        assert_ne!(scroll_id("s1", 0), scroll_id("s2", 0), "another owner");
        let mut events = Vec::new();
        let mut values = std::collections::HashMap::new();
        let ctx = super::RenderCtx::new(&mut events, &mut values, "s1");
        let base = super::code_scroll_id(&ctx, 0, 1);
        assert_ne!(base, super::code_scroll_id(&ctx, 0, 2), "block position");
        assert_ne!(base, super::code_scroll_id(&ctx, 1, 1), "another message");
        assert_ne!(base, scroll_id("s1", 7), "nested or file-salted path");
    }

    /// M9: checklist values react locally (the click writes `check:{i}`
    /// into the block's values) and travel with the block's next form
    /// submit — all checkbox states as one `true`/`false` array under
    /// `checklist`. `changed_inputs` (a ui-js tool's state) sends the
    /// same array once the user touched any checkbox. SKILL.md does not
    /// name a concrete shape, so this array form is the documented one.
    #[test]
    fn checklist_states_travel_with_the_block_s_submit() {
        use crate::model::CheckItem;
        use std::collections::HashMap;
        let check = |text: &str, done: bool| CheckItem {
            text: text.into(),
            done,
        };
        let nodes = vec![
            UiNode::Checklist {
                items: vec![
                    check("eins", false),
                    check("zwei", true),
                    check("drei", false),
                ],
            },
            UiNode::Toggle {
                name: "t".into(),
                label: "T".into(),
                value: Some(false),
            },
        ];

        // untouched: the item defaults travel as one array under
        // "checklist" with the block's form submit
        let mut events = Vec::new();
        let mut values = HashMap::new();
        {
            let ctx = super::RenderCtx::new(&mut events, &mut values, "chat");
            let inputs = super::block_inputs(&ctx, 0, 0, &nodes);
            let (_, value) = inputs
                .iter()
                .find(|(n, _)| n == "checklist")
                .expect("the checklist state is part of the block's submit");
            assert_eq!(*value, serde_json::json!([false, true, false]));
            assert!(
                !super::changed_inputs(&ctx, 0, 0, &nodes)
                    .iter()
                    .any(|(n, _)| n == "checklist"),
                "without a user change, no checklist entry reaches the tool state"
            );
        }

        // a click on item 2: the checker painter stores "true" under
        // check:2 of this block — the same rule reads it back
        values.insert(
            super::field_key("chat", 0, 0, "check:2"),
            "true".to_string(),
        );
        let expected = serde_json::json!([false, true, true]);
        {
            let ctx = super::RenderCtx::new(&mut events, &mut values, "chat");
            assert!(
                super::block_inputs(&ctx, 0, 0, &nodes)
                    .iter()
                    .any(|(n, v)| n == "checklist" && *v == expected),
                "the clicked state travels with the submit"
            );
            assert!(
                super::changed_inputs(&ctx, 0, 0, &nodes)
                    .iter()
                    .any(|(n, v)| n == "checklist" && *v == expected),
                "one touched checkbox sends the whole block state"
            );
        }

        // another block's check keys never leak into this block's state,
        // and a block without a checklist sends none
        {
            let mut other = HashMap::new();
            other.insert(
                super::field_key("chat", 0, 1, "check:0"),
                "true".to_string(),
            );
            let ctx = super::RenderCtx::new(&mut events, &mut other, "chat");
            let inputs = super::block_inputs(&ctx, 0, 0, &nodes);
            let (_, value) = inputs
                .iter()
                .find(|(n, _)| n == "checklist")
                .expect("the checklist still travels");
            assert_eq!(
                *value,
                serde_json::json!([false, true, false]),
                "the foreign block's check keys are not this block's state"
            );
            assert!(
                !super::changed_inputs(&ctx, 0, 0, &nodes)
                    .iter()
                    .any(|(n, _)| n == "checklist"),
                "only this block's own touched checkboxes count as changes"
            );
            let plain = vec![UiNode::Toggle {
                name: "t".into(),
                label: "T".into(),
                value: Some(false),
            }];
            assert!(
                !super::block_inputs(&ctx, 0, 0, &plain)
                    .iter()
                    .any(|(n, _)| n == "checklist"),
                "a block without a checklist sends no checklist entry"
            );
        }
    }

    /// N17: a ```ui block nested in a text node of another ui block used
    /// to restart the block counter at 0, colliding its field keys with
    /// the outer block 0 of the same message — two fields "x" shared one
    /// stored value. The outer nodes are built directly: parse_ui_blocks
    /// (model.rs) stops at any ``` sequence inside the json, so a text
    /// node with a fence arrives through ui-js-built nodes, not through
    /// the wire parse; the numbering under test lives here either way.
    /// Rendered headless with real clicks on both toggles: each keeps
    /// its own entry. Final audit G: the nested run counts in its own
    /// space and cannot shift the outer numbers painted after it.
    #[test]
    fn nested_ui_blocks_keep_their_own_field_values() {
        use std::collections::HashMap;
        let nested = concat!(
            "```ui\n",
            "[{\"type\":\"toggle\",\"name\":\"x\",\"label\":\"Innen\"}]\n",
            "```\n"
        );
        let outer = vec![
            UiNode::Card {
                title: Some("Card".to_string()),
                children: vec![UiNode::Text {
                    text: nested.to_string(),
                }],
            },
            UiNode::Toggle {
                name: "x".to_string(),
                label: "Aussen".to_string(),
                value: Some(false),
            },
        ];
        let ctx = egui::Context::default();
        // the app installs its fonts on creation (main.rs); a bare
        // Context cannot lay the labels out without them
        crate::fonts::install_fallback_fonts(&ctx);
        let mut ui_events: Vec<UiEvent> = Vec::new();
        let mut values: HashMap<String, String> = HashMap::new();
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let frame = |ctx_run: &egui::Context,
                     events: Vec<egui::Event>,
                     ui_events: &mut Vec<UiEvent>,
                     values: &mut HashMap<String, String>|
         -> Vec<egui::epaint::ClippedShape> {
            let out = ctx_run.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut rctx = super::RenderCtx::new(ui_events, values, "s");
                        // the numbering `render_blocks` applies to its
                        // first ```ui block: it takes 0 and advances the
                        // running counter before its nodes paint
                        let block = rctx.block_offset;
                        rctx.block_offset += 1;
                        super::render_ui_block(ui, &outer, 0, block, &mut rctx);
                    });
                },
            );
            out.shapes
        };
        // settle pass: locate both toggle labels in the painted text
        let shapes = frame(&ctx, vec![], &mut ui_events, &mut values);
        let label_pos = |shapes: &[egui::epaint::ClippedShape], needle: &str| -> egui::Pos2 {
            shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t),
                    _ => None,
                })
                .find(|t| t.galley.job.text.contains(needle))
                .map(|t| t.pos + egui::vec2(4.0, 4.0))
                .unwrap_or_else(|| panic!("the label {needle} is painted"))
        };
        let click = |pos: egui::Pos2,
                     ui_events: &mut Vec<UiEvent>,
                     values: &mut HashMap<String, String>| {
            let press = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            };
            let release = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            };
            frame(
                &ctx,
                vec![egui::Event::PointerMoved(pos)],
                ui_events,
                values,
            );
            frame(&ctx, vec![press], ui_events, values);
            frame(&ctx, vec![release], ui_events, values);
        };
        let aussen = label_pos(&shapes, "Aussen");
        let innen = label_pos(&shapes, "Innen");
        click(aussen, &mut ui_events, &mut values);
        assert_eq!(
            values
                .get(&super::field_key("s", 0, 0, "x"))
                .map(String::as_str),
            Some("true"),
            "the outer toggle stored its value under its own block"
        );
        click(innen, &mut ui_events, &mut values);
        // the nested block numbers in its own space (1_000_000 + step of
        // its host block 0): it cannot collide with the outer numbers
        assert_eq!(
            values
                .get(&super::field_key("s", 0, 1_000_000, "x"))
                .map(String::as_str),
            Some("true"),
            "the nested toggle stored its value under its own number"
        );
        assert_eq!(
            values.len(),
            2,
            "both fields keep their own value: {values:?}"
        );
    }

    /// Final audit G: an earlier block's nested ```ui run once shifted
    /// every outer block number after it, so the same later block got
    /// different field/script keys depending on how much a SIBLING
    /// contained — form values jumped between variants of the same
    /// message. The offset after painting must depend on the outer
    /// blocks alone.
    #[test]
    fn nested_blocks_do_not_shift_the_outer_numbering() {
        use std::collections::HashMap;
        let nested_text = concat!(
            "```ui\n",
            "[{\"type\":\"toggle\",\"name\":\"n\",\"label\":\"N\"}]\n",
            "```\n"
        );
        let outer_blocks = |text: &str| {
            vec![
                UiNode::Card {
                    title: Some("Card".to_string()),
                    children: vec![UiNode::Text {
                        text: text.to_string(),
                    }],
                },
                UiNode::Toggle {
                    name: "y".to_string(),
                    label: "Y".to_string(),
                    value: Some(false),
                },
            ]
        };
        let paint = |text: &str| -> usize {
            let ctx = egui::Context::default();
            crate::fonts::install_fallback_fonts(&ctx);
            let mut ui_events: Vec<UiEvent> = Vec::new();
            let mut values: HashMap<String, String> = HashMap::new();
            let mut offset = 0;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut rctx = super::RenderCtx::new(&mut ui_events, &mut values, "s");
                        // render_ui_block consumes its block number itself
                        let block = rctx.block_offset;
                        rctx.block_offset += 1;
                        super::render_ui_block(ui, &outer_blocks(text), 0, block, &mut rctx);
                        offset = rctx.block_offset;
                    });
                },
            );
            offset
        };
        let with_nested = paint(nested_text);
        let without_nested = paint("plain text");
        // one outer block at the top level, whatever its text nests
        assert_eq!(
            with_nested, without_nested,
            "a nested ui run must not consume the outer counter"
        );
        assert_eq!(
            with_nested, 1,
            "only the outer block numbered at the top level"
        );
    }

    #[test]
    fn workspace_file_resolution_resolves_only_existing_files_inside_cwd() {
        let dir = std::env::temp_dir().join(format!("cw-chips-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("analysis.py"), "x").unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "x").unwrap();
        let cwd = dir.canonicalize().unwrap();
        // relative paths and inline-code punctuation resolve
        assert_eq!(
            super::resolve_workspace_file(&cwd, "analysis.py").unwrap(),
            cwd.join("analysis.py")
        );
        assert_eq!(
            super::resolve_workspace_file(&cwd, "`analysis.py`").unwrap(),
            cwd.join("analysis.py")
        );
        assert_eq!(
            super::resolve_workspace_file(&cwd, "src/main.rs").unwrap(),
            cwd.join("src").join("main.rs")
        );
        assert_eq!(
            super::resolve_workspace_file(&cwd, "./analysis.py").unwrap(),
            cwd.join("analysis.py")
        );
        // an absolute path inside the workspace resolves
        let abs = cwd.join("analysis.py");
        assert_eq!(
            super::resolve_workspace_file(&cwd, abs.to_str().unwrap()).unwrap(),
            abs
        );
        // outside the workspace stays plain text
        assert!(super::resolve_workspace_file(&cwd, "/etc/passwd").is_none());
        assert!(super::resolve_workspace_file(&cwd, "../outside.py").is_none());
        // function names and symbols are not candidates
        assert!(super::resolve_workspace_file(&cwd, "daily_stats").is_none());
        assert!(super::resolve_workspace_file(&cwd, "load(path)").is_none());
        // path-like but not existing stays plain text
        assert!(super::resolve_workspace_file(&cwd, "missing.py").is_none());
        // sentence and list punctuation around a path is not part of it
        assert_eq!(
            super::resolve_workspace_file(&cwd, "analysis.py.").unwrap(),
            cwd.join("analysis.py")
        );
        assert_eq!(
            super::resolve_workspace_file(&cwd, "(src/main.rs),").unwrap(),
            cwd.join("src").join("main.rs")
        );
        assert_eq!(super::chip_core("(src/main.rs),"), "src/main.rs");
        assert_eq!(super::chip_core("analysis.py. "), "analysis.py");
        // a workspace opened through a symlink resolves the same files
        let link = std::env::temp_dir().join(format!("cw-chips-link-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&cwd, &link).unwrap();
        assert_eq!(
            super::resolve_workspace_file(&link, "analysis.py").unwrap(),
            cwd.join("analysis.py")
        );
        let via_link = link.join("analysis.py");
        assert!(super::resolve_workspace_file(&link, via_link.to_str().unwrap()).is_some());
        let _ = std::fs::remove_file(&link);
        assert!(super::resolve_workspace_file(&cwd, "2.5").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A symlink inside the workspace that points outside must not become a
    /// chip: `metadata` follows links, so the existence check alone passes;
    /// the canonical re-check rejects the candidate (audit 2026-11: Opus'
    /// finding on `resolve_workspace_file`).
    #[test]
    fn workspace_file_resolution_rejects_symlinks_pointing_outside() {
        let base = std::env::temp_dir().join(format!("cw-chips-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let ws = base.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let outside = base.join("outside.txt");
        std::fs::write(&outside, "secret\n").unwrap();
        std::fs::write(ws.join("real.txt"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("link.txt")).unwrap();
        // a linked folder: `dirlink/f.txt` is lexically inside the workspace
        let real_dir = base.join("real-dir");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::write(real_dir.join("f.txt"), "x").unwrap();
        std::os::unix::fs::symlink(&real_dir, ws.join("dirlink")).unwrap();
        let cwd = ws.canonicalize().unwrap();
        assert!(super::resolve_workspace_file(&cwd, "real.txt").is_some());
        assert!(
            super::resolve_workspace_file(&cwd, "link.txt").is_none(),
            "a symlink to outside the workspace is no chip"
        );
        assert!(
            super::resolve_workspace_file(&cwd, "dirlink/f.txt").is_none(),
            "a path through a linked folder is no chip either"
        );
        assert!(
            super::is_outside_workspace(&cwd, &ws.join("link.txt")),
            "the helper sees the symlink target outside"
        );
        assert!(!super::is_outside_workspace(&cwd, &ws.join("real.txt")));
        // the cached variant used per frame agrees with the direct check
        assert!(super::outside_workspace_cached(&cwd, &ws.join("link.txt")));
        assert!(!super::outside_workspace_cached(&cwd, &ws.join("real.txt")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
