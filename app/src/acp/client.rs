//! Process and threads: spawns `vibe-acp` as a child process, speaks
//! line-based JSON-RPC 2.0 over its stdin/stdout and delivers typed
//! `AcpEvent`s over a `std::sync::mpsc` channel. No async, no UI.
//!
//! Layout: one reader thread per transport (stdout, or any `BufRead` in
//! tests), one stderr drain thread that keeps only a bounded tail (so the
//! pipe can never deadlock), and a mutex-guarded writer so lines never
//! interleave.

use crate::acp::protocol as proto;
use crate::acp::rpc::{self, Incoming};
use crate::i18n::{tr, trf};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Called after every event delivered to the channel (e.g. to wake the UI).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// How much of stderr we keep for error reporting (the rest is drained and
/// dropped so the agent can never block on a full pipe).
const STDERR_TAIL_BYTES: usize = 64 * 1024;

/// Which request a `SessionCreated` event answers.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionOrigin {
    /// A normal `session/new` the user's conversation lives in.
    New,
    /// The startup probe session (only exists to learn config defaults).
    Probe,
    /// `session/fork` from `source`, copying up to `message_id`'s turn.
    Fork {
        source: String,
        message_id: Option<String>,
    },
}

/// Which of our requests a response id belongs to.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestKind {
    Initialize,
    NewSession {
        cwd: PathBuf,
        /// The startup probe (`new_probe_session`): the same `session/new`,
        /// but its failure must not be treated as the user's failed first
        /// prompt (both carry the same cwd, so this flag is the only
        /// difference the harness sees).
        probe: bool,
    },
    LoadSession {
        session_id: String,
    },
    /// `session/resume` of an open session: answered with `{}`, its point
    /// is the fresh usage_update Vibe sends with it.
    ResumeSession {
        session_id: String,
    },
    ForkSession {
        cwd: PathBuf,
        /// The session whose context the fork copies (`sessionId` on the wire).
        fork_session_id: String,
        /// The fork copies only up to this user message's turn (`None`: all).
        message_id: Option<String>,
    },
    ListSessions {
        cwd: Option<PathBuf>,
        /// Pagination cursor of `session/list` (`nextCursor`): `None` asks
        /// for the first page.
        cursor: Option<String>,
    },
    Prompt {
        session_id: String,
    },
    SetMode {
        session_id: String,
        mode_id: String,
    },
    SetConfigOption {
        session_id: String,
        config_id: String,
    },
    CloseSession {
        session_id: String,
    },
    /// A Vibe extension request (`_…` method).
    Ext(ExtKind),
}

/// Vibe's ACP extension requests, each with everything its wire form needs
/// (method names and params verified against Vibe 3.1.0, see docs/acp.md).
#[derive(Debug, Clone, PartialEq)]
pub enum ExtKind {
    /// Rename a session (also a saved one that is not open).
    SetTitle {
        session_id: String,
        title: String,
        cwd: PathBuf,
    },
    /// Delete a saved session for good.
    DeleteSession {
        session_id: String,
        cwd: PathBuf,
    },
    /// Which files a rewind to `message_id` would restore.
    RewindPreview {
        session_id: String,
        message_id: String,
    },
    /// Rewind the conversation (and optionally the files) to `message_id`.
    RewindTo {
        session_id: String,
        message_id: String,
        restore_files: bool,
    },
    /// Plan and monthly budget of the signed-in account.
    Account {
        session_id: String,
    },
    /// Name, e-mail and workspace of the signed-in account.
    Identity {
        session_id: String,
    },
    /// Record a workspace trust decision for `cwd`.
    TrustDecision {
        session_id: String,
        cwd: PathBuf,
        decision: String,
    },
    LoopsList {
        session_id: String,
    },
    LoopsCreate {
        session_id: String,
        interval: String,
        prompt: String,
    },
    LoopsDelete {
        session_id: String,
        loop_id: String,
    },
    /// Delete every routine of a session (`_loops/clear`).
    LoopsClear {
        session_id: String,
    },
    /// The agent's file changes in this session.
    ReviewState {
        session_id: String,
    },
    /// `review/approve` (`approve = true`) or `review/revert` for `target`.
    ReviewMutate {
        session_id: String,
        approve: bool,
        target: Value,
    },
    /// One file's content before and after the turn `turn_id` caused.
    ReviewTurnDiff {
        session_id: String,
        path: String,
        turn_id: String,
    },
    ConnectorsList {
        session_id: String,
    },
    ConnectorsToggle {
        session_id: String,
        name: String,
        disabled: bool,
    },
    /// Start Vibe's voice transcription (`_voice/transcribeStart`). Vibe
    /// records the microphone and transcribes through Mistral; the text
    /// arrives as `_voice/transcriptionDelta` notifications.
    VoiceTranscribeStart {
        language: String,
    },
    /// Stop the transcription and flush what is left (`_voice/transcribeStop`).
    VoiceTranscribeStop,
    /// Abort the transcription without flushing (`_voice/transcribeCancel`).
    VoiceTranscribeCancel,
}

impl ExtKind {
    /// The wire method (ACP extension methods start with `_`) and params.
    pub fn wire(&self) -> (&'static str, Value) {
        match self {
            ExtKind::SetTitle {
                session_id, title, ..
            } => (
                "_session/set_title",
                json!({ "sessionId": session_id, "title": title }),
            ),
            ExtKind::DeleteSession { session_id, .. } => {
                ("_session/delete", json!({ "sessionId": session_id }))
            }
            ExtKind::RewindPreview {
                session_id,
                message_id,
            } => (
                "_rewind/preview",
                json!({ "sessionId": session_id, "messageId": message_id }),
            ),
            ExtKind::RewindTo {
                session_id,
                message_id,
                restore_files,
            } => (
                "_rewind/to",
                json!({
                    "sessionId": session_id,
                    "messageId": message_id,
                    "restoreFiles": restore_files,
                }),
            ),
            ExtKind::Account { session_id } => {
                ("_account/read", json!({ "sessionId": session_id }))
            }
            ExtKind::Identity { session_id } => {
                ("_identity/read", json!({ "sessionId": session_id }))
            }
            ExtKind::TrustDecision {
                session_id,
                cwd,
                decision,
            } => (
                "_trust/decision",
                json!({
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "decision": decision,
                }),
            ),
            ExtKind::LoopsList { session_id } => {
                ("_loops/list", json!({ "sessionId": session_id }))
            }
            ExtKind::LoopsCreate {
                session_id,
                interval,
                prompt,
            } => (
                "_loops/create",
                json!({ "sessionId": session_id, "interval": interval, "prompt": prompt }),
            ),
            ExtKind::LoopsDelete {
                session_id,
                loop_id,
            } => (
                "_loops/delete",
                json!({ "sessionId": session_id, "loopId": loop_id }),
            ),
            ExtKind::LoopsClear { session_id } => {
                ("_loops/clear", json!({ "sessionId": session_id }))
            }
            ExtKind::ReviewState { session_id } => {
                ("_review/state", json!({ "sessionId": session_id }))
            }
            ExtKind::ReviewMutate {
                session_id,
                approve,
                target,
            } => (
                if *approve {
                    "_review/approve"
                } else {
                    "_review/revert"
                },
                json!({ "sessionId": session_id, "target": target }),
            ),
            ExtKind::ReviewTurnDiff {
                session_id,
                path,
                turn_id,
            } => (
                "_review/turnDiff",
                json!({
                    "sessionId": session_id,
                    "path": path,
                    "owner": { "turnId": turn_id },
                }),
            ),
            ExtKind::ConnectorsList { session_id } => {
                ("_connectors/list", json!({ "sessionId": session_id }))
            }
            ExtKind::ConnectorsToggle {
                session_id,
                name,
                disabled,
            } => (
                "_connectors/toggle",
                json!({ "sessionId": session_id, "name": name, "disabled": disabled }),
            ),
            // The voice methods carry no session id: Vibe uses the first
            // session open in its process (docs/acp.md).
            ExtKind::VoiceTranscribeStart { language } => {
                ("_voice/transcribeStart", json!({ "language": language }))
            }
            ExtKind::VoiceTranscribeStop => ("_voice/transcribeStop", json!({})),
            ExtKind::VoiceTranscribeCancel => ("_voice/transcribeCancel", json!({})),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AcpEvent {
    Initialized(proto::InitializeResult),
    SessionCreated {
        cwd: PathBuf,
        origin: SessionOrigin,
        result: proto::NewSessionResult,
    },
    SessionLoaded {
        session_id: String,
        result: proto::LoadSessionResult,
    },
    SessionsListed {
        cwd: Option<PathBuf>,
        /// The pagination cursor its own request carried (`None`: the
        /// request asked for the first page). The answer knows whether
        /// it continues the request chain, so no cwd marker or response
        /// order is needed to tell a page from a first page.
        cursor: Option<String>,
        result: proto::ListSessionsResult,
    },
    PromptFinished {
        session_id: String,
        result: proto::PromptResult,
    },
    ModeSet {
        session_id: String,
        mode_id: String,
    },
    ConfigOptionsChanged {
        session_id: String,
        options: Vec<proto::ConfigOption>,
    },
    SessionClosed {
        session_id: String,
    },
    /// `session/resume` answered (see `RequestKind::ResumeSession`).
    SessionResumed,
    Update {
        session_id: String,
        update: proto::SessionUpdate,
    },
    PermissionRequested {
        request_id: Value,
        request: proto::PermissionRequest,
    },
    /// The agent asks the user something (`elicitation/create`); the raw
    /// params are parsed by the harness (`extras::parse_question`).
    QuestionAsked {
        request_id: Value,
        params: Value,
    },
    /// The answer to an extension request.
    ExtResult {
        kind: ExtKind,
        result: Value,
    },
    /// The running turn is being retried by Vibe (`_session/retrying`).
    Retrying {
        session_id: String,
        category: String,
        detail: String,
    },
    /// `_voice/transcribeStart` answered: `ok`, or Vibe's own error text.
    VoiceStarted {
        ok: bool,
        error: Option<String>,
    },
    /// `_voice/transcribeStop` answered: `ok`, or Vibe's own error text.
    VoiceStopped {
        ok: bool,
        error: Option<String>,
    },
    /// `_voice/transcribeCancel` answered (the payload carries nothing).
    VoiceCancelled,
    /// A piece of recognised text to append (`text`), or a failure
    /// (`error`) of the running transcription (`_voice/transcriptionDelta`).
    VoiceDelta {
        text: Option<String>,
        error: Option<String>,
    },
    RequestFailed {
        request: RequestKind,
        error: rpc::RpcError,
    },
    /// Non-fatal: unparsable line, response for unknown id, … (never contains
    /// line content, prompts or file contents).
    ProtocolError(String),
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
}

/// State shared between the client handle and the reader thread.
struct Shared {
    tx: mpsc::Sender<AcpEvent>,
    /// `None` once stdin has been closed (shutdown).
    writer_slot: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    pending: Arc<Mutex<HashMap<u64, RequestKind>>>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    stderr_handle: Mutex<Option<JoinHandle<()>>>,
    child: Arc<Mutex<Option<Child>>>,
}

pub struct AcpClient {
    writer_slot: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    pending: Arc<Mutex<HashMap<u64, RequestKind>>>,
    next_id: AtomicU64,
    child: Arc<Mutex<Option<Child>>>,
}

impl AcpClient {
    /// Spawn the agent binary with piped stdin/stdout/stderr.
    pub fn spawn(
        program: &str,
        args: &[String],
        waker: Option<Waker>,
    ) -> Result<(AcpClient, mpsc::Receiver<AcpEvent>), String> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                trf(
                    "{program} could not be started: {error}",
                    &[("program", &program), ("error", &e)],
                )
            })?;
        let unavailable = |stream: &str| {
            trf(
                "{program} could not be started: {error}",
                &[
                    ("program", &program),
                    (
                        "error",
                        &trf("{stream} not available", &[("stream", &stream)]),
                    ),
                ],
            )
        };
        let stdin = child.stdin.take().ok_or_else(|| unavailable("stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| unavailable("stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| unavailable("stderr"))?;
        Ok(Self::assemble(
            BufReader::new(stdout),
            Box::new(stdin),
            waker,
            Some(child),
            Some(stderr),
        ))
    }

    /// Same reader loop, but over arbitrary streams (used by tests; EOF
    /// yields `Exited { code: None, stderr_tail: "" }`).
    #[cfg(test)]
    pub fn from_streams<R: BufRead + Send + 'static, W: Write + Send + 'static>(
        reader: R,
        writer: W,
        waker: Option<Waker>,
    ) -> (AcpClient, mpsc::Receiver<AcpEvent>) {
        Self::assemble(reader, Box::new(writer), waker, None, None)
    }

    fn assemble<R: BufRead + Send + 'static>(
        reader: R,
        writer: Box<dyn Write + Send>,
        waker: Option<Waker>,
        child: Option<Child>,
        stderr: Option<ChildStderr>,
    ) -> (AcpClient, mpsc::Receiver<AcpEvent>) {
        let (tx, rx) = mpsc::channel::<AcpEvent>();
        let shared = Arc::new(Shared {
            tx,
            writer_slot: Arc::new(Mutex::new(Some(writer))),
            pending: Arc::new(Mutex::new(HashMap::new())),
            stderr_tail: Arc::new(Mutex::new(Vec::new())),
            stderr_handle: Mutex::new(None),
            child: Arc::new(Mutex::new(child)),
        });
        if let Some(stderr) = stderr {
            let tail = shared.stderr_tail.clone();
            let handle = std::thread::spawn(move || drain_stderr(stderr, tail));
            *shared.stderr_handle.lock().unwrap() = Some(handle);
        }
        {
            let shared = shared.clone();
            std::thread::spawn(move || reader_loop(reader, shared, waker));
        }
        let client = AcpClient {
            writer_slot: shared.writer_slot.clone(),
            pending: shared.pending.clone(),
            next_id: AtomicU64::new(1),
            child: shared.child.clone(),
        };
        (client, rx)
    }

    pub fn initialize(&self) -> Result<u64, String> {
        self.send_request(
            RequestKind::Initialize,
            "initialize",
            proto::initialize_params("catwright", env!("CARGO_PKG_VERSION")),
        )
    }

    pub fn new_session(&self, cwd: &Path) -> Result<u64, String> {
        self.send_new_session(cwd, false)
    }

    /// `session/new` for the startup probe: an empty session that only
    /// exists to learn the offered config options. Marked as `probe` so a
    /// failing probe cannot delete the prompt the user is waiting for.
    pub fn new_probe_session(&self, cwd: &Path) -> Result<u64, String> {
        self.send_new_session(cwd, true)
    }

    fn send_new_session(&self, cwd: &Path, probe: bool) -> Result<u64, String> {
        self.send_request(
            RequestKind::NewSession {
                cwd: cwd.to_path_buf(),
                probe,
            },
            "session/new",
            proto::new_session_params(cwd),
        )
    }

    pub fn load_session(&self, cwd: &Path, session_id: &str) -> Result<u64, String> {
        self.send_request(
            RequestKind::LoadSession {
                session_id: session_id.to_string(),
            },
            "session/load",
            proto::load_session_params(cwd, session_id),
        )
    }

    /// `session/resume` of a session that is open in this process: Vibe
    /// answers `{}` and re-sends the session's usage_update (the current
    /// context window included).
    pub fn resume_session(&self, cwd: &Path, session_id: &str) -> Result<u64, String> {
        self.send_request(
            RequestKind::ResumeSession {
                session_id: session_id.to_string(),
            },
            "session/resume",
            proto::resume_session_params(cwd, session_id),
        )
    }

    /// `session/fork`: a new session with the context of `fork_session_id`,
    /// optionally only up to the turn of `message_id`. The response has
    /// `session/new`'s shape, so it arrives as `SessionCreated`.
    pub fn fork_session(
        &self,
        cwd: &Path,
        fork_session_id: &str,
        message_id: Option<&str>,
    ) -> Result<u64, String> {
        self.send_request(
            RequestKind::ForkSession {
                cwd: cwd.to_path_buf(),
                fork_session_id: fork_session_id.to_string(),
                message_id: message_id.map(|m| m.to_string()),
            },
            "session/fork",
            proto::fork_session_params(cwd, fork_session_id, message_id),
        )
    }

    /// Send a Vibe extension request; the answer arrives as `ExtResult`.
    pub fn ext(&self, kind: ExtKind) -> Result<u64, String> {
        let (method, params) = kind.wire();
        self.send_request(RequestKind::Ext(kind), method, params)
    }

    pub fn list_sessions(&self, cwd: Option<&Path>) -> Result<u64, String> {
        self.list_sessions_page(cwd, None)
    }

    /// `session/list` with a pagination cursor (`nextCursor` of the previous
    /// page).
    pub fn list_sessions_page(
        &self,
        cwd: Option<&Path>,
        cursor: Option<&str>,
    ) -> Result<u64, String> {
        self.send_request(
            RequestKind::ListSessions {
                cwd: cwd.map(|p| p.to_path_buf()),
                cursor: cursor.map(|c| c.to_string()),
            },
            "session/list",
            proto::list_sessions_params(cwd, cursor),
        )
    }

    pub fn prompt(&self, session_id: &str, blocks: &[proto::PromptBlock]) -> Result<u64, String> {
        self.send_request(
            RequestKind::Prompt {
                session_id: session_id.to_string(),
            },
            "session/prompt",
            proto::prompt_params(session_id, blocks),
        )
    }

    /// `session/cancel` is a notification (no id, no response).
    pub fn cancel(&self, session_id: &str) -> Result<(), String> {
        self.send_line(&rpc::encode_notification(
            "session/cancel",
            proto::session_id_params(session_id),
        ))
    }

    pub fn set_mode(&self, session_id: &str, mode_id: &str) -> Result<u64, String> {
        self.send_request(
            RequestKind::SetMode {
                session_id: session_id.to_string(),
                mode_id: mode_id.to_string(),
            },
            "session/set_mode",
            proto::set_mode_params(session_id, mode_id),
        )
    }

    pub fn set_config_option(
        &self,
        session_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<u64, String> {
        self.send_request(
            RequestKind::SetConfigOption {
                session_id: session_id.to_string(),
                config_id: config_id.to_string(),
            },
            "session/set_config_option",
            proto::set_config_option_params(session_id, config_id, value),
        )
    }

    pub fn close_session(&self, session_id: &str) -> Result<u64, String> {
        self.send_request(
            RequestKind::CloseSession {
                session_id: session_id.to_string(),
            },
            "session/close",
            proto::session_id_params(session_id),
        )
    }

    /// Answer an agent `session/request_permission`; echo its id verbatim.
    pub fn respond_permission(
        &self,
        request_id: &Value,
        option_id: Option<&str>,
    ) -> Result<(), String> {
        self.send_line(&rpc::encode_result(
            request_id,
            proto::permission_outcome(option_id),
        ))
    }

    /// Answer an agent `elicitation/create`; echo its id verbatim.
    pub fn respond_question(
        &self,
        request_id: &Value,
        action: &str,
        content: Option<Value>,
    ) -> Result<(), String> {
        self.send_line(&rpc::encode_result(
            request_id,
            proto::elicitation_response(action, content),
        ))
    }

    /// Close stdin, wait up to 2 s for the child to exit, then kill it.
    pub fn shutdown(&self) -> Result<(), String> {
        let _ = self.writer_slot.lock().unwrap().take();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = {
                let mut guard = self.child.lock().unwrap();
                match guard.as_mut() {
                    Some(child) => child.try_wait(),
                    None => return Ok(()),
                }
            };
            match status {
                Ok(Some(_)) => {
                    self.child.lock().unwrap().take();
                    return Ok(());
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let mut guard = self.child.lock().unwrap();
                        if let Some(mut child) = guard.take() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => {
                    return Err(trf(
                        "The child process could not be queried: {error}",
                        &[("error", &e)],
                    ))
                }
            }
        }
    }

    /// One line + `\n`, flushed, under the writer mutex.
    fn send_line(&self, line: &str) -> Result<(), String> {
        let mut slot = self
            .writer_slot
            .lock()
            .map_err(|_| tr("Internal error: write lock lost").to_string())?;
        let w = slot
            .as_mut()
            .ok_or_else(|| tr("The connection to vibe-acp is already closed").to_string())?;
        w.write_all(line.as_bytes())
            .and_then(|_| w.write_all(b"\n"))
            .and_then(|_| w.flush())
            .map_err(|e| trf("Sending to vibe-acp failed: {error}", &[("error", &e)]))
    }

    fn send_request(&self, kind: RequestKind, method: &str, params: Value) -> Result<u64, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        // Register the id BEFORE sending, so an instant reply still finds it.
        self.pending.lock().unwrap().insert(id, kind);
        if let Err(e) = self.send_line(&rpc::encode_request(id, method, params)) {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        Ok(id)
    }
}

impl Drop for AcpClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Drain stderr continuously, keeping only the last STDERR_TAIL_BYTES.
fn drain_stderr(mut reader: ChildStderr, tail: Arc<Mutex<Vec<u8>>>) {
    let mut buf = [0u8; 8192];
    let mut acc: Vec<u8> = Vec::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                if acc.len() > STDERR_TAIL_BYTES {
                    acc.drain(..acc.len() - STDERR_TAIL_BYTES);
                }
                *tail.lock().unwrap() = acc.clone();
            }
        }
    }
}

fn reader_loop<R: BufRead + Send + 'static>(
    mut reader: R,
    shared: Arc<Shared>,
    waker: Option<Waker>,
) {
    // A line beyond the budget is skipped, not read into memory; a real
    // read error and EOF both end the loop. Invalid UTF-8 does not end
    // anything: `lines()` used to hand it out as a read error behind
    // `Some(Err(..))`, ending the session view as a clean exit over one
    // stray byte; now the lossy text simply reaches the parser.
    loop {
        match next_wire_line(&mut reader, MAX_WIRE_LINE_BYTES) {
            Ok(WireLine::Line(line)) if line.trim().is_empty() => continue,
            Ok(WireLine::Line(line)) => {
                if let Some(ev) = handle_line(&line, &shared) {
                    if shared.tx.send(ev).is_err() {
                        return; // receiver dropped
                    }
                    if let Some(w) = &waker {
                        w();
                    }
                }
            }
            Ok(WireLine::TooLong) => {
                let ev = AcpEvent::ProtocolError(trf(
                    "The agent sent a line longer than {bytes} bytes; it was skipped.",
                    &[("bytes", &MAX_WIRE_LINE_BYTES.to_string())],
                ));
                if shared.tx.send(ev).is_err() {
                    return;
                }
                if let Some(w) = &waker {
                    w();
                }
            }
            Ok(WireLine::Eof) | Err(_) => break,
        }
    }
    // stdout EOF: give the child a moment to exit, then report Exited with
    // the stderr tail. Never block in wait() while holding the child mutex:
    // a child that closed stdout but keeps running would otherwise deadlock
    // shutdown()/Drop on the UI thread.
    let exit = wait_after_eof(&shared.child, EOF_WAIT);
    // Only an ended child's stderr drain thread finishes on its own; a
    // still-running one keeps stderr open, and joining it would park this
    // thread forever — the harness would never hear Exited (A1.4). The
    // transport is gone either way: report what there is, the drain
    // thread dies with the process, its last chunk unobserved.
    if matches!(exit, ExitPoll::Ended(_)) {
        if let Some(h) = shared.stderr_handle.lock().unwrap().take() {
            let _ = h.join();
        }
    }
    let stderr_tail = String::from_utf8_lossy(&shared.stderr_tail.lock().unwrap()).into_owned();
    let code = match exit {
        ExitPoll::Ended(code) => code,
        ExitPoll::StillRunning => None,
    };
    let _ = shared.tx.send(AcpEvent::Exited { code, stderr_tail });
    if let Some(w) = &waker {
        w();
    }
}

/// How large one agent line may be before it is skipped instead of read.
/// Wire answers — a big `session/load` replay, `_review/turnDiff`
/// results, image blocks — fit many times over; beyond this the peer is
/// broken or hostile, and the reader must not starve the machine on it.
const MAX_WIRE_LINE_BYTES: usize = 64 * 1024 * 1024;

/// How long after stdout EOF the child may take to exit before Exited is
/// reported from a still-running (stdout-abandoned) child.
const EOF_WAIT: Duration = Duration::from_secs(3);

/// One line of the agent's stdout, as the reader hands it on.
#[derive(Debug, PartialEq)]
enum WireLine {
    /// A complete line (or the last before EOF), newline and a preceding
    /// \r stripped, UTF-8 made lossy. Never empty-checked here.
    Line(String),
    /// A line blew the budget: everything up to its newline was skipped.
    TooLong,
    /// The stream ended before any newline and nothing had accumulated.
    Eof,
}

/// Read one line bounded by `budget`: bytes beyond the budget are skipped
/// until the line's newline (reported as [`WireLine::TooLong`]), so a peer
/// writing one endless line cannot grow the buffer without limit. The
/// newline-less rest at EOF is the last line.
fn next_wire_line<R: BufRead>(reader: &mut R, budget: usize) -> std::io::Result<WireLine> {
    let mut line: Vec<u8> = Vec::with_capacity(8 * 1024);
    let mut discarding = false;
    loop {
        // EINTR is a hiccup, not the end: `read_line` inside `lines()`
        // retried interrupted calls; the wire reader must too, or one
        // stray signal would present a dead session as a clean exit.
        let buf = match reader.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buf.is_empty() {
            // EOF: no newline seen. A discarded line never completed, so
            // it owns nothing of what is left to say — that is the end.
            return Ok(if discarding || line.is_empty() {
                WireLine::Eof
            } else {
                WireLine::Line(lossy_line(&line))
            });
        }
        match buf.iter().position(|&b| b == b'\n') {
            Some(at) => {
                // the completed line would grow past the budget (or the
                // rest of a discarded one ends here): skip it, read on
                let over = discarding || line.len() + at > budget;
                if !over {
                    line.extend_from_slice(&buf[..at]);
                }
                reader.consume(at + 1);
                return Ok(if over {
                    WireLine::TooLong
                } else {
                    WireLine::Line(lossy_line(&line))
                });
            }
            None => {
                let n = buf.len();
                if discarding {
                    reader.consume(n);
                } else if line.len() + n <= budget {
                    line.extend_from_slice(buf);
                    reader.consume(n);
                } else {
                    // fill to the budget for the honest skip message, then
                    // drop the buffer — the rest of the line is unread
                    let keep = budget - line.len();
                    reader.consume(keep);
                    discarding = true;
                    line.clear();
                    line.shrink_to_fit();
                }
            }
        }
    }
}

/// The wire line without its newline (and a preceding `\r`, as
/// `BufRead::lines` stripped both), invalid UTF-8 read lossily: the
/// parser judges the JSON, not the bytes.
fn lossy_line(bytes: &[u8]) -> String {
    let mut line = bytes;
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    String::from_utf8_lossy(line).into_owned()
}

/// Poll the child for up to `timeout` (lock released between polls). A
/// child that is still running afterwards is left for shutdown()/Drop;
/// `Ended` is the exited one (the code is None when a signal killed it).
fn wait_after_eof(child: &Mutex<Option<Child>>, timeout: Duration) -> ExitPoll {
    let deadline = Instant::now() + timeout;
    loop {
        // no child at all (stream tests): the report goes out at once
        let mut guard = child.lock().unwrap();
        let Some(live) = guard.as_mut() else {
            return ExitPoll::Ended(None);
        };
        match live.try_wait() {
            Ok(Some(s)) => return ExitPoll::Ended(s.code()),
            Ok(None) if Instant::now() < deadline => {
                drop(guard);
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => return ExitPoll::StillRunning,
        }
    }
}

/// What `wait_after_eof` found: an ended child's exit code (None for a
/// signal), or a child that closed stdout but lives on.
enum ExitPoll {
    Ended(Option<i32>),
    StillRunning,
}

/// Handle one non-empty line; returns the event to deliver (if any).
/// Error replies to the agent are sent directly.
fn handle_line(line: &str, shared: &Shared) -> Option<AcpEvent> {
    match rpc::parse_incoming(line) {
        Err(msg) => Some(AcpEvent::ProtocolError(msg)),
        Ok(Incoming::Response { id, result }) => {
            let kind = shared.pending.lock().unwrap().remove(&id);
            match kind {
                None => Some(AcpEvent::ProtocolError(trf(
                    "Response with unknown id {id}",
                    &[("id", &id)],
                ))),
                Some(kind) => match result {
                    Err(error) => Some(AcpEvent::RequestFailed {
                        request: kind,
                        error,
                    }),
                    Ok(value) => match route_response(kind.clone(), value) {
                        Ok(ev) => Some(ev),
                        Err(error) => Some(AcpEvent::RequestFailed {
                            request: kind,
                            error,
                        }),
                    },
                },
            }
        }
        Ok(Incoming::Notification { method, params }) => {
            if method == "session/update" {
                match serde_json::from_value::<proto::SessionNotification>(params) {
                    Ok(n) => Some(AcpEvent::Update {
                        session_id: n.session_id,
                        update: n.update,
                    }),
                    Err(_) => Some(AcpEvent::ProtocolError(
                        tr("The session/update message could not be read").to_string(),
                    )),
                }
            } else if method == "_session/retrying" {
                let text = |key: &str| {
                    params
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                Some(AcpEvent::Retrying {
                    session_id: text("sessionId"),
                    category: text("category"),
                    detail: text("detail"),
                })
            } else if method == "_voice/transcriptionDelta" {
                // a piece of recognised text to append, or Vibe's error
                let field = |key: &str| params.get(key).and_then(Value::as_str).map(str::to_string);
                Some(AcpEvent::VoiceDelta {
                    text: field("text"),
                    error: field("error"),
                })
            } else {
                None // unknown notifications are ignored
            }
        }
        Ok(Incoming::Request { id, method, params }) => {
            if method == "session/request_permission" {
                match serde_json::from_value::<proto::PermissionRequest>(params) {
                    Ok(request) => Some(AcpEvent::PermissionRequested {
                        request_id: id,
                        request,
                    }),
                    Err(_) => {
                        send_raw(
                            shared,
                            &rpc::encode_error(&id, rpc::INVALID_PARAMS, "Invalid params"),
                        );
                        Some(AcpEvent::ProtocolError(
                            tr("The session/request_permission could not be read").to_string(),
                        ))
                    }
                }
            } else if method == "elicitation/create" {
                Some(AcpEvent::QuestionAsked {
                    request_id: id,
                    params,
                })
            } else {
                // fs/*, terminal/* and anything else: unsupported
                send_raw(
                    shared,
                    &rpc::encode_error(&id, rpc::METHOD_NOT_FOUND, "Method not found"),
                );
                None
            }
        }
    }
}

/// A response the agent answered with something we cannot parse must not
/// leave its request dangling (a turn would stay "running" forever): it is
/// reported as `RequestFailed` for the request instead.
/// Marker in `RpcError::data` of an error this client made up because the
/// agent's answer could not be read (the request itself was processed).
const UNREADABLE_RESPONSE: &str = "client:unreadable-response";

/// Is `error` this client's "the answer could not be read" (the agent did
/// process the request), rather than an error the agent sent?
pub fn is_unreadable_response(error: &rpc::RpcError) -> bool {
    error
        .data
        .as_ref()
        .and_then(|d| d.get("origin"))
        .and_then(Value::as_str)
        == Some(UNREADABLE_RESPONSE)
}

pub(crate) fn parse_error(method: &str) -> rpc::RpcError {
    rpc::RpcError {
        code: rpc::INVALID_PARAMS,
        message: trf(
            "The {method} response could not be read",
            &[("method", &method)],
        ),
        data: Some(json!({ "origin": UNREADABLE_RESPONSE })),
    }
}

fn route_response(kind: RequestKind, value: Value) -> Result<AcpEvent, rpc::RpcError> {
    match kind {
        RequestKind::Initialize => {
            let result = serde_json::from_value::<proto::InitializeResult>(value)
                .map_err(|_| parse_error("initialize"))?;
            Ok(AcpEvent::Initialized(result))
        }
        RequestKind::NewSession { cwd, probe } => {
            let result = serde_json::from_value::<proto::NewSessionResult>(value)
                .map_err(|_| parse_error("session/new"))?;
            let origin = if probe {
                SessionOrigin::Probe
            } else {
                SessionOrigin::New
            };
            Ok(AcpEvent::SessionCreated {
                cwd,
                origin,
                result,
            })
        }
        RequestKind::LoadSession { session_id } => {
            let result = serde_json::from_value::<proto::LoadSessionResult>(value)
                .map_err(|_| parse_error("session/load"))?;
            Ok(AcpEvent::SessionLoaded { session_id, result })
        }
        RequestKind::ForkSession {
            cwd,
            fork_session_id,
            message_id,
        } => {
            let result = serde_json::from_value::<proto::NewSessionResult>(value)
                .map_err(|_| parse_error("session/fork"))?;
            Ok(AcpEvent::SessionCreated {
                cwd,
                origin: SessionOrigin::Fork {
                    source: fork_session_id,
                    message_id,
                },
                result,
            })
        }
        RequestKind::ListSessions { cwd, cursor } => {
            let result = serde_json::from_value::<proto::ListSessionsResult>(value)
                .map_err(|_| parse_error("session/list"))?;
            Ok(AcpEvent::SessionsListed {
                cwd,
                cursor,
                result,
            })
        }
        RequestKind::Prompt { session_id } => {
            let result = serde_json::from_value::<proto::PromptResult>(value)
                .map_err(|_| parse_error("session/prompt"))?;
            Ok(AcpEvent::PromptFinished { session_id, result })
        }
        // set_mode and close answer with `{}`; the payload carries no data.
        RequestKind::SetMode {
            session_id,
            mode_id,
        } => Ok(AcpEvent::ModeSet {
            session_id,
            mode_id,
        }),
        RequestKind::SetConfigOption { session_id, .. } => {
            let r = serde_json::from_value::<proto::ConfigOptions>(value)
                .map_err(|_| parse_error("session/set_config_option"))?;
            Ok(AcpEvent::ConfigOptionsChanged {
                session_id,
                options: r.config_options,
            })
        }
        RequestKind::CloseSession { session_id } => Ok(AcpEvent::SessionClosed { session_id }),
        // the payload carries nothing the harness needs; the usage_update
        // Vibe sends with it arrives as a normal session update
        RequestKind::ResumeSession { .. } => Ok(AcpEvent::SessionResumed),
        RequestKind::Ext(kind) => match kind {
            // The voice answers are `{"ok":true}` or
            // `{"ok":false,"error":"…"}`; the error text is Vibe's own.
            ExtKind::VoiceTranscribeStart { .. } => {
                let (ok, error) = voice_answer(&value);
                Ok(AcpEvent::VoiceStarted { ok, error })
            }
            ExtKind::VoiceTranscribeStop => {
                let (ok, error) = voice_answer(&value);
                Ok(AcpEvent::VoiceStopped { ok, error })
            }
            ExtKind::VoiceTranscribeCancel => Ok(AcpEvent::VoiceCancelled),
            kind => Ok(AcpEvent::ExtResult {
                kind,
                result: value,
            }),
        },
    }
}

/// `ok` and Vibe's `error` text of a `_voice/*` answer.
fn voice_answer(value: &Value) -> (bool, Option<String>) {
    let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let error = value
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string);
    (ok, error)
}

/// Best-effort write from the reader thread (error replies).
fn send_raw(shared: &Shared, line: &str) {
    let mut slot = shared.writer_slot.lock().unwrap();
    if let Some(w) = slot.as_mut() {
        let _ = w.write_all(line.as_bytes());
        let _ = w.write_all(b"\n");
        let _ = w.flush();
    }
}

// ---------------------------------------------------------------------------
// tests: the test itself plays the agent over a UnixStream pair
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The wire form of every extension request, verified against Vibe 3.1.0.
    #[test]
    fn ext_requests_use_the_verified_wire_format() {
        let sid = "s1".to_string();
        let (m, p) = ExtKind::SetTitle {
            session_id: sid.clone(),
            title: "Neu".into(),
            cwd: PathBuf::from("/w"),
        }
        .wire();
        assert_eq!(m, "_session/set_title");
        assert_eq!(p, json!({"sessionId": "s1", "title": "Neu"}));
        let (m, p) = ExtKind::RewindTo {
            session_id: sid.clone(),
            message_id: "m1".into(),
            restore_files: true,
        }
        .wire();
        assert_eq!(m, "_rewind/to");
        assert_eq!(
            p,
            json!({"sessionId": "s1", "messageId": "m1", "restoreFiles": true})
        );
        let (m, p) = ExtKind::LoopsCreate {
            session_id: sid.clone(),
            interval: "5m".into(),
            prompt: "x".into(),
        }
        .wire();
        assert_eq!(m, "_loops/create");
        assert_eq!(p["interval"], json!("5m"));
        let (m, p) = ExtKind::ReviewMutate {
            session_id: sid.clone(),
            approve: false,
            target: json!({"kind": "all"}),
        }
        .wire();
        assert_eq!(m, "_review/revert");
        assert_eq!(p["target"]["kind"], json!("all"));
        let (m, _) = ExtKind::ReviewMutate {
            session_id: sid.clone(),
            approve: true,
            target: json!({"kind": "all"}),
        }
        .wire();
        assert_eq!(m, "_review/approve");
        let (m, p) = ExtKind::ReviewTurnDiff {
            session_id: sid.clone(),
            path: "/w/src/a.rs".into(),
            turn_id: "m1".into(),
        }
        .wire();
        assert_eq!(m, "_review/turnDiff");
        assert_eq!(
            p,
            json!({"sessionId": "s1", "path": "/w/src/a.rs", "owner": {"turnId": "m1"}})
        );
        let (m, p) = ExtKind::TrustDecision {
            session_id: sid,
            cwd: PathBuf::from("/w"),
            decision: "trust_cwd".into(),
        }
        .wire();
        assert_eq!(m, "_trust/decision");
        assert_eq!(p["cwd"], json!("/w"));
        assert_eq!(p["decision"], json!("trust_cwd"));
        // the voice requests carry no session id (Vibe uses the first open
        // session of its process)
        let (m, p) = ExtKind::VoiceTranscribeStart {
            language: "de".into(),
        }
        .wire();
        assert_eq!(m, "_voice/transcribeStart");
        assert_eq!(p, json!({"language": "de"}));
        let (m, p) = ExtKind::VoiceTranscribeStop.wire();
        assert_eq!(m, "_voice/transcribeStop");
        assert_eq!(p, json!({}));
        let (m, p) = ExtKind::VoiceTranscribeCancel.wire();
        assert_eq!(m, "_voice/transcribeCancel");
        assert_eq!(p, json!({}));
    }

    /// `_voice/transcriptionDelta` carries either a piece of text or Vibe's
    /// error; both reach the harness.
    #[test]
    fn transcription_delta_notification_becomes_an_event() {
        let (tx, rx) = mpsc::channel::<AcpEvent>();
        std::mem::forget(rx);
        let shared = Shared {
            tx,
            writer_slot: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            stderr_tail: Arc::new(Mutex::new(Vec::new())),
            stderr_handle: Mutex::new(None),
            child: Arc::new(Mutex::new(None)),
        };
        let line =
            r#"{"jsonrpc":"2.0","method":"_voice/transcriptionDelta","params":{"text":"Hallo "}}"#;
        match handle_line(line, &shared) {
            Some(AcpEvent::VoiceDelta { text, error }) => {
                assert_eq!(text.as_deref(), Some("Hallo "));
                assert_eq!(error, None);
            }
            other => panic!("unexpected {other:?}"),
        }
        let line = r#"{"jsonrpc":"2.0","method":"_voice/transcriptionDelta","params":{"error":"No audio input device found"}}"#;
        match handle_line(line, &shared) {
            Some(AcpEvent::VoiceDelta { text, error }) => {
                assert_eq!(text, None);
                assert_eq!(error.as_deref(), Some("No audio input device found"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// The three voice answers are routed to their own events, with Vibe's
    /// error text kept as it came.
    #[test]
    fn voice_answers_are_routed() {
        let started = route_response(
            RequestKind::Ext(ExtKind::VoiceTranscribeStart {
                language: "en".into(),
            }),
            json!({"ok": true}),
        )
        .expect("started");
        assert!(matches!(
            started,
            AcpEvent::VoiceStarted {
                ok: true,
                error: None
            }
        ));
        let failed = route_response(
            RequestKind::Ext(ExtKind::VoiceTranscribeStart {
                language: "en".into(),
            }),
            json!({"ok": false, "error": "Transcription already in progress"}),
        )
        .expect("failed start");
        match failed {
            AcpEvent::VoiceStarted { ok, error } => {
                assert!(!ok);
                assert_eq!(error.as_deref(), Some("Transcription already in progress"));
            }
            other => panic!("unexpected {other:?}"),
        }
        let stopped = route_response(
            RequestKind::Ext(ExtKind::VoiceTranscribeStop),
            json!({"ok": true, "text": ""}),
        )
        .expect("stopped");
        assert!(matches!(
            stopped,
            AcpEvent::VoiceStopped {
                ok: true,
                error: None
            }
        ));
        let cancelled = route_response(
            RequestKind::Ext(ExtKind::VoiceTranscribeCancel),
            json!({"ok": true}),
        )
        .expect("cancelled");
        assert!(matches!(cancelled, AcpEvent::VoiceCancelled));
    }

    /// `elicitation/create` is delivered to the harness instead of being
    /// answered with -32601.
    #[test]
    fn elicitation_request_becomes_an_event() {
        let shared = {
            let (tx, rx) = mpsc::channel::<AcpEvent>();
            std::mem::forget(rx);
            Shared {
                tx,
                writer_slot: Arc::new(Mutex::new(None)),
                pending: Arc::new(Mutex::new(HashMap::new())),
                stderr_tail: Arc::new(Mutex::new(Vec::new())),
                stderr_handle: Mutex::new(None),
                child: Arc::new(Mutex::new(None)),
            }
        };
        let line =
            r#"{"jsonrpc":"2.0","id":7,"method":"elicitation/create","params":{"message":"m"}}"#;
        match handle_line(line, &shared) {
            Some(AcpEvent::QuestionAsked { request_id, params }) => {
                assert_eq!(request_id, json!(7));
                assert_eq!(params["message"], json!("m"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn retrying_notification_becomes_an_event() {
        let (tx, rx) = mpsc::channel::<AcpEvent>();
        std::mem::forget(rx);
        let shared = Shared {
            tx,
            writer_slot: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            stderr_tail: Arc::new(Mutex::new(Vec::new())),
            stderr_handle: Mutex::new(None),
            child: Arc::new(Mutex::new(None)),
        };
        let line = r#"{"jsonrpc":"2.0","method":"_session/retrying","params":{"sessionId":"s","category":"rate_limited","detail":"429"}}"#;
        match handle_line(line, &shared) {
            Some(AcpEvent::Retrying {
                session_id,
                category,
                detail,
            }) => {
                assert_eq!(session_id, "s");
                assert_eq!(category, "rate_limited");
                assert_eq!(detail, "429");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    const HANDSHAKE: &str = include_str!("../../tests/fixtures/acp/handshake.jsonl");
    const RECV: Duration = Duration::from_secs(5);

    /// The test's end of the wire ("the agent").
    struct Agent {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
    }

    impl Agent {
        fn read_msg(&mut self) -> Value {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).expect("agent read");
            assert!(n > 0, "agent read hit EOF");
            serde_json::from_str(line.trim()).expect("agent line is JSON")
        }

        fn send(&mut self, msg: Value) {
            let line = format!("{}\n", serde_json::to_string(&msg).unwrap());
            self.writer.write_all(line.as_bytes()).expect("agent write");
            self.writer.flush().expect("agent flush");
        }
    }

    fn setup() -> (AcpClient, mpsc::Receiver<AcpEvent>, Agent) {
        let (a, b) = UnixStream::pair().expect("socket pair");
        a.set_read_timeout(Some(RECV)).unwrap();
        b.set_read_timeout(Some(RECV)).unwrap();
        // socketpair: what is written to `a` is readable at `b` and vice versa
        let (client, rx) = AcpClient::from_streams(
            BufReader::new(a.try_clone().expect("clone a")),
            a.try_clone().expect("clone a again"),
            None,
        );
        let agent = Agent {
            reader: BufReader::new(b.try_clone().expect("clone b")),
            writer: b,
        };
        (client, rx, agent)
    }

    fn fixture_initialize_result() -> Value {
        for line in HANDSHAKE.lines() {
            let v: Value = serde_json::from_str(line).unwrap();
            if v["dir"] == "a2c" && v["msg"]["id"] == 1 && v["msg"].get("result").is_some() {
                return v["msg"]["result"].clone();
            }
        }
        panic!("fixture initialize result not found");
    }

    #[test]
    fn initialize_request_and_initialized_event() {
        let (client, rx, mut agent) = setup();
        let id = client.initialize().unwrap();
        assert_eq!(id, 1, "client request ids start at 1");
        let req = agent.read_msg();
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["id"], json!(1));
        assert_eq!(req["method"], "initialize");
        assert_eq!(req["params"]["protocolVersion"], json!(1));
        assert_eq!(
            req["params"]["clientCapabilities"]["terminal"],
            json!(false)
        );
        assert_eq!(
            req["params"]["clientCapabilities"]["fs"]["readTextFile"],
            json!(false)
        );

        agent.send(json!({"jsonrpc": "2.0", "id": 1, "result": fixture_initialize_result()}));
        match rx.recv_timeout(RECV).expect("Initialized event") {
            AcpEvent::Initialized(r) => {
                assert_eq!(r.protocol_version, 1);
                assert!(r.agent_capabilities.load_session);
            }
            other => panic!("expected Initialized, got {other:?}"),
        }
    }

    #[test]
    fn permission_request_event_and_verbatim_response() {
        let (client, rx, mut agent) = setup();
        agent.send(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "session/request_permission",
            "params": {
                "sessionId": "s1",
                "toolCall": {"toolCallId": "t1"},
                "options": [
                    {"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"},
                    {"optionId": "reject_once", "name": "Deny", "kind": "reject_once"}
                ]
            }
        }));
        let (request_id, request) = match rx.recv_timeout(RECV).expect("PermissionRequested") {
            AcpEvent::PermissionRequested {
                request_id,
                request,
            } => (request_id, request),
            other => panic!("expected PermissionRequested, got {other:?}"),
        };
        assert_eq!(request_id, json!(0));
        assert_eq!(request.session_id, "s1");
        assert_eq!(request.tool_call.tool_call_id, "t1");
        assert_eq!(request.options.len(), 2);
        assert_eq!(request.options[0].kind, proto::PermissionKind::AllowOnce);

        client
            .respond_permission(&json!(0), Some("allow_once"))
            .unwrap();
        let resp = agent.read_msg();
        assert_eq!(
            resp,
            json!({"jsonrpc": "2.0", "id": 0, "result": {"outcome": {"outcome": "selected", "optionId": "allow_once"}}})
        );

        client.respond_permission(&json!(0), None).unwrap();
        let resp = agent.read_msg();
        assert_eq!(
            resp,
            json!({"jsonrpc": "2.0", "id": 0, "result": {"outcome": {"outcome": "cancelled"}}})
        );
    }

    #[test]
    fn unknown_agent_requests_get_method_not_found() {
        let (client, rx, mut agent) = setup();
        let _ = client.initialize().unwrap();
        let _ = agent.read_msg();

        agent.send(json!({"jsonrpc": "2.0", "id": 7, "method": "fs/read_text_file", "params": {"path": "/x"}}));
        let resp = agent.read_msg();
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], json!(7));
        assert_eq!(resp["error"]["code"], json!(-32601));
        assert_eq!(resp["error"]["message"], "Method not found");

        // string ids are echoed verbatim
        agent.send(
            json!({"jsonrpc": "2.0", "id": "abc", "method": "terminal/create", "params": {}}),
        );
        let resp = agent.read_msg();
        assert_eq!(resp["id"], json!("abc"));
        assert_eq!(resp["error"]["code"], json!(rpc::METHOD_NOT_FOUND));

        // no event is emitted for unsupported requests
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn cancel_is_a_notification_without_id() {
        let (client, _rx, mut agent) = setup();
        client.cancel("s1").unwrap();
        let msg = agent.read_msg();
        assert_eq!(msg["jsonrpc"], "2.0");
        assert_eq!(msg["method"], "session/cancel");
        assert!(msg.get("id").is_none(), "cancel must not carry an id");
        assert_eq!(msg["params"], json!({"sessionId": "s1"}));
    }

    #[test]
    fn error_response_maps_to_request_failed() {
        let (client, rx, mut agent) = setup();
        let id = client.new_session(Path::new("/tmp/ws")).unwrap();
        let req = agent.read_msg();
        assert_eq!(req["method"], "session/new");
        assert_eq!(req["id"], json!(id));
        assert_eq!(req["params"]["cwd"], json!("/tmp/ws"));
        assert_eq!(req["params"]["mcpServers"], json!([]));

        agent.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32000, "message": "authentication required"}
        }));
        match rx.recv_timeout(RECV).expect("RequestFailed") {
            AcpEvent::RequestFailed { request, error } => {
                assert_eq!(
                    request,
                    RequestKind::NewSession {
                        cwd: PathBuf::from("/tmp/ws"),
                        probe: false,
                    }
                );
                assert_eq!(error.code, rpc::AUTH_REQUIRED);
                assert_eq!(error.code, -32000);
            }
            other => panic!("expected RequestFailed, got {other:?}"),
        }
    }

    #[test]
    fn probe_session_new_is_marked_and_still_routes() {
        let (client, rx, mut agent) = setup();
        // the probe request looks like a normal session/new on the wire …
        let id = client.new_probe_session(Path::new("/tmp/ws")).unwrap();
        let req = agent.read_msg();
        assert_eq!(req["method"], "session/new");
        assert_eq!(req["params"]["cwd"], json!("/tmp/ws"));
        // … its successful answer routes as a probe-marked SessionCreated …
        agent.send(json!({"jsonrpc": "2.0", "id": id, "result": {"sessionId": "probe"}}));
        match rx.recv_timeout(RECV).expect("SessionCreated event") {
            AcpEvent::SessionCreated {
                cwd,
                origin,
                result,
            } => {
                assert_eq!(cwd, PathBuf::from("/tmp/ws"));
                assert_eq!(origin, SessionOrigin::Probe);
                assert_eq!(result.session_id, "probe");
            }
            other => panic!("expected SessionCreated, got {other:?}"),
        }
        // … and a failing answer keeps the probe marker, so the harness can
        // tell it apart from the user's own session/new for the same cwd
        let _ = client.new_probe_session(Path::new("/tmp/ws")).unwrap();
        let req = agent.read_msg();
        let id = req["id"].clone();
        agent.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32000, "message": "authentication required"}
        }));
        match rx.recv_timeout(RECV).expect("RequestFailed event") {
            AcpEvent::RequestFailed {
                request: RequestKind::NewSession { cwd, probe },
                ..
            } => {
                assert_eq!(cwd, PathBuf::from("/tmp/ws"));
                assert!(probe, "the probe request must stay recognisable");
            }
            other => panic!("expected RequestFailed for NewSession, got {other:?}"),
        }
    }

    /// Bug 3 (Handoff 2026-11-09): a response the agent worded unexpectedly
    /// must not leave its request dangling (a turn would stay "running");
    /// it is reported as `RequestFailed` for the request instead.
    #[test]
    fn unparsable_result_becomes_request_failed_for_the_request() {
        let (client, rx, mut agent) = setup();
        let _ = client.prompt("s1", &[]).unwrap();
        let req = agent.read_msg();
        assert_eq!(req["method"], "session/prompt");
        agent.send(json!({"jsonrpc": "2.0", "id": req["id"], "result": "fertig"}));
        match rx.recv_timeout(RECV).expect("RequestFailed event") {
            AcpEvent::RequestFailed { request, error } => {
                assert_eq!(
                    request,
                    RequestKind::Prompt {
                        session_id: "s1".to_string()
                    }
                );
                assert_eq!(error.code, rpc::INVALID_PARAMS);
                assert!(error.message.contains("session/prompt"));
            }
            other => panic!("expected RequestFailed, got {other:?}"),
        }
    }

    #[test]
    fn session_update_notification_becomes_update_event() {
        let (client, rx, mut agent) = setup();
        let _ = client.initialize().unwrap();
        let _ = agent.read_msg();

        agent.send(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {"sessionId": "s1", "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "Hallo"},
                "messageId": "m1"
            }}
        }));
        match rx.recv_timeout(RECV).expect("Update event") {
            AcpEvent::Update { session_id, update } => {
                assert_eq!(session_id, "s1");
                match update {
                    proto::SessionUpdate::AgentMessageChunk(chunk) => {
                        assert_eq!(chunk.content.as_text(), Some("Hallo"));
                        assert_eq!(chunk.message_id.as_deref(), Some("m1"));
                    }
                    other => panic!("expected AgentMessageChunk, got {other:?}"),
                }
            }
            other => panic!("expected Update, got {other:?}"),
        }

        // unknown notifications are ignored silently
        agent.send(json!({"jsonrpc": "2.0", "method": "something/else", "params": {}}));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn eof_on_stream_yields_exited_without_code() {
        let (client, rx, mut agent) = setup();
        let _ = client.initialize().unwrap();
        let _ = agent.read_msg();
        agent.writer.shutdown(Shutdown::Write).unwrap();
        match rx.recv_timeout(RECV).expect("Exited event") {
            AcpEvent::Exited { code, stderr_tail } => {
                assert_eq!(code, None);
                assert_eq!(stderr_tail, "");
            }
            other => panic!("expected Exited, got {other:?}"),
        }
    }

    #[test]
    fn waker_fires_after_events() {
        let (a, b) = UnixStream::pair().unwrap();
        a.set_read_timeout(Some(RECV)).unwrap();
        b.set_read_timeout(Some(RECV)).unwrap();
        let counter = Arc::new(AtomicU64::new(0));
        let waker: Waker = {
            let c = counter.clone();
            Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            })
        };
        let (client, rx) = AcpClient::from_streams(
            BufReader::new(a.try_clone().unwrap()),
            a.try_clone().unwrap(),
            Some(waker),
        );
        let mut agent = Agent {
            reader: BufReader::new(b.try_clone().unwrap()),
            writer: b,
        };
        let _ = client.initialize().unwrap();
        let _ = agent.read_msg();
        agent.send(json!({"jsonrpc": "2.0", "id": 1, "result": fixture_initialize_result()}));
        let _ = rx.recv_timeout(RECV).expect("event");
        for _ in 0..100 {
            if counter.load(Ordering::SeqCst) >= 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            counter.load(Ordering::SeqCst) >= 1,
            "waker must run after the event"
        );
    }

    /// Live check against the real `vibe-acp` (no prompt, no model call):
    /// `cargo test -- --ignored live_vibe_acp_handshake`
    #[test]
    #[ignore = "needs a local vibe-acp installation"]
    fn live_vibe_acp_handshake() {
        let (client, rx) = AcpClient::spawn("vibe-acp", &[], None).expect("spawn vibe-acp");
        let next = |rx: &mpsc::Receiver<AcpEvent>| loop {
            match rx.recv_timeout(Duration::from_secs(60)).expect("event") {
                AcpEvent::Update { .. } => continue,
                other => break other,
            }
        };
        client.initialize().unwrap();
        assert!(matches!(next(&rx), AcpEvent::Initialized(r) if r.agent_capabilities.load_session));
        let dir = std::env::temp_dir().join(format!("acp-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        client.new_session(&dir).unwrap();
        let sid = match next(&rx) {
            AcpEvent::SessionCreated { result, .. } => result.session_id,
            other => panic!("expected SessionCreated, got {other:?}"),
        };
        client.close_session(&sid).unwrap();
        assert!(matches!(next(&rx), AcpEvent::SessionClosed { .. }));
        client.shutdown().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shutdown_closes_the_writer() {
        let (client, _rx, mut agent) = setup();
        let _ = client.initialize().unwrap();
        let _ = agent.read_msg();
        client.shutdown().unwrap();
        assert!(
            client.initialize().is_err(),
            "sending after shutdown must fail"
        );
    }

    #[test]
    fn wire_lines_strip_crlf_and_read_lossy_utf8() {
        let mut r = std::io::Cursor::new(b"{\"id\":1}\r\n\xff kein json\nlast".to_vec());
        // \r\n is stripped like BufRead::lines did
        assert_eq!(
            next_wire_line(&mut r, 1024).unwrap(),
            WireLine::Line("{\"id\":1}".into())
        );
        // invalid UTF-8 arrives as a lossy line, not as the end of the agent
        let second = match next_wire_line(&mut r, 1024).unwrap() {
            WireLine::Line(l) => l,
            other => panic!("expected a line, got {other:?}"),
        };
        assert!(second.contains("kein json"));
        assert!(second.contains('�'));
        // the rest without a newline is the last line, then EOF
        assert_eq!(
            next_wire_line(&mut r, 1024).unwrap(),
            WireLine::Line("last".into())
        );
        assert_eq!(next_wire_line(&mut r, 1024).unwrap(), WireLine::Eof);
    }

    #[test]
    fn wire_line_over_the_budget_is_skipped_not_read() {
        let mut r = std::io::Cursor::new(b"0123456789ABCDEF\nnext\n".to_vec());
        // a line past the budget is dropped whole; the stream continues
        assert_eq!(next_wire_line(&mut r, 8).unwrap(), WireLine::TooLong);
        assert_eq!(
            next_wire_line(&mut r, 8).unwrap(),
            WireLine::Line("next".into())
        );
        assert_eq!(next_wire_line(&mut r, 8).unwrap(), WireLine::Eof);
        // exactly at the budget is still a line
        let mut r = std::io::Cursor::new(b"01234567\n".to_vec());
        assert_eq!(
            next_wire_line(&mut r, 8).unwrap(),
            WireLine::Line("01234567".into())
        );
        // a line that blows the budget only at EOF counts as the end:
        // it never completed
        let mut r = std::io::Cursor::new(b"0123456789ABCDEF".to_vec());
        assert_eq!(next_wire_line(&mut r, 8).unwrap(), WireLine::Eof);
    }

    #[test]
    fn wire_line_accumulates_across_tiny_reads() {
        let mut r = std::io::Cursor::new(b"ab\ncd".to_vec());
        let mut piecewise = std::io::BufReader::with_capacity(1, &mut r);
        assert_eq!(
            next_wire_line(&mut piecewise, 100).unwrap(),
            WireLine::Line("ab".into())
        );
        assert_eq!(
            next_wire_line(&mut piecewise, 100).unwrap(),
            WireLine::Line("cd".into())
        );
        assert_eq!(next_wire_line(&mut piecewise, 100).unwrap(), WireLine::Eof);
    }

    /// The reader thread survives invalid UTF-8 (one stray byte no longer
    /// ends the session as a clean exit) and still parses what follows.
    #[test]
    fn reader_loop_survives_invalid_utf8_and_reports_exit() {
        let (tx, rx) = mpsc::channel::<AcpEvent>();
        let shared = Shared {
            tx,
            writer_slot: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            stderr_tail: Arc::new(Mutex::new(Vec::new())),
            stderr_handle: Mutex::new(None),
            child: Arc::new(Mutex::new(None)),
        };
        let junk_then_real = b"\xff\xfe\xfd garbage\n\
            {\"jsonrpc\":\"2.0\",\"method\":\"_session/retrying\",\"params\":{\"sessionId\":\"s\",\"category\":\"rate_limited\",\"detail\":\"429\"}}\n"
            .to_vec();
        // rx stays ours throughout: the events land while we hold it
        reader_loop(std::io::Cursor::new(junk_then_real), Arc::new(shared), None);
        // the junk line is reported as a protocol error, not as the end
        match rx.try_recv() {
            Ok(AcpEvent::ProtocolError(msg)) => assert!(msg.contains("not valid JSON")),
            other => panic!("expected a protocol error for the junk line, got {other:?}"),
        }
        // and the session lives on: the next line parses
        match rx.try_recv() {
            Ok(AcpEvent::Retrying { category, .. }) => assert_eq!(category, "rate_limited"),
            other => panic!("expected Retrying after invalid UTF-8, got {other:?}"),
        }
        match rx.try_recv() {
            Ok(AcpEvent::Exited { code, .. }) => assert_eq!(code, None),
            other => panic!("expected Exited, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "nothing follows the report");
    }
}
