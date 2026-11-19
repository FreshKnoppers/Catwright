//! embedded pure-Rust engine `boa_engine`, returns native `UiNode`s.
//!
//! Contract (also in vibe-skills/catwright-ui/SKILL.md):
//! - `function render(state)` (required) returns one node or an array.
//! - `function init()` (optional) returns the initial state object.
//! - `function onAction(action, state)` (optional): returning an object makes
//!   it the new state (handled locally); returning nothing sends the click
//!   to the agent.
//!
//! Sandbox: boa without any host functions, so no console, network, files
//! or clock beyond what the language itself offers. Loop and recursion
//! limits stop runaway scripts; a prelude caps the common memory hogs
//! (`repeat`, `padStart/End`, huge `Array(n)`/`fill`/`Array.from`). Data
//! crosses the boundary only as JSON strings.
//!
//! Every evaluation runs in a fresh **worker child process**
//! (`catwright --ui-js-worker`): one JSON request on stdin, one JSON
//! response on stdout, then exit. The parent waits at most `EVAL_TIMEOUT`
//! and kills the worker on timeout, so a script with heavy per-iteration
//! work can no longer block the UI thread forever (Handoff 2.7): the
//! pinned `boa_engine` 0.22 has no interrupt/abort hook (its
//! `RuntimeLimits` are purely counter-based), so the wall-clock budget is
//! enforced from outside the engine. An out-of-control allocation kills
//! the worker (child process), not the app; there is no `RLIMIT_AS`
//! because that would need a new crate. Cost: one process spawn per
//! evaluation. While the user drags a slider, re-evaluations are
//! additionally throttled to one per `DRAG_EVAL_GAP`.

use crate::i18n::{lang, parse_po, tr, tr_in, trf, Lang};
use crate::model::UiNode;
use boa_engine::{Context, Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Maximum script size in characters.
pub const MAX_SOURCE_CHARS: usize = 20_000;
/// Maximum size of one render result (JSON text).
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;
/// Maximum number of nodes in one render result (counted recursively).
pub const MAX_NODES: usize = 2_000;
const LOOP_LIMIT: u64 = 1_000_000;
const RECURSION_LIMIT: usize = 256;

/// CLI flag that turns the binary into one ui-js evaluation worker.
pub const WORKER_FLAG: &str = "--ui-js-worker";

/// Hard wall-clock budget for one worker run (process spawn plus
/// evaluation). Legit skill scripts evaluate in single-digit
/// milliseconds; a script near the loop limit with heavy work per
/// iteration may take a moment longer, so the budget is generous —
/// but finite: on expiry the worker is killed and the block shows an
/// error instead of freezing the UI thread forever (Handoff 2.7).
pub const EVAL_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the parent checks whether the worker has finished.
const WAIT_TICK: Duration = Duration::from_millis(5);
/// Maximum serialized size of one tool's state (JSON text). The state is
/// interaction data — a few hundred fields, not bulk storage — and it
/// crosses the wire with every request and response, so 64 KiB leaves
/// room for realistic forms (thousands of fields or a few thousand lines
/// of text) while keeping both directions bounded. `set_value` enforces
/// this cap: a field that would blow it is rejected on the spot instead
/// of letting the request die at the worker's read cap later.
pub const MAX_STATE_BYTES: usize = 64 * 1024;
/// One worker REQUEST fits in this many bytes: worst case the source is
/// up to [`MAX_SOURCE_CHARS`] characters, each six serialized bytes
/// (every character may need a `\uXXXX` escape), the state is at most
/// [`MAX_OUTPUT_BYTES`] when an oversized `init()` state was adopted
/// (`eval_json` caps it there), the envelope keys and the action string
/// live in the slack:
/// 6 x 20 000 + 256 KiB + 16 KiB = 389 KiB.
const MAX_REQUEST_BYTES: u64 = 6 * MAX_SOURCE_CHARS as u64 + MAX_OUTPUT_BYTES as u64 + 16 * 1024;
/// One worker RESPONSE fits in this many bytes: a legal answer may carry
/// two output-sized payloads at once — `nodes` and the echoed `state`
/// each reach [`MAX_OUTPUT_BYTES`] (`eval_json` caps both; `init()` and
/// `onAction()` results are output, too) — plus [`MAX_STATE_BYTES`] of
/// headroom and slack for the envelope keys:
/// 2 x 256 KiB + 64 KiB + 16 KiB = 592 KiB. The request budget is NOT
/// enough here: nodes plus the state echo exceed it, so a response
/// above the request budget but below this budget must still arrive.
/// Everything past this budget is truncated on purpose (the reader
/// drops the pipe) and reported as the size problem it is (see
/// [`run_worker`]).
const MAX_RESPONSE_BYTES: u64 = 2 * MAX_OUTPUT_BYTES as u64 + MAX_STATE_BYTES as u64 + 16 * 1024;

/// Minimum wall-clock gap between two evaluations of a script while one
/// of its sliders is being dragged: egui paints one frame per pointer
/// change, and without this gap every frame would re-run the script
/// (one worker spawn per evaluation, Handoff 2.7). 100 ms ≈ 10 updates
/// per second, enough for live feedback. The moment the drag ends, the
/// pending value is always evaluated (see [`should_evaluate`]).
const DRAG_EVAL_GAP: Duration = Duration::from_millis(100);

/// How long a failed evaluation waits before its single retry (M2):
/// the common transient causes — a busy machine, a spawn hiccup — clear
/// within a second or two. The retry fires only when the UI paints again
/// after this gap (no timer of its own), at most once per failure: it
/// must never turn into a per-frame respawn loop. Only a state or code
/// change starts a new attempt cycle.
const RETRY_COOLDOWN: Duration = Duration::from_secs(2);

/// Caps the usual ways to allocate huge strings/arrays by accident.
const PRELUDE: &str = r#"(function () {
  "use strict";
  const MAX = 100000;
  const big = (n) => Number(n) > MAX;
  const rep = String.prototype.repeat;
  Object.defineProperty(String.prototype, "repeat", { value: function (n) {
    if (this.length * Number(n) > MAX) throw new RangeError("repeat: result too large");
    return rep.call(this, n);
  }, writable: false, configurable: false });
  for (const k of ["padStart", "padEnd"]) {
    const f = String.prototype[k];
    Object.defineProperty(String.prototype, k, { value: function (n, s) {
      if (big(n)) throw new RangeError(k + ": result too large");
      return f.call(this, n, s);
    }, writable: false, configurable: false });
  }
  const fill = Array.prototype.fill;
  Object.defineProperty(Array.prototype, "fill", { value: function (...a) {
    if (big(this.length)) throw new RangeError("fill: array too large");
    return fill.apply(this, a);
  }, writable: false, configurable: false });
  const A = Array;
  const from = A.from;
  const W = function (...args) {
    if (args.length === 1 && typeof args[0] === "number" && big(args[0]))
      throw new RangeError("Array: too large");
    return new A(...args);
  };
  W.prototype = A.prototype;
  Object.setPrototypeOf(W, A);
  W.from = function (obj, ...rest) {
    if (obj != null && big(obj.length)) throw new RangeError("Array.from: too large");
    return from.call(A, obj, ...rest);
  };
  globalThis.Array = W;
})();"#;

/// What happened with a button click inside a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionOutcome {
    /// `onAction` returned a new state; the tool re-renders locally.
    Local,
    /// Not handled by the script: send the click to the agent.
    SendToAgent,
}

// ---------------------------------------------------------------------------
// Worker protocol: one JSON request line in, one JSON response line out.
// ---------------------------------------------------------------------------

/// A request the parent sends to the worker. `init` means: seed the state
/// from `init()` (falling back to `{}`) instead of using `state`.
#[derive(Serialize)]
struct WorkerRequest<'a> {
    source: &'a str,
    state: &'a Value,
    init: bool,
    action: Option<&'a str>,
}

/// The owned form the worker parses.
#[derive(Deserialize)]
struct WorkerInput {
    source: String,
    state: Value,
    init: bool,
    action: Option<String>,
}

/// The worker's answer: `nodes` for a render, `outcome` (`local`/`send`)
/// for an action, `error` when `ok` is false. `load` marks errors that
/// happened while loading (prelude/source/init) — the parent swallows
/// those on button clicks, exactly like the in-process engine did.
#[derive(Default, Debug, Serialize, Deserialize)]
struct WorkerResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    nodes: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    load: Option<bool>,
}

fn error_response(error: String, load: bool) -> WorkerResponse {
    WorkerResponse {
        ok: false,
        error: Some(error),
        load: Some(load),
        ..Default::default()
    }
}

/// Evaluate one request with the in-process engine. This is the whole
/// worker logic minus the I/O; the `--ui-js-worker` CLI (see
/// [`worker_main`]) wraps it, and tests drive it directly.
fn dispatch(input: &WorkerInput) -> WorkerResponse {
    let mut engine = match ScriptEngine::new(&input.source) {
        Ok(engine) => engine,
        Err(e) => return error_response(e, true),
    };
    let state = if input.init {
        match engine.init_state() {
            Ok(Some(state)) => state,
            Ok(None) => input.state.clone(),
            Err(e) => return error_response(e, true),
        }
    } else {
        input.state.clone()
    };
    match input.action.as_deref() {
        Some(action) => match engine.on_action(action, &state) {
            Ok(Some(new_state)) => WorkerResponse {
                ok: true,
                state: Some(new_state),
                outcome: Some("local".into()),
                ..Default::default()
            },
            Ok(None) => WorkerResponse {
                ok: true,
                state: Some(state),
                outcome: Some("send".into()),
                ..Default::default()
            },
            Err(e) => error_response(e, false),
        },
        None => match engine.render(&state) {
            Ok(nodes) => WorkerResponse {
                ok: true,
                state: Some(state),
                nodes: Some(nodes),
                ..Default::default()
            },
            Err(e) => error_response(e, false),
        },
    }
}

/// The `--ui-js-worker` mode: read one JSON request, evaluate it, write
/// one JSON response, done. Generic over reader/writer so tests can run
/// it against cursors.
pub fn worker_main(reader: impl Read, mut writer: impl Write) {
    let mut request = String::new();
    // one byte PAST the request budget: that is enough to tell an
    // oversized request (stopped at the cap) apart from a malformed one
    // when reporting the cause
    let read = reader
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_string(&mut request)
        .unwrap_or(0);
    let response = if read as u64 > MAX_REQUEST_BYTES {
        error_response(tr("Script error: request too large.").to_string(), true)
    } else {
        match serde_json::from_str::<WorkerInput>(request.trim()) {
            Ok(input) => dispatch(&input),
            // the worker child has no settings, so its messages are
            // English msgids on the wire; the parent translates the
            // known ones for display (translate_worker_error_in)
            Err(_) => error_response(tr("Script error: unreadable request.").to_string(), true),
        }
    };
    if let Ok(text) = serde_json::to_string(&response) {
        let _ = writeln!(writer, "{text}");
    }
}

// ---------------------------------------------------------------------------
// Parent side: spawn the worker, enforce the wall-clock budget.
// ---------------------------------------------------------------------------

/// How [`ScriptTool`] reaches an evaluation: a request goes in, a
/// response or a transport-level error comes out.
type EvalFn = for<'a> fn(&'a WorkerRequest<'a>) -> Result<WorkerResponse, String>;

/// Run `cmd` as one evaluation worker: write `request`, wait at most
/// `timeout`, kill on expiry, parse the response. The worker is killed
/// and waited for on every branch, so no child outlives the call.
fn run_worker(
    cmd: &mut Command,
    request: &str,
    timeout: Duration,
) -> Result<WorkerResponse, String> {
    // reject an oversized request BEFORE spawning: sending it anyway
    // would truncate it at the worker's read cap, and the honest cause
    // is the size, not an "unreadable request" at the worker (+ 1 for the
    // trailing newline the send below appends)
    if request.len() as u64 + 1 > MAX_REQUEST_BYTES {
        return Err(tr("Script error: request too large.").to_string());
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            tr("Script error: the evaluation process could not be started.").to_string()
        })?;
    {
        // closing stdin tells the worker the request is complete
        let mut stdin = child.stdin.take().expect("stdin piped");
        let _ = stdin.write_all(request.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    // stdout must be drained WHILE the child runs: a response bigger than
    // the pipe buffer (~64 KiB) otherwise blocks the worker forever and the
    // wait below turns into the timeout (Handoff 2026-11-09, Bug 1).
    let stdout = child.stdout.take().expect("stdout piped");
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        // the RESPONSE budget is wider than the request budget: a legal
        // answer may carry nodes plus a state echo (see MAX_RESPONSE_BYTES)
        let _ = stdout.take(MAX_RESPONSE_BYTES).read_to_end(&mut out);
        let _ = tx.send(out);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(WAIT_TICK),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(trf(
                    "Script error: time limit exceeded ({secs} s).",
                    &[("secs", &format!("{:.0}", timeout.as_secs_f32()))],
                ));
            }
            Err(_) => {
                // try_wait itself failed: the child is in an unknown state,
                // so keep the doc promise above — killed and waited for
                // on every branch — and not leave it behind un-reaped
                let _ = child.kill();
                let _ = child.wait();
                return Err(tr("Script error: the evaluation process failed.").into());
            }
        }
    };
    if !status.success() {
        return Err(tr("Script error: the evaluation process crashed.").into());
    }
    let out = rx
        .recv()
        .map_err(|_| tr("Script error: no answer from the evaluation process.").to_string())?;
    match serde_json::from_slice::<WorkerResponse>(&out) {
        Ok(resp) => Ok(resp),
        // the reader stopped at the response budget: the worker's answer
        // is simply too big — the honest cause is the size, while the
        // worker itself exits 0 (its writeln dies of EPIPE, swallowed)
        Err(_) if out.len() as u64 >= MAX_RESPONSE_BYTES => {
            Err(tr("Script error: the answer from the evaluation process is too large.").into())
        }
        Err(_) => Err(tr("Script error: no valid answer from the evaluation process.").into()),
    }
}

/// Evaluate a request in a fresh worker (`catwright --ui-js-worker`).
fn eval_in_worker(req: &WorkerRequest<'_>) -> Result<WorkerResponse, String> {
    let exe = std::env::current_exe()
        .map_err(|_| tr("Script error: the program path cannot be determined.").to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg(WORKER_FLAG);
    let request = serde_json::to_string(req)
        .map_err(|_| tr("Script error: the request cannot be serialised."))?;
    run_worker(&mut cmd, &request, EVAL_TIMEOUT)
}

/// Evaluate a request in-process with the same engine the worker uses.
#[cfg(test)]
fn eval_in_process(req: &WorkerRequest<'_>) -> Result<WorkerResponse, String> {
    let input = WorkerInput {
        source: req.source.to_string(),
        state: req.state.clone(),
        init: req.init,
        action: req.action.map(str::to_string),
    };
    Ok(dispatch(&input))
}

// ---------------------------------------------------------------------------
// Worker errors become display text in the parent.
// ---------------------------------------------------------------------------

/// The German catalog for [`translate_worker_error_in`]. `tr`/`trf` take
/// compile-time literals only (their result borrows `&'static`), while a
/// worker error is a runtime string off the wire — this parses the same
/// `de.po` the i18n module embeds, so the catalog stays the one source
/// of the translations.
fn german_catalog() -> &'static HashMap<String, String> {
    static DE: OnceLock<HashMap<String, String>> = OnceLock::new();
    DE.get_or_init(|| parse_po(include_str!("../i18n/de.po")))
}

/// Worker error msgids whose `{…}` arguments the child fills in before
/// sending: the child has no settings, so `trf` there resolves to the
/// English template with the values in place, and the catalog lookup
/// below misses them. `translate_worker_error_in` recovers the arguments
/// from the English text and re-renders the template through the catalog.
/// Every new parametrized worker error must be added here — otherwise
/// only its `Script error:` head localises (final audit D).
const PARAMETRIZED_WORKER_MSGIDS: &[&str] = &[
    "Script error: invalid JSON: {detail}",
    "Script error: source too long (max. {chars} characters).",
    "Script error: too many nodes ({count}, max. {max}).",
    "Script error: result too large (max. {kib} KiB).",
];

/// Read `template`'s `{name}` arguments back out of the English text the
/// child sent. Literals must match; an inner placeholder consumes the
/// text up to the next template literal (`{a}{b}` without a literal
/// between is ambiguous and therefore rejected), a placeholder at the
/// end of the template takes the rest. Pure, so the recovery is
/// unit-tested.
fn template_args<'a>(template: &'a str, text: &str) -> Option<Vec<(&'a str, String)>> {
    let mut args = Vec::new();
    let mut tmpl: &'a str = template;
    let mut rest = text;
    loop {
        // the literals up to the next placeholder (or the template's end)
        let (literal, tail) = match tmpl.find('{') {
            Some(open) => (&tmpl[..open], &tmpl[open..]),
            None => (tmpl, ""),
        };
        if !rest.starts_with(literal) {
            return None;
        }
        rest = &rest[literal.len()..];
        if tail.is_empty() {
            return if rest.is_empty() { Some(args) } else { None };
        }
        // the placeholder's name runs to the closing brace in the template
        let close = tail.find('}')?;
        let name = &tail[1..close];
        tmpl = &tail[close + 1..];
        // an inner placeholder ends where the template continues: the
        // literals up to the NEXT placeholder must open it
        let next_open = tmpl.find('{');
        let next_lit = &tmpl[..next_open.unwrap_or(tmpl.len())];
        if next_lit.is_empty() {
            if next_open.is_some() {
                return None;
            }
            args.push((name, rest.to_string()));
            return Some(args);
        }
        let at = rest.find(next_lit)?;
        args.push((name, rest[..at].to_string()));
        rest = &rest[at..];
    }
}

/// One worker error in the UI language of the parent. The engine — and
/// with it every error text it throws — runs in the worker child, which
/// has no settings and therefore no UI language: its texts arrive as
/// English msgids on the wire. The parent owns the language and the
/// catalog, so this is where they become display text:
/// - a known msgid turns German through the catalog;
/// - a parametrized msgid (arguments filled in by the child) is matched
///   against the known templates and re-rendered through the catalog;
/// - an unknown text under our own `Script error:` head (raw engine
///   failures) keeps its foreign detail and only has the head
///   re-composed from the catalog;
/// - anything else is returned unchanged.
fn translate_worker_error_in(language: Lang, error: String) -> String {
    if language == Lang::En {
        return error;
    }
    if let Some(known) = german_catalog()
        .get(&error)
        .filter(|translated| !translated.is_empty())
    {
        return known.clone();
    }
    // the child fills the template's arguments before sending, so the
    // filled text misses the catalog: recover the arguments and render
    // the template in the UI language
    for template in PARAMETRIZED_WORKER_MSGIDS {
        if let Some(args) = template_args(template, &error) {
            let named: Vec<(&str, &dyn std::fmt::Display)> = args
                .iter()
                .map(|(name, value)| (*name, value as &dyn std::fmt::Display))
                .collect();
            // language-aware, unlike `trf`: the catalog lookup must use
            // THIS function's language, not the ambient one a unit test
            // runs in
            return crate::i18n::fill(tr_in(language, template), &named);
        }
    }
    // the head the child's script_error_line composes is English there
    // (it has no settings): translate our own head around the detail
    let head = format!("{}: ", tr_in(Lang::En, "Script error"));
    if let Some(detail) = error.strip_prefix(&head) {
        return format!("{}: {detail}", tr_in(language, "Script error"));
    }
    error
}

/// The current UI language's form of a worker error; called where the
/// parent stores a worker error for display, never inside the worker.
fn translate_worker_error(error: String) -> String {
    translate_worker_error_in(lang(), error)
}

// ---------------------------------------------------------------------------
// ScriptTool: the block the renderer paints.
// ---------------------------------------------------------------------------

/// One ```ui-js block with its state. Evaluation goes through `eval`
/// (the worker child process in production).
pub struct ScriptTool {
    source: String,
    state: Value,
    /// `init()` has not seeded the state yet (first evaluation).
    init_pending: bool,
    last: Result<Vec<UiNode>, String>,
    dirty: bool,
    /// When the last evaluation finished (throttle anchor, Handoff 2.7).
    last_eval: Option<Instant>,
    /// One bounded retry may follow a failed evaluation
    /// ([`RETRY_COOLDOWN`] later); the budget is spent by the retry
    /// itself and refreshed only by a new evaluation (M2).
    retry_available: bool,
    /// A slider of this block was dragged during the last paint (the
    /// renderer feeds this in after painting, `set_dragging`).
    dragging: bool,
    eval: EvalFn,
    /// A background evaluation in flight (production): the paint thread
    /// only spawns and polls, it never waits for the worker — one hostile
    /// block near the loop limit must not freeze the UI for `EVAL_TIMEOUT`
    /// on every first paint (N blocks would be N x 2 s). The answer lands
    /// on this channel and is picked up by a later `nodes_at`.
    pending: Option<PendingEval>,
    /// Generation counter: bumped by every spawn AND every state change
    /// (`set_value`, `on_action`). A result whose generation is stale is
    /// dropped instead of overwriting newer state (race: state changed
    /// while the worker was running).
    gen: u64,
    /// Tests evaluate synchronously (`with_eval`), so their virtual-clock
    /// assertions stay deterministic; production (`new`) goes async.
    sync: bool,
}

/// Concurrent ui-js workers allowed app-wide: each is a process with a boa
/// engine. The per-harness tool cap bounds the total; this bounds the
/// in-flight evaluations.
const MAX_CONCURRENT_WORKERS: usize = 4;

fn workers_running() -> &'static std::sync::atomic::AtomicUsize {
    static W: std::sync::OnceLock<std::sync::atomic::AtomicUsize> = std::sync::OnceLock::new();
    W.get_or_init(|| std::sync::atomic::AtomicUsize::new(0))
}

/// The receiver of a pending evaluation plus the generation it was spawned
/// at; dropping releases the worker slot (even if the tool goes away).
struct PendingEval {
    rx: std::sync::mpsc::Receiver<Result<WorkerResponse, String>>,
    gen: u64,
}

impl Drop for PendingEval {
    fn drop(&mut self) {
        workers_running().fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl ScriptTool {
    pub fn new(source: &str) -> Self {
        Self::with_eval_mode(source, eval_in_worker, false)
    }

    #[cfg(test)]
    fn with_eval(source: &str, eval: EvalFn) -> Self {
        Self::with_eval_mode(source, eval, true)
    }

    fn with_eval_mode(source: &str, eval: EvalFn, sync: bool) -> Self {
        Self {
            source: source.to_string(),
            state: Value::Object(Default::default()),
            init_pending: true,
            last: Ok(Vec::new()),
            dirty: true,
            last_eval: None,
            retry_available: false,
            dragging: false,
            eval,
            pending: None,
            gen: 0,
            sync,
        }
    }

    /// The current render result; re-renders only when the state changed.
    /// While `set_dragging(true)` is in effect, re-renders are throttled
    /// to one per `DRAG_EVAL_GAP`. After a failed evaluation, one retry
    /// fires on the first paint at least [`RETRY_COOLDOWN`] later; after
    /// that the error stays until the state (or code) changes (M2).
    pub fn nodes(&mut self) -> &Result<Vec<UiNode>, String> {
        self.nodes_at(Instant::now())
    }

    /// The same, with the clock passed in (pure enough to unit-test the
    /// throttle without sleeping).
    fn nodes_at(&mut self, now: Instant) -> &Result<Vec<UiNode>, String> {
        if self.sync {
            // test path: evaluate inline so the clock assertions below
            // stay deterministic
            if should_evaluate(self.dirty, self.last_eval, now, self.dragging) {
                self.dirty = false;
                self.last_eval = Some(now);
                self.last = self.render_now();
                // a failed evaluation (transient or not) earns one bounded
                // retry; a success spends the budget (M2)
                self.retry_available = self.last.is_err();
            } else if should_retry(
                self.last.is_err(),
                self.retry_available,
                self.last_eval,
                now,
            ) {
                self.last_eval = Some(now);
                self.retry_available = false;
                self.last = self.render_now();
            }
            return &self.last;
        }
        // production path: poll a running background evaluation, never
        // block the paint thread on it
        if let Some(p) = self.pending.take() {
            match p.rx.try_recv() {
                Ok(res) => {
                    if p.gen == self.gen {
                        self.last = self.finish_render(res);
                        self.retry_available = self.last.is_err();
                    }
                    // else: stale — the state changed while the worker ran;
                    // the answer is dropped, the change re-evaluates below
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // still running: put the receiver back, the caller
                    // repaints (eval_pending) and a later frame polls again
                    self.pending = Some(p);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if p.gen == self.gen {
                        self.last =
                            Err(tr("Script error: the evaluation process died.").to_string());
                        self.retry_available = true;
                    }
                }
            }
        }
        if self.pending.is_none() && should_evaluate(self.dirty, self.last_eval, now, self.dragging)
        {
            if let Some(p) = self.spawn_render() {
                self.dirty = false;
                self.last_eval = Some(now);
                self.pending = Some(p);
                self.retry_available = false;
            }
            // else: worker cap reached — stay dirty, the next frame retries
        } else if self.pending.is_none()
            && should_retry(
                self.last.is_err(),
                self.retry_available,
                self.last_eval,
                now,
            )
        {
            // the bounded wakeup of M2, same rule as the sync path: one
            // retry per failure, never before RETRY_COOLDOWN, and it can
            // never become a per-frame respawn loop
            if let Some(p) = self.spawn_render() {
                self.last_eval = Some(now);
                self.retry_available = false;
                self.pending = Some(p);
            }
        }
        &self.last
    }

    /// A changed state could not be evaluated yet (throttled away during
    /// a drag, or a background evaluation still runs): the caller should
    /// request a repaint so the update is applied as soon as it can, and
    /// never stays stale.
    pub fn eval_pending(&self) -> bool {
        self.dirty || self.pending.is_some()
    }

    /// Whether a slider of this block is being dragged. The renderer calls
    /// this after painting the block, so the flag lags one frame behind
    /// the pointer; the drag's first frame evaluates immediately.
    pub fn set_dragging(&mut self, dragging: bool) {
        self.dragging = dragging;
    }

    pub fn state(&self) -> &Value {
        &self.state
    }

    /// Set one state field (from an input widget); re-renders on change.
    /// The state crosses the worker wire with every request, so its
    /// serialized size is capped at [`MAX_STATE_BYTES`]: a field that
    /// would blow the cap is rejected on the spot with the real cause
    /// instead of letting the request die at the worker's read cap later.
    pub fn set_value(&mut self, name: &str, value: Value) {
        if self.state.get(name) == Some(&value) {
            return;
        }
        let mut next = self.state.clone();
        let Some(map) = next.as_object_mut() else {
            return;
        };
        map.insert(name.to_string(), value);
        if serde_json::to_string(&next)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
            > MAX_STATE_BYTES
        {
            // the insert is reverted: the state stays as it was, the
            // error card carries the reason until a change fits again
            self.last = Err(trf(
                "Script error: the state is too large (max. {kib} KiB).",
                &[("kib", &(MAX_STATE_BYTES / 1024))],
            ));
            return;
        }
        self.state = next;
        self.gen += 1; // an in-flight render result is now stale
        self.dirty = true;
    }

    /// Run `onAction(action, state)`. An object result becomes the new state.
    pub fn on_action(&mut self, action: &str) -> ActionOutcome {
        let request = self.request(Some(action));
        match (self.eval)(&request) {
            Ok(resp) if resp.ok => {
                self.adopt(&resp);
                self.gen += 1; // an in-flight render result is now stale
                if resp.outcome.as_deref() == Some("local") {
                    self.dirty = true;
                    ActionOutcome::Local
                } else {
                    ActionOutcome::SendToAgent
                }
            }
            // a script that never loaded cannot handle clicks: pass them on
            Ok(resp) if resp.load == Some(true) => ActionOutcome::SendToAgent,
            Ok(resp) => {
                // worker errors arrive as English msgids (the child has
                // no UI language): the parent turns them into display text
                self.last = Err(match resp.error {
                    Some(error) => translate_worker_error(error),
                    None => tr("Script error: onAction failed.").to_string(),
                });
                ActionOutcome::Local
            }
            Err(e) => {
                self.last = Err(e);
                ActionOutcome::Local
            }
        }
    }

    // -----------------------------------------------------------------------

    fn request<'a>(&'a self, action: Option<&'a str>) -> WorkerRequest<'a> {
        WorkerRequest {
            source: &self.source,
            state: &self.state,
            init: self.init_pending,
            action,
        }
    }

    /// Take over the worker's state and mark `init()` as done.
    fn adopt(&mut self, resp: &WorkerResponse) {
        if let Some(state) = &resp.state {
            self.state = state.clone();
        }
        self.init_pending = false;
    }

    /// Evaluate inline and take over the answer (the sync/test path, and
    /// `on_action` on a user click — bounded by `EVAL_TIMEOUT` either way).
    fn render_now(&mut self) -> Result<Vec<UiNode>, String> {
        let request = self.request(None);
        let res = (self.eval)(&request);
        self.finish_render(res)
    }

    /// Spawn one background evaluation: the request is cloned into owned
    /// data for the thread, the answer comes back over the channel.
    /// Spawn one background evaluation, if a worker slot is free. Bumps the
    /// generation, so a result of an older evaluation is dropped as stale.
    fn spawn_render(&mut self) -> Option<PendingEval> {
        let w = workers_running();
        let mut cur = w.load(std::sync::atomic::Ordering::Relaxed);
        loop {
            if cur >= MAX_CONCURRENT_WORKERS {
                return None; // cap reached: the caller stays dirty and retries
            }
            match w.compare_exchange_weak(
                cur,
                cur + 1,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(c) => cur = c,
            }
        }
        self.gen += 1;
        let gen = self.gen;
        let eval = self.eval;
        let source = self.source.clone();
        let state = self.state.clone();
        let init = self.init_pending;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let request = WorkerRequest {
                source: &source,
                state: &state,
                init,
                action: None,
            };
            let _ = tx.send(eval(&request));
        });
        Some(PendingEval { rx, gen })
    }

    /// Turn a worker answer into the render result, adopting its state.
    fn finish_render(
        &mut self,
        res: Result<WorkerResponse, String>,
    ) -> Result<Vec<UiNode>, String> {
        match res {
            Ok(resp) if resp.ok => {
                self.adopt(&resp);
                let nodes = resp.nodes.as_ref().ok_or_else(|| {
                    tr("Script error: render(state) returned nothing.").to_string()
                })?;
                Self::convert_nodes(nodes)
            }
            Ok(resp) => Err(match resp.error {
                // worker errors arrive as English msgids (the child has
                // no UI language): the parent turns them into display text
                Some(error) => translate_worker_error(error),
                None => tr("Script error: evaluation failed.").to_string(),
            }),
            Err(e) => Err(e),
        }
    }

    fn convert_nodes(v: &Value) -> Result<Vec<UiNode>, String> {
        let nodes = match v {
            Value::Array(items) => items
                .iter()
                .cloned()
                .map(serde_json::from_value::<UiNode>)
                .collect::<Result<Vec<_>, _>>(),
            other => serde_json::from_value::<UiNode>(other.clone()).map(|n| vec![n]),
        };
        // this runs in the parent (the child sends raw JSON), so the
        // text composes in the UI language directly
        nodes.map_err(|e| trf("Script error: invalid node: {detail}", &[("detail", &e)]))
    }
}

// ---------------------------------------------------------------------------
// The engine: what the worker runs. boa without host functions.
// ---------------------------------------------------------------------------

/// A loaded script: prelude and source evaluated, `render` verified.
/// The state is passed in and out per call — the engine itself is
/// stateless between evaluations.
struct ScriptEngine {
    ctx: Context,
}

impl ScriptEngine {
    fn new(source: &str) -> Result<Self, String> {
        if source.chars().count() > MAX_SOURCE_CHARS {
            return Err(trf(
                "Script error: source too long (max. {chars} characters).",
                &[("chars", &MAX_SOURCE_CHARS)],
            ));
        }
        let mut ctx = Context::default();
        ctx.runtime_limits_mut()
            .set_loop_iteration_limit(LOOP_LIMIT);
        ctx.runtime_limits_mut()
            .set_recursion_limit(RECURSION_LIMIT);
        ctx.eval(Source::from_bytes(PRELUDE))
            .map_err(|e| ui_error(&e.to_string()))?;
        ctx.eval(Source::from_bytes(source))
            .map_err(|e| ui_error(&e.to_string()))?;
        let mut engine = Self { ctx };
        let has_render = engine.eval_json("JSON.stringify(typeof render === 'function')")?;
        if has_render != Some(Value::Bool(true)) {
            return Err(
                tr("Script error: the script defines no function render(state).").to_string(),
            );
        }
        Ok(engine)
    }

    /// `init()` if the script defines it, `None` otherwise (the caller
    /// keeps its state, like the in-process engine did).
    fn init_state(&mut self) -> Result<Option<Value>, String> {
        let init = self.eval_json("typeof init === 'function' ? JSON.stringify(init()) : null")?;
        match init {
            Some(v) if v.is_object() => Ok(Some(v)),
            Some(_) => Err(tr("Script error: init() must return an object.").to_string()),
            None => Ok(None),
        }
    }

    /// `render(state)` as raw JSON (node object or array).
    fn render(&mut self, state: &Value) -> Result<Value, String> {
        let call = format!(
            "JSON.stringify(render(JSON.parse({})))",
            js_string(&state.to_string())
        );
        let Some(v) = self.eval_json(&call)? else {
            return Err(tr("Script error: render(state) returned nothing.").to_string());
        };
        let count = count_nodes(&v);
        if count > MAX_NODES {
            return Err(trf(
                "Script error: too many nodes ({count}, max. {max}).",
                &[("count", &count), ("max", &MAX_NODES)],
            ));
        }
        Ok(v)
    }

    /// `onAction(action, state)`: `Some(object)` = new state (local),
    /// `None` = not handled (send to the agent).
    fn on_action(&mut self, action: &str, state: &Value) -> Result<Option<Value>, String> {
        let call = format!(
            "(() => {{ if (typeof onAction !== 'function') return null; \
               const r = onAction({}, JSON.parse({})); \
               return (r === undefined || r === null) ? null : JSON.stringify(r); }})()",
            js_string(action),
            js_string(&state.to_string()),
        );
        match self.eval_json(&call) {
            Ok(Some(v)) if v.is_object() => Ok(Some(v)),
            Ok(_) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Evaluate `code`, which must yield a JSON string or null.
    fn eval_json(&mut self, code: &str) -> Result<Option<Value>, String> {
        let v = self
            .ctx
            .eval(Source::from_bytes(code))
            .map_err(|e| ui_error(&e.to_string()))?;
        if v.is_null() || v.is_undefined() {
            return Ok(None);
        }
        let text = v
            .to_string(&mut self.ctx)
            .map_err(|e| ui_error(&e.to_string()))?
            .to_std_string_escaped();
        if text.len() > MAX_OUTPUT_BYTES {
            return Err(trf(
                "Script error: result too large (max. {kib} KiB).",
                &[("kib", &(MAX_OUTPUT_BYTES / 1024))],
            ));
        }
        serde_json::from_str(&text).map(Some).map_err(|e| {
            trf(
                "Script error: invalid JSON: {detail}",
                &[("detail", &e.to_string())],
            )
        })
    }
}

/// May a script whose state changed re-evaluate at `now`? The throttle
/// decision of Handoff 2.7, pure so the rule is unit-tested:
/// - unchanged state: never (nothing to do);
/// - first evaluation: immediately, even mid-drag;
/// - not dragging: immediately (the default: every change re-renders);
/// - dragging: only after `DRAG_EVAL_GAP` since the last evaluation, so
///   a slider drag does not re-run the script on every frame.
fn should_evaluate(
    changed: bool,
    last_eval: Option<Instant>,
    now: Instant,
    dragging: bool,
) -> bool {
    if !changed {
        return false;
    }
    let Some(last) = last_eval else {
        return true;
    };
    !dragging || now.duration_since(last) >= DRAG_EVAL_GAP
}

/// May a failed evaluation get its single bounded retry at `now` (M2)?
/// The rule of the wakeup path, pure so it is unit-tested without
/// sleeping: one retry per failure (`retry_available`), never before
/// [`RETRY_COOLDOWN`] has passed since the failed attempt — after the
/// retry the tool stays quiet until a state or code change evaluates
/// again (see [`ScriptTool::nodes_at`]). A retry deliberately ignores
/// the drag throttle: it repairs a broken block that has no sliders to
/// drag, and it can fire at most once per failure.
fn should_retry(
    failed: bool,
    retry_available: bool,
    last_attempt: Option<Instant>,
    now: Instant,
) -> bool {
    failed
        && retry_available
        && last_attempt.is_some_and(|t| now.duration_since(t) >= RETRY_COOLDOWN)
}

/// A JS string literal (JSON string syntax plus escaped U+2028/U+2029).
fn js_string(s: &str) -> String {
    serde_json::to_string(s)
        .unwrap_or_else(|_| "\"\"".into())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// One engine error line for the UI: our `Script error` head plus a
/// detail. The engine runs in the worker child, where `tr` resolves to
/// the English source (no settings there), so this travels the wire as
/// an English msgid head with the raw detail behind it; the parent
/// composes the display language around it (see
/// [`translate_worker_error_in`]).
fn script_error_line(detail: &str) -> String {
    format!("{}: {detail}", tr("Script error"))
}

/// First line of an engine error, short and prefixed for the UI.
fn ui_error(raw: &str) -> String {
    if raw.contains("RuntimeLimitError") {
        return tr("Script error: loop limit reached (endless loop or too deep recursion?)")
            .to_string();
    }
    let first = raw.lines().next().unwrap_or("").trim();
    let first: String = first.chars().take(300).collect();
    script_error_line(&first)
}

/// Nodes (objects with a "type") in a JSON value, counted recursively.
fn count_nodes(v: &Value) -> usize {
    match v {
        Value::Array(a) => a.iter().map(count_nodes).sum(),
        Value::Object(o) => {
            let own = usize::from(o.contains_key("type"));
            own + o.values().map(count_nodes).sum::<usize>()
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;
    use std::io::Cursor;

    fn tool(src: &str) -> ScriptTool {
        ScriptTool::with_eval(src, eval_in_process)
    }

    fn nodes(t: &mut ScriptTool) -> Vec<UiNode> {
        t.nodes().clone().expect("render ok")
    }

    #[test]
    fn renders_nodes_from_state() {
        let mut t = tool(
            "function init(){ return {n: 2}; }
             function render(s){ return [{type:'text', text:'n=' + s.n}, {type:'divider'}]; }",
        );
        let n = nodes(&mut t);
        assert_eq!(n.len(), 2);
        assert!(matches!(&n[0], UiNode::Text { text } if text == "n=2"));
        assert_eq!(t.state(), &json!({"n": 2}));
    }

    #[test]
    fn single_node_result_and_set_value_rerenders() {
        let mut t = tool("function render(s){ return {type:'text', text:String(s.x || 0)}; }");
        assert!(matches!(&nodes(&mut t)[0], UiNode::Text { text } if text == "0"));
        t.set_value("x", json!(7));
        assert!(matches!(&nodes(&mut t)[0], UiNode::Text { text } if text == "7"));
    }

    /// An input value that pushes the state past `MAX_STATE_BYTES` is
    /// rejected on the spot with the real cause, not sent into a pipe that
    /// truncates it (M1).
    #[test]
    fn oversized_state_is_rejected_with_the_real_cause() {
        let mut t =
            tool("function render(s){ return {type:'text', text: String((s.x || '').length)}; }");
        let _ = nodes(&mut t);
        t.set_value("x", json!("x".repeat(90 * 1024)));
        let e = t.nodes().clone().unwrap_err();
        assert!(e.contains("too large"), "{e}");
        assert!(!e.contains("unreadable request"), "{e}");
        // the field never landed: the state is unchanged
        assert_eq!(t.state(), &json!({}));
        // a change that fits re-renders and clears the error card
        t.set_value("x", json!(1));
        assert!(t.nodes().is_ok());
    }

    #[test]
    fn on_action_local_or_send() {
        let src = "function init(){ return {items: []}; }
            function render(s){ return {type:'text', text:'items=' + s.items.length}; }
            function onAction(a, s){ if (a === 'add') { s.items.push(1); return s; } }";
        let mut t = tool(src);
        let _ = nodes(&mut t);
        assert_eq!(t.on_action("add"), ActionOutcome::Local);
        assert!(matches!(&nodes(&mut t)[0], UiNode::Text { text } if text == "items=1"));
        assert_eq!(t.on_action("ask"), ActionOutcome::SendToAgent);
        let mut no_handler = tool("function render(){ return {type:'divider'}; }");
        let _ = nodes(&mut no_handler);
        assert_eq!(no_handler.on_action("x"), ActionOutcome::SendToAgent);
    }

    #[test]
    fn on_action_before_first_render_seeds_from_init() {
        let mut t = tool(
            "function init(){ return {n: 1}; }
             function render(s){ return {type:'text', text:String(s.n)}; }
             function onAction(a, s){ s.n = s.n + 1; return s; }",
        );
        assert_eq!(t.on_action("x"), ActionOutcome::Local);
        assert_eq!(t.state(), &json!({"n": 2}));
        assert!(matches!(&nodes(&mut t)[0], UiNode::Text { text } if text == "2"));
    }

    #[test]
    fn on_action_load_error_goes_to_the_agent() {
        let mut t = tool("const x = 1;");
        assert_eq!(t.on_action("x"), ActionOutcome::SendToAgent);
    }

    #[test]
    fn runaway_scripts_are_stopped() {
        let mut lp = tool("function render(){ while(true){} }");
        let e = lp.nodes().clone().unwrap_err();
        assert!(e.contains("loop limit"), "{e}");
        let mut rec = tool("function f(n){ return f(n+1); } function render(){ return f(0); }");
        assert!(rec.nodes().clone().unwrap_err().contains("loop limit"));
    }

    #[test]
    fn should_evaluate_decides_the_drag_throttle() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        // unchanged state: never evaluate, dragging or not
        assert!(!should_evaluate(false, None, at(0), false));
        assert!(!should_evaluate(false, Some(t0), at(200), false));
        // first evaluation (nothing rendered yet): immediately, even mid-drag
        assert!(should_evaluate(true, None, at(0), false));
        assert!(should_evaluate(true, None, at(0), true));
        // not dragging: every change evaluates immediately
        assert!(should_evaluate(true, Some(t0), at(0), false));
        assert!(should_evaluate(true, Some(at(0)), at(1), false));
        // dragging: not before the gap has passed
        assert!(!should_evaluate(true, Some(at(0)), at(99), true));
        // … at or after the gap it does
        assert!(should_evaluate(true, Some(at(0)), at(100), true));
        assert!(should_evaluate(true, Some(at(0)), at(500), true));
        // released (dragging false): immediately, even before the gap
        assert!(should_evaluate(true, Some(at(0)), at(10), false));
    }

    thread_local! {
        /// Spawn log of `flaky_eval`: how many attempts ran, and how many
        /// of them must fail. Cargo runs one thread per test, so parallel
        /// tests never see each other's counters.
        static FLAKY_CALLS: Cell<usize> = const { Cell::new(0) };
        static FLAKY_FAILURES: Cell<usize> = const { Cell::new(0) };
    }

    /// An eval that transports fine but fails its first calls, like a
    /// worker spawn or run that dies transiently (timeout, busy machine).
    fn flaky_eval(_req: &WorkerRequest<'_>) -> Result<WorkerResponse, String> {
        let attempt = FLAKY_CALLS.with(|c| c.replace(c.get() + 1));
        if attempt < FLAKY_FAILURES.with(Cell::get) {
            Err("Script error: the evaluation process failed.".into())
        } else {
            Ok(WorkerResponse {
                ok: true,
                nodes: Some(json!({ "type": "text", "text": "recovered" })),
                state: Some(json!({})),
                ..Default::default()
            })
        }
    }

    fn flaky_calls() -> usize {
        FLAKY_CALLS.with(Cell::get)
    }

    #[test]
    fn should_retry_decides_the_bounded_wakeup() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let some = Some(t0);
        // nothing failed, no retry left, no attempt yet: never
        assert!(!should_retry(false, true, some, at(60_000)));
        assert!(!should_retry(true, false, some, at(60_000)));
        assert!(!should_retry(true, true, None, at(60_000)));
        // inside the cooldown: never — no per-frame respawn
        assert!(!should_retry(true, true, some, at(0)));
        assert!(!should_retry(true, true, some, at(1_999)));
        // at or after the cooldown: the one shot fires
        assert!(should_retry(true, true, some, at(2_000)));
        assert!(should_retry(true, true, some, at(60_000)));
    }

    /// Worker errors are English msgids on the wire (the child has no UI
    /// language): the parent is where they become display text. A known
    /// msgid turns German whole; a raw failure that no catalog entry
    /// knows keeps its foreign detail under a translated head; anything
    /// else — and an English parent — is returned unchanged.
    #[test]
    fn worker_errors_become_display_text_in_the_parent() {
        // a known msgid (the engine's loop limit) is translated whole
        let known = translate_worker_error_in(
            Lang::De,
            tr_in(
                Lang::En,
                "Script error: loop limit reached (endless loop or too deep recursion?)",
            )
            .to_string(),
        );
        assert!(known.contains("Rechenlimit"), "{known}");
        assert!(!known.contains("Script error"), "{known}");
        // a raw failure under our head: the detail stays, the head turns
        let raw = translate_worker_error_in(Lang::De, "Script error: SyntaxError at 1:1".into());
        assert_eq!(raw, "Skript-Fehler: SyntaxError at 1:1");
        // foreign text passes untouched
        assert_eq!(
            translate_worker_error_in(Lang::De, "no text of ours".into()),
            "no text of ours"
        );
        // an English parent changes nothing
        assert_eq!(
            translate_worker_error_in(Lang::En, "Script error: onAction failed.".into()),
            "Script error: onAction failed."
        );
    }

    /// Final audit D: parametrized errors arrive with the values filled
    /// in (the child has no UI language), so the catalog's exact lookup
    /// misses them. The parent matches them against its known templates
    /// and re-renders the template through the catalog — the values stay
    /// verbatim, everything around them is display text.
    #[test]
    fn parametrized_worker_errors_localize_in_the_parent() {
        let cases = [
            (
                "Script error: too many nodes (2049, max. 2000).",
                "Skript-Fehler: zu viele Bausteine (2049, max. 2000).",
            ),
            (
                "Script error: source too long (max. 5000 characters).",
                "Skript-Fehler: Quelltext zu lang (max. 5000 Zeichen).",
            ),
            (
                "Script error: result too large (max. 64 KiB).",
                "Skript-Fehler: Ergebnis zu groß (max. 64 KiB).",
            ),
            (
                "Script error: invalid JSON: expected value at line 1 column 2",
                "Skript-Fehler: ungültiges JSON: expected value at line 1 column 2",
            ),
        ];
        for (filled, expected) in cases {
            assert_eq!(
                translate_worker_error_in(Lang::De, filled.to_string()),
                expected
            );
        }
        // an English parent gets the wire text unchanged either way
        assert_eq!(
            translate_worker_error_in(
                Lang::En,
                "Script error: too many nodes (2049, max. 2000).".to_string()
            ),
            "Script error: too many nodes (2049, max. 2000)."
        );
    }

    /// The recovery must not overreach: a foreign text that merely LOOKS
    /// like one of our templates with values filled in (parentheses and
    /// all) must not be re-rendered — it keeps the head-fallback path.
    #[test]
    fn unknown_parametrized_looking_worker_errors_fall_back_to_the_head() {
        let unrelated = "Script error: nothing of ours (12, max. 3).".to_string();
        assert_eq!(
            translate_worker_error_in(Lang::De, unrelated),
            "Skript-Fehler: nothing of ours (12, max. 3)."
        );
    }

    /// M2: a failed evaluation gets exactly one more attempt after the
    /// cooldown, then stays quiet — no double spawn in the same frame,
    /// no respawn loop. A state change wakes the tool again.
    #[test]
    fn a_failed_evaluation_retries_once_then_stays_quiet() {
        FLAKY_FAILURES.with(|c| c.set(usize::MAX)); // every attempt fails
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut t = ScriptTool::with_eval(
            "function render(s){ return {type:'text', text:'k'}; }",
            flaky_eval,
        );
        // the first evaluation fails
        assert!(t.nodes_at(t0).is_err());
        assert_eq!(flaky_calls(), 1);
        // frames inside the cooldown stay quiet — no per-frame respawn loop
        assert!(t.nodes_at(at(0)).is_err());
        assert!(t.nodes_at(at(1_999)).is_err());
        assert_eq!(flaky_calls(), 1);
        // after the cooldown exactly one retry fires...
        assert!(t.nodes_at(at(2_000)).is_err());
        assert_eq!(flaky_calls(), 2);
        // ...and never a second one, in the same frame or any later one
        assert!(t.nodes_at(at(2_000)).is_err());
        assert!(t.nodes_at(at(60_000)).is_err());
        assert_eq!(flaky_calls(), 2);
        // a state change starts a new cycle: attempt plus one fresh retry
        t.set_value("x", json!(1));
        assert!(t.nodes_at(at(60_001)).is_err());
        assert_eq!(flaky_calls(), 3);
        assert!(t.nodes_at(at(62_001)).is_err());
        assert_eq!(flaky_calls(), 4);
        assert!(t.nodes_at(at(70_000)).is_err());
        assert_eq!(flaky_calls(), 4);
    }

    /// M2: the retry exists so a transient failure (timeout, spawn error)
    /// does not leave the session dead until a reconnect: the cooldown
    /// retry brings the tool back.
    #[test]
    fn a_transient_failure_recovers_on_the_retry() {
        FLAKY_FAILURES.with(|c| c.set(1)); // only the first attempt fails
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut t = ScriptTool::with_eval(
            "function render(s){ return {type:'text', text:'k'}; }",
            flaky_eval,
        );
        assert!(t.nodes_at(t0).is_err());
        assert_eq!(flaky_calls(), 1);
        // the block stays failed (error card) through the cooldown
        assert!(t.nodes_at(at(1_000)).is_err());
        assert_eq!(flaky_calls(), 1);
        // after the cooldown the retry succeeds: the tool is alive again
        let nodes = t.nodes_at(at(2_500)).clone().expect("retry recovers");
        assert!(matches!(&nodes[0], UiNode::Text { text } if text == "recovered"));
        assert_eq!(flaky_calls(), 2);
        // a stable state does not spawn anything further
        assert!(t.nodes_at(at(2_600)).is_ok());
        assert_eq!(flaky_calls(), 2);
        assert!(!t.eval_pending());
    }

    #[test]
    fn drag_throttle_skips_until_released() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut t = tool("function render(s){ return {type:'text', text:'v=' + (s.v ?? 0)}; }");
        let _ = t.nodes_at(t0); // renders "v=0", anchors last_eval at t0

        t.set_dragging(true);
        t.set_value("v", json!(3));
        // mid-drag, before the gap: the stale result stays, update pending
        assert!(matches!(&t.nodes_at(at(40)).clone().unwrap()[0],
            UiNode::Text { text } if text == "v=0"));
        assert!(t.eval_pending());
        // the drag continues: the newer value is still throttled away
        t.set_value("v", json!(4));
        assert!(matches!(&t.nodes_at(at(80)).clone().unwrap()[0],
            UiNode::Text { text } if text == "v=0"));
        assert!(t.eval_pending());
        // after the gap, the drag still delivers one update
        assert!(matches!(&t.nodes_at(at(120)).clone().unwrap()[0],
            UiNode::Text { text } if text == "v=4"));
        // released: the pending value applies promptly, never stale
        t.set_value("v", json!(5));
        t.set_dragging(false);
        assert!(matches!(&t.nodes_at(at(130)).clone().unwrap()[0],
            UiNode::Text { text } if text == "v=5"));
        assert!(!t.eval_pending());
        // without a drag: instant as before
        t.set_value("v", json!(7));
        assert!(matches!(&t.nodes_at(at(131)).clone().unwrap()[0],
            UiNode::Text { text } if text == "v=7"));
    }

    #[test]
    fn memory_hogs_are_capped() {
        let mut r = tool("function render(){ return {type:'text', text:'x'.repeat(1e7)}; }");
        assert!(r.nodes().clone().unwrap_err().contains("too large"));
        let mut a = tool(
            "function render(){ const ok = [] instanceof Array && Array.isArray([1]); \
             new Array(1e7); return {type:'text', text:String(ok)}; }",
        );
        assert!(a.nodes().clone().unwrap_err().contains("too large"));
        let mut ok = tool(
            "function render(){ return {type:'text', text:String([] instanceof Array && Array.from([1,2]).length === 2 && new Array(3).length === 3)}; }",
        );
        assert!(matches!(&nodes(&mut ok)[0], UiNode::Text { text } if text == "true"));
    }

    #[test]
    fn no_host_access() {
        let mut t = tool(
            "function render(){ return {type:'text', text:[typeof console, typeof fetch, typeof require, typeof XMLHttpRequest].join(',')}; }",
        );
        assert!(
            matches!(&nodes(&mut t)[0], UiNode::Text { text } if text == "undefined,undefined,undefined,undefined")
        );
    }

    #[test]
    fn invalid_scripts_report_errors() {
        let long = format!(
            "function render(){{ return {{type:'divider'}}; }} //{}",
            "x".repeat(MAX_SOURCE_CHARS)
        );
        assert!(tool(&long)
            .nodes()
            .clone()
            .unwrap_err()
            .contains("too long"));
        assert!(tool("const x = 1;")
            .nodes()
            .clone()
            .unwrap_err()
            .contains("render(state)"));
        assert!(tool("function render(){ return 42; }")
            .nodes()
            .clone()
            .is_err());
        assert!(tool("function render(){ return {nope: 1}; }")
            .nodes()
            .clone()
            .is_err());
        assert!(tool("function render( {").nodes().clone().is_err());
        let many = "function render(){ const a=[]; for(let i=0;i<3000;i++) a.push({type:'divider'}); return a; }";
        assert!(tool(many).nodes().clone().unwrap_err().contains("too many"));
    }

    #[test]
    fn skill_examples_run_in_the_sandbox() {
        let skill = include_str!("../../vibe-skills/catwright-ui/SKILL.md");
        let mut found = 0;
        let mut rest = skill;
        // fences at the start of a line only (the heading also mentions ```ui-js)
        while let Some(start) = rest.find("\n```ui-js\n") {
            let body = &rest[start + "\n```ui-js\n".len()..];
            let end = body.find("```").expect("closed example");
            let mut t = tool(&body[..end]);
            let nodes = t.nodes().clone();
            assert!(nodes.is_ok(), "skill example fails: {nodes:?}");
            assert!(!nodes.unwrap().is_empty());
            found += 1;
            rest = &body[end + 3..];
        }
        assert!(found >= 2, "the skill ships runnable ui-js examples");
        // the budget example handles "add" locally
        let start = skill
            .find("function init() {\n  return { items")
            .expect("budget example");
        let end = skill[start..].find("```").unwrap();
        let mut budget = tool(&skill[start..start + end]);
        let _ = budget.nodes();
        assert_eq!(budget.on_action("add"), ActionOutcome::Local);
        assert_eq!(budget.state()["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn strings_with_quotes_and_separators_round_trip() {
        let mut t = tool("function render(s){ return {type:'text', text: s.q}; }");
        t.set_value("q", json!("a\"b'c\u{2028}d"));
        assert!(matches!(&nodes(&mut t)[0], UiNode::Text { text } if text == "a\"b'c\u{2028}d"));
    }

    // -----------------------------------------------------------------------
    // Worker protocol and transport.
    // -----------------------------------------------------------------------

    fn request_json(source: &str, state: Value, init: bool, action: Option<&str>) -> String {
        serde_json::to_string(&WorkerRequest {
            source,
            state: &state,
            init,
            action,
        })
        .expect("request serializes")
    }

    fn run_worker_main(request: &str) -> WorkerResponse {
        let mut out = Vec::new();
        worker_main(Cursor::new(request.as_bytes()), &mut out);
        serde_json::from_slice(&out).expect("response JSON")
    }

    #[test]
    fn worker_main_renders_and_returns_state() {
        let request = request_json(
            "function init(){ return {n: 2}; }
             function render(s){ return {type:'text', text:'n=' + s.n}; }",
            json!({}),
            true,
            None,
        );
        let resp = run_worker_main(&request);
        assert!(resp.ok, "{resp:?}");
        assert_eq!(resp.state, Some(json!({"n": 2})));
        assert_eq!(resp.nodes, Some(json!({"type": "text", "text": "n=2"})));
    }

    #[test]
    fn worker_main_runs_actions() {
        let request = request_json(
            "function render(){ return {type:'divider'}; }
             function onAction(a, s){ if (a === 'x') { s.done = true; return s; } }",
            json!({"other": 1}),
            false,
            Some("x"),
        );
        let resp = run_worker_main(&request);
        assert!(resp.ok, "{resp:?}");
        assert_eq!(resp.outcome.as_deref(), Some("local"));
        assert_eq!(resp.state, Some(json!({"other": 1, "done": true})));
    }

    #[test]
    fn worker_main_marks_load_errors() {
        let resp = run_worker_main(&request_json("const x = 1;", json!({}), true, None));
        assert!(!resp.ok);
        assert_eq!(resp.load, Some(true));
        let resp = run_worker_main(&request_json(
            "function render(){ while(true){} }",
            json!({}),
            true,
            None,
        ));
        assert!(!resp.ok);
        assert_eq!(resp.load, Some(false));
    }

    #[test]
    fn worker_main_rejects_unreadable_requests() {
        let resp = run_worker_main("not json at all");
        assert!(!resp.ok);
        assert!(resp.error.unwrap().contains("unreadable request"));
    }

    /// A request past the request budget is a SIZE problem, not malformed
    /// JSON: the cause must say so (M1). Before, the worker truncated the
    /// request at its read cap and reported "unreadable request".
    #[test]
    fn worker_main_marks_oversized_requests_as_too_large() {
        let state = json!({ "p": "x".repeat(MAX_REQUEST_BYTES as usize + 1) });
        let resp = run_worker_main(&request_json(
            "function render(){ return {type:'divider'}; }",
            state,
            false,
            None,
        ));
        assert!(!resp.ok);
        // a script that never loaded cannot handle clicks either: the
        // click pass-through stays (load=true), the cause is honest now
        assert_eq!(resp.load, Some(true));
        assert!(resp.error.unwrap().contains("request too large"));
    }

    /// The parent checks the request size BEFORE spawning (M1): an
    /// oversized request never reaches the pipe, so no worker has to die
    /// of it and the error names the size.
    #[test]
    fn oversized_requests_are_rejected_before_the_spawn() {
        let mut missing = Command::new("catwright-worker-that-does-not-exist");
        let huge = "x".repeat(MAX_REQUEST_BYTES as usize + 1);
        let err = run_worker(&mut missing, &huge, EVAL_TIMEOUT).unwrap_err();
        assert!(err.contains("request too large"), "{err}");
        assert!(!err.contains("could not be started"), "{err}");
    }

    #[test]
    fn worker_timeout_kills_the_child() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("sleep 30");
        let err = run_worker(&mut cmd, "{}", Duration::from_millis(100)).unwrap_err();
        assert!(err.contains("time limit"), "{err}");
    }

    /// Bug 1 (Handoff 2026-11-09): a response bigger than the pipe buffer
    /// (~64 KiB) must still arrive; the parent reads stdout while waiting
    /// instead of letting the worker block on a full pipe until the timeout.
    #[test]
    fn worker_output_beyond_the_pipe_buffer_arrives() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(
            "printf '{\"ok\":true,\"state\":{\"p\":\"'; head -c 90000 /dev/zero | tr '\\0' x; printf '\"}}'",
        );
        let resp = run_worker(&mut cmd, "{}", EVAL_TIMEOUT).expect("large response");
        assert!(resp.ok);
        assert_eq!(
            resp.state.as_ref().unwrap()["p"].as_str().map(str::len),
            Some(90_000)
        );
    }

    /// A legal answer carries nodes plus a state echo, so it may exceed the
    /// request budget (`MAX_REQUEST_BYTES`, ~389 KiB) — the response budget
    /// (`2 x MAX_OUTPUT_BYTES + MAX_STATE_BYTES + 16 KiB`) must accept it.
    /// Below this fix the reader capped the pipe at the request budget, the
    /// worker died of EPIPE mid-answer and the parent reported "no valid
    /// answer" for a feature that was working (Wirecap review, M11).
    #[test]
    fn worker_response_beyond_the_request_budget_arrives() {
        let mut cmd = Command::new("sh");
        // 500_000 response bytes: above MAX_REQUEST_BYTES (398_528), safely
        // below MAX_RESPONSE_BYTES (606_208)
        cmd.arg("-c").arg(
            "printf '{\"ok\":true,\"state\":{\"p\":\"'; head -c 500000 /dev/zero | tr '\\0' x; printf '\"}}'",
        );
        let resp = run_worker(&mut cmd, "{}", EVAL_TIMEOUT).expect("response budget");
        assert!(resp.ok, "{resp:?}");
        assert_eq!(
            resp.state.as_ref().unwrap()["p"].as_str().map(str::len),
            Some(500_000)
        );
    }

    /// An answer past even the response budget is cut on purpose and the
    /// cause must say so: the worker exits 0 (its writeln dies of EPIPE,
    /// swallowed), the parent must not misreport "no valid answer" (M11).
    #[test]
    fn worker_answer_past_the_response_budget_reports_the_size() {
        let mut cmd = Command::new("sh");
        // `; true` swallows the inevitable SIGPIPE so the child exits 0,
        // exactly like the real worker does
        cmd.arg("-c")
            .arg("head -c 700000 /dev/zero | tr '\\0' x; true");
        let err = run_worker(&mut cmd, "{}", EVAL_TIMEOUT).unwrap_err();
        assert!(err.contains("too large"), "{err}");
        assert!(!err.contains("no valid answer"), "{err}");
    }

    #[test]
    fn worker_crash_and_spawn_failure_report_errors() {
        // exits non-zero without a response
        let mut crash = Command::new("sh");
        crash.arg("-c").arg("exit 1");
        let err = run_worker(&mut crash, "{}", EVAL_TIMEOUT).unwrap_err();
        assert!(err.contains("crashed"), "{err}");
        // exits zero but says nothing
        let mut silent = Command::new("true");
        let err = run_worker(&mut silent, "{}", EVAL_TIMEOUT).unwrap_err();
        assert!(err.contains("no valid answer"), "{err}");
        // cannot be spawned at all
        let mut missing = Command::new("catwright-worker-that-does-not-exist");
        let err = run_worker(&mut missing, "{}", EVAL_TIMEOUT).unwrap_err();
        assert!(err.contains("could not be started"), "{err}");
    }
}
