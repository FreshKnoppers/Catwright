//! Harness state: drives `vibe-acp` over ACP (see docs/acp.md) and folds
//! `AcpEvent`s into `SessionView`s, for every workspace (the chat workspace
//! "Allgemein" and the project folders) in one view. No egui drawing in
//! here — the egui code lives in `harness::ui`. UI strings are German;
//! nothing is logged.

pub mod context;
pub mod extras;
pub mod state;
pub mod ui;
pub mod watch;

use crate::acp::client::{AcpClient, AcpEvent, ExtKind, RequestKind, SessionOrigin, Waker};
use crate::acp::protocol::{
    AvailableCommand, ConfigOption, ModeState, PromptBlock, SessionInfo, SessionMode,
    SessionUpdate, StopReason,
};
use crate::acp::rpc;
use crate::acp::session::{Item, PendingPermission, SessionView, TurnState};
use crate::dialogs::FolderPicker;
use crate::i18n::{tr, trf};
use crate::model::UiEvent;
use crate::render;
use extras::{
    AccountInfo, ConnectorInfo, IdentityInfo, LoopInfo, PendingQuestion, ReviewFile, ReviewScope,
    TrustInfo,
};
use serde_json::{json, Value};
use state::{
    ActivityEvent, ActivityKind, PreviewMode, PreviewState, QueuedPrompt, TurnDiffView, TurnDiffs,
    UiState,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Automatic restarts of `vibe-acp` before giving up (delays 1, 2, 4, 8, 16 s).
pub const MAX_RECONNECTS: u32 = 5;
/// A connection that stayed up this long resets the restart counter.
const STABLE_AFTER: Duration = Duration::from_secs(60);
/// How long `initialize` may stay unanswered before the silent agent counts
/// as lost: `poll` then kills the child through `connection_lost` and the
/// reconnect schedule starts (review M6 — a living but mute `vibe-acp`
/// used to leave `Conn::Starting` forever, with no exit ever arriving).
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// How many pages of one `session/list` the harness asks for before it
/// ends the chain itself (audit N2: the cursor chain is the agent's word).
const MAX_LIST_PAGES: u32 = 200;

/// Connection state to the `vibe-acp` child process.
#[derive(Debug, Clone, PartialEq)]
pub enum Conn {
    Off,
    Starting,
    Ready {
        agent: String,
    },
    /// `vibe-acp` ended unexpectedly; restart attempt `attempt` is due at `at`.
    Reconnecting {
        attempt: u32,
        at: Instant,
    },
    Failed(String),
}

/// One setting applied to a fresh session before its first prompt.
#[derive(Debug, Clone, PartialEq)]
enum SetupStep {
    Mode(String),
    Config(String, String),
}

/// Where [`Harness::park_or_restore`] put a message: into the composer
/// on screen, parked under the session that owns it (it shows up when
/// the user opens that session, review H3/M13), or nothing to put back.
/// `attachments` says the wire blocks also carried attachment blocks
/// (image / resource link): their content cannot go back into the
/// attachment list, so it is dropped — the status line must say so.
#[derive(Debug, Clone, PartialEq)]
enum DraftSpot {
    Nothing {
        attachments: bool,
    },
    Composer {
        attachments: bool,
    },
    Parked {
        sid: Option<String>,
        attachments: bool,
    },
}

/// Sidebar name of the chat workspace (the pinned workspace for sessions
/// that belong to no project folder), in the current language.
pub fn general_workspace() -> &'static str {
    tr("General")
}

/// The whole harness state: one view over every workspace (the pinned chat
/// workspace "Allgemein" plus the project folders). Built from real ACP
/// events only.
pub struct Harness {
    /// The chat workspace ("Allgemein"): a dedicated, otherwise empty folder
    /// for sessions without a project. Pinned on top of the sidebar, never
    /// part of `projects` and never removable.
    pub chat_dir: Option<PathBuf>,
    /// The user picked the workspace for new sessions in this run (opened
    /// a session, chose a workspace, added a project). Until then the
    /// workspace whose sessions were updated last is preselected
    /// (`last_used_workspace`).
    workspace_chosen: bool,
    /// Vibe models / thinking levels seen last (value, name), so they can be
    /// chosen before a session exists (persisted by the app).
    pub known_models: Vec<(String, String)>,
    pub known_thinking: Vec<String>,
    /// Thinking levels each model offered when it was last active, with the
    /// level it had (Vibe keeps the level per model). Lets the start screen
    /// offer the right levels for the chosen model (persisted by the app).
    pub thinking_by_model: BTreeMap<String, (Vec<String>, String)>,
    /// Vibe's current default model / thinking level (from the probe session).
    pub default_model: Option<String>,
    pub default_thinking: Option<String>,
    /// Model / thinking level for the NEXT new session (None = Vibe default).
    pub start_model: Option<String>,
    pub start_thinking: Option<String>,
    /// Setup still to apply to a fresh session before its held first prompt.
    pending_setup: Vec<SetupStep>,
    /// A `git worktree add` running in the background (`create_worktree`).
    worktree_job: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    client: Option<AcpClient>,
    rx: Option<mpsc::Receiver<AcpEvent>>,
    pub conn: Conn,
    pub sessions: HashMap<String, SessionView>,
    pub active: Option<String>,
    pub projects: Vec<PathBuf>,
    /// The workspace a new session starts in (a project folder or the chat
    /// workspace); follows the session the user opens.
    pub current_project: Option<PathBuf>,
    pub listed: HashMap<PathBuf, Vec<SessionInfo>>,
    /// Pages of `session/list` seen per workspace: a broken agent that
    /// always answers with a fresh cursor must not page the harness
    /// forever (audit N2). A first page (a request without a cursor)
    /// resets its workspace's counter.
    list_pages: HashMap<PathBuf, u32>,
    /// Prompt waiting for session/new to finish: (cwd, blocks).
    queued: Option<(PathBuf, Vec<PromptBlock>)>,
    /// Prompt held back until `set_mode` answered for this session:
    /// (session_id, blocks). A fresh Vibe session starts in `accept-edits`;
    /// the first prompt must only go out once the start mode is active.
    pending_mode_prompt: Option<(String, Vec<PromptBlock>)>,
    /// Blocks of each `session/prompt` we sent and have not heard about
    /// (per session id): Vibe echoes the prompt back only on success, so a
    /// failed request needs the text to put it into the draft again.
    sent_prompts: HashMap<String, Vec<PromptBlock>>,
    /// Permission requests for sessions with no view (subagent `child-…`
    /// ids, or a race right after a reconnect, Handoff 2.2). Never
    /// silently dropped: the turn blocks until an answer arrives, and the
    /// answer needs only the request id, not a session map entry. Rendered
    /// as cards in the active view (harness::ui).
    orphan_permissions: Vec<PendingPermission>,
    /// Mode for the NEXT new session (from the settings / start screen).
    pub start_mode: String,
    /// Last seen `available_modes` (for the start screen's mode combo).
    pub known_modes: Vec<SessionMode>,
    /// Folder picker dialog ("+ Projekt"), while open.
    pub folder_picker: Option<FolderPicker>,
    /// Project waiting for the remove confirmation dialog.
    pub remove_confirm: Option<PathBuf>,
    /// Session to open as soon as the agent is ready: the CLI's `--session`,
    /// the session on screen when `vibe-acp` ended, or one the user clicked
    /// while it restarted (the latest of these wins).
    open_on_ready: Option<(PathBuf, String)>,
    /// CLI `--activity`: bring the activity feed back after the
    /// `--session` of the same start opens (open_session resets the view).
    pub(crate) activity_on_ready: bool,
    /// Message to send once the agent (and the CLI's session) is ready.
    prompt_on_ready: Option<String>,
    /// Restarts since the last stable connection, and when it became ready.
    reconnects: u32,
    ready_since: Option<Instant>,
    /// When `Conn::Starting` began; drives the start watchdog in `poll`.
    starting_since: Option<Instant>,
    pub draft: String,
    pub attachments: Vec<PathBuf>,
    pub status: String,
    /// The catwright-ui skill is installed: the first prompt of a new
    /// session gets a short hint that this GUI renders ```ui blocks.
    pub ui_hint: bool,
    /// Sessions created in this run that have not received a prompt yet:
    /// their first prompt carries the `[Catwright]` hint (once per
    /// session, not globally — another new session must not consume it).
    hint_pending: HashSet<String>,
    /// Pinned session ids (persisted through the settings): they sort to
    /// the top of their workspace's sidebar list.
    pub pinned: HashSet<String>,
    /// The agent accepts `image` blocks (agentCapabilities.promptCapabilities
    /// from `initialize`); PNG attachments need it, else they go as links.
    agent_images: bool,
    // render state (consumed by harness::ui; RenderCtx needs these two).
    // Every per-interaction key starts with the owning session id
    // (Handoff 2.5), so one session's state never surfaces in another and
    // a dropped session's entries go with it (drop_session_ui_state).
    ui_events: Vec<UiEvent>,
    form_values: HashMap<String, String>,
    /// State of ```ui-js tools in agent answers, one boa context per entry.
    scripts: HashMap<String, crate::script::ScriptTool>,
    /// Context breakdown per session: (`used` when read, sizes from Vibe's file).
    ctx_sizes: HashMap<String, (u64, Option<context::ContextSizes>)>,
    /// Expanded tool-group rows, keyed "<session_id>#grp#<first tool-call id>"
    /// (the fallback before any id exists is the group's item position).
    open_groups: HashSet<String>,
    /// Expanded diff previews, keyed "<session_id>#<toolCallId>::<path>".
    diff_open: HashSet<String>,
    /// A `session/fork` is in flight: the source session whose transcript
    /// the new view copies when the answer arrives, and the fork point.
    fork_pending: Option<(String, Option<String>)>,
    /// The Artefakte view (left-sidebar entry) replaces the transcript in
    /// the central area; any session/project navigation closes it.
    pub artifacts_open: bool,
    /// Path of the artifact whose preview is shown, if any.
    pub artifact_selected: Option<String>,
    /// Transient UI state (open dialogs, palette, panel visibility).
    pub ui: UiState,
    /// Local git / file-list lookups.
    pub watcher: watch::Watcher,
    /// Plan and budget of the signed-in account, once read. Read on the
    /// user's request only (`load_account`), or at startup with the
    /// `auto_account` setting.
    pub account: Option<AccountInfo>,
    pub identity: Option<IdentityInfo>,
    /// Account requests in flight.
    account_pending: u8,
    /// Read the account as soon as the probe session answers.
    pub auto_account: bool,
    account_after_probe: bool,
    /// A probe session opened only for the account read; closed afterwards.
    account_probe_session: Option<String>,
    /// Workspace trust per folder (from `_meta.workspace_trust`).
    pub trust: HashMap<PathBuf, TrustInfo>,
    /// Trust banners the user waved away in this run.
    pub trust_dismissed: HashSet<PathBuf>,
    /// Open `elicitation/create` requests (Vibe's questions).
    pub questions: Vec<PendingQuestion>,
    /// Messages typed while a turn was running, per session.
    pub queue: HashMap<String, Vec<QueuedPrompt>>,
    pub loops: HashMap<String, Vec<LoopInfo>>,
    pub review: HashMap<String, Vec<ReviewFile>>,
    /// Vibe's per-turn scopes (`scopes[]` of `_review/state`).
    pub review_scopes: HashMap<String, Vec<ReviewScope>>,
    /// Turn diffs loaded on click, per session.
    pub turn_diffs: HashMap<String, TurnDiffs>,
    pub connectors: HashMap<String, Vec<ConnectorInfo>>,
    /// Sessions whose connector list was requested (the card loads lazily).
    pub connectors_requested: HashSet<String>,
    /// Start of the running turn per session (local clock).
    pub turn_started: HashMap<String, Instant>,
    /// Length of the last finished turn per session, in seconds.
    pub last_turn_secs: HashMap<String, u64>,
    /// Why Vibe is retrying the running turn, per session.
    pub retrying: HashMap<String, String>,
    /// Sessions that finished a turn while another one was on screen.
    pub unseen: HashSet<String>,
    /// Sessions whose last turn failed (red FAILED in the sidebar until the
    /// next turn or open).
    pub failed: HashSet<String>,
    /// The activity feed across all sessions (bounded).
    pub activity: Vec<ActivityEvent>,
    /// Archived session ids (persisted through the settings): hidden from
    /// the sidebar's session lists, shown in their own group.
    pub archived: HashSet<String>,
    /// Slash commands last reported by any session (for the start screen).
    pub known_commands: Vec<AvailableCommand>,
}

/// What the sidebar shows for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Running,
    /// A permission or question waits for the user.
    Waiting,
    /// A turn finished while another session was on screen.
    Unseen,
    /// The last turn of this session failed.
    Failed,
}

impl Harness {
    pub fn new(projects: Vec<PathBuf>, start_mode: String) -> Self {
        let current_project = projects.first().cloned();
        Self {
            chat_dir: None,
            workspace_chosen: false,
            known_models: Vec::new(),
            known_thinking: Vec::new(),
            thinking_by_model: BTreeMap::new(),
            default_model: None,
            default_thinking: None,
            start_model: None,
            start_thinking: None,
            pending_setup: Vec::new(),
            worktree_job: None,
            client: None,
            rx: None,
            conn: Conn::Off,
            sessions: HashMap::new(),
            active: None,
            current_project,
            projects,
            listed: HashMap::new(),
            list_pages: HashMap::new(),
            queued: None,
            pending_mode_prompt: None,
            sent_prompts: HashMap::new(),
            orphan_permissions: Vec::new(),
            start_mode,
            known_modes: Vec::new(),
            folder_picker: None,
            remove_confirm: None,
            open_on_ready: None,
            activity_on_ready: false,
            prompt_on_ready: None,
            artifacts_open: false,
            artifact_selected: None,
            fork_pending: None,
            reconnects: 0,
            ready_since: None,
            starting_since: None,
            draft: String::new(),
            attachments: Vec::new(),
            status: String::new(),
            // set by the app from store::vibe_skill_installed(); keeping the
            // constructor free of file access keeps tests independent of ~/.vibe
            ui_hint: false,
            hint_pending: HashSet::new(),
            pinned: HashSet::new(),
            agent_images: true,
            ui_events: Vec::new(),
            form_values: HashMap::new(),
            scripts: HashMap::new(),
            ctx_sizes: HashMap::new(),
            open_groups: HashSet::new(),
            diff_open: HashSet::new(),
            ui: UiState::default(),
            watcher: watch::Watcher::default(),
            account: None,
            identity: None,
            account_pending: 0,
            auto_account: false,
            account_after_probe: false,
            account_probe_session: None,
            trust: HashMap::new(),
            trust_dismissed: HashSet::new(),
            questions: Vec::new(),
            queue: HashMap::new(),
            loops: HashMap::new(),
            review: HashMap::new(),
            review_scopes: HashMap::new(),
            turn_diffs: HashMap::new(),
            connectors: HashMap::new(),
            connectors_requested: HashSet::new(),
            turn_started: HashMap::new(),
            last_turn_secs: HashMap::new(),
            retrying: HashMap::new(),
            unseen: HashSet::new(),
            failed: HashSet::new(),
            activity: Vec::new(),
            archived: HashSet::new(),
            known_commands: Vec::new(),
        }
    }

    /// Spawn `vibe-acp` and send `initialize` (once, when still Off). The
    /// waker repaints the UI on every incoming event.
    /// After an unexpected exit, restarts once the backoff delay has passed.
    pub fn ensure_started(&mut self, ctx: &egui::Context) {
        match self.conn {
            Conn::Off => {}
            Conn::Reconnecting { at, .. } => {
                let now = Instant::now();
                if now < at {
                    ctx.request_repaint_after(at - now);
                    return;
                }
            }
            _ => return,
        }
        let waker: Waker = {
            let ctx = ctx.clone();
            Arc::new(move || ctx.request_repaint())
        };
        // `vibe-acp` from PATH, and (pip installs without a system bin dir
        // land there) from ~/.local/bin
        let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
        let mut candidates = vec!["vibe-acp".to_string()];
        if let Some(home) = home {
            candidates.push(
                home.join(".local/bin/vibe-acp")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        let mut spawned = None;
        let mut error = String::new();
        for program in candidates {
            match AcpClient::spawn(&program, &[], Some(waker.clone())) {
                Ok(pair) => {
                    spawned = Some(pair);
                    break;
                }
                Err(e) => error = e,
            }
        }
        match spawned {
            Some((client, rx)) => {
                self.attach(client, rx);
                let res = self.client.as_ref().map(|c| c.initialize());
                if let Some(Err(e)) = res {
                    self.status = e;
                }
            }
            None => {
                self.conn = Conn::Failed(error);
            }
        }
    }

    /// Attach an existing client + event channel (used by tests).
    pub fn attach(&mut self, client: AcpClient, rx: mpsc::Receiver<AcpEvent>) {
        self.client = Some(client);
        self.rx = Some(rx);
        self.conn = Conn::Starting;
        self.starting_since = Some(Instant::now());
    }

    /// Whether a start that began at `since` has run out of watchdog time.
    /// Pure so tests can move the clock: `START_TIMEOUT` after the attach,
    /// a mute agent counts as lost.
    fn starting_expired(since: Option<Instant>, now: Instant) -> bool {
        since.is_some_and(|s| now.duration_since(s) >= START_TIMEOUT)
    }

    /// Drain all pending events into `handle_event`.
    pub fn poll(&mut self) {
        self.watcher.poll();
        self.poll_worktree();
        // Start watchdog (review M6): a child that lives but never answers
        // `initialize` produces no exit event, so `connection_lost` would
        // never run on its own. Killing it here feeds the normal reconnect
        // schedule and the status tells the user what happened.
        if matches!(self.conn, Conn::Starting)
            && Self::starting_expired(self.starting_since, Instant::now())
        {
            self.connection_lost(None, tr("initialize did not answer in time"));
        }
        let mut events = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(ev) = rx.try_recv() {
                events.push(ev);
            }
        }
        for ev in events {
            self.handle_event(ev);
        }
    }

    /// Fold one ACP event into the state.
    pub fn handle_event(&mut self, ev: AcpEvent) {
        match ev {
            AcpEvent::Initialized(r) => {
                // Vibe's own answer about what its prompts accept: PNG
                // attachments only go out as `image` blocks when the agent
                // says it takes them (docs/acp.md, agentCapabilities).
                self.agent_images = r.agent_capabilities.prompt_capabilities.image;
                let agent = match &r.agent_info {
                    Some(info) => {
                        let title = info.title.clone().unwrap_or_else(|| info.name.clone());
                        if info.version.is_empty() {
                            title
                        } else {
                            format!("{title} {}", info.version)
                        }
                    }
                    None => "Vibe".to_string(),
                };
                self.conn = Conn::Ready { agent };
                self.ready_since = Some(Instant::now());
                self.starting_since = None;
                if let Some((cwd, sid)) = self.open_on_ready.take() {
                    // the CLI's `--session`, or the session that was open
                    // when the connection broke (Handoff 2.8.7)
                    self.open_session(cwd, &sid);
                }
                if self.activity_on_ready {
                    self.activity_on_ready = false;
                    self.ui.activity_open = true;
                }
                // learn the current model / thinking choices and defaults
                // (an empty session: no model call, nothing stays on disk).
                // Marked as probe: if it fails, no queued user prompt may
                // be taken for lost.
                if let Some(dir) = self.chat_dir.clone() {
                    if self
                        .client
                        .as_ref()
                        .map(|c| c.new_probe_session(&dir))
                        .is_some_and(|r| r.is_ok())
                    {
                        // the user opted into showing the budget at startup
                        self.account_after_probe = self.auto_account;
                    }
                }
                let mut dirs = self.projects.clone();
                dirs.extend(self.chat_dir.clone());
                for p in dirs {
                    let res = self.client.as_ref().map(|c| c.list_sessions(Some(&p)));
                    if let Some(Err(e)) = res {
                        self.status = e;
                    }
                }
                // without a session to load, the CLI's prompt starts a new one
                if self.active.is_none() {
                    self.send_prompt_on_ready();
                }
            }
            AcpEvent::SessionsListed {
                cwd: Some(p),
                cursor,
                result,
            } => {
                // The answer itself says whether it continues its own
                // request (cursor set: a page — it extends the same cwd's
                // list) or starts a fresh first page (it replaces whatever
                // was there; review M4). No cwd marker, no response-order
                // guessing: parallel refreshes answer in any order without
                // appending onto a wrong request. Two chains of the same
                // cwd can still interleave their pages, though — both
                // enumerate the same sessions — so a page merges by
                // session id and a session that a parallel chain already
                // delivered cannot enter the list twice.
                if cursor.is_some() {
                    let list = self.listed.entry(p.clone()).or_default();
                    for s in result.sessions {
                        if !list.iter().any(|e| e.session_id == s.session_id) {
                            list.push(s);
                        }
                    }
                } else {
                    self.listed.insert(p.clone(), result.sessions);
                }
                // Vibe pages `session/list` (nextCursor): fetch the rest —
                // but the harness drives the loop, so it also ends it: an
                // agent that keeps answering with a fresh cursor never
                // pages its caller past this budget (audit N2). A page
                // answered without a request cursor is a fresh start, and
                // those two chains cannot share the count.
                let pages = match cursor.as_ref() {
                    None => {
                        self.list_pages.insert(p.clone(), 1);
                        1
                    }
                    Some(_) => {
                        let n = self.list_pages.entry(p.clone()).or_insert(0);
                        *n += 1;
                        *n
                    }
                };
                if let Some(cursor) = result.next_cursor {
                    if pages >= MAX_LIST_PAGES {
                        self.status = trf(
                            "The session list of {dir} stopped paging after {pages} pages.",
                            &[
                                ("dir", &p.to_string_lossy()),
                                ("pages", &MAX_LIST_PAGES.to_string()),
                            ],
                        );
                    } else {
                        let res = self
                            .client
                            .as_ref()
                            .map(|c| c.list_sessions_page(Some(&p), Some(&cursor)));
                        if let Some(Err(e)) = res {
                            self.status = e;
                        }
                    }
                }
                // until the user picks a workspace, new sessions start where
                // the newest session lives (the lists arrive one by one)
                if !self.workspace_chosen && self.active.is_none() {
                    if let Some(w) = last_used_workspace(&self.listed, &self.workspaces()) {
                        self.current_project = Some(w);
                    }
                }
            }
            AcpEvent::SessionsListed { cwd: None, .. } => {}
            AcpEvent::SessionCreated {
                cwd,
                origin,
                result,
            } => {
                let sid = result.session_id.clone();
                if let Some(t) = extras::trust_of_meta(result.meta.as_ref()) {
                    self.trust.insert(cwd.clone(), t);
                }
                match origin {
                    // a fork's answer: the new view shows the source transcript
                    // (this client's copy — `session/fork` does not replay it)
                    SessionOrigin::Fork { source, message_id } => {
                        self.fork_pending = None;
                        let mut carries_the_transcript = true;
                        let mut view = match self.sessions.get(&source) {
                            Some(src) => {
                                let mut base = src.clone();
                                if let Some(m) = &message_id {
                                    base.keep_through_turn_of(m);
                                }
                                base.forked(sid.clone())
                            }
                            // The source view no longer exists (closed or
                            // deleted while the fork was in flight), yet the
                            // fork session does — server-side. It still
                            // becomes a real, active view instead of falling
                            // through to the new-session path silently, and
                            // the status says why the transcript is empty
                            // (review N1).
                            None => {
                                carries_the_transcript = false;
                                SessionView::new(sid.clone(), cwd.clone())
                            }
                        };
                        view.set_session_info(result.modes.clone(), result.config_options.clone());
                        self.sessions.insert(sid.clone(), view);
                        self.activate(sid, &cwd);
                        self.status = if carries_the_transcript {
                            tr("Fork created.").to_string()
                        } else {
                            tr("Fork created without the transcript – the source session no longer exists.").to_string()
                        };
                        self.refresh_list(&cwd);
                        return;
                    }
                    // The probe session: remember its choices and close it
                    // again. It never becomes a visible chat: the user's own
                    // session/new answers as `New`, whatever the arrival
                    // order (start race).
                    SessionOrigin::Probe => {
                        self.remember_config(&result.config_options);
                        for o in &result.config_options {
                            if let ConfigOption::Select {
                                id, current_value, ..
                            } = o
                            {
                                match id.as_str() {
                                    "model" => self.default_model = Some(current_value.clone()),
                                    "thinking" => {
                                        self.default_thinking = Some(current_value.clone())
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if let Some(modes) = &result.modes {
                            self.known_modes = modes.available_modes.clone();
                        }
                        if self.account_after_probe {
                            // the probe doubles as the session the account read
                            // needs; it is closed when both answers are in
                            self.account_after_probe = false;
                            self.account_probe_session = Some(sid.clone());
                            self.account_pending = 0;
                            self.request_account(&sid);
                            // Both reads failed to send (the client is gone
                            // or the wire broken): no answer will ever close
                            // the probe — close it right away instead of
                            // waiting for an `account_answered` that cannot
                            // come (review N9).
                            if self.account_pending == 0 {
                                self.account_probe_session = None;
                                if let Some(c) = &self.client {
                                    let _ = c.close_session(&sid);
                                }
                            }
                            return;
                        }
                        if let Some(c) = &self.client {
                            let _ = c.close_session(&sid);
                        }
                        return;
                    }
                    SessionOrigin::New => {}
                }
                let mut view = SessionView::new(sid.clone(), cwd.clone());
                view.set_session_info(result.modes.clone(), result.config_options.clone());
                if let Some(modes) = &view.modes {
                    self.known_modes = modes.available_modes.clone();
                }
                self.sessions.insert(sid.clone(), view);
                // permission requests that raced ahead of this session's
                // creation now have a view: the orphan cards migrate into
                // it and render like normal requests afterwards
                self.adopt_orphan_permissions(&sid);
                // the session the user's message goes into is the one on
                // screen, in whatever workspace it was started
                self.activate(sid.clone(), &cwd);
                // a fresh session: its first prompt carries the UI hint
                self.hint_pending.insert(sid.clone());
                // a queued prompt for this cwd goes out — but only once the
                // start mode is active: a fresh Vibe session defaults to
                // accept-edits, which would allow file changes without asking
                self.remember_config(&result.config_options);
                let take = matches!(&self.queued, Some((q, _)) if *q == cwd);
                if take {
                    let (_, blocks) = self.queued.take().expect("checked above");
                    let steps = self.setup_steps(&sid);
                    if steps.is_empty() {
                        self.send_prompt(&sid, &blocks);
                    } else {
                        // hold the prompt until mode/model/thinking are set
                        self.pending_mode_prompt = Some((sid.clone(), blocks));
                        self.pending_setup = steps;
                        self.continue_setup(&sid);
                    }
                }
                // refresh the sidebar list for this project
                let res = self.client.as_ref().map(|c| c.list_sessions(Some(&cwd)));
                if let Some(Err(e)) = res {
                    self.status = e;
                }
            }
            AcpEvent::SessionLoaded { session_id, result } => {
                if let Some(v) = self.sessions.get_mut(&session_id) {
                    v.set_session_info(result.modes, result.config_options.clone());
                    if let Some(modes) = &v.modes {
                        self.known_modes = modes.available_modes.clone();
                    }
                    if let Some(t) = extras::trust_of_meta(result.meta.as_ref()) {
                        let cwd = v.cwd.clone();
                        self.trust.insert(cwd, t);
                    }
                }
                self.remember_config(&result.config_options);
                self.load_session_extras(&session_id);
                if self.active.as_deref() == Some(session_id.as_str()) {
                    self.send_prompt_on_ready();
                }
            }
            AcpEvent::Update { session_id, update } => {
                // the probe session reports the commands, too: remember them
                // for the start screen's slash list
                if let SessionUpdate::AvailableCommandsUpdate(c) = &update {
                    self.known_commands = c.available_commands.clone();
                }
                // output is flowing again: the retry note is over
                if matches!(
                    update,
                    SessionUpdate::AgentMessageChunk(_)
                        | SessionUpdate::AgentThoughtChunk(_)
                        | SessionUpdate::ToolCall(_)
                ) {
                    self.retrying.remove(&session_id);
                }
                if let Some(v) = self.sessions.get_mut(&session_id) {
                    v.apply_update(&update);
                }
            }
            AcpEvent::QuestionAsked { request_id, params } => {
                match extras::parse_question(request_id.clone(), &params) {
                    Some(q) => {
                        let sid = q.session_id.clone().unwrap_or_default();
                        self.push_activity(
                            ActivityKind::Waiting,
                            &sid,
                            tr("Vibe asks a question").to_string(),
                            Some(request_id.clone()),
                        );
                        self.questions.push(q);
                    }
                    None => {
                        // not renderable as a form: an honest "declined"
                        // instead of leaving the turn hanging
                        if let Some(c) = &self.client {
                            let _ = c.respond_question(&request_id, "decline", None);
                        }
                        self.status = tr("Vibe asked a question that cannot be shown.").to_string();
                    }
                }
            }
            AcpEvent::ExtResult { kind, result } => self.handle_ext_result(kind, result),
            AcpEvent::Retrying {
                session_id,
                category,
                detail,
            } => {
                self.retrying
                    .insert(session_id, retry_note(&category, &detail));
            }
            AcpEvent::PermissionRequested {
                request_id,
                request,
            } => {
                let sid = request.session_id.clone();
                let text = request
                    .tool_call
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| tr("Permission request").to_string());
                if let Some(v) = self.sessions.get_mut(&request.session_id) {
                    v.add_permission(request_id.clone(), request);
                } else {
                    // Handoff 2.2: never silently drop. The turn blocks
                    // until an answer arrives, and the answer goes over the
                    // wire by request id alone, so the request stays
                    // answerable without a session view. Shown as a marked
                    // card in the active view (harness::ui), together with
                    // its raw session id.
                    self.orphan_permissions.push(PendingPermission {
                        request_id: request_id.clone(),
                        request,
                    });
                    self.status = tr(
                        "A permission of an unknown session was requested (maybe a subagent) – please answer on its card.",
                    )
                    .to_string();
                }
                self.push_activity(ActivityKind::Waiting, &sid, text, Some(request_id));
            }
            AcpEvent::PromptFinished { session_id, result } => {
                // the turn is over: no text to restore for it anymore
                self.sent_prompts.remove(&session_id);
                self.retrying.remove(&session_id);
                if let Some(started) = self.turn_started.remove(&session_id) {
                    self.last_turn_secs
                        .insert(session_id.clone(), started.elapsed().as_secs());
                }
                if self.active.as_deref() != Some(session_id.as_str()) {
                    self.unseen.insert(session_id.clone());
                }
                if result.stop_reason != StopReason::Cancelled {
                    let text = stop_reason_note(&result.stop_reason)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| tr("Turn finished").to_string());
                    self.push_activity(ActivityKind::Finished, &session_id, text, None);
                }
                let mut cwd = None;
                if let Some(v) = self.sessions.get_mut(&session_id) {
                    v.finish_turn(&result);
                    cwd = Some(v.cwd.clone());
                }
                if let Some(dir) = &cwd {
                    self.watcher.invalidate_git(dir);
                }
                // Vibe persists a session (and its list entry) only with its
                // first message, and the sidebar list was last refreshed
                // before that message: reload it now so the fresh session
                // and its title show up without an app restart.
                if let Some(cwd) = cwd {
                    let res = self.client.as_ref().map(|c| c.list_sessions(Some(&cwd)));
                    if let Some(Err(e)) = res {
                        self.status = e;
                    }
                }
                self.load_session_extras(&session_id);
                // The next queued message goes out, unless the user stopped
                // the turn: then the queue comes back into the composer of
                // its own session instead of starting new work on its own.
                if result.stop_reason == StopReason::Cancelled {
                    match self.restore_queue(&session_id) {
                        Some(DraftSpot::Composer { attachments }) => {
                            self.status =
                                tr("Cancelled – queued messages are back in the input field.")
                                    .to_string();
                            if attachments {
                                self.note_attachments_lost();
                            }
                        }
                        Some(spot @ DraftSpot::Parked { .. }) => {
                            // stopped while another session is on screen:
                            // the texts wait where they belong
                            self.status = tr("Cancelled").to_string();
                            self.note_not_sent(spot);
                        }
                        Some(DraftSpot::Nothing { attachments: true }) => {
                            self.status = tr("Cancelled").to_string();
                            self.note_attachments_lost();
                        }
                        _ => {}
                    }
                } else {
                    self.send_next_queued(&session_id);
                }
            }
            AcpEvent::ModeSet {
                session_id,
                mode_id,
            } => {
                if let Some(v) = self.sessions.get_mut(&session_id) {
                    if let Some(modes) = &mut v.modes {
                        modes.current_mode_id = mode_id.clone();
                    }
                    for opt in &mut v.config_options {
                        if let ConfigOption::Select {
                            id, current_value, ..
                        } = opt
                        {
                            if id == "mode" {
                                *current_value = mode_id.clone();
                            }
                        }
                    }
                }
                // the start mode is active: next setup step or the held prompt
                self.continue_setup(&session_id);
            }
            AcpEvent::ConfigOptionsChanged {
                session_id,
                options,
            } => {
                self.remember_config(&options);
                // A model switch carries no usage_update (verified against
                // Vibe 3.1.2), so the shown size would stay the old model's.
                // That size is Vibe's auto-compaction threshold, which only
                // Vibe knows (it comes from its model catalogue and differs
                // from config.toml's max_context_length). The stale size is
                // cleared ("–", never an estimate: user decision) and
                // `session/resume` asks Vibe for a fresh usage_update, which
                // carries the new model's threshold.
                let switched_model = options
                    .iter()
                    .find_map(|o| match o {
                        ConfigOption::Select {
                            id, current_value, ..
                        } if id == "model" => Some(current_value.as_str()),
                        _ => None,
                    })
                    .filter(|new| {
                        self.sessions
                            .get(&session_id)
                            .and_then(|v| v.current_config_value("model"))
                            .is_some_and(|old| old != *new)
                    });
                if switched_model.is_some() {
                    if let Some(v) = self.sessions.get_mut(&session_id) {
                        v.set_model_window(None);
                        let cwd = v.cwd.clone();
                        // a failed resume stays quiet (see RequestFailed):
                        // the "–" then lasts until the turn's usage_update
                        if let Some(c) = &self.client {
                            let _ = c.resume_session(&cwd, &session_id);
                        }
                    }
                }
                if let Some(v) = self.sessions.get_mut(&session_id) {
                    v.config_options = options;
                }
                self.continue_setup(&session_id);
            }
            // the fresh usage_update came as a session update already
            AcpEvent::SessionResumed => {}
            // Vibe may drop session/resume (it is not advertised): no error
            // for the user, the context size stays "–" until the next
            // usage_update
            AcpEvent::RequestFailed {
                request: RequestKind::ResumeSession { .. },
                ..
            } => {}
            AcpEvent::SessionClosed { session_id } => {
                // the agent confirmed a close (probe session, or a session
                // Vibe dropped): the local view goes with it — for good
                if self.sessions.remove(&session_id).is_some() {
                    self.drop_session_ui_state(&session_id, true);
                    if self.active.as_deref() == Some(session_id.as_str()) {
                        self.active = None;
                    }
                }
            }
            AcpEvent::RequestFailed {
                request: RequestKind::Ext(kind),
                error,
            } => self.ext_failed(&kind, &error),
            AcpEvent::RequestFailed { request, error } => {
                if error.code == rpc::AUTH_REQUIRED {
                    self.status = tr("Vibe is not signed in. Please start `vibe` once in a terminal and sign in.").to_string();
                } else {
                    self.status = trf(
                        "Error ({code}): {message}",
                        &[("code", &error.code), ("message", &error.message)],
                    );
                }
                match &request {
                    RequestKind::Prompt { session_id } => {
                        self.push_activity(
                            ActivityKind::Failed,
                            session_id,
                            error.message.clone(),
                            None,
                        );
                        self.failed.insert(session_id.clone());
                        self.unseen.remove(session_id);
                        self.turn_started.remove(session_id);
                        self.retrying.remove(session_id);
                        // The request is dead, so the turn is over — the
                        // failed branch ends it like PromptFinished would
                        // (finish_turn): the open approvals of a request
                        // that can no longer proceed expire with it, else
                        // session_state would claim "Waiting" forever
                        // (review M5). The user is told, like on
                        // connection_lost.
                        let mut approvals_expired = false;
                        if let Some(v) = self.sessions.get_mut(session_id) {
                            v.turn = TurnState::Idle;
                            approvals_expired = !v.permissions.is_empty();
                            v.permissions.clear();
                        }
                        // Vibe echoes the prompt (user_message_chunk) only
                        // on success. The texts that never ran — the sent
                        // prompt and the queued follow-ups — go back
                        // TOGETHER into the composer of their own session
                        // (sent first, in the order they were typed), not
                        // the one on screen (park_or_restore, review M13);
                        // the queue does not fire on its own after a
                        // failure — the user decides the next send (M5).
                        // Only when Vibe did not answer at all: when it answered
                        // and just the answer could not be read, the turn ran
                        // and the message was delivered — sending it again
                        // would repeat the turn.
                        let sent = self.sent_prompts.remove(session_id).unwrap_or_default();
                        let unreadable = crate::acp::client::is_unreadable_response(&error);
                        let mut blocks = if unreadable { Vec::new() } else { sent };
                        if unreadable {
                            self.status = tr(
                                "Vibe’s answer to the message could not be read. The turn has ended; the message was delivered.",
                            )
                            .to_string();
                        }
                        if let Some(queued) = self.queue.remove(session_id) {
                            blocks.extend(queued.into_iter().flat_map(|q| q.blocks));
                        }
                        if !blocks.is_empty() {
                            let spot = self.park_or_restore(Some(session_id), &blocks);
                            self.note_not_sent(spot);
                        }
                        if approvals_expired {
                            self.status
                                .push_str(tr(" Open approvals were discarded (please ask again)."));
                        }
                    }
                    RequestKind::NewSession { probe: false, .. } => {
                        // the session never came to be: put the held prompt
                        // back into the composer of the start screen it was
                        // typed in — parked when another session is on
                        // screen (park_or_restore), never dropped
                        if let Some((_, blocks)) = self.queued.take() {
                            let spot = self.park_or_restore(None, &blocks);
                            self.note_not_sent(spot);
                        }
                    }
                    RequestKind::NewSession { probe: true, .. } => {
                        // the probe's session/new failed: a prompt queued for
                        // the user's own session/new stays where it is
                        if self.account_after_probe {
                            self.account_after_probe = false;
                            self.account_pending = 0;
                        }
                    }
                    RequestKind::ForkSession { .. } => {
                        // the fork never happened (Vibe may not implement
                        // session/fork): say so, the source stays untouched
                        self.fork_pending = None;
                        self.status = trf(
                            "Fork failed: {detail}",
                            &[("detail", &error.message.lines().next().unwrap_or("").trim())],
                        );
                    }
                    RequestKind::LoadSession { session_id } => {
                        // loading failed (deleted session, child id): remove
                        // the empty view instead of leaving an orphan screen
                        // behind; the start screen shows again
                        self.sessions.remove(session_id);
                        // with the view goes its UI state (Handoff 2.5)
                        self.drop_session_ui_state(session_id, true);
                        if self.active.as_deref() == Some(session_id.as_str()) {
                            self.active = None;
                        }
                    }
                    RequestKind::SetMode { session_id, .. } => {
                        // The start mode did not take: the session still runs
                        // in Vibe's default (accept-edits, which changes files
                        // without asking). The held prompt must not go out in
                        // a mode the user did not choose — it goes back into
                        // the composer instead.
                        if matches!(&self.pending_mode_prompt, Some((s, _)) if s == session_id) {
                            let detail = error.message.lines().next().unwrap_or("").trim();
                            self.abort_setup(detail);
                        }
                    }
                    RequestKind::SetConfigOption { session_id, .. } => {
                        // a model / thinking step failed: warn and go on (the
                        // mode is already set, the prompt is still sent)
                        if matches!(&self.pending_mode_prompt, Some((s, _)) if s == session_id) {
                            let sid = session_id.clone();
                            self.status =
                                tr("A setting could not be applied – the prompt is sent anyway.")
                                    .to_string();
                            self.continue_setup(&sid);
                        }
                    }
                    _ => {}
                }
            }
            AcpEvent::ProtocolError(m) => {
                self.status = m;
            }
            AcpEvent::Exited { code, stderr_tail } => {
                let tail: String = stderr_tail
                    .chars()
                    .rev()
                    .take(300)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                self.connection_lost(code, tail.trim());
            }
        }
    }

    /// Where a new worktree for `project` goes: under the app config dir
    /// (never inside the repository — it would show up as untracked
    /// files), named after the project with the smallest free counter.
    pub fn worktree_dir(base: &Path, project: &Path) -> PathBuf {
        let name = project
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "projekt".to_string());
        let dir = base.join("worktrees");
        let mut n = 1;
        loop {
            let candidate = dir.join(format!("{name}-{n}"));
            if !candidate.exists() {
                return candidate;
            }
            n += 1;
        }
    }

    /// Give the current project its own git worktree and make it the
    /// workspace (Handoff Ideenliste 6): `session/list` matches cwd
    /// exactly, so every worktree is its own workspace with its own
    /// sessions. The branch is `catwright/<worktree name>`.
    pub fn create_worktree(&mut self) {
        // The checkout runs on a background thread (a big repository takes
        // a while, and the UI must not freeze); `poll` picks the result up.
        // A failed or timed-out checkout is cleaned up again, so no half
        // worktree or orphan branch stays behind.
        if self.worktree_job.is_some() {
            return;
        }
        let Some(project) = self.current_project.clone() else {
            self.status = tr("No project open.").to_string();
            return;
        };
        let Some(base) = crate::store::config_dir() else {
            self.status = tr("No config folder found.").to_string();
            return;
        };
        let dir = Self::worktree_dir(&base, &project);
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(make_worktree(&project, &dir));
        });
        self.worktree_job = Some(rx);
        self.status = trf("Creating worktree “{name}” …", &[("name", &name)]);
    }

    /// Is a worktree checkout still running?
    pub fn worktree_running(&self) -> bool {
        self.worktree_job.is_some()
    }

    /// Take the result of a finished worktree checkout, if any.
    fn poll_worktree(&mut self) {
        let Some(rx) = &self.worktree_job else {
            return;
        };
        let done = match rx.try_recv() {
            Ok(done) => done,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("git".to_string()),
        };
        self.worktree_job = None;
        match done {
            Ok(dir) => {
                let name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.add_project(&dir.to_string_lossy());
                self.status = trf("Worktree “{name}” created.", &[("name", &name)]);
            }
            Err(e) => {
                self.status = trf(
                    "The worktree could not be created: {error}",
                    &[("error", &e)],
                );
            }
        }
    }

    /// Open `path` in the preview pane beside the transcript (a clicked
    /// path in a tool row, diff chip or changes row). Relative paths
    /// resolve against the session's workspace.
    pub fn open_preview(&mut self, sid: &str, path: &str) {
        let resolved = if Path::new(path).is_absolute() {
            Some(PathBuf::from(path))
        } else {
            self.sessions.get(sid).map(|v| v.cwd.join(path))
        };
        if let Some(p) = resolved {
            self.ui.preview = Some(PreviewState {
                path: p.to_string_lossy().into_owned(),
                width: 380.0,
                mode: PreviewMode::Code,
            });
        }
    }

    /// Pin/unpin a session (persisted through the settings by the app).
    pub fn toggle_pinned(&mut self, sid: &str) {
        if !self.pinned.remove(sid) {
            self.pinned.insert(sid.to_string());
        }
    }

    /// Show `sid` (a session in `cwd`): its workspace becomes the one new
    /// sessions start in.
    fn activate(&mut self, sid: String, cwd: &Path) {
        self.active = Some(sid);
        self.current_project = Some(cwd.to_path_buf());
        self.workspace_chosen = true;
    }

    // -----------------------------------------------------------------------
    // Workspaces: the chat workspace "Allgemein" and the project folders
    // -----------------------------------------------------------------------

    /// Every workspace in sidebar order: the chat workspace first (pinned),
    /// then the projects.
    pub fn workspaces(&self) -> Vec<PathBuf> {
        let mut out = Vec::with_capacity(self.projects.len() + 1);
        out.extend(self.chat_dir.clone());
        out.extend(self.projects.iter().cloned());
        out
    }

    /// Is `dir` the chat workspace ("Allgemein")?
    pub fn is_chat_dir(&self, dir: &Path) -> bool {
        self.chat_dir.as_deref() == Some(dir)
    }

    /// The name a workspace is shown under: "General" ("Allgemein") for
    /// the chat workspace, the folder name for a project.
    pub fn workspace_name(&self, dir: &Path) -> String {
        if self.is_chat_dir(dir) {
            general_workspace().to_string()
        } else {
            project_name(dir)
        }
    }

    /// The user picked `dir` as the workspace for the next new session.
    pub fn select_workspace(&mut self, dir: PathBuf) {
        self.current_project = Some(dir);
        self.workspace_chosen = true;
    }

    /// The workspace to fall back to when the current one goes away: the
    /// one whose sessions were updated last, else the first project, else
    /// the chat workspace.
    fn fallback_workspace(&self) -> Option<PathBuf> {
        last_used_workspace(&self.listed, &self.workspaces())
            .or_else(|| self.projects.first().cloned())
            .or_else(|| self.chat_dir.clone())
    }

    /// Put the text of a prompt that never reached the agent back where it
    /// was typed: the composer of the session that owns it. That session's
    /// composer is filled right away when it is on screen (`sid` matches
    /// the active session; `None` is the start screen), otherwise the text
    /// is parked under its id — `swap_draft` shows it when the user opens
    /// the session. Nothing is ever discarded: an occupied composer keeps
    /// the fresh typing and gets the text appended below it, a parked spot
    /// merges with text parked there earlier.
    fn park_or_restore(&mut self, sid: Option<&str>, blocks: &[PromptBlock]) -> DraftSpot {
        let text = blocks
            .iter()
            .filter_map(|b| match b {
                PromptBlock::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let attachments = blocks.iter().any(|b| !matches!(b, PromptBlock::Text(_)));
        if text.trim().is_empty() {
            // attachments only: no text to put back (their loss is said)
            return DraftSpot::Nothing { attachments };
        }
        let on_screen = match sid {
            Some(s) => self.active.as_deref() == Some(s),
            None => self.active.is_none(),
        };
        if on_screen {
            if !self.draft.trim().is_empty() {
                self.draft.push_str("\n\n");
            }
            self.draft.push_str(&text);
            return DraftSpot::Composer { attachments };
        }
        let key = sid.map(|s| s.to_string());
        let (mut parked, parked_attachments) =
            self.ui.parked_drafts.remove(&key).unwrap_or_default();
        if !parked.trim().is_empty() {
            parked.push_str("\n\n");
        }
        parked.push_str(&text);
        self.ui
            .parked_drafts
            .insert(key.clone(), (parked, parked_attachments));
        DraftSpot::Parked {
            sid: key,
            attachments,
        }
    }

    /// The status note naming the spot parked messages wait in: the
    /// session named the way the sidebar names it (so the user can find
    /// it), or the start screen.
    fn parked_note_text(&self, sid: Option<&str>) -> String {
        match sid {
            None => tr(" – Messages that were not sent are waiting in the input field of the start screen.").to_string(),
            Some(sid) => {
                let cwd = self
                    .sessions
                    .get(sid)
                    .map(|v| v.cwd.clone())
                    .unwrap_or_default();
                let name = self
                    .sidebar_rows(&cwd, tr("New session"))
                    .into_iter()
                    .find(|e| e.session_id == sid)
                    .map(|e| e.title)
                    .unwrap_or_else(|| tr("New session").to_string());
                trf(
                    " – Messages that were not sent are waiting in the input field of session “{name}”.",
                    &[("name", &name)],
                )
            }
        }
    }

    /// Status note that attachments were dropped: only the text goes back
    /// into the composer, the wire blocks cannot become attachment paths.
    fn note_attachments_lost(&mut self) {
        self.status.push_str(tr(
            " – Attachments could not be restored; please add them again.",
        ));
    }

    /// Status note for a message the wire rejected ([`RequestFailed`]): where
    /// its text is — on screen, or parked for the session it belongs to.
    fn note_not_sent(&mut self, spot: DraftSpot) {
        match spot {
            DraftSpot::Nothing { attachments } => {
                if attachments {
                    self.note_attachments_lost();
                }
            }
            DraftSpot::Composer { attachments } => {
                self.status.push_str(tr(
                    " – The message was not sent and is back in the input field.",
                ));
                if attachments {
                    self.note_attachments_lost();
                }
            }
            DraftSpot::Parked { sid, attachments } => {
                let note = self.parked_note_text(sid.as_deref());
                self.status.push_str(&note);
                if attachments {
                    self.note_attachments_lost();
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Actions (each sets a German `status` when the client is missing)
    // -----------------------------------------------------------------------

    /// Send the draft + attachments: with an active session as
    /// `session/prompt`, otherwise queue it behind `session/new` for the
    /// current project.
    pub fn send(&mut self) {
        let blocks = prompt_blocks(&self.draft, &self.attachments, self.agent_images);
        if blocks.is_empty() {
            return;
        }
        if self.client.is_none() {
            self.status = tr("Vibe is not connected.").to_string();
            return;
        }
        // Enter is still active while a turn runs (only the button changes):
        // never start a second prompt or a second session/new in parallel.
        let busy = self
            .active
            .as_deref()
            .and_then(|sid| self.sessions.get(sid))
            .map(|v| v.turn != TurnState::Idle)
            .unwrap_or(false);
        if busy {
            // Vibe is still working: this becomes the next message
            if let Some(sid) = self.active.clone() {
                self.queue.entry(sid).or_default().push(QueuedPrompt {
                    text: self.draft.trim().to_string(),
                    blocks,
                });
                self.draft.clear();
                self.attachments.clear();
                return;
            }
        }
        if self.queued.is_some() || self.pending_mode_prompt.is_some() {
            self.status = tr("Vibe is still working – please wait or stop it.").to_string();
            return;
        }
        if let Some(sid) = self.active.clone() {
            if self.sessions.contains_key(&sid) {
                // one path for every outgoing prompt (begin_turn +
                // session/prompt), so a failed request is treated the same
                // way no matter where the prompt came from
                self.send_prompt(&sid, &blocks);
            } else {
                self.active = None;
            }
        } else {
            let Some(cwd) = self.current_project.clone() else {
                self.status = tr("Please add a project on the left first.").to_string();
                return;
            };
            let res = self.client.as_ref().map(|c| c.new_session(&cwd));
            match res {
                Some(Ok(_)) => {
                    self.queued = Some((cwd, blocks));
                }
                Some(Err(e)) => self.status = e,
                None => self.status = tr("Vibe is not connected.").to_string(),
            }
        }
        self.draft.clear();
        self.attachments.clear();
    }

    /// Open a session from the sidebar: already open -> just activate,
    /// otherwise create a fresh view and `session/load` it.
    ///
    /// Without a connection (`vibe-acp` restarting, or given up) nothing
    /// could load the session, and an empty view would stay empty: the click
    /// is remembered instead — it replaces the session that was due to be
    /// reopened — and `Initialized` opens it once the agent is ready. Until
    /// then its row is selected and the start screen shows the connection
    /// state.
    pub fn open_session(&mut self, cwd: PathBuf, session_id: &str) {
        self.artifacts_open = false;
        self.ui.activity_open = false;
        self.unseen.remove(session_id);
        self.failed.remove(session_id);
        self.ui.focus_composer = true;
        if !self.sessions.contains_key(session_id) {
            if self.client.is_none() {
                self.open_on_ready = Some((cwd.clone(), session_id.to_string()));
            } else {
                self.load_into_view(cwd.clone(), session_id);
            }
        }
        self.activate(session_id.to_string(), &cwd);
    }

    /// `vibe-acp` ended: drop the dead client and schedule a restart that
    /// reloads the open sessions (their history comes back via
    /// `session/load`). Requests in flight are lost; a held first prompt
    /// goes back into the composer. After `MAX_RECONNECTS` failed restarts
    /// in a row the state becomes `Failed` (manual retry in the status bar).
    fn connection_lost(&mut self, code: Option<i32>, stderr_tail: &str) {
        self.client = None;
        self.rx = None;
        self.starting_since = None;
        if self
            .ready_since
            .is_some_and(|t| t.elapsed() >= STABLE_AFTER)
        {
            self.reconnects = 0;
        }
        self.ready_since = None;
        let was_busy = self.sessions.values().any(|v| v.turn != TurnState::Idle);
        // open approval cards die with the process: the restarted agent
        // never saw their request ids, so no answer reaches it. They must
        // not vanish silently — the status line mentions them (2.8.7).
        let lost_approvals = !self.orphan_permissions.is_empty()
            || self.sessions.values().any(|v| !v.permissions.is_empty());
        // sessions to reload after the restart (child sessions cannot be loaded)
        let loadable = |sid: &Option<String>, sessions: &HashMap<String, SessionView>| {
            sid.as_ref()
                .filter(|s| !s.starts_with("child-"))
                .and_then(|s| sessions.get(s).map(|v| (v.cwd.clone(), s.clone())))
        };
        if self.open_on_ready.is_none() {
            self.open_on_ready = loadable(&self.active, &self.sessions);
        }
        // Queued messages and open questions die with the turn — the texts
        // do not: every session's queue parks under its own session id
        // and shows up when the user opens it again after the restart
        // (joining them into one composer text would mix sessions,
        // review M13). This intentionally runs BEFORE the per-session
        // UI-state drop, which disarms the queue map with the view, and
        // while the views still exist: the status notes name their
        // sessions.
        let mut parked_notes: Vec<String> = Vec::new();
        let mut attachments_lost = false;
        let drained: Vec<(String, Vec<QueuedPrompt>)> = self.queue.drain().collect();
        for (sid, queued) in drained {
            let blocks = queued
                .into_iter()
                .flat_map(|q| q.blocks)
                .collect::<Vec<_>>();
            if blocks.is_empty() {
                continue;
            }
            match self.park_or_restore(Some(&sid), &blocks) {
                DraftSpot::Nothing { attachments } | DraftSpot::Composer { attachments } => {
                    attachments_lost |= attachments
                }
                DraftSpot::Parked { attachments, .. } => {
                    attachments_lost |= attachments;
                    parked_notes.push(self.parked_note_text(Some(&sid)));
                }
            }
        }
        // the held prompt (session/new in flight, or waiting for the start
        // mode) was typed on the start screen, and with the connection the
        // fresh session that was to receive it is gone (Vibe persists a
        // session only with its first message): back to the start screen
        // it goes — visible when it is on screen, parked for it otherwise
        let held_blocks = self
            .queued
            .take()
            .map(|(_, blocks)| blocks)
            .or_else(|| self.pending_mode_prompt.take().map(|(_, blocks)| blocks));
        let held_spot = held_blocks.map(|blocks| self.park_or_restore(None, &blocks));
        match &held_spot {
            Some(DraftSpot::Parked { sid, attachments }) => {
                attachments_lost |= *attachments;
                parked_notes.push(self.parked_note_text(sid.as_deref()));
            }
            Some(DraftSpot::Nothing { attachments })
            | Some(DraftSpot::Composer { attachments }) => attachments_lost |= *attachments,
            None => {}
        }
        let held_visible = matches!(held_spot, Some(DraftSpot::Composer { .. }));
        // no view survives the restart: take every session's UI state with
        // it — form inputs, ui-js tools (their boa contexts), open groups
        // and diff previews (Handoff 2.5). The reload re-seeds whatever
        // the user opens again; a dropped ```ui-js tool is evaluated once
        // more on its next frame. Not a final drop: the views reload
        // with the same ids after the restart, so the parked drafts and
        // UI hints must survive it (drop_session_ui_state).
        for sid in self.sessions.keys().cloned().collect::<Vec<_>>() {
            self.drop_session_ui_state(&sid, false);
        }
        self.questions.clear();
        self.turn_started.clear();
        self.retrying.clear();
        self.account_pending = 0;
        self.account_after_probe = false;
        self.account_probe_session = None;
        self.sessions.clear();
        self.orphan_permissions.clear(); // dead: see lost_approvals above
        self.active = None; // restored by open_on_ready
        self.pending_setup.clear();
        self.fork_pending = None;
        // The CLI prompt belongs to the start this connection never
        // finished: after the restart it would fire hours later into
        // whatever session happens to load then — it goes with the
        // process, nothing fires out of the blue later (review N3).
        self.prompt_on_ready = None;
        self.sent_prompts.clear();
        self.reconnects += 1;
        if self.reconnects > MAX_RECONNECTS {
            let detail = if stderr_tail.is_empty() {
                String::new()
            } else {
                format!(": {stderr_tail}")
            };
            let exit = match code {
                Some(c) => format!(" (Code {c})"),
                None => String::new(),
            };
            self.conn = Conn::Failed(trf(
                "vibe-acp ended{exit}{detail}",
                &[("exit", &exit), ("detail", &detail)],
            ));
            // Failed is terminal until the user retries: the dropped
            // approvals must not vanish silently here either, and the
            // status tells where unsent texts wait
            self.status = if held_visible {
                tr("Connection to Vibe interrupted. The message was not sent and is back in the input field.")
                    .to_string()
            } else if lost_approvals {
                tr("Open approvals were discarded.").to_string()
            } else {
                tr("Connection to Vibe interrupted.").to_string()
            };
            for note in parked_notes {
                self.status.push_str(&note);
            }
            if attachments_lost {
                self.note_attachments_lost();
            }
            return;
        }
        let delay = Duration::from_secs(1 << (self.reconnects - 1));
        self.conn = Conn::Reconnecting {
            attempt: self.reconnects,
            at: Instant::now() + delay,
        };
        self.status = if held_visible {
            tr("Connection to Vibe interrupted. The message was not sent and is back in the input field.").into()
        } else if was_busy {
            tr("Connection to Vibe interrupted. The running answer was cancelled.").into()
        } else {
            tr("Connection to Vibe interrupted.").into()
        };
        // texts that were not sent wait in the composer of the session they
        // belong to — said, so nothing is silently parked away
        for note in parked_notes {
            self.status.push_str(&note);
        }
        if attachments_lost {
            self.note_attachments_lost();
        }
        if lost_approvals {
            self.status
                .push_str(tr(" Open approvals were discarded (please ask again)."));
        }
    }

    /// Manual retry after `Failed`: start again with a fresh restart budget.
    pub fn retry_connection(&mut self) {
        if matches!(self.conn, Conn::Failed(_)) {
            self.reconnects = 0;
            self.conn = Conn::Off;
        }
    }

    /// App exit: close the agent's stdin and let it exit by itself (up to
    /// 2 s, then kill). Without this the child gets SIGKILL via `Drop` and
    /// its own child processes keep running (Handoff 2026-11-09).
    pub fn shutdown_agent(&mut self) {
        if let Some(c) = &self.client {
            let _ = c.shutdown();
        }
    }

    /// Create a view for `session_id` and `session/load` it, without
    /// changing the active session.
    fn load_into_view(&mut self, cwd: PathBuf, session_id: &str) {
        if self.sessions.contains_key(session_id) {
            return;
        }
        let mut view = SessionView::new(session_id.to_string(), cwd.clone());
        view.reset_items();
        self.sessions.insert(session_id.to_string(), view);
        // requests orphaned while this session was unknown migrate into
        // the new view (Handoff 2.2)
        self.adopt_orphan_permissions(session_id);
        let res = self
            .client
            .as_ref()
            .map(|c| c.load_session(&cwd, session_id));
        if let Some(Err(e)) = res {
            self.status = e;
        }
    }

    /// Is a `session/fork` still in flight?
    pub fn fork_in_flight(&self) -> bool {
        self.fork_pending.is_some()
    }

    /// Back to the start screen (no active session). A session still due to
    /// be opened once Vibe is ready (after a restart, or clicked meanwhile)
    /// is dropped: the user's latest choice wins.
    pub fn new_session_view(&mut self) {
        self.active = None;
        self.open_on_ready = None;
        self.artifacts_open = false;
        self.ui.activity_open = false;
        self.ui.focus_composer = true;
    }

    /// Fork the active session (`session/fork`, ACP v1): Vibe copies the
    /// context server-side; when the answer arrives, the new view shows
    /// this client's copy of the transcript (the fork response does not
    /// replay it). Refused while a turn runs or another fork is in flight.
    pub fn fork_active_session(&mut self) {
        self.fork_active_at(None);
    }

    /// Fork the active session after the turn of the user message
    /// `message_id` (`None`: the whole conversation).
    pub fn fork_active_at(&mut self, message_id: Option<String>) {
        let Some(sid) = self.active.clone() else {
            return;
        };
        if self.fork_pending.is_some() || self.client.is_none() {
            return;
        }
        let (cwd, idle) = match self.sessions.get(&sid) {
            Some(s) => (s.cwd.clone(), s.turn == TurnState::Idle),
            None => return,
        };
        if !idle {
            self.status = tr("Fork is not possible while an answer is running.").to_string();
            return;
        }
        let sent = self
            .client
            .as_ref()
            .map(|c| c.fork_session(&cwd, &sid, message_id.as_deref()));
        match sent {
            Some(Ok(_)) => {
                self.fork_pending = Some((sid, message_id));
                self.status = tr("Creating fork …").to_string();
            }
            Some(Err(e)) => self.status = e,
            None => {}
        }
    }

    /// The sidebar rows of one folder: the persisted sessions from
    /// `session/list`, with the active session merged in (see
    /// `sidebar_entries`) — a session Vibe has not listed yet stays visible
    /// and clickable in the sidebar.
    pub fn sidebar_rows(&self, dir: &Path, default_title: &str) -> Vec<SidebarEntry> {
        let list = self.listed.get(dir).cloned().unwrap_or_default();
        let list: Vec<SessionInfo> = list
            .into_iter()
            .filter(|s| !self.archived.contains(&s.session_id))
            .collect();
        let active = self
            .active
            .as_deref()
            .and_then(|sid| self.sessions.get(sid))
            .filter(|v| v.cwd == dir)
            .filter(|v| !self.archived.contains(&v.session_id));
        sidebar_entries(&list, active, default_title, &self.pinned)
    }

    /// The archived sessions of one workspace, for the sidebar's own group.
    pub fn sidebar_rows_archived(&self, dir: &Path, default_title: &str) -> Vec<SidebarEntry> {
        let list = self.listed.get(dir).cloned().unwrap_or_default();
        let list: Vec<SessionInfo> = list
            .into_iter()
            .filter(|s| self.archived.contains(&s.session_id))
            .collect();
        sidebar_entries(&list, None, default_title, &self.pinned)
    }

    /// Mark a session unread (its dot shows in the sidebar until opened).
    pub fn mark_unread(&mut self, session_id: &str) {
        self.unseen.insert(session_id.to_string());
    }

    /// Archive a session locally: it leaves the sidebar's session list and
    /// appears in the workspace's "Archived" group. Persisted through the
    /// settings by the app.
    pub fn archive_session(&mut self, session_id: &str) {
        self.archived.insert(session_id.to_string());
        self.unseen.remove(session_id);
    }

    pub fn unarchive_session(&mut self, session_id: &str) {
        self.archived.remove(session_id);
    }

    /// Append one entry to the activity feed (bounded to the newest 100).
    pub fn push_activity(
        &mut self,
        kind: ActivityKind,
        sid: &str,
        text: String,
        request_id: Option<Value>,
    ) {
        const MAX: usize = 100;
        let (cwd, title) = match self.sessions.get(sid) {
            Some(v) => (v.cwd.clone(), v.title.clone().unwrap_or_default()),
            None => self
                .listed
                .values()
                .flatten()
                .find(|s| s.session_id == sid)
                .map(|s| (PathBuf::from(&s.cwd), s.title.clone().unwrap_or_default()))
                .unwrap_or_default(),
        };
        self.activity.push(ActivityEvent {
            kind,
            sid: sid.to_string(),
            cwd,
            title,
            text,
            request_id,
            at: extras::now_epoch(),
        });
        if self.activity.len() > MAX {
            self.activity.drain(..self.activity.len() - MAX);
        }
    }

    /// Use `dir` (created if missing) as the chat workspace "Allgemein". It
    /// is never a project: an entry for the same folder leaves the project
    /// list (the sidebar shows it once, pinned on top). Without any project
    /// it is where new sessions start.
    pub fn set_chat_dir(&mut self, dir: PathBuf) {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            // The folder does not exist and could not be created: say so
            // and keep whatever chat dir was set before — switching to a
            // dead path would start new sessions in a folder that does
            // not exist (no half state; review's low list).
            let error = e.to_string();
            self.status = trf(
                "The chat folder could not be created: {error}",
                &[("error", &error)],
            );
            return;
        }
        let dir = dir.canonicalize().unwrap_or(dir);
        self.projects.retain(|p| *p != dir);
        if self.current_project.is_none() {
            self.current_project = Some(dir.clone());
        }
        self.chat_dir = Some(dir);
    }

    /// Cancel the active turn: answer every open permission request with
    /// "cancelled", then send `session/cancel`. Questions without a live
    /// session view (no session id, a subagent's `child-…` id, or a
    /// session whose view was dropped) are answered "cancel" as well —
    /// with nothing to stop them they would stay as dead cards whose
    /// turn can never proceed. Questions of other sessions with live
    /// views stay: their turns are not being stopped (review N4).
    pub fn cancel(&mut self) {
        let Some(sid) = self.active.clone() else {
            return;
        };
        let ids = if let Some(v) = self.sessions.get_mut(&sid) {
            v.request_cancel()
        } else {
            Vec::new()
        };
        let open_questions: Vec<Value> = self
            .questions
            .iter()
            .filter(|q| q.session_id.as_deref() == Some(sid.as_str()))
            .map(|q| q.request_id.clone())
            .collect();
        self.questions
            .retain(|q| q.session_id.as_deref() != Some(sid.as_str()));
        // questions without a live view have no session to cancel them
        // from: the stop of the active session is their only way out —
        // except children whose parent view is still alive: their cards
        // belong to that parent, and the parent's own stop will reach
        // them, so an unrelated stop must not answer them (final
        // audit C). A child of the stopped session itself is not in
        // this set — its parent view just went away.
        let foreign_children: HashSet<String> = self
            .sessions
            .iter()
            .filter(|(id, _)| id.as_str() != sid.as_str())
            .flat_map(|(_, v)| v.subagents())
            .filter_map(|info| info.child_session_id)
            .collect();
        let viewless_questions: Vec<Value> = self
            .questions
            .iter()
            .filter(|q| match q.session_id.as_deref() {
                None => true,
                Some(s) => !self.sessions.contains_key(s) && !foreign_children.contains(s),
            })
            .map(|q| q.request_id.clone())
            .collect();
        self.questions.retain(|q| match q.session_id.as_deref() {
            None => false,
            Some(s) => self.sessions.contains_key(s) || foreign_children.contains(s),
        });
        if let Some(c) = &self.client {
            for id in ids {
                let _ = c.respond_permission(&id, None);
            }
            for id in open_questions {
                let _ = c.respond_question(&id, "cancel", None);
            }
            for id in viewless_questions {
                let _ = c.respond_question(&id, "cancel", None);
            }
            let _ = c.cancel(&sid);
        }
    }

    /// Answer one permission request with the chosen option id.
    pub fn answer_permission(&mut self, session_id: &str, request_id: &Value, option_id: &str) {
        let pending = self
            .sessions
            .get_mut(session_id)
            .and_then(|v| v.take_permission(request_id));
        if let (Some(c), Some(p)) = (&self.client, pending) {
            let _ = c.respond_permission(&p.request_id, Some(option_id));
        }
    }

    /// Answer one orphaned permission request (unknown session) with the
    /// chosen option id: the answer goes over the wire by request id, so
    /// it works although no session view exists (Handoff 2.2).
    pub fn answer_orphan_permission(&mut self, request_id: &Value, option_id: &str) {
        let pending = self.take_orphan_permission(request_id);
        if let (Some(c), Some(p)) = (&self.client, pending) {
            let _ = c.respond_permission(&p.request_id, Some(option_id));
        }
    }

    /// Cancel one orphaned permission request ("cancelled" outcome). The
    /// orphan cards carry their own cancel button because the composer's
    /// Stop only targets the active session.
    pub fn cancel_orphan_permission(&mut self, request_id: &Value) {
        let pending = self.take_orphan_permission(request_id);
        if let (Some(c), Some(p)) = (&self.client, pending) {
            let _ = c.respond_permission(&p.request_id, None);
        }
    }

    /// Remove and return the orphaned permission request with this id, if
    /// any. Only the request id matters — that is all the answer carries.
    fn take_orphan_permission(&mut self, request_id: &Value) -> Option<PendingPermission> {
        let pos = self
            .orphan_permissions
            .iter()
            .position(|p| &p.request_id == request_id)?;
        Some(self.orphan_permissions.remove(pos))
    }

    /// Move orphaned permission requests into the session view that just
    /// became known for them (Handoff 2.2): the card migrates from the
    /// active view into its own session, where it renders like a normal
    /// request. Any request stays answerable either way.
    fn adopt_orphan_permissions(&mut self, session_id: &str) {
        for p in std::mem::take(&mut self.orphan_permissions) {
            if p.request.session_id == session_id {
                if let Some(v) = self.sessions.get_mut(session_id) {
                    v.add_permission(p.request_id, p.request);
                }
            } else {
                self.orphan_permissions.push(p);
            }
        }
    }

    /// Send the interactions the user made with ```ui blocks of the
    /// transcript to the active session. All events of one frame go out as
    /// ONE `session/prompt` — a text block per event, in event order
    /// (Handoff 2.8.3): the session must never carry two prompts at once.
    /// While a turn runs nothing is sent (and nothing is queued silently):
    /// the events are dropped with a visible status line.
    pub fn handle_ui_events(&mut self) {
        let events = std::mem::take(&mut self.ui_events);
        if events.is_empty() {
            return;
        }
        let busy = self
            .active
            .as_deref()
            .and_then(|sid| self.sessions.get(sid))
            .map(|v| v.turn != TurnState::Idle)
            .unwrap_or(false);
        if busy || self.queued.is_some() || self.pending_mode_prompt.is_some() {
            self.status = tr("Vibe is still working – interaction discarded.").to_string();
            return;
        }
        let Some(sid) = self.active.clone() else {
            self.status = tr("No active session – interaction discarded.").to_string();
            return;
        };
        if self.client.is_none() {
            self.status = tr("Vibe is not connected.").to_string();
            return;
        }
        // one prompt for the whole frame (Handoff 2.8.3): a text block per
        // event, in event order, instead of a parallel prompt per event
        let mut blocks = Vec::with_capacity(events.len());
        for e in events {
            let text = match e {
                // model-facing protocol (vibe-skills/catwright-ui),
                // English whatever the UI language is
                UiEvent::Button { action } => format!("[UI] Button \"{action}\" clicked."),
                UiEvent::Form { submit, values } => {
                    format!("[UI] Form \"{submit}\" submitted: {values}")
                }
            };
            blocks.push(PromptBlock::Text(text));
        }
        self.send_prompt(&sid, &blocks);
    }

    /// Steps that bring a fresh session to the start mode/model/thinking.
    fn setup_steps(&self, sid: &str) -> Vec<SetupStep> {
        let Some(v) = self.sessions.get(sid) else {
            return Vec::new();
        };
        let mut steps = Vec::new();
        if let Some(modes) = &v.modes {
            // A start mode stored before the app hid the `claude-delegate*`
            // profiles (or picked from the CLI-minded old picker) may name a
            // hidden id. The stored settings value is not rewritten; it is
            // ignored here so Vibe's own default (accept-edits) stays active
            // instead of switching the session into the hidden profile.
            if modes.current_mode_id != self.start_mode && mode_selectable(&self.start_mode) {
                steps.push(SetupStep::Mode(self.start_mode.clone()));
            }
        }
        let mut model_switch = false;
        if let Some(want) = &self.start_model {
            let offered = select_levels(&v.config_options, "model").contains(want);
            if offered && v.current_config_value("model") != Some(want.as_str()) {
                steps.push(SetupStep::Config("model".to_string(), want.clone()));
                model_switch = true;
            }
        }
        if let Some(want) = &self.start_thinking {
            // After a model switch the levels are the new model's: the step
            // is checked against them in `continue_setup`, not against the
            // levels of the model the session started with.
            if model_switch || v.current_config_value("thinking") != Some(want.as_str()) {
                steps.push(SetupStep::Config("thinking".to_string(), want.clone()));
            }
        }
        steps
    }

    /// Send the next setup step for `sid`, or the held prompt when none is left.
    fn continue_setup(&mut self, sid: &str) {
        if !matches!(&self.pending_mode_prompt, Some((s, _)) if s == sid) {
            return;
        }
        while !self.pending_setup.is_empty() {
            let mut step = self.pending_setup.remove(0);
            if let SetupStep::Config(id, want) = &step {
                if id == "thinking" {
                    match self.fit_session_thinking(sid, want) {
                        Some(fitted) => step = SetupStep::Config(id.clone(), fitted),
                        None => continue, // already active
                    }
                }
            }
            let res = self.client.as_ref().map(|c| match &step {
                SetupStep::Mode(m) => c.set_mode(sid, m),
                SetupStep::Config(id, v) => c.set_config_option(sid, id, v),
            });
            match res {
                Some(Ok(_)) => return, // wait for ModeSet / ConfigOptionsChanged
                Some(Err(e)) if matches!(step, SetupStep::Mode(_)) => {
                    // the start mode could not even be requested: same as
                    // a failed set_mode (see RequestFailed)
                    self.abort_setup(&e);
                    return;
                }
                Some(Err(_)) => {
                    self.status = tr("A setting could not be applied – the prompt is sent anyway.")
                        .to_string();
                }
                None => {
                    self.status = tr("Vibe is not connected.").to_string();
                }
            }
        }
        let (_, blocks) = self.pending_mode_prompt.take().expect("checked above");
        self.send_prompt(sid, &blocks);
    }

    /// The start mode could not be set: drop the rest of the setup and put the
    /// held first prompt back into its composer, instead of sending it in a
    /// mode the user did not choose.
    fn abort_setup(&mut self, detail: &str) {
        self.pending_setup.clear();
        self.status = trf(
            "The start mode could not be set ({detail}).",
            &[("detail", &detail)],
        );
        if let Some((sid, blocks)) = self.pending_mode_prompt.take() {
            let spot = self.park_or_restore(Some(&sid), &blocks);
            self.note_not_sent(spot);
        }
    }

    /// The thinking level to send for `want` in session `sid`, fitted to the
    /// levels its current model offers (Vibe rejects others, e.g. `max` on
    /// a model that only takes low/medium/high). `None`: nothing to send,
    /// the level is already active. Unknown levels pass through unchanged.
    fn fit_session_thinking(&self, sid: &str, want: &str) -> Option<String> {
        let Some(v) = self.sessions.get(sid) else {
            return Some(want.to_string());
        };
        let levels = select_levels(&v.config_options, "thinking");
        let fitted = if levels.is_empty() {
            want.to_string()
        } else {
            extras::fit_thinking(want, &levels)?
        };
        (v.current_config_value("thinking") != Some(fitted.as_str())).then_some(fitted)
    }

    /// Thinking levels to offer for `model` before a session exists: what
    /// that model offered last, else the levels seen last.
    pub fn thinking_levels_for(&self, model: Option<&str>) -> Vec<String> {
        model
            .and_then(|m| self.thinking_by_model.get(m))
            .map(|(levels, _)| levels.clone())
            .unwrap_or_else(|| self.known_thinking.clone())
    }

    /// The thinking level `model` had when it was last active.
    pub fn thinking_of_model(&self, model: Option<&str>) -> Option<String> {
        model
            .and_then(|m| self.thinking_by_model.get(m))
            .map(|(_, current)| current.clone())
    }

    /// Pick the model for the next new session. A chosen start thinking
    /// level the model does not offer is fitted right away (e.g. `max` →
    /// `high`), so the picker never shows a level Vibe would reject.
    pub fn choose_start_model(&mut self, model: String) {
        if let (Some(want), Some((levels, _))) =
            (&self.start_thinking, self.thinking_by_model.get(&model))
        {
            if let Some(fitted) = extras::fit_thinking(want, levels) {
                self.start_thinking = Some(fitted);
            }
        }
        self.start_model = Some(model);
    }

    /// Remember the model / thinking choices a session offers.
    fn remember_config(&mut self, options: &[ConfigOption]) {
        let current = |want: &str| {
            options.iter().find_map(|o| match o {
                ConfigOption::Select {
                    id, current_value, ..
                } if id == want => Some(current_value.clone()),
                _ => None,
            })
        };
        let levels = select_levels(options, "thinking");
        if let (Some(model), Some(thinking)) = (current("model"), current("thinking")) {
            if !levels.is_empty() {
                self.thinking_by_model.insert(model, (levels, thinking));
            }
        }
        for o in options {
            if let ConfigOption::Select { id, options, .. } = o {
                match id.as_str() {
                    "model" if !options.is_empty() => {
                        self.known_models = options
                            .iter()
                            .map(|x| (x.value.clone(), x.name.clone()))
                            .collect();
                    }
                    "thinking" if !options.is_empty() => {
                        self.known_thinking = options.iter().map(|x| x.value.clone()).collect();
                    }
                    _ => {}
                }
            }
        }
    }

    /// begin_turn + `session/prompt` for `sid` (queued and held prompts).
    fn send_prompt(&mut self, sid: &str, blocks: &[PromptBlock]) {
        // a new turn clears the failed mark of the last one
        self.failed.remove(sid);
        // remember what is being sent: Vibe echoes the prompt back only on
        // success, so a failed request needs the text for the draft
        self.sent_prompts.insert(sid.to_string(), blocks.to_vec());
        let blocks = self.with_ui_hint(sid, blocks);
        if let Some(v) = self.sessions.get_mut(sid) {
            v.begin_turn();
        }
        self.turn_started.insert(sid.to_string(), Instant::now());
        self.ui.scroll_to_end = true;
        let res = self.client.as_ref().map(|c| c.prompt(sid, &blocks));
        if let Some(Err(e)) = res {
            self.status = e;
        }
    }

    /// Prepend the `[Catwright]` hint to the first prompt of a session
    /// created in this run (only when the catwright-ui skill is installed).
    /// Loaded old sessions and follow-up prompts stay untouched.
    fn with_ui_hint(&mut self, sid: &str, blocks: &[PromptBlock]) -> Vec<PromptBlock> {
        if !self.ui_hint || !self.hint_pending.remove(sid) {
            return blocks.to_vec();
        }
        let mut out = Vec::with_capacity(blocks.len() + 1);
        out.push(PromptBlock::Text(UI_HINT.to_string()));
        out.extend_from_slice(blocks);
        out
    }

    pub fn set_mode(&mut self, session_id: &str, mode_id: &str) {
        let res = self
            .client
            .as_ref()
            .map(|c| c.set_mode(session_id, mode_id));
        match res {
            Some(Ok(_)) => {}
            Some(Err(e)) => self.status = e,
            None => self.status = tr("Vibe is not connected.").to_string(),
        }
    }

    pub fn set_config(&mut self, session_id: &str, config_id: &str, value: &str) {
        let res = self
            .client
            .as_ref()
            .map(|c| c.set_config_option(session_id, config_id, value));
        match res {
            Some(Ok(_)) => {}
            Some(Err(e)) => self.status = e,
            None => self.status = tr("Vibe is not connected.").to_string(),
        }
    }

    /// Add a project folder (must be an existing directory, canonicalized).
    /// Duplicates are ignored. Returns true when the list is usable.
    /// Vibe's saved context sizes for a session; re-read only when the
    /// session's `used` tokens changed (Vibe rewrites the file per model call).
    pub fn context_sizes(&mut self, session_id: &str, used: u64) -> Option<context::ContextSizes> {
        match self.ctx_sizes.get(session_id) {
            Some((at, sizes)) if *at == used => *sizes,
            _ => {
                let sizes = context::load_sizes(session_id);
                self.ctx_sizes.insert(session_id.to_string(), (used, sizes));
                sizes
            }
        }
    }

    /// Load `session_id` (in `cwd`) once `initialize` has completed.
    pub fn open_when_ready(&mut self, cwd: PathBuf, session_id: String) {
        self.open_on_ready = Some((cwd, session_id));
    }

    /// Send `text` as soon as Vibe is ready: into the session given with
    /// `open_when_ready` once it has loaded, otherwise as a new session.
    pub fn prompt_when_ready(&mut self, text: Option<String>) {
        self.prompt_on_ready = text.filter(|t| !t.trim().is_empty());
    }

    /// Send the CLI's start prompt, if one is still waiting. A draft the
    /// user typed meanwhile is never overwritten (review N3): the CLI
    /// prompt takes this send, and the typed text and its attachments go
    /// back to the session they were typed in — visible in its composer
    /// when that view is on screen, parked under its id otherwise — and
    /// the status names the spot (the parked drafts of park_or_restore).
    fn send_prompt_on_ready(&mut self) {
        let Some(text) = self.prompt_on_ready.take() else {
            return;
        };
        // hold the typing back: `send` clears the composer it sends from
        let typed = std::mem::take(&mut self.draft);
        let typed_attachments = std::mem::take(&mut self.attachments);
        let keep = !typed.trim().is_empty() || !typed_attachments.is_empty();
        self.draft = text;
        self.send();
        if !keep {
            return;
        }
        let owner = self.ui.draft_owner.clone();
        if typed.trim().is_empty() {
            // attachments only: they go to their owner's spot directly —
            // their paths are kept, no wire block round trip is possible
            let on_screen = match owner.as_deref() {
                Some(s) => self.active.as_deref() == Some(s),
                None => self.active.is_none(),
            };
            if on_screen {
                self.attachments.extend(typed_attachments);
            } else {
                self.status
                    .push_str(&self.parked_note_text(owner.as_deref()));
                let (parked, mut attachments) =
                    self.ui.parked_drafts.remove(&owner).unwrap_or_default();
                attachments.extend(typed_attachments);
                self.ui.parked_drafts.insert(owner, (parked, attachments));
            }
            return;
        }
        // the text joins its owner's spot through the parked-draft merge
        // rules (park_or_restore); the attachments keep their paths and
        // move to the same spot, not through the blocks
        let spot = self.park_or_restore(owner.as_deref(), &[PromptBlock::Text(typed)]);
        match spot {
            DraftSpot::Parked { sid, .. } => {
                self.status.push_str(&self.parked_note_text(sid.as_deref()));
                let (parked, mut attachments) =
                    self.ui.parked_drafts.remove(&sid).unwrap_or_default();
                attachments.extend(typed_attachments);
                self.ui.parked_drafts.insert(sid, (parked, attachments));
            }
            DraftSpot::Composer { .. } => {
                self.attachments.extend(typed_attachments);
                self.status
                    .push_str(tr(" – Text typed meanwhile stays in the input field."));
            }
            // unreachable here: the typed text is not empty
            DraftSpot::Nothing { .. } => {}
        }
    }

    pub fn add_project(&mut self, path: &str) -> bool {
        let input = Path::new(path.trim());
        if !input.is_dir() {
            self.status = trf("“{path}” is not an existing directory.", &[("path", &path)]);
            return false;
        }
        let canon = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
        // the chat workspace is pinned already: picking it selects it
        if !self.projects.contains(&canon) && !self.is_chat_dir(&canon) {
            self.projects.push(canon.clone());
        }
        self.select_workspace(canon.clone());
        let res = self.client.as_ref().map(|c| c.list_sessions(Some(&canon)));
        if let Some(Err(e)) = res {
            self.status = e;
        }
        true
    }

    /// Drop every per-interaction state entry of `session_id` (Handoff
    /// 2.5): form inputs, ```ui-js tools including their boa contexts,
    /// expanded tool groups and diff previews. Called whenever a session
    /// view goes away, so its state cannot surface in another view at the
    /// same transcript position and the maps stay bounded.
    ///
    /// `dying` for a session that will never come back (closed, deleted,
    /// load failed, project removed): what it left behind goes with it —
    /// the not-yet-echoed prompt (a late `RequestFailed` would otherwise
    /// park its text under the dead id again), the UI hint it never
    /// consumed, and its parked draft (no ghost restore into a view of a
    /// session that no longer exists; review N6). A view dropped for a
    /// reload (connection lost) passes `false`: the same session id
    /// comes back, its parked texts and the hint must survive the
    /// restart.
    fn drop_session_ui_state(&mut self, sid: &str, dying: bool) {
        render::drop_owner_entries(&mut self.form_values, sid);
        render::drop_owner_entries(&mut self.scripts, sid);
        let prefix = render::owner_key_prefix(sid);
        self.open_groups.retain(|k| !k.starts_with(&prefix));
        self.diff_open.retain(|k| !k.starts_with(&prefix));
        self.ctx_sizes.remove(sid);
        self.loops.remove(sid);
        self.review.remove(sid);
        self.review_scopes.remove(sid);
        self.turn_diffs.remove(sid);
        self.connectors.remove(sid);
        self.connectors_requested.remove(sid);
        self.queue.remove(sid);
        self.turn_started.remove(sid);
        self.last_turn_secs.remove(sid);
        self.retrying.remove(sid);
        self.unseen.remove(sid);
        if dying {
            self.sent_prompts.remove(sid);
            self.hint_pending.remove(sid);
            self.ui.parked_drafts.remove(&Some(sid.to_string()));
        }
    }

    /// Close a project: it leaves the list, and its sessions close with it
    /// — really: every session of the folder is cancelled and closed on the
    /// agent first (open approvals answered, running turns stopped, review
    /// M7), then the views, listed entries and their UI state go (Handoff
    /// 2.8.4, 2.5). `active` is cleared when it points at a dropped session;
    /// sessions of other projects stay untouched. The chat workspace
    /// cannot be removed.
    pub fn remove_project(&mut self, path: &Path) {
        if self.is_chat_dir(path) {
            return;
        }
        // Server-side first (review M7): nothing of this project keeps
        // running once it is gone. Every dropped session gets the same
        // treatment as `cancel()` — its open approvals the "cancelled"
        // answer (the request can no longer proceed) — then
        // `session/cancel` stops the running turn and `session/close`
        // ends the session and its loops. Only then do the views go.
        let dropped: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, v)| v.cwd == path)
            .map(|(sid, _)| sid.clone())
            .collect();
        if let Some(c) = &self.client {
            for sid in &dropped {
                let ids = self
                    .sessions
                    .get_mut(sid)
                    .map(|v| v.request_cancel())
                    .unwrap_or_default();
                for id in ids {
                    let _ = c.respond_permission(&id, None);
                }
                let _ = c.cancel(sid);
                let _ = c.close_session(sid);
            }
        }
        self.projects.retain(|p| p != path);
        self.listed.remove(path);
        for sid in &dropped {
            self.drop_session_ui_state(sid, true);
        }
        self.sessions.retain(|_, v| v.cwd != path);
        if self
            .active
            .as_deref()
            .is_some_and(|sid| !self.sessions.contains_key(sid))
        {
            self.active = None;
        }
        if self.current_project.as_deref() == Some(path) {
            self.current_project = self.fallback_workspace();
        }
    }
}

// ---------------------------------------------------------------------------
// Vibe's ACP extensions (verified against vibe-acp 3.1.0, see docs/acp.md)
// ---------------------------------------------------------------------------

impl Harness {
    /// Send an extension request; failures to send land in the status line.
    fn send_ext(&mut self, kind: ExtKind) -> bool {
        match self.client.as_ref().map(|c| c.ext(kind)) {
            Some(Ok(_)) => true,
            Some(Err(e)) => {
                self.status = e;
                false
            }
            None => {
                self.status = tr("Vibe is not connected.").to_string();
                false
            }
        }
    }

    fn refresh_list(&mut self, cwd: &Path) {
        let res = self.client.as_ref().map(|c| c.list_sessions(Some(cwd)));
        if let Some(Err(e)) = res {
            self.status = e;
        }
    }

    /// What the sidebar shows for a session.
    pub fn session_state(&self, session_id: &str) -> SessionState {
        let waiting = self
            .sessions
            .get(session_id)
            .is_some_and(|v| !v.permissions.is_empty())
            || self
                .questions
                .iter()
                .any(|q| q.session_id.as_deref() == Some(session_id));
        if waiting {
            return SessionState::Waiting;
        }
        if self
            .sessions
            .get(session_id)
            .is_some_and(|v| v.turn != TurnState::Idle)
        {
            return SessionState::Running;
        }
        if self.failed.contains(session_id) {
            return SessionState::Failed;
        }
        if self.unseen.contains(session_id) {
            return SessionState::Unseen;
        }
        SessionState::Idle
    }

    /// The sidebar's mark on a workspace row: something in `dir` waits for
    /// the user — a session's permission request or question, or the
    /// request of a subagent one of its sessions spawned.
    pub fn workspace_waiting(&self, dir: &Path) -> bool {
        workspace_waiting_permission(&self.sessions, &self.orphan_permissions, dir)
            || self
                .sessions
                .values()
                .any(|v| v.cwd == dir && self.session_state(&v.session_id) == SessionState::Waiting)
    }

    /// The live slash commands: the active session's, else the last seen.
    pub fn available_commands(&self) -> Vec<AvailableCommand> {
        self.active
            .as_deref()
            .and_then(|sid| self.sessions.get(sid))
            .map(|v| v.commands.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| self.known_commands.clone())
    }

    /// `/name args` as a message of the active session (queued while a
    /// turn runs). Vibe handles it like any prompt text.
    pub fn run_command(&mut self, command: &str) {
        let Some(sid) = self.active.clone() else {
            self.draft = command.to_string();
            self.ui.focus_composer = true;
            return;
        };
        let blocks = vec![PromptBlock::Text(command.to_string())];
        let busy = self
            .sessions
            .get(&sid)
            .is_some_and(|v| v.turn != TurnState::Idle);
        if busy {
            self.queue.entry(sid).or_default().push(QueuedPrompt {
                text: command.to_string(),
                blocks,
            });
            return;
        }
        // the same guard as `send` (review N8): a session/new still in
        // flight or a prompt held back for the start mode must never run
        // in parallel with another prompt
        if self.queued.is_some() || self.pending_mode_prompt.is_some() {
            self.status = tr("Vibe is still working – please wait or stop it.").to_string();
            return;
        }
        if self.sessions.contains_key(&sid) {
            self.send_prompt(&sid, &blocks);
        }
    }

    /// Send the next queued message of an idle session.
    pub fn send_next_queued(&mut self, session_id: &str) {
        let next = self.queue.get_mut(session_id).and_then(|q| {
            if q.is_empty() {
                None
            } else {
                Some(q.remove(0))
            }
        });
        if let Some(prompt) = next {
            self.send_prompt(session_id, &prompt.blocks);
        }
    }

    /// Put a session's queued messages back without sending them: the turn
    /// was stopped (cancel) or failed before it ran (prompt error), so the
    /// texts go to the session they were typed in — visible in its composer
    /// when it is on screen, parked under its id otherwise (park_or_restore,
    /// review M13). Returns where they went; `None`: nothing was queued.
    fn restore_queue(&mut self, session_id: &str) -> Option<DraftSpot> {
        let queued = self.queue.remove(session_id)?;
        let blocks = queued
            .into_iter()
            .flat_map(|q| q.blocks)
            .collect::<Vec<_>>();
        Some(self.park_or_restore(Some(session_id), &blocks))
    }

    pub fn rename_session(&mut self, session_id: &str, cwd: &Path, title: &str) {
        let title = title.trim();
        if !title.is_empty() {
            self.send_ext(ExtKind::SetTitle {
                session_id: session_id.to_string(),
                title: title.to_string(),
                cwd: cwd.to_path_buf(),
            });
        }
    }

    pub fn delete_session(&mut self, session_id: &str, cwd: &Path) {
        if self
            .sessions
            .get(session_id)
            .is_some_and(|v| v.turn != TurnState::Idle)
        {
            self.status = tr("Deleting is not possible while an answer is running.").to_string();
            return;
        }
        self.send_ext(ExtKind::DeleteSession {
            session_id: session_id.to_string(),
            cwd: cwd.to_path_buf(),
        });
    }

    /// Ask Vibe which files a rewind to `message_id` would restore; the
    /// answer opens the confirmation dialog.
    pub fn begin_rewind(&mut self, session_id: &str, message_id: &str) {
        let idle = self
            .sessions
            .get(session_id)
            .is_some_and(|v| v.turn == TurnState::Idle);
        if !idle {
            self.status = tr("Rewinding is possible once the answer is done.").to_string();
            return;
        }
        self.send_ext(ExtKind::RewindPreview {
            session_id: session_id.to_string(),
            message_id: message_id.to_string(),
        });
    }

    pub fn confirm_rewind(&mut self, session_id: &str, message_id: &str, restore_files: bool) {
        self.send_ext(ExtKind::RewindTo {
            session_id: session_id.to_string(),
            message_id: message_id.to_string(),
            restore_files,
        });
    }

    /// Read plan, budget and identity. Requires a session: the active one,
    /// or a short-lived probe. Runs only on the user's request or with the
    /// `auto_account` setting (no network call the user did not ask for).
    pub fn load_account(&mut self) {
        if self.account_pending > 0 || self.account_after_probe {
            return;
        }
        let live = self
            .active
            .clone()
            .filter(|s| self.sessions.contains_key(s))
            .or_else(|| {
                self.sessions
                    .keys()
                    .find(|s| !s.starts_with("child-"))
                    .cloned()
            });
        if let Some(sid) = live {
            self.request_account(&sid);
            return;
        }
        let Some(dir) = self
            .chat_dir
            .clone()
            .or_else(|| self.current_project.clone())
        else {
            self.status = tr("Vibe is not connected.").to_string();
            return;
        };
        match self.client.as_ref().map(|c| c.new_probe_session(&dir)) {
            Some(Ok(_)) => {
                self.account_after_probe = true;
                self.account_pending = 1; // shows as loading until the probe answers
            }
            Some(Err(e)) => self.status = e,
            None => self.status = tr("Vibe is not connected.").to_string(),
        }
    }

    /// Is an account read in flight?
    pub fn account_loading(&self) -> bool {
        self.account_pending > 0 || self.account_after_probe
    }

    fn request_account(&mut self, session_id: &str) {
        let sid = session_id.to_string();
        let mut sent = 0;
        if self.send_ext(ExtKind::Account {
            session_id: sid.clone(),
        }) {
            sent += 1;
        }
        if self.send_ext(ExtKind::Identity { session_id: sid }) {
            sent += 1;
        }
        self.account_pending = sent;
    }

    /// One account answer arrived (or failed): the probe closes with the last.
    fn account_answered(&mut self) {
        self.account_pending = self.account_pending.saturating_sub(1);
        if self.account_pending == 0 {
            if let Some(sid) = self.account_probe_session.take() {
                if let Some(c) = &self.client {
                    let _ = c.close_session(&sid);
                }
            }
        }
    }

    pub fn decide_trust(&mut self, session_id: &str, cwd: &Path, decision: &str) {
        self.send_ext(ExtKind::TrustDecision {
            session_id: session_id.to_string(),
            cwd: cwd.to_path_buf(),
            decision: decision.to_string(),
        });
    }

    /// Refresh the data of the right panel that lives in Vibe (local reads,
    /// no model call). Skipped for child sessions, which cannot be queried.
    pub fn load_session_extras(&mut self, session_id: &str) {
        if session_id.starts_with("child-") || !self.sessions.contains_key(session_id) {
            return;
        }
        let sid = session_id.to_string();
        self.send_ext(ExtKind::LoopsList {
            session_id: sid.clone(),
        });
        self.send_ext(ExtKind::ReviewState { session_id: sid });
    }

    pub fn create_loop(&mut self, session_id: &str, interval: &str, prompt: &str) {
        self.send_ext(ExtKind::LoopsCreate {
            session_id: session_id.to_string(),
            interval: interval.trim().to_string(),
            prompt: prompt.trim().to_string(),
        });
    }

    pub fn delete_loop(&mut self, session_id: &str, loop_id: &str) {
        self.send_ext(ExtKind::LoopsDelete {
            session_id: session_id.to_string(),
            loop_id: loop_id.to_string(),
        });
    }

    /// Mark changes as reviewed (`approve`) or undo them in the working
    /// tree. `target` is Vibe's review target, e.g. `{"kind":"all"}`.
    pub fn review_action(&mut self, session_id: &str, approve: bool, target: Value) {
        self.send_ext(ExtKind::ReviewMutate {
            session_id: session_id.to_string(),
            approve,
            target,
        });
    }

    pub fn review_file_target(path: &str) -> Value {
        json!({ "kind": "file", "path": path })
    }

    pub fn review_all_target() -> Value {
        json!({ "kind": "all" })
    }

    /// Load one file's diff for the turn `turn_id` caused
    /// (`_review/turnDiff`); the transcript row shows the result. A second
    /// click while the answer is in flight sends nothing.
    pub fn load_turn_diff(&mut self, session_id: &str, turn_id: &str, path: &str) {
        let view = self.turn_diffs.entry(session_id.to_string()).or_default();
        if view.loading(turn_id, path) {
            return;
        }
        view.insert(turn_id, path, TurnDiffView::Loading);
        self.send_ext(ExtKind::ReviewTurnDiff {
            session_id: session_id.to_string(),
            path: path.to_string(),
            turn_id: turn_id.to_string(),
        });
    }

    /// Forget the loaded turn diffs after a new `_review/state` answer: the
    /// baseline of a turn can have moved (a revert, a rewind).
    pub fn review_refresh(&mut self, session_id: &str) {
        if let Some(diffs) = self.turn_diffs.get_mut(session_id) {
            diffs.clear();
        }
    }

    /// Load a session's connector list once (the card opens lazily).
    pub fn request_connectors(&mut self, session_id: &str) {
        if self.connectors_requested.insert(session_id.to_string()) {
            self.send_ext(ExtKind::ConnectorsList {
                session_id: session_id.to_string(),
            });
        }
    }

    pub fn toggle_connector(&mut self, session_id: &str, name: &str, disabled: bool) {
        self.send_ext(ExtKind::ConnectorsToggle {
            session_id: session_id.to_string(),
            name: name.to_string(),
            disabled,
        });
    }

    /// Answer the question at `index` with the collected form content, or
    /// decline it. The turn continues either way.
    pub fn answer_question(&mut self, index: usize, accept: bool) {
        if index >= self.questions.len() {
            return;
        }
        let content = if accept {
            match self.questions[index].content() {
                Some(c) => Some(c),
                None => return, // not all answered: the button stays disabled
            }
        } else {
            None
        };
        let q = self.questions.remove(index);
        if let Some(c) = &self.client {
            let action = if accept { "accept" } else { "decline" };
            let _ = c.respond_question(&q.request_id, action, content);
        }
    }

    fn handle_ext_result(&mut self, kind: ExtKind, result: Value) {
        match kind {
            ExtKind::SetTitle {
                session_id,
                title,
                cwd,
            } => {
                if let Some(v) = self.sessions.get_mut(&session_id) {
                    v.title = Some(title);
                }
                self.refresh_list(&cwd);
            }
            ExtKind::DeleteSession { session_id, cwd } => {
                self.sessions.remove(&session_id);
                self.drop_session_ui_state(&session_id, true);
                if self.active.as_deref() == Some(session_id.as_str()) {
                    self.active = None;
                }
                if let Some(list) = self.listed.get_mut(&cwd) {
                    list.retain(|s| s.session_id != session_id);
                }
                self.refresh_list(&cwd);
                self.status = tr("Session deleted.").to_string();
            }
            ExtKind::RewindPreview {
                session_id,
                message_id,
            } => {
                let paths = string_list(&result["paths"]);
                self.ui.rewind = Some(state::RewindDialog {
                    session_id,
                    message_id,
                    paths,
                    restore_files: true,
                });
            }
            ExtKind::RewindTo {
                session_id,
                message_id,
                ..
            } => {
                if let Some(v) = self.sessions.get_mut(&session_id) {
                    v.rewind_to_message(&message_id);
                }
                // the rewound message goes back to the composer of its own
                // session (M13): visible when it is on screen, parked under
                // the session id otherwise. Nothing is discarded — an
                // occupied composer gets the text appended below the fresh
                // typing, instead of the silent drop this used to be.
                let mut parked_sid = None;
                if let Some(text) = result["messageContent"].as_str() {
                    let spot = self.park_or_restore(
                        Some(&session_id),
                        &[PromptBlock::Text(without_ui_hint(text).to_string())],
                    );
                    if let DraftSpot::Parked { sid, .. } = spot {
                        parked_sid = sid;
                    }
                }
                self.ui.focus_composer = true;
                let restored = string_list(&result["restoredPaths"]).len();
                let failed = string_list(&result["restoreErrors"]).len();
                self.status = match (restored, failed) {
                    (_, n) if n > 0 => {
                        trf("Rewound – {n} file(s) could not be restored.", &[("n", &n)])
                    }
                    (0, _) => tr("Rewound.").to_string(),
                    (1, _) => tr("Rewound – 1 file restored.").to_string(),
                    (n, _) => trf("Rewound – {n} files restored.", &[("n", &n)]),
                };
                // the rewound text went to another session's composer: say
                // where it is, it is not silently gone
                if let Some(sid) = parked_sid {
                    let note = self.parked_note_text(Some(sid.as_str()));
                    self.status.push_str(&note);
                }
                self.load_session_extras(&session_id);
            }
            ExtKind::Account { .. } => {
                self.account = Some(extras::parse_account(&result));
                self.account_answered();
            }
            ExtKind::Identity { .. } => {
                self.identity = Some(extras::parse_identity(&result));
                self.account_answered();
            }
            ExtKind::TrustDecision { cwd, .. } => {
                let trust = extras::trust_of_answer(&result);
                self.status = if trust.trusted {
                    tr("Folder trusted – AGENTS.md, skills and hooks apply from the next session.")
                        .to_string()
                } else {
                    tr("The folder stays untrusted.").to_string()
                };
                self.trust.insert(cwd, trust);
            }
            ExtKind::LoopsList { session_id } => {
                self.loops.insert(session_id, extras::parse_loops(&result));
            }
            ExtKind::LoopsCreate { session_id, .. } | ExtKind::LoopsDelete { session_id, .. } => {
                self.send_ext(ExtKind::LoopsList { session_id });
            }
            ExtKind::ReviewState { session_id } => {
                let state = extras::parse_review_state(&result);
                self.review.insert(session_id.clone(), state.files);
                self.review_scopes.insert(session_id.clone(), state.scopes);
                // a revert or rewind may have moved the baselines
                self.review_refresh(&session_id);
            }
            ExtKind::ReviewTurnDiff {
                session_id,
                path,
                turn_id,
            } => {
                let view = match extras::parse_turn_diff(&result) {
                    Some(d) => TurnDiffView::Loaded(d),
                    None => TurnDiffView::Failed(extras::turn_diff_note(
                        result["error"].as_str().or(result["status"].as_str()),
                    )),
                };
                self.turn_diffs
                    .entry(session_id)
                    .or_default()
                    .insert(&turn_id, &path, view);
            }
            ExtKind::ReviewMutate {
                session_id,
                approve,
                ..
            } => {
                self.status = if approve {
                    tr("Changes accepted.").to_string()
                } else {
                    tr("Changes reverted.").to_string()
                };
                if let Some(dir) = self.sessions.get(&session_id).map(|v| v.cwd.clone()) {
                    self.watcher.invalidate_git(&dir);
                }
                self.send_ext(ExtKind::ReviewState { session_id });
            }
            ExtKind::ConnectorsList { session_id }
            | ExtKind::ConnectorsToggle { session_id, .. } => {
                self.connectors
                    .insert(session_id, extras::parse_connectors(&result));
            }
        }
    }

    /// An extension request failed: say so (background refreshes stay quiet).
    fn ext_failed(&mut self, kind: &ExtKind, error: &rpc::RpcError) {
        let detail = error.message.lines().next().unwrap_or("").trim();
        let what = match kind {
            ExtKind::SetTitle { .. } => tr("Rename"),
            ExtKind::DeleteSession { .. } => tr("Delete"),
            ExtKind::RewindPreview { .. } | ExtKind::RewindTo { .. } => tr("Rewind"),
            ExtKind::Account { .. } | ExtKind::Identity { .. } => {
                self.account_answered();
                tr("Read account")
            }
            ExtKind::TrustDecision { .. } => tr("Save trust"),
            ExtKind::LoopsCreate { .. } => tr("Create routine"),
            ExtKind::LoopsDelete { .. } => tr("Delete routine"),
            ExtKind::ReviewMutate { .. } => tr("Apply changes"),
            ExtKind::ReviewTurnDiff {
                session_id,
                path,
                turn_id,
            } => {
                // the click is answered honestly in the transcript row
                let note = extras::turn_diff_note(Some(detail));
                self.turn_diffs
                    .entry(session_id.clone())
                    .or_default()
                    .insert(turn_id, path, TurnDiffView::Failed(note));
                return;
            }
            ExtKind::ConnectorsToggle { .. } => tr("Toggle connector"),
            // background refreshes: an older Vibe or a busy session is no
            // reason to bother the user
            ExtKind::LoopsList { .. } | ExtKind::ReviewState { .. } => return,
            ExtKind::ConnectorsList { session_id } => {
                self.connectors_requested.remove(session_id);
                return;
            }
        };
        self.status = if error.code == rpc::AUTH_REQUIRED {
            tr("Vibe is not signed in. Please start `vibe` once in a terminal and sign in.")
                .to_string()
        } else {
            trf(
                "{what} failed: {detail}",
                &[("what", &what), ("detail", &detail)],
            )
        };
    }
}

// ---------------------------------------------------------------------------
// pure helpers
// ---------------------------------------------------------------------------

/// How long a worktree checkout may take (a big repository on a slow disk).
const WORKTREE_TIMEOUT: Duration = Duration::from_secs(600);

/// `git worktree add -b catwright/<name> <dir> HEAD` for `project`; on
/// failure the half-made worktree and its branch are removed again. Runs
/// on a background thread (see `Harness::create_worktree`).
fn make_worktree(project: &Path, dir: &Path) -> Result<PathBuf, String> {
    use crate::harness::watch::run_git;
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let branch = format!("catwright/{name}");
    let dir_str = dir.to_string_lossy().into_owned();
    let added = run_git(
        project,
        &["worktree", "add", "-b", &branch, &dir_str, "HEAD"],
        WORKTREE_TIMEOUT,
    );
    match added {
        Ok(_) => Ok(dir.to_path_buf()),
        Err(e) => {
            // best effort: whatever git got done before it failed goes again
            let short = Duration::from_secs(30);
            let _ = run_git(project, &["worktree", "remove", "--force", &dir_str], short);
            let _ = run_git(project, &["worktree", "prune"], short);
            let _ = run_git(project, &["branch", "-D", &branch], short);
            Err(e)
        }
    }
}

/// Prepended to the first prompt of a new session when the catwright-ui
/// skill is installed: tells Vibe that this GUI renders ```ui blocks.
pub const UI_HINT: &str = "[Catwright] This GUI renders ```ui blocks as native widgets \
(skill: catwright-ui). Use them when an interactive answer helps.";

/// `text` without the leading hint, if it starts with it.
fn strip_hint(text: &str) -> Option<&str> {
    text.strip_prefix(UI_HINT)
}

/// The four Vibe standard modes with Vibe's own names (as `vibe-acp`
/// reports them), used on the start screen until a session reported its
/// `available_modes`. Display names go through `mode_label` (see there).
pub const DEFAULT_MODE_NAMES: [(&str, &str); 4] = [
    ("ask", "Ask"),
    ("accept-edits", "Accept Edits"),
    ("smart-approve", "Smart Approve"),
    ("auto-approve", "Auto Approve"),
];

/// Mode ids hidden from this app's selection lists: the user's personal
/// `claude-delegate` profile family (from `~/.vibe/agents/`) is meant for
/// the Vibe CLI, not for this GUI. Matched by prefix so both profiles
/// (`claude-delegate`, `claude-delegate-rust`) and natural future suffixes
/// are covered. Hiding is display-only: ids are sent unchanged on the
/// wire, and a session already running in such a mode keeps showing it.
const HIDDEN_MODE_PREFIX: &str = "claude-delegate";

/// True when a mode may be offered in the app's selection lists
/// (see `HIDDEN_MODE_PREFIX`).
pub fn mode_selectable(id: &str) -> bool {
    !id.starts_with(HIDDEN_MODE_PREFIX)
}

/// Display label for a Vibe mode: Vibe's own name, with one app-side
/// rename — `ask` is shown as "Plan". The meaning of the mode is
/// unchanged, so only the label differs: the id `ask` still goes over the
/// wire (`session/set_mode`, `settings.json`), and anything but `ask`
/// keeps Vibe's own name. This is the single place where the rename lives.
pub fn mode_label<'a>(id: &'a str, name: &'a str) -> &'a str {
    if id == "ask" {
        "Plan"
    } else {
        name
    }
}

/// Label for the mode a session is currently running in — the one shown on
/// the session's mode combo: the current mode's display name, or its raw
/// id if the session's mode list does not name it. A hidden `claude-delegate*`
/// mode (a session started from the CLI, say) still renders its Vibe name
/// here: hiding applies to the selectable list, never to an active mode.
pub fn current_mode_label(modes: &ModeState) -> String {
    modes
        .available_modes
        .iter()
        .find(|m| m.id == modes.current_mode_id)
        .map(|m| mode_label(&m.id, &m.name).to_owned())
        .unwrap_or_else(|| modes.current_mode_id.clone())
}

/// Modes the user can pick for the NEXT session (start screen and
/// settings): (id, display label) pairs — the last seen `available_modes`,
/// or the four Vibe defaults when no session has reported any yet —
/// without the hidden `claude-delegate*` profiles and with `ask` labelled
/// as "Plan" (`mode_selectable`, `mode_label`).
pub fn start_modes(known: &[SessionMode]) -> Vec<(String, String)> {
    if known.is_empty() {
        DEFAULT_MODE_NAMES
            .iter()
            .map(|(id, name)| (id.to_string(), mode_label(id, name).to_string()))
            .collect()
    } else {
        known
            .iter()
            .filter(|m| mode_selectable(&m.id))
            .map(|m| (m.id.clone(), mode_label(&m.id, &m.name).to_owned()))
            .collect()
    }
}

/// Compact token count: 950 -> "950", 121_400 -> "121k", 800_000 -> "800k",
/// 1_000_000 -> "1M", 1_250_000 -> "1.3M".
pub fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        return format!("{}k", n / 1_000);
    }
    // round to 0.1M
    let tenths = (n + 50_000) / 100_000;
    let whole = tenths / 10;
    let frac = tenths % 10;
    if frac == 0 {
        format!("{whole}M")
    } else {
        format!("{whole}.{frac}M")
    }
}

/// German one-liner for a turn's stop reason (None = nothing worth showing).
pub fn stop_reason_note(reason: &StopReason) -> Option<&'static str> {
    match reason {
        StopReason::EndTurn => None,
        StopReason::Cancelled => Some(tr("Cancelled")),
        StopReason::MaxTokens => Some(tr("Token limit reached")),
        StopReason::MaxTurnRequests => Some(tr("Step limit reached")),
        StopReason::Refusal => Some(tr("Refused")),
        StopReason::Other => None,
    }
}

/// Slash-command matches for the draft: only while the draft starts with '/'
/// and contains no whitespace yet; prefix match on the command name.
pub fn filter_commands<'a>(cmds: &'a [AvailableCommand], draft: &str) -> Vec<&'a AvailableCommand> {
    let Some(prefix) = draft.strip_prefix('/') else {
        return Vec::new();
    };
    if prefix.contains(char::is_whitespace) {
        return Vec::new();
    }
    let needle = prefix.to_lowercase();
    cmds.iter()
        .filter(|c| c.name.to_lowercase().starts_with(&needle))
        .collect()
}

/// Draft + attachments as `session/prompt` blocks: the text (if not
/// empty), then one block per file — a PNG within the size cap becomes
/// an `image` block (base64, Vibe's `promptCapabilities.image`),
/// everything else a `resource_link` the agent reads itself.
pub fn prompt_blocks(text: &str, attachments: &[PathBuf], images: bool) -> Vec<PromptBlock> {
    let mut out = Vec::new();
    if !text.trim().is_empty() {
        out.push(PromptBlock::Text(text.to_string()));
    }
    for a in attachments {
        let abs = a.canonicalize().unwrap_or_else(|_| a.clone());
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| abs.to_string_lossy().into_owned());
        if images {
            if let Some(bytes) = read_png(&abs) {
                out.push(PromptBlock::Image {
                    data_base64: base64_encode(&bytes),
                    mime_type: "image/png".into(),
                });
                continue;
            }
        }
        out.push(PromptBlock::ResourceLink {
            uri: file_uri(&abs),
            name,
        });
    }
    out
}

/// `file://` URI with the path percent-encoded (spaces and non-ASCII must
/// not arrive raw in the agent's link).
pub fn file_uri(path: &Path) -> String {
    const SAFE: &str = "-._~/";
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || SAFE.bytes().any(|s| s == b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// How large an attached image may be before it is sent as a link.
pub const MAX_IMAGE_ATTACHMENT_BYTES: u64 = 5 * 1024 * 1024;

/// Read a PNG attachment for an `image` block: the file must exist, end
/// in `.png` and stay within [`MAX_IMAGE_ATTACHMENT_BYTES`] — anything
/// else returns None and goes out as a `resource_link` instead.
/// One open, and the read bounded through that same handle: whatever
/// happens to the path between the size check and the read (the file
/// grows, is swapped by a rename), the allocation stays capped.
fn read_png(path: &Path) -> Option<Vec<u8>> {
    if !path
        .extension()
        .and_then(|e| e.to_str())?
        .eq_ignore_ascii_case("png")
    {
        return None;
    }
    // The path's type decides before the open (final audit B guards the
    // same class for the previews): a FIFO or device named `*.png`, or a
    // symlink to one, must not block the open on the UI thread. Only a
    // regular file within the cap proceeds.
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_IMAGE_ATTACHMENT_BYTES {
        return None;
    }
    // One open, and the read bounded through that same handle: whatever
    // happens to the path between the check and the read (the file grows,
    // is swapped by a rename), the allocation stays capped.
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_IMAGE_ATTACHMENT_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    let read = file
        .take(MAX_IMAGE_ATTACHMENT_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    if read as u64 > MAX_IMAGE_ATTACHMENT_BYTES {
        return None;
    }
    Some(bytes)
}

/// Standard base64 (RFC 4648, with padding), no crate: the only use in
/// the app is encoding image attachments.
fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Case-insensitive sidebar search: an empty query matches everything.
pub fn matches_query(text: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || text.to_lowercase().contains(&q)
}

/// Which sessions of one folder the sidebar shows under a query:
/// `None` hides the folder. An empty query shows everything; a folder
/// whose name matches shows all its sessions; otherwise only sessions
/// whose title match, and the folder goes away when none does.
pub fn filter_entries(
    folder: &str,
    entries: Vec<SidebarEntry>,
    query: &str,
) -> Option<Vec<SidebarEntry>> {
    if !query.trim().is_empty() && matches_query(folder, query) {
        return Some(entries);
    }
    let kept: Vec<_> = entries
        .into_iter()
        .filter(|s| matches_query(&s.title, query))
        .collect();
    if kept.is_empty() && !matches_query(folder, query) && !query.trim().is_empty() {
        return None;
    }
    Some(kept)
}

/// Note for a turn Vibe is retrying (`_session/retrying`).
pub fn retry_note(category: &str, detail: &str) -> String {
    let reason = match category {
        "rate_limited" => tr("rate limit reached").to_string(),
        "server_error" => tr("server error").to_string(),
        "timed_out" => tr("timeout").to_string(),
        "connection" => tr("connection problem").to_string(),
        _ if detail.is_empty() => tr("temporary error").to_string(),
        _ => detail.lines().next().unwrap_or("").trim().to_string(),
    };
    trf("Retrying: {reason}", &[("reason", &reason)])
}

fn string_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Last path component (fallback: the whole path).
pub fn project_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// sidebar lists (pure helpers; the sidebars call `Harness::sidebar_rows`)
// ---------------------------------------------------------------------------

/// Cap for a session title derived from the first user message.
const SIDEBAR_TITLE_MAX: usize = 48;

/// One row of a sidebar session list: a session of one workspace with the
/// title it is shown under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarEntry {
    pub session_id: String,
    pub title: String,
}

/// The sessions a sidebar shows for one folder: persisted ones from
/// `session/list`, with the active session merged in at the top
/// while Vibe has not listed it yet (Vibe persists a session only with its
/// first message). The active session never appears twice. Its own title
/// from `session_info_update` (`SessionView::title`) wins over the
/// possibly stale title of the list entry. `default_title` is the German
/// placeholder for sessions without a title.
pub fn sidebar_entries(
    listed: &[SessionInfo],
    active: Option<&SessionView>,
    default_title: &str,
    pinned: &HashSet<String>,
) -> Vec<SidebarEntry> {
    // pinned first, then most recently updated (`updatedAt` is ISO-8601,
    // so string order is time order; sessions without one sort last)
    let mut listed: Vec<&SessionInfo> = listed.iter().collect();
    listed.sort_by(|a, b| {
        let pa = pinned.contains(&a.session_id);
        let pb = pinned.contains(&b.session_id);
        pb.cmp(&pa).then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    let mut out = Vec::with_capacity(listed.len() + 1);
    if let Some(v) = active {
        if !listed.iter().any(|s| s.session_id == v.session_id) {
            let title = match &v.title {
                Some(t) => display_title(t, default_title),
                // nothing from Vibe yet: derive from what the user sent
                None => v
                    .items
                    .iter()
                    .find_map(|i| match i {
                        Item::User { text, .. } => derive_title(text),
                        _ => None,
                    })
                    .unwrap_or_else(|| default_title.to_string()),
            };
            out.push(SidebarEntry {
                session_id: v.session_id.clone(),
                title,
            });
        }
    }
    for s in listed {
        // the open session's own title is the freshest one
        let own = active
            .filter(|v| v.session_id == s.session_id)
            .and_then(|v| v.title.clone());
        let title = own.or_else(|| s.title.clone()).map_or_else(
            || default_title.to_string(),
            |t| display_title(&t, default_title),
        );
        out.push(SidebarEntry {
            session_id: s.session_id.clone(),
            title,
        });
    }
    out
}

/// A message as the user wrote it: without the `[Catwright]` hint this
/// harness prepends to a session's first prompt (Vibe echoes it back).
pub fn without_ui_hint(text: &str) -> &str {
    strip_hint(text.trim_start()).map_or(text, str::trim_start)
}

/// One attached image inside a user message as Vibe mirrors it back: the
/// `<image alias="…">description</image>` text block with which Vibe's
/// describer replaces the pixels. The description is meant for the model,
/// never for the user's screen.
pub struct ImageBlock {
    pub alias: String,
    pub description: String,
}

/// Split a mirrored user message into the text the user sees and Vibe's
/// `<image>` description blocks. The visible part stays borrowed when the
/// message holds no block (the common case); an unclosed `<image …>` (the
/// description is still streaming) takes the rest of the message as its
/// description, so no pixel description ever shows up as message text.
pub fn split_image_blocks(text: &str) -> (std::borrow::Cow<'_, str>, Vec<ImageBlock>) {
    let mut visible = String::new();
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<image ") {
        let (before, from_tag) = rest.split_at(start);
        visible.push_str(before);
        let after_tag = match from_tag.find('>') {
            Some(pos) => &from_tag[pos + 1..],
            None => "",
        };
        let tag = &from_tag[..from_tag.len() - after_tag.len()];
        let alias = tag
            .find("alias=\"")
            .and_then(|p| tag[p + 7..].find('"').map(|e| &tag[p + 7..p + 7 + e]))
            .unwrap_or("image")
            .to_string();
        let (description, tail) = match after_tag.find("</image>") {
            Some(end) => (&after_tag[..end], &after_tag[end + 8..]),
            None => (after_tag, ""),
        };
        blocks.push(ImageBlock {
            alias,
            description: description.trim().to_string(),
        });
        rest = tail;
    }
    visible.push_str(rest);
    if blocks.is_empty() {
        return (std::borrow::Cow::Borrowed(text), blocks);
    }
    (std::borrow::Cow::Owned(visible.trim().to_string()), blocks)
}

/// A user message without Vibe's `<image>` description blocks (just the
/// part before them).
pub fn without_image_blocks(text: &str) -> std::borrow::Cow<'_, str> {
    split_image_blocks(text).0
}

/// A session title for display. Vibe titles a session with the start of its
/// first message, which can begin with our hint: that part is dropped, and a
/// title that was only the hint falls back to `default_title`.
pub fn display_title(title: &str, default_title: &str) -> String {
    let cleaned = without_ui_hint(title);
    let stem = cleaned.trim_end_matches('…');
    if cleaned.trim().is_empty() || UI_HINT.starts_with(stem) {
        default_title.to_string()
    } else {
        cleaned.to_string()
    }
}

/// Sidebar title derived from one user message: the first line, capped at
/// `SIDEBAR_TITLE_MAX` chars with an ellipsis, without the `[Catwright]`
/// hint and without Vibe's image descriptions. `None` when nothing remains.
fn derive_title(text: &str) -> Option<String> {
    let visible = without_image_blocks(text);
    let t = visible.trim();
    let t = strip_hint(t).map(str::trim).unwrap_or(t);
    if t.is_empty() {
        return None;
    }
    let first_line = t.lines().next().unwrap_or_default();
    if first_line.chars().count() <= SIDEBAR_TITLE_MAX {
        Some(first_line.to_string())
    } else {
        let mut cut: String = first_line.chars().take(SIDEBAR_TITLE_MAX).collect();
        cut.push('…');
        Some(cut)
    }
}

// ---------------------------------------------------------------------------
// workspaces (pure helpers)
// ---------------------------------------------------------------------------

/// The workspace among `candidates` whose newest listed session was updated
/// last (`updatedAt` of `session/list`; ISO-8601, so string order is time
/// order): where a new session starts until the user picks a workspace.
/// On a tie the earlier candidate wins (sidebar order). `None` when no
/// candidate has a session with a timestamp yet.
pub fn last_used_workspace(
    listed: &HashMap<PathBuf, Vec<SessionInfo>>,
    candidates: &[PathBuf],
) -> Option<PathBuf> {
    let mut best: Option<(&str, &PathBuf)> = None;
    for dir in candidates {
        let newest = listed
            .get(dir)
            .into_iter()
            .flatten()
            .filter_map(|s| s.updated_at.as_deref())
            .max();
        if let Some(at) = newest {
            if best.is_none_or(|(b, _)| at > b) {
                best = Some((at, dir));
            }
        }
    }
    best.map(|(_, dir)| dir.clone())
}

/// Whether the workspace `dir` has permission requests waiting for the
/// user's answer (Handoff 2.2), so the sidebar can mark its row even while
/// it is folded: a request in one of its sessions, or a request orphaned
/// because its session is unknown but was spawned by one of its sessions
/// (a subagent's `child-…` id). An orphan without such a parent counts
/// for no workspace (the active session shows it as an orphan card).
pub fn workspace_waiting_permission(
    sessions: &HashMap<String, SessionView>,
    orphans: &[PendingPermission],
    dir: &Path,
) -> bool {
    sessions.values().filter(|v| v.cwd == dir).any(|v| {
        let child_of_it = |p: &PendingPermission| {
            v.subagents()
                .iter()
                .any(|a| a.child_session_id.as_deref() == Some(p.request.session_id.as_str()))
        };
        !v.permissions.is_empty() || orphans.iter().any(child_of_it)
    })
}

// ---------------------------------------------------------------------------
// tests (offline; the test itself plays the agent over a UnixStream pair)
// ---------------------------------------------------------------------------

/// The option values of the select config option `id` (empty if absent).
fn select_levels(options: &[ConfigOption], id: &str) -> Vec<String> {
    options
        .iter()
        .find_map(|o| match o {
            ConfigOption::Select {
                id: oid, options, ..
            } if oid == id => Some(options.iter().map(|x| x.value.clone()).collect()),
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
