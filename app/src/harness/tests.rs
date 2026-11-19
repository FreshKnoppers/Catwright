/// One view over every workspace (was: each of the two views kept its
/// own active session and project). The chat workspace is "Allgemein",
/// pinned before the projects; opening a session of any workspace shows
/// it and makes its workspace the one new sessions start in.
#[test]
fn opening_a_session_moves_to_its_workspace() {
    let proj = std::env::temp_dir().canonicalize().unwrap();
    let chat = proj.join(format!("od-chat-{}", std::process::id()));
    let (mut h, mut agent) = setup(vec![proj.clone()], "ask");
    h.set_chat_dir(chat.clone());
    assert!(chat.is_dir());
    let chat = chat.canonicalize().unwrap();
    assert_eq!(h.workspaces(), vec![chat.clone(), proj.clone()]);
    assert_eq!(h.workspace_name(&chat), "General");
    assert_eq!(tr_in(Lang::De, "General"), "Allgemein");
    assert_eq!(h.workspace_name(&proj), project_name(&proj));
    assert_eq!(
        h.current_project.as_deref(),
        Some(proj.as_path()),
        "with a project, the first one is preselected"
    );

    h.open_session(chat.clone(), "chat-session");
    assert_eq!(agent.read_msg()["method"], "session/load");
    assert_eq!(h.active.as_deref(), Some("chat-session"));
    assert_eq!(h.current_project.as_deref(), Some(chat.as_path()));
    h.open_session(proj.clone(), "code-session");
    assert_eq!(agent.read_msg()["method"], "session/load");
    assert_eq!(h.active.as_deref(), Some("code-session"));
    assert_eq!(h.current_project.as_deref(), Some(proj.as_path()));
    // both stay open side by side: going back needs no reload
    assert!(h.sessions.contains_key("chat-session"));
    h.open_session(chat.clone(), "chat-session");
    assert_eq!(h.active.as_deref(), Some("chat-session"));
    assert_eq!(h.current_project.as_deref(), Some(chat.as_path()));
    let _ = std::fs::remove_dir_all(&chat);
}

use super::*;
use crate::acp::client::{AcpClient, SessionOrigin};
use crate::acp::protocol as proto;
use crate::acp::session::repeated_tool_call;
use crate::harness::state::Dictation;
use crate::i18n::{tr_in, Lang};
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const RECV: Duration = Duration::from_secs(5);

/// The test's end of the wire ("the agent"): only reads what the client
/// sends; replies are injected via `handle_event`.
struct Agent {
    reader: BufReader<UnixStream>,
}

impl Agent {
    fn read_msg(&mut self) -> Value {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("agent read");
        assert!(n > 0, "agent read hit EOF");
        serde_json::from_str(line.trim()).expect("agent line is JSON")
    }
}

fn setup(projects: Vec<PathBuf>, start_mode: &str) -> (Harness, Agent) {
    let (a, b) = UnixStream::pair().expect("socket pair");
    a.set_read_timeout(Some(RECV)).unwrap();
    b.set_read_timeout(Some(RECV)).unwrap();
    let (client, rx) = AcpClient::from_streams(
        BufReader::new(a.try_clone().unwrap()),
        a.try_clone().unwrap(),
        None,
    );
    let mut h = Harness::new(projects, start_mode.to_string());
    h.attach(client, rx);
    let agent = Agent {
        reader: BufReader::new(b),
    };
    (h, agent)
}

fn new_session_result(sid: &str) -> proto::NewSessionResult {
    serde_json::from_value(json!({"sessionId": sid})).unwrap()
}

/// session/new result that reports a ModeState (like the real vibe-acp).
fn new_session_result_with_modes(sid: &str, current_mode_id: &str) -> proto::NewSessionResult {
    serde_json::from_value(json!({
        "sessionId": sid,
        "modes": {
            "currentModeId": current_mode_id,
            "availableModes": [
                {"id": "ask", "name": "Ask"},
                {"id": "accept-edits", "name": "Accept Edits"},
                {"id": "smart-approve", "name": "Smart Approve"},
                {"id": "auto-approve", "name": "Auto Approve"}
            ]
        }
    }))
    .unwrap()
}

/// session/new result whose mode list also contains the user's
/// `claude-delegate` profiles, as the real vibe-acp reports them
/// (see tests/fixtures/acp/subagent_session.jsonl).
fn new_session_result_with_delegate_modes(
    sid: &str,
    current_mode_id: &str,
) -> proto::NewSessionResult {
    serde_json::from_value(json!({
        "sessionId": sid,
        "modes": {
            "currentModeId": current_mode_id,
            "availableModes": [
                {"id": "ask", "name": "Ask"},
                {"id": "accept-edits", "name": "Accept Edits"},
                {"id": "claude-delegate", "name": "Claude Delegate"},
                {"id": "claude-delegate-rust", "name": "Claude Delegate (Rust)"}
            ]
        }
    }))
    .unwrap()
}

fn permission_request(sid: &str) -> proto::PermissionRequest {
    serde_json::from_value(json!({
        "sessionId": sid,
        "toolCall": {"toolCallId": "t1"},
        "options": [
            {"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"},
            {"optionId": "reject_once", "name": "Deny", "kind": "reject_once"}
        ]
    }))
    .unwrap()
}

/// `user_message_chunk` (the echo of a prompt) with its own message id,
/// as the real vibe-acp sends it.
fn user_chunk(msg_id: &str, text: &str) -> proto::SessionUpdate {
    serde_json::from_value(json!({
        "sessionUpdate": "user_message_chunk",
        "content": {"type": "text", "text": text},
        "messageId": msg_id
    }))
    .unwrap()
}

/// `agent_message_chunk` with its own message id.
fn agent_chunk(msg_id: &str, text: &str) -> proto::SessionUpdate {
    serde_json::from_value(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": text},
        "messageId": msg_id
    }))
    .unwrap()
}

/// `session_info_update` with a title.
fn title_update(title: &str) -> proto::SessionUpdate {
    serde_json::from_value(json!({
        "sessionUpdate": "session_info_update",
        "title": title
    }))
    .unwrap()
}

/// A `session/list` answer for the sessions of one folder.
fn sessions_listed(sessions: Value) -> proto::ListSessionsResult {
    serde_json::from_value(json!({"sessions": sessions})).unwrap()
}

fn create_session(h: &mut Harness, agent: &mut Agent, cwd: &Path, sid: &str) {
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: cwd.to_path_buf(),
        result: new_session_result(sid),
    });
    // the harness refreshes the sidebar list after session/new
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
}

#[test]
fn send_without_session_queues_then_prompts_after_create() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    assert!(h.draft.is_empty(), "draft is cleared after send");

    // session/new went out with the project as cwd
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");
    assert_eq!(msg["params"]["cwd"], json!(dir.to_string_lossy()));
    assert!(
        h.queued.is_some(),
        "prompt is queued until session/new answers"
    );
    assert!(h.active.is_none());

    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result("s1"),
    });
    // first the queued prompt, then the sidebar refresh
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "Hallo Vibe"})
    );
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");

    let s = h.sessions.get("s1").expect("session view exists");
    assert_eq!(s.turn, TurnState::Running);
    assert_eq!(h.active.as_deref(), Some("s1"));
    assert!(h.queued.is_none());
}

#[test]
fn start_model_and_thinking_are_set_before_the_first_prompt() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.start_model = Some("deepseek-v4-1-flash".into());
    h.start_thinking = Some("low".into());
    h.draft = "Hallo".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    let result: proto::NewSessionResult = serde_json::from_value(json!({
            "sessionId": "s1",
            "modes": {"currentModeId": "ask", "availableModes": [{"id": "ask", "name": "Ask"}]},
            "configOptions": [
                {"id": "model", "name": "Model", "type": "select", "currentValue": "mistral-large-4-0-3",
                 "options": [{"value": "mistral-large-4-0-3", "name": "Mistral Large 4"},
                             {"value": "deepseek-v4-1-flash", "name": "DeepSeek V4.1 Flash"}]},
                {"id": "thinking", "name": "Thinking", "type": "select", "currentValue": "high",
                 "options": [{"value": "low", "name": "Low"}, {"value": "medium", "name": "Medium"},
                             {"value": "high", "name": "High"}]}
            ]
        }))
        .unwrap();
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result,
    });
    // mode already right: model first …
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/set_config_option");
    assert_eq!(msg["params"]["configId"], json!("model"));
    assert_eq!(msg["params"]["value"], json!("deepseek-v4-1-flash"));
    assert_eq!(agent.read_msg()["method"], "session/list");
    assert_eq!(
        h.known_models.len(),
        2,
        "models are remembered for the start screen"
    );
    assert_eq!(h.known_thinking, vec!["low", "medium", "high"]);
    // … then thinking …
    h.handle_event(AcpEvent::ConfigOptionsChanged {
        session_id: "s1".into(),
        options: vec![],
    });
    let msg = agent.read_msg();
    assert_eq!(msg["params"]["configId"], json!("thinking"));
    assert_eq!(msg["params"]["value"], json!("low"));
    // … then the held prompt
    h.handle_event(AcpEvent::ConfigOptionsChanged {
        session_id: "s1".into(),
        options: vec![],
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["prompt"][0]["text"], json!("Hallo"));
}

/// A `usage_update` notification as the real vibe-acp sends it.
fn usage_update(used: u64, size: u64) -> proto::SessionUpdate {
    serde_json::from_value(json!({
        "sessionUpdate": "usage_update",
        "used": used,
        "size": size,
    }))
    .unwrap()
}

/// configOptions with a `model` select in state `current` (ACP carries
/// the full list; the model is the option that matters here).
fn model_options(current: &str) -> Vec<proto::ConfigOption> {
    serde_json::from_value(json!([{
        "id": "model", "name": "Model", "type": "select", "currentValue": current,
        "options": [
            {"value": "mistral-large-4-0-3", "name": "Mistral Large 4"},
            {"value": "deepseek-v4-1-flash", "name": "DeepSeek V4.1 Flash"},
            {"value": "mistral-medium-3.5", "name": "Mistral Medium 3.5"}
        ]
    }]))
    .unwrap()
}

/// A session mid-conversation: model `current`, window measured by a
/// usage_update of `used`/`size` tokens.
fn session_with_usage(h: &mut Harness, cwd: &Path, model: &str, used: u64, size: u64) {
    let mut view = SessionView::new("s1", cwd.to_path_buf());
    view.set_session_info(None, model_options(model));
    view.apply_update(&usage_update(used, size));
    h.sessions.insert("s1".to_string(), view);
}

/// The size Vibe reports is its auto-compaction threshold, which only Vibe
/// knows (verified against Vibe 3.1.2: Large 4 400k, DeepSeek V4.1 Flash
/// 256k, while config.toml says 1048576 for DeepSeek). A model switch
/// carries no usage_update: the stale size goes ("–") and `session/resume`
/// fetches Vibe's own new value.
#[test]
fn model_switch_clears_the_size_and_asks_vibe_for_the_new_one() {
    let cwd = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![], "ask");
    session_with_usage(&mut h, &cwd, "mistral-large-4-0-3", 50_000, 400_000);
    h.handle_event(AcpEvent::ConfigOptionsChanged {
        session_id: "s1".into(),
        options: model_options("deepseek-v4-1-flash"),
    });
    let v = h.sessions.get("s1").expect("session still exists");
    assert!(
        v.usage.is_none(),
        "the old model's size is cleared, not kept"
    );
    assert_eq!(v.current_config_value("model"), Some("deepseek-v4-1-flash"));
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/resume");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(msg["params"]["cwd"], json!(cwd.to_string_lossy()));
    // Vibe answers with a usage_update: same used, the new threshold
    h.handle_event(AcpEvent::Update {
        session_id: "s1".into(),
        update: usage_update(50_000, 256_000),
    });
    let u = h
        .sessions
        .get("s1")
        .unwrap()
        .usage
        .as_ref()
        .expect("Vibe's value");
    assert_eq!((u.used, u.size), (50_000, 256_000));
    h.handle_event(AcpEvent::SessionResumed);
    assert!(h.sessions.get("s1").unwrap().usage.is_some());
}

#[test]
fn model_switch_without_exact_value_shows_the_dash_until_usage_update() {
    let cwd = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![], "ask");
    // until Vibe sends its value, nothing exact is known
    session_with_usage(&mut h, &cwd, "mistral-large-4-0-3", 50_000, 800_000);
    h.handle_event(AcpEvent::ConfigOptionsChanged {
        session_id: "s1".into(),
        options: model_options("mistral-medium-3.5"),
    });
    assert!(
        h.sessions.get("s1").unwrap().usage.is_none(),
        "stale 800k window is cleared, not guessed"
    );
    // the next usage_update is authoritative again
    h.handle_event(AcpEvent::Update {
        session_id: "s1".into(),
        update: usage_update(120_000, 262_144),
    });
    let u = h
        .sessions
        .get("s1")
        .unwrap()
        .usage
        .as_ref()
        .expect("usage_update restores a real number");
    assert_eq!(u.used, 120_000);
    assert_eq!(u.size, 262_144);
}

/// A failed `session/resume` (Vibe does not advertise it) is no error for
/// the user: the size stays "–" until the turn's usage_update, and switching
/// back to Large 4 takes Vibe's value again, never a builtin.
#[test]
fn failed_resume_stays_quiet_and_the_next_usage_update_counts() {
    let cwd = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![], "ask");
    session_with_usage(&mut h, &cwd, "deepseek-v4-1-flash", 50_000, 256_000);
    h.handle_event(AcpEvent::ConfigOptionsChanged {
        session_id: "s1".into(),
        options: model_options("mistral-large-4-0-3"),
    });
    assert_eq!(agent.read_msg()["method"], "session/resume");
    let status_before = h.status.clone();
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::ResumeSession {
            session_id: "s1".into(),
        },
        error: rpc::RpcError {
            code: -32601,
            message: "Method not found".into(),
            data: None,
        },
    });
    assert_eq!(h.status, status_before, "no error shown for the resume");
    assert!(h.sessions.get("s1").unwrap().usage.is_none());
    h.handle_event(AcpEvent::Update {
        session_id: "s1".into(),
        update: usage_update(52_000, 400_000),
    });
    let u = h
        .sessions
        .get("s1")
        .unwrap()
        .usage
        .as_ref()
        .expect("Vibe's value");
    assert_eq!((u.used, u.size), (52_000, 400_000));
}

#[test]
fn config_change_without_model_switch_keeps_the_usage() {
    let cwd = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![], "ask");
    // a thinking switch sends ConfigOptionsChanged as well; the model
    // is unchanged, so the measured usage must survive untouched
    session_with_usage(&mut h, &cwd, "mistral-medium-3.5", 50_000, 262_144);
    h.handle_event(AcpEvent::ConfigOptionsChanged {
        session_id: "s1".into(),
        options: model_options("mistral-medium-3.5"),
    });
    let u = h
        .sessions
        .get("s1")
        .unwrap()
        .usage
        .as_ref()
        .expect("unchanged model keeps its usage");
    assert_eq!(u.used, 50_000);
    assert_eq!(u.size, 262_144);
}

fn exited() -> AcpEvent {
    AcpEvent::Exited {
        code: Some(1),
        stderr_tail: "boom".into(),
    }
}

#[test]
fn lost_connection_reconnects_and_reloads_the_open_session() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.open_session(dir.clone(), "s1");
    assert_eq!(agent.read_msg()["method"], "session/load");
    h.handle_event(exited());
    assert!(matches!(h.conn, Conn::Reconnecting { attempt: 1, .. }));
    assert!(h.client.is_none() && h.sessions.is_empty() && h.active.is_none());
    assert!(h.status.contains("interrupted"));

    // the restarted agent: initialize done -> the session is loaded again
    let (a, b) = UnixStream::pair().expect("socket pair");
    a.set_read_timeout(Some(RECV)).unwrap();
    b.set_read_timeout(Some(RECV)).unwrap();
    let (client, rx) = AcpClient::from_streams(
        BufReader::new(a.try_clone().unwrap()),
        a.try_clone().unwrap(),
        None,
    );
    h.attach(client, rx);
    let mut agent2 = Agent {
        reader: BufReader::new(b),
    };
    let init: proto::InitializeResult = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": {"loadSession": true}
    }))
    .unwrap();
    h.handle_event(AcpEvent::Initialized(init));
    assert!(matches!(h.conn, Conn::Ready { .. }));
    let msg = agent2.read_msg();
    assert_eq!(msg["method"], "session/load");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(h.active.as_deref(), Some("s1"));
}

/// A fresh client for the restarted agent, made ready (`initialize`
/// answered): the test's end of its wire.
fn restart_agent(h: &mut Harness) -> Agent {
    let (a, b) = UnixStream::pair().expect("socket pair");
    a.set_read_timeout(Some(RECV)).unwrap();
    b.set_read_timeout(Some(RECV)).unwrap();
    let (client, rx) = AcpClient::from_streams(
        BufReader::new(a.try_clone().unwrap()),
        a.try_clone().unwrap(),
        None,
    );
    h.attach(client, rx);
    let init: proto::InitializeResult = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": {"loadSession": true}
    }))
    .unwrap();
    let agent = Agent {
        reader: BufReader::new(b),
    };
    h.handle_event(AcpEvent::Initialized(init));
    agent
}

/// A session clicked while `vibe-acp` restarts gets no empty view
/// (nothing could load it): the click is remembered, its row selected,
/// and the session opens once the agent is ready — instead of the one
/// that was on screen before. "Neue Sitzung" meanwhile drops the
/// pending open: the user's latest choice wins.
#[test]
fn session_clicked_during_a_restart_opens_once_vibe_is_ready() {
    let dir = std::env::temp_dir().canonicalize().unwrap();
    let other = dir.join(format!("od-restart-{}", std::process::id()));
    std::fs::create_dir_all(&other).unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone(), other.clone()], "ask");
    h.open_session(dir.clone(), "s1");
    assert_eq!(agent.read_msg()["method"], "session/load");
    h.handle_event(exited());
    assert!(h.client.is_none() && h.sessions.is_empty());

    // the click: no view, nothing sent, but selected
    h.open_session(other.clone(), "s2");
    assert!(!h.sessions.contains_key("s2"), "no empty view");
    assert_eq!(h.active.as_deref(), Some("s2"));
    assert_eq!(h.current_project.as_deref(), Some(other.as_path()));

    // ready: the clicked session loads, not the one on screen before
    let mut agent2 = restart_agent(&mut h);
    let msg = agent2.read_msg();
    assert_eq!(msg["method"], "session/load");
    assert_eq!(msg["params"]["sessionId"], json!("s2"));
    assert_eq!(msg["params"]["cwd"], json!(other.to_string_lossy()));
    assert!(h.sessions.contains_key("s2"));
    assert_eq!(h.active.as_deref(), Some("s2"));
    while agent2.read_msg()["method"] != "session/list" {}

    // another restart; this time the user goes to "Neue Sitzung"
    h.handle_event(exited());
    h.new_session_view();
    let mut agent3 = restart_agent(&mut h);
    // the probe would come first if the chat workspace were set; here
    // the first message is a sidebar refresh, no session/load
    assert_eq!(agent3.read_msg()["method"], "session/list");
    assert!(h.active.is_none(), "the start screen stays");
    assert!(h.sessions.is_empty());
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn held_first_prompt_returns_to_the_composer() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".into();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    assert!(h.draft.is_empty());
    h.handle_event(exited());
    assert_eq!(h.draft, "Hallo Vibe");
    assert!(h.status.contains("input field"));
}

/// Thinking levels are per model in Vibe: `max` picked for one model must
/// not be sent to a model that only takes low/medium/high (Vibe rejects it).
#[test]
fn start_thinking_follows_the_chosen_model() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    let lmh: Vec<String> = ["low", "medium", "high"].map(String::from).to_vec();
    // the start picker: choosing the model fits the level right away
    h.thinking_by_model
        .insert("mistral-large-4".into(), (lmh.clone(), "medium".into()));
    h.start_thinking = Some("max".into());
    h.choose_start_model("mistral-large-4".into());
    assert_eq!(h.start_thinking.as_deref(), Some("high"));
    assert_eq!(h.thinking_levels_for(Some("mistral-large-4")), lmh);

    // a model never seen: the level is fitted when the session switches
    h.thinking_by_model.clear();
    h.start_thinking = Some("max".into());
    h.choose_start_model("mistral-large-4".into());
    assert_eq!(h.start_thinking.as_deref(), Some("max"));
    h.current_project = Some(dir.clone());
    h.draft = "Hallo".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    let result: proto::NewSessionResult = serde_json::from_value(json!({
        "sessionId": "s1",
        "modes": {"currentModeId": "ask", "availableModes": [{"id": "ask", "name": "Ask"}]},
        "configOptions": [
            {"id": "model", "name": "Model", "type": "select", "currentValue": "other",
             "options": [{"value": "other", "name": "Other"},
                         {"value": "mistral-large-4", "name": "Mistral Large 4"}]},
            {"id": "thinking", "name": "Thinking", "type": "select", "currentValue": "max",
             "options": [{"value": "low", "name": "Low"}, {"value": "high", "name": "High"},
                         {"value": "max", "name": "Max"}]}
        ]
    }))
    .unwrap();
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result,
    });
    let msg = agent.read_msg();
    assert_eq!(msg["params"]["configId"], json!("model"));
    assert_eq!(agent.read_msg()["method"], "session/list");
    let options: Vec<ConfigOption> = serde_json::from_value(json!([
        {"id": "model", "name": "Model", "type": "select", "currentValue": "mistral-large-4",
         "options": [{"value": "other", "name": "Other"},
                     {"value": "mistral-large-4", "name": "Mistral Large 4"}]},
        {"id": "thinking", "name": "Thinking", "type": "select", "currentValue": "medium",
         "options": [{"value": "low", "name": "Low"}, {"value": "medium", "name": "Medium"},
                     {"value": "high", "name": "High"}]}
    ]))
    .unwrap();
    h.handle_event(AcpEvent::ConfigOptionsChanged {
        session_id: "s1".into(),
        options,
    });
    // the model switch first asks Vibe for the new context size …
    assert_eq!(agent.read_msg()["method"], "session/resume");
    // … then the held setup goes on with the thinking level
    let msg = agent.read_msg();
    assert_eq!(msg["params"]["configId"], json!("thinking"));
    assert_eq!(
        msg["params"]["value"],
        json!("high"),
        "max is fitted, not sent"
    );
    assert_eq!(
        h.thinking_by_model.get("mistral-large-4"),
        Some(&(lmh, "medium".to_string())),
        "the new model's levels are remembered"
    );
}

#[test]
fn gives_up_after_max_reconnects_and_retries_on_request() {
    let (mut h, _agent) = setup(vec![], "ask");
    for i in 1..=MAX_RECONNECTS {
        h.handle_event(exited());
        assert!(matches!(h.conn, Conn::Reconnecting { attempt, .. } if attempt == i));
    }
    h.handle_event(exited());
    assert!(matches!(&h.conn, Conn::Failed(m) if m.contains("boom")));
    h.retry_connection();
    assert_eq!(h.conn, Conn::Off);
    h.handle_event(exited());
    assert!(matches!(h.conn, Conn::Reconnecting { attempt: 1, .. }));
}

#[test]
fn a_mute_agent_counts_as_lost_after_the_start_timeout() {
    let (mut h, _agent) = setup(vec![], "ask");
    assert!(matches!(h.conn, Conn::Starting));
    // within the budget the start just continues
    h.poll();
    assert!(matches!(h.conn, Conn::Starting));
    // the watchdog decision is pure so the clock can move in tests
    let now = Instant::now();
    assert!(!Harness::starting_expired(None, now));
    assert!(!Harness::starting_expired(Some(now), now));
    assert!(Harness::starting_expired(Some(now - START_TIMEOUT), now));
    // an expired start kills the silent child and feeds the reconnect
    h.starting_since = Some(Instant::now() - START_TIMEOUT);
    h.poll();
    assert!(matches!(h.conn, Conn::Reconnecting { attempt: 1, .. }));
    assert_eq!(h.starting_since, None);
}

#[test]
fn probe_session_only_teaches_choices_and_defaults() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![], "ask");
    let result: proto::NewSessionResult = serde_json::from_value(json!({
            "sessionId": "probe",
            "configOptions": [
                {"id": "model", "name": "Model", "type": "select", "currentValue": "mistral-large-4-0-3",
                 "options": [{"value": "mistral-large-4-0-3", "name": "Mistral Large 4"}]},
                {"id": "thinking", "name": "Thinking", "type": "select", "currentValue": "medium",
                 "options": [{"value": "low", "name": "Low"}, {"value": "medium", "name": "Medium"},
                             {"value": "high", "name": "High"}]}
            ]
        }))
        .unwrap();
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::Probe,
        cwd: dir.clone(),
        result,
    });
    assert_eq!(h.default_model.as_deref(), Some("mistral-large-4-0-3"));
    assert_eq!(h.default_thinking.as_deref(), Some("medium"));
    assert_eq!(h.known_thinking, vec!["low", "medium", "high"]);
    assert!(h.active.is_none(), "the probe never becomes a visible chat");
    assert!(!h.sessions.contains_key("probe"));
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/close");
    assert_eq!(msg["params"]["sessionId"], json!("probe"));
}

/// Start race: the probe's session/new and the user's own are in flight
/// for the same folder. Each answer carries its origin, so the probe's
/// empty session is closed and the user's becomes the conversation the
/// message goes into — whatever the arrival order.
#[test]
fn probe_race_keeps_the_message_in_the_real_session() {
    let dir = std::env::temp_dir().join(format!("od-race-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let chat = dir.canonicalize().unwrap();
    let (mut h, mut agent) = setup(vec![], "ask");
    h.set_chat_dir(chat.clone());
    h.select_workspace(chat.clone());
    // the probe is out, the user types before it answers
    h.draft = "Hallo Vibe".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");

    // the PROBE answers first: closed, no empty view over the
    // conversation the message is waiting for
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::Probe,
        cwd: chat.clone(),
        result: new_session_result("probe"),
    });
    assert!(!h.sessions.contains_key("probe"));
    assert!(h.active.is_none());
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/close");
    assert_eq!(msg["params"]["sessionId"], json!("probe"));

    // the user's own session/new answers: the message goes into it
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: chat.clone(),
        result: new_session_result("s1"),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({ "type": "text", "text": "Hallo Vibe" })
    );
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
    assert_eq!(h.active.as_deref(), Some("s1"));
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A failed `session/new` (e.g. Vibe not logged in) puts the held
/// message back into the composer instead of losing it.
#[test]
fn failed_new_session_restores_the_queued_prompt_to_the_draft() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    assert!(h.draft.is_empty());
    assert!(h.queued.is_some());

    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::NewSession {
            cwd: dir.clone(),
            probe: false,
        },
        error: rpc::RpcError {
            code: rpc::AUTH_REQUIRED,
            message: "authentication required".to_string(),
            data: None,
        },
    });
    assert_eq!(h.draft, "Hallo Vibe", "the text goes back into the draft");
    assert!(h.queued.is_none());
    assert!(h.active.is_none(), "no empty view was ever created");
    assert!(h.status.contains("signed in"), "the failure is shown");
    assert!(
        h.status.contains("input field"),
        "the user is told the message is back"
    );
}

/// The probe's own session/new must never eat a waiting user prompt
/// (both request the same cwd — only the probe marker tells them apart).
#[test]
fn failed_probe_new_session_keeps_the_queued_prompt() {
    let dir = std::env::temp_dir().join(format!("od-probe-fail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let chat = dir.canonicalize().unwrap();
    let (mut h, mut agent) = setup(vec![], "ask");
    h.set_chat_dir(chat.clone());
    // boot: initialize opens the probe session for the chat workspace
    let init: proto::InitializeResult = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": {"loadSession": true}
    }))
    .unwrap();
    h.handle_event(AcpEvent::Initialized(init));
    assert_eq!(agent.read_msg()["method"], "session/new");
    assert_eq!(agent.read_msg()["method"], "session/list");

    // the user starts the chat workspace session before the probe answered
    h.select_workspace(chat.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    assert!(h.queued.is_some());

    // the PROBE fails (Vibe not logged in): the user's prompt must survive
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::NewSession {
            cwd: chat.clone(),
            probe: true,
        },
        error: rpc::RpcError {
            code: rpc::AUTH_REQUIRED,
            message: "authentication required".to_string(),
            data: None,
        },
    });
    assert!(
        h.queued.is_some(),
        "a failed probe must not delete the waiting prompt"
    );
    assert!(h.draft.is_empty());
    assert!(h.status.contains("signed in"));

    // the user's own session/new answers later: the prompt goes to it
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: chat.clone(),
        result: new_session_result("s1"),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "Hallo Vibe"})
    );
    assert_eq!(agent.read_msg()["method"], "session/list");
    assert_eq!(h.active.as_deref(), Some("s1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A failed `session/prompt` (Vibe echoes the text only on success)
/// puts the text back into the composer; a finished turn is forgotten,
/// so no later failure can bring it back.
#[test]
fn failed_prompt_restores_the_text_to_the_draft() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");

    h.draft = "Schreib bitte eine Datei".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);
    assert!(h.draft.is_empty());

    // the agent rejects the prompt: the text comes back, the turn ends
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    assert_eq!(h.sessions["s1"].turn, TurnState::Idle);
    assert_eq!(h.draft, "Schreib bitte eine Datei");
    assert!(h.status.contains("input field"));

    // a finished turn clears the memory: a later failure without a new
    // prompt must not resurrect the old text
    h.draft = "Zweite Frage".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/prompt");
    let done: proto::PromptResult =
        serde_json::from_value(json!({"stopReason": "end_turn"})).unwrap();
    h.handle_event(AcpEvent::PromptFinished {
        session_id: "s1".to_string(),
        result: done,
    });
    // the finished turn refreshes the sidebar list (Handoff 1a)
    assert_eq!(agent.read_msg()["method"], "session/list");
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    assert!(
        h.draft.is_empty(),
        "a finished prompt must not be resurrected by later failures"
    );
}

/// M13 (review H3): a prompt of session A that fails while the user
/// watches session B may not land in B's composer. Its text parks under
/// A's id — B stays untouched, and opening A shows the text in its
/// composer. The status names the place.
#[test]
fn failed_prompt_of_a_background_session_parks_under_its_session() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "a");
    create_session(&mut h, &mut agent, &dir, "b");
    assert_eq!(h.active.as_deref(), Some("b"), "the user watches b");

    // session a runs a turn in the background; its prompt fails while
    // the user types something new in b's composer
    h.send_prompt("a", &[PromptBlock::Text("Frage A".to_string())]);
    assert_eq!(agent.read_msg()["params"]["sessionId"], json!("a"));
    h.draft = "Neu bei b".to_string();

    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "a".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    assert_eq!(
        h.draft, "Neu bei b",
        "b's composer keeps only what was typed there"
    );
    let (text, attachments) =
        h.ui.parked_drafts
            .get(&Some("a".to_string()))
            .cloned()
            .unwrap_or_default();
    assert_eq!(text, "Frage A", "the failed prompt parks under a");
    assert!(attachments.is_empty());
    assert!(
        h.status.contains("input field"),
        "the status says where the text is: {}",
        h.status
    );

    // opening a brings its parked text back into the composer
    h.open_session(dir.clone(), "a");
    ui::swap_draft(&mut h);
    assert_eq!(h.draft, "Frage A");
}

/// M13: a restore never discards anything. An occupied composer keeps
/// the fresh typing and appends the texts below it (attachments are
/// dropped, and the status says so); a parked spot merges with the text
/// parked there earlier and keeps its attachments.
#[test]
fn nothing_is_discarded_on_a_restore() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");

    // (1) occupied visible composer: the text joins below the fresh
    // typing, the attachment is dropped — openly
    h.send_prompt(
        "s1",
        &[
            PromptBlock::Text("Gescheitert".to_string()),
            PromptBlock::ResourceLink {
                uri: "file:///x.png".to_string(),
                name: "x.png".to_string(),
            },
        ],
    );
    assert_eq!(agent.read_msg()["params"]["sessionId"], json!("s1"));
    h.draft = "Neue Frage".to_string();
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    assert_eq!(
        h.draft, "Neue Frage\n\nGescheitert",
        "the fresh typing stays, nothing is dropped"
    );
    assert!(
        h.status.contains("Attachments"),
        "the lost attachment is said: {}",
        h.status
    );

    // (2) an occupied parked spot keeps its text AND its attachments
    h.active = None;
    h.draft = String::new();
    h.ui.parked_drafts.insert(
        Some("s1".to_string()),
        ("Vorhandenes".to_string(), vec![PathBuf::from("/x")]),
    );
    h.send_prompt("s1", &[PromptBlock::Text("Zweite".to_string())]);
    let _ = agent.read_msg();
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    let (text, attachments) =
        h.ui.parked_drafts
            .get(&Some("s1".to_string()))
            .cloned()
            .unwrap_or_default();
    assert_eq!(
        text, "Vorhandenes\n\nZweite",
        "an earlier parked text is merged, not overwritten"
    );
    assert_eq!(attachments, vec![PathBuf::from("/x")]);
}

/// M13: a lost connection parks every session's queued messages under
/// their own session id — two queues become two parked entries, not one
/// joined draft that waits under a foreign composer until the restart
/// reloads the same session.
#[test]
fn connection_loss_parks_queued_messages_per_session() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "a");
    create_session(&mut h, &mut agent, &dir, "b");
    // the start screen is on: both queues park under their session
    h.new_session_view();
    let queued = |text: &str| QueuedPrompt {
        text: text.to_string(),
        blocks: vec![PromptBlock::Text(text.to_string())],
    };
    h.queue.insert("a".to_string(), vec![queued("A-eins")]);
    h.queue.insert("b".to_string(), vec![queued("B-eins")]);

    h.handle_event(exited());

    assert!(
        h.draft.is_empty(),
        "no session is on screen: no text lands in the global composer"
    );
    let parked = |sid: &str| {
        h.ui.parked_drafts
            .get(&Some(sid.to_string()))
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(parked("a").0, "A-eins", "a's queue parks under a");
    assert_eq!(parked("b").0, "B-eins", "b's queue parks under b");
    assert_eq!(h.ui.parked_drafts.len(), 2, "two separate entries");
    assert!(
        h.status.contains("input field"),
        "the status says where the texts wait: {}",
        h.status
    );

    // the restart: "a" is due to be reopened (open_on_ready); once its
    // view is back, its parked text is in the composer again — the user
    // cannot lose sight of it
    h.open_when_ready(dir.clone(), "a".to_string());
    let mut agent2 = restart_agent(&mut h);
    let msg = agent2.read_msg();
    assert_eq!(msg["method"], "session/load");
    assert_eq!(msg["params"]["sessionId"], json!("a"));
    assert_eq!(h.active.as_deref(), Some("a"));
    ui::swap_draft(&mut h);
    assert_eq!(h.draft, "A-eins");
}

/// M13: the first prompt held back for a fresh session's start mode was
/// typed on the start screen. When the connection dies before it is
/// sent, the fresh session cannot reopen after the restart (Vibe
/// persists a session only with its first message), so the text goes
/// back to the start screen — parked for it while another session is on
/// screen, visible once the user returns.
#[test]
fn connection_loss_returns_a_held_first_prompt_to_the_start_screen() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    // the new session defaults to accept-edits, the start mode is ask: the
    // prompt stays held until ModeSet
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result_with_modes("s1", "accept-edits"),
    });
    assert_eq!(agent.read_msg()["method"], "session/set_mode");
    assert_eq!(agent.read_msg()["method"], "session/list");
    assert!(h.pending_mode_prompt.is_some());
    assert_eq!(h.active.as_deref(), Some("s1"), "the fresh session is open");

    h.handle_event(exited());
    let (text, _) = h.ui.parked_drafts.get(&None).cloned().unwrap_or_default();
    assert_eq!(
        text, "Hallo Vibe",
        "the held first prompt parks for the start screen, not the dead id"
    );
    assert!(
        h.status.contains("input field"),
        "the status says where the text is: {}",
        h.status
    );
    // returning to the start screen shows it in the composer again
    h.ui.draft_owner = Some("s1".to_string());
    h.new_session_view();
    ui::swap_draft(&mut h);
    assert_eq!(h.draft, "Hallo Vibe");
}

/// N3: the CLI's start prompt (`--prompt`) never overwrites a draft the
/// user typed meanwhile — the CLI prompt takes the send, the typed text
/// stays in the composer of the session it was typed in, and the status
/// says so. Both are accounted for: the CLI prompt on the wire, the
/// typing in the input field.
#[test]
fn prompt_on_ready_keeps_a_typed_draft_in_its_own_composer() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    // the user typed in s1's composer while the CLI's session was still
    // loading (the swap puts the draft into s1's ownership)
    ui::swap_draft(&mut h);
    h.draft = "Getippt".to_string();
    h.prompt_when_ready(Some("CLI-Prompt".to_string()));

    h.handle_event(AcpEvent::SessionLoaded {
        session_id: "s1".to_string(),
        result: serde_json::from_value(json!({})).unwrap(),
    });
    // the loaded session refreshes its extras, then the CLI prompt goes
    // out — as its own turn, without the typing
    for expected in ["_loops/list", "_review/state"] {
        assert_eq!(agent.read_msg()["method"], expected);
    }
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(
        msg["params"]["prompt"],
        json!([{ "type": "text", "text": "CLI-Prompt" }])
    );
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);
    // the typed draft survived — in s1's composer, not the dead global one
    assert_eq!(
        h.draft, "Getippt",
        "the CLI prompt must not overwrite the typing"
    );
    assert!(
        h.status.contains("stays in the input field"),
        "the status names where the typing is: {}",
        h.status
    );
    assert!(h.prompt_on_ready.is_none(), "the CLI prompt was consumed");
}

/// N3: the same while another session is on screen and the CLI's own
/// session loads (stale restart timing): the CLI prompt still goes out,
/// the typing is parked under the session it belongs to — found again
/// when its view reloads, with a status naming it.
#[test]
fn prompt_on_ready_parks_a_typed_draft_of_another_session() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    // the restart dropped the views: the start screen is on (active
    // None), but the composer still holds s1's leftover typing — its
    // owner has not been swapped yet
    h.active = None;
    h.ui.draft_owner = Some("s1".to_string());
    h.draft = "Getippt".to_string();
    h.prompt_when_ready(Some("CLI-Prompt".to_string()));

    let init: proto::InitializeResult = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": {"loadSession": true}
    }))
    .unwrap();
    h.handle_event(AcpEvent::Initialized(init));
    // sidebar refresh first, then the CLI prompt as a fresh session
    assert_eq!(agent.read_msg()["method"], "session/list");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");
    assert!(h.queued.is_some(), "the CLI prompt waits for the session");
    // the typing parked under its own session — visible once s1 opens
    let (text, _) =
        h.ui.parked_drafts
            .get(&Some("s1".to_string()))
            .cloned()
            .unwrap_or_default();
    assert_eq!(text, "Getippt");
    assert!(
        h.status.contains("input field of session"),
        "the status names the parked spot: {}",
        h.status
    );
}

/// N3: `connection_lost` takes the CLI prompt with it — after the
/// restart it would fire out of the blue, hours later, into whatever
/// session happens to be loaded then.
#[test]
fn connection_lost_drops_the_waiting_cli_prompt() {
    let (mut h, _agent) = setup(vec![], "ask");
    h.prompt_when_ready(Some("spaeter".to_string()));
    assert!(h.prompt_on_ready.is_some());
    h.handle_event(exited());
    assert!(
        h.prompt_on_ready.is_none(),
        "the CLI prompt goes with the process"
    );
}

/// M5: a failed prompt ends the turn like PromptFinished would — its
/// pending approvals nobody can answer anymore expire instead of keeping
/// the session_state \"Waiting\" forever, and the user is told.
#[test]
fn failed_prompt_expires_waiting_approvals() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });
    assert_eq!(
        h.session_state("s1"),
        SessionState::Waiting,
        "the approval blocks the turn"
    );

    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    assert_eq!(
        h.session_state("s1"),
        SessionState::Failed,
        "the dead request leaves no waiting behind — the session is marked failed"
    );
    assert!(h.sessions["s1"].permissions.is_empty());
    assert!(
        h.status.contains("Open approvals were discarded"),
        "the expiry is said, like on connection_lost: {}",
        h.status
    );
}

/// M5: after a failed prompt the session queue must not fire on its own
/// — the queued texts come back with the sent one, and nothing new goes
/// over the wire until the user sends again.
#[test]
fn failed_prompt_returns_the_queue_without_resending() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.send_prompt("s1", &[PromptBlock::Text("Erste".to_string())]);
    let _ = agent.read_msg();
    let queued = |text: &str| QueuedPrompt {
        text: text.to_string(),
        blocks: vec![PromptBlock::Text(text.to_string())],
    };
    h.queue
        .insert("s1".to_string(), vec![queued("B"), queued("C")]);

    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    assert_eq!(
        h.draft, "Erste\n\nB\n\nC",
        "the sent text and the queue come back in typing order"
    );
    assert!(!h.queue.contains_key("s1"), "the queue is disarmed");
    assert!(
        h.turn_started.is_empty() && h.sent_prompts.is_empty(),
        "no new prompt was sent — send_next_queued must not fire"
    );
}

/// Handoff 1a: right after session/new (before the first message) Vibe
/// has not persisted the session, so `listed` stays empty. The sidebar
/// shows the visible session anyway: placeholder first, then the title
/// derived from the echoed message, then Vibe's own title.
#[test]
fn new_session_shows_in_the_sidebar_before_vibe_lists_it() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Was ist Rust? Bitte kurz.".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result("s1"),
    });
    assert_eq!(agent.read_msg()["method"], "session/prompt");
    assert_eq!(agent.read_msg()["method"], "session/list");

    // session/list answers nothing (Vibe lists no session without
    // messages): the session is missing from `listed` …
    assert!(
        h.listed.get(&dir).is_none_or(Vec::is_empty),
        "a fresh session is not listed before its first message"
    );

    // … but the sidebar shows the active session at the top. No title
    // and no echoed message yet: the German placeholder.
    let rows = h.sidebar_rows(&dir, "Neue Sitzung");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, "s1");
    assert_eq!(rows[0].title, "Neue Sitzung");

    // the echo of the prompt arrives: the row derives its title from it
    h.handle_event(AcpEvent::Update {
        session_id: "s1".to_string(),
        update: user_chunk("m1", "Was ist Rust? Bitte kurz."),
    });
    let rows = h.sidebar_rows(&dir, "Neue Sitzung");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Was ist Rust? Bitte kurz.");

    // Vibe sends its own title: it wins
    h.handle_event(AcpEvent::Update {
        session_id: "s1".to_string(),
        update: title_update("Rust erklären"),
    });
    assert_eq!(
        h.sidebar_rows(&dir, "Neue Sitzung")[0].title,
        "Rust erklären"
    );
}

/// Handoff 1a: when a turn finishes, Vibe has just persisted the
/// session — the harness reloads the sidebar list (a `session/list`
/// goes out on the wire) so the fresh session and its title show up
/// without an app restart.
#[test]
fn prompt_finished_refreshes_the_session_list_for_the_cwd() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result("s1"),
    });
    assert_eq!(agent.read_msg()["method"], "session/prompt");
    assert_eq!(agent.read_msg()["method"], "session/list");

    let done: proto::PromptResult =
        serde_json::from_value(json!({"stopReason": "end_turn"})).unwrap();
    h.handle_event(AcpEvent::PromptFinished {
        session_id: "s1".to_string(),
        result: done,
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
    assert_eq!(msg["params"]["cwd"], json!(dir.to_string_lossy()));

    // the answer lands in `listed`: the active session is in it now,
    // without a duplicate fallback row
    h.handle_event(AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(dir.clone()),
        result: sessions_listed(json!([{
            "sessionId": "s1", "cwd": dir.to_string_lossy(), "title": "Hallo"
        }])),
    });
    let rows = h.sidebar_rows(&dir, "Neuer Chat");
    assert_eq!(
        rows.len(),
        1,
        "the listed active session appears exactly once"
    );
    assert_eq!(rows[0].session_id, "s1");
    assert_eq!(rows[0].title, "Hallo");
}

/// Handoff 1a: `listed` can hold a stale title; a title that arrived
/// later via `session_info_update` (`SessionView.title`) wins in the
/// sidebar.
#[test]
fn session_info_update_title_wins_over_the_listed_title() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    // the persisted list entry still carries the old title
    h.listed.insert(
        dir.clone(),
        vec![serde_json::from_value(json!({
            "sessionId": "s1", "cwd": dir.to_string_lossy(), "title": "Alter Titel"
        }))
        .unwrap()],
    );
    create_session(&mut h, &mut agent, &dir, "s1");
    // Vibe renamed the session live
    h.handle_event(AcpEvent::Update {
        session_id: "s1".to_string(),
        update: title_update("Frischer Titel"),
    });
    let rows = h.sidebar_rows(&dir, "Neuer Chat");
    assert_eq!(
        rows.len(),
        1,
        "the listed active session appears exactly once"
    );
    assert_eq!(rows[0].title, "Frischer Titel", "the listed title loses");
}

/// The pure merge rules: fallback row only for the unlisted active
/// session, no duplicates, title derived from the first user message
/// (first line only, capped with an ellipsis, without the
/// `[Catwright]` hint this harness prepends), placeholder as the
/// last resort.
#[test]
fn sidebar_entries_merge_and_derivation_rules() {
    let listed: Vec<SessionInfo> = vec![
        serde_json::from_value(json!({"sessionId": "a", "cwd": "/x", "title": "Titel A"})).unwrap(),
        serde_json::from_value(json!({"sessionId": "b", "cwd": "/x"})).unwrap(),
    ];

    // no active session: the list as it is; without a title the
    // German placeholder
    let rows = sidebar_entries(&listed, None, "Neuer Chat", &HashSet::new());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title, "Titel A");
    assert_eq!(rows[1].title, "Neuer Chat");

    // active session NOT yet listed: one fallback row on top; a long
    // multi-line message becomes its first line, capped with "…"
    let mut v = SessionView::new("c", "/x");
    v.apply_update(&user_chunk(
        "m1",
        &format!("{}\nzweite Zeile", "x".repeat(60)),
    ));
    let rows = sidebar_entries(&listed, Some(&v), "Neuer Chat", &HashSet::new());
    assert_eq!(
        rows.len(),
        3,
        "the fallback row is added, nothing duplicated"
    );
    assert_eq!(rows[0].session_id, "c");
    assert_eq!(
        rows[0].title,
        format!("{}…", "x".repeat(SIDEBAR_TITLE_MAX)),
        "capped on the first line with an ellipsis"
    );
    assert_eq!(rows[1].session_id, "a");

    // the first user message holds only the [Catwright] hint that
    // this harness prepends to a session's first prompt: it is no
    // title, the next user message counts
    let mut v = SessionView::new("c", "/x");
    v.apply_update(&user_chunk("m1", UI_HINT));
    v.apply_update(&user_chunk("m2", "Echte Frage"));
    let rows = sidebar_entries(&listed, Some(&v), "Neuer Chat", &HashSet::new());
    assert_eq!(rows[0].title, "Echte Frage");

    // hint and message echoed as one chunk: the remainder wins
    let mut v = SessionView::new("c", "/x");
    v.apply_update(&user_chunk("m1", &format!("{UI_HINT}\n\nEchte Frage")));
    let rows = sidebar_entries(&listed, Some(&v), "Neuer Chat", &HashSet::new());
    assert_eq!(rows[0].title, "Echte Frage");

    // nothing sent yet: the placeholder
    let v = SessionView::new("c", "/x");
    let rows = sidebar_entries(&listed, Some(&v), "Neuer Chat", &HashSet::new());
    assert_eq!(rows[0].title, "Neuer Chat");

    // the active session IS listed: no fallback row, its own title
    // from session_info_update beats the listed one
    let mut v = SessionView::new("b", "/x");
    v.apply_update(&title_update("Vibes Titel"));
    let rows = sidebar_entries(&listed, Some(&v), "Neuer Chat", &HashSet::new());
    assert_eq!(rows.len(), 2, "no duplicate for the listed active session");
    assert_eq!(rows[1].session_id, "b");
    assert_eq!(rows[1].title, "Vibes Titel");
}

/// One view (was: a session created for the folder of the other view
/// was parked there). A new session starts in the workspace of the
/// session opened last, and the session the message went into is the
/// one on screen once Vibe created it — in whatever workspace — so the
/// message never ends up in a session the sidebar cannot show yet.
#[test]
fn new_session_starts_in_the_workspace_of_the_last_opened_session() {
    let proj = std::env::temp_dir().join(format!("od-park-{}", std::process::id()));
    std::fs::create_dir_all(&proj).unwrap();
    let chat = proj.join("ws");
    std::fs::create_dir_all(&chat).unwrap();
    let (mut h, mut agent) = setup(vec![proj.clone()], "ask");
    h.set_chat_dir(chat.clone());
    let chat = chat.canonicalize().unwrap();

    // a chat of "Allgemein" is open …
    h.open_session(chat.clone(), "chat-open");
    assert_eq!(agent.read_msg()["method"], "session/load");
    assert_eq!(h.active.as_deref(), Some("chat-open"));

    // … "Neue Sitzung" starts in its workspace, not in the project
    h.new_session_view();
    h.draft = "Zweite Frage".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");
    assert_eq!(msg["params"]["cwd"], json!(chat.to_string_lossy()));
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: chat.clone(),
        result: new_session_result("s1"),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(agent.read_msg()["method"], "session/list");
    assert_eq!(h.active.as_deref(), Some("s1"));
    assert_eq!(h.current_project.as_deref(), Some(chat.as_path()));

    // the user picks the project and sends its first message, then
    // looks at the open chat while `session/new` is in flight
    h.select_workspace(proj.clone());
    h.new_session_view();
    h.draft = "Bitte bauen".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");
    assert_eq!(msg["params"]["cwd"], json!(proj.to_string_lossy()));
    h.open_session(chat.clone(), "chat-open"); // open already: no reload
    assert_eq!(h.active.as_deref(), Some("chat-open"));

    // the project session is created: the message goes into it and it
    // is shown (Vibe lists it only after its first turn)
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: proj.clone(),
        result: new_session_result("s2"),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s2"));
    assert_eq!(agent.read_msg()["method"], "session/list");
    assert_eq!(h.active.as_deref(), Some("s2"));
    assert_eq!(h.current_project.as_deref(), Some(proj.as_path()));
    assert!(
        h.sidebar_rows(&proj, "Neue Sitzung")
            .iter()
            .any(|r| r.session_id == "s2"),
        "the unlisted new session is in its workspace's sidebar rows"
    );
    // the chat stays open in the background
    assert!(h.sessions.contains_key("chat-open"));
    assert!(h.sessions.contains_key("s1"));
    let _ = std::fs::remove_dir_all(&proj);
}

/// The artifacts view stays open while events come in in the background
/// (was: while the app shell's per-frame same-view call ran); only
/// navigation to a session or the start screen closes it.
#[test]
fn artifacts_view_closes_only_on_navigation() {
    let dir = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    h.artifacts_open = true;
    h.poll();
    h.handle_event(AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(dir.clone()),
        result: sessions_listed(json!([{
            "sessionId": "l1", "cwd": dir.to_string_lossy(), "title": "Alt",
            "updatedAt": "2026-11-08T10:00:00Z"
        }])),
    });
    h.select_workspace(dir.clone());
    assert!(h.artifacts_open, "background work keeps the artifacts view");
    h.new_session_view();
    assert!(!h.artifacts_open, "the start screen closes it");
    h.artifacts_open = true;
    h.open_session(dir.clone(), "l1");
    assert!(!h.artifacts_open, "opening a session closes it");
}

/// A failed `session/load` (deleted session, child id) must not leave an
/// orphan empty view behind: the view is removed and the start screen
/// shows again.
#[test]
fn failed_load_leaves_no_empty_view() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.open_session(dir.clone(), "child-1");
    assert_eq!(agent.read_msg()["method"], "session/load");
    assert_eq!(h.active.as_deref(), Some("child-1"));
    assert!(h.sessions.contains_key("child-1"));

    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::LoadSession {
            session_id: "child-1".to_string(),
        },
        error: rpc::RpcError {
            code: -31002,
            message: "A subagent Session cannot be resumed independently".to_string(),
            data: None,
        },
    });
    assert!(
        !h.sessions.contains_key("child-1"),
        "the empty view is gone"
    );
    assert!(h.active.is_none(), "back to the start screen");
    assert!(!h.status.is_empty(), "the failure itself stays visible");

    // a failed background load (not the session on screen, e.g. a
    // reload) leaves no view either, and the visible session stays
    // (was: it cleared the parked view's slot)
    h.open_session(dir.clone(), "s-visible");
    assert_eq!(agent.read_msg()["method"], "session/load");
    h.load_into_view(dir.clone(), "gone-too");
    assert_eq!(agent.read_msg()["method"], "session/load");
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::LoadSession {
            session_id: "gone-too".to_string(),
        },
        error: rpc::RpcError {
            code: -31002,
            message: "session not found".to_string(),
            data: None,
        },
    });
    assert!(!h.sessions.contains_key("gone-too"));
    assert_eq!(h.active.as_deref(), Some("s-visible"));
    assert!(h.sessions.contains_key("s-visible"));
}

#[test]
fn queued_prompt_waits_for_mode_set_when_start_mode_differs() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");

    // the new session reports accept-edits (Vibe's default), but we want ask
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result_with_modes("s1", "accept-edits"),
    });
    // first session/set_mode with modeId "ask" …
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/set_mode");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(msg["params"]["modeId"], json!("ask"));
    // … and NO session/prompt: the next wire message is the sidebar refresh
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
    assert!(h.queued.is_none());
    assert!(
        h.pending_mode_prompt.is_some(),
        "prompt is held back until ModeSet"
    );
    assert_eq!(
        h.sessions["s1"].turn,
        TurnState::Idle,
        "turn only starts when the prompt goes out"
    );
    // the start screen can show the modes the session reported
    assert_eq!(h.known_modes.len(), 4);
    assert_eq!(h.known_modes[0].id, "ask");

    // ModeSet for this session releases the prompt
    h.handle_event(AcpEvent::ModeSet {
        session_id: "s1".to_string(),
        mode_id: "ask".to_string(),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "Hallo Vibe"})
    );
    assert!(h.pending_mode_prompt.is_none());
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);
    assert_eq!(
        h.sessions["s1"].modes.as_ref().unwrap().current_mode_id,
        "ask"
    );
}

#[test]
fn queued_prompt_goes_out_immediately_when_mode_already_matches() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");

    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result_with_modes("s1", "ask"),
    });
    // no set_mode detour: prompt first, then the sidebar refresh
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert!(h.pending_mode_prompt.is_none());
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);
}

#[test]
fn stored_hidden_start_mode_is_skipped_and_vibes_default_applies() {
    // settings.json may still hold a `claude-delegate*` id from before
    // the profiles were hidden in the pickers (the stored value is not
    // rewritten). It must not be applied either: no set_mode goes out,
    // so Vibe's own default (accept-edits) stays active.
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "claude-delegate-rust");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");

    // the new session reports accept-edits (Vibe's default), and the
    // agent offers the hidden profiles as well
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result_with_delegate_modes("s1", "accept-edits"),
    });
    // no set_mode with the dead stored id: the prompt goes out right
    // away, then the sidebar refresh
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
    assert!(h.pending_mode_prompt.is_none());
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);
    // the wire data is untouched: the profiles stay in the mode list
    assert_eq!(h.known_modes.len(), 4);
    assert!(h.known_modes.iter().any(|m| m.id == "claude-delegate-rust"));
    // and the session keeps Vibe's default (no switch happened)
    assert_eq!(
        h.sessions["s1"].modes.as_ref().unwrap().current_mode_id,
        "accept-edits"
    );
}

#[test]
fn set_mode_failure_holds_the_prompt_back_and_warns() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");

    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result_with_modes("s1", "accept-edits"),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/set_mode");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
    assert!(h.pending_mode_prompt.is_some());

    // set_mode fails on the wire: the session is still in Vibe's default
    // (accept-edits), so the prompt must NOT go out in that mode; it comes
    // back into the composer, with a warning
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::SetMode {
            session_id: "s1".to_string(),
            mode_id: "ask".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    assert!(h.pending_mode_prompt.is_none());
    assert!(h.pending_setup.is_empty());
    assert!(h.status.contains("internal error"), "{}", h.status);
    assert_eq!(h.sessions["s1"].turn, TurnState::Idle, "no turn started");
    // the text is back for s1 (on screen: the composer; else parked)
    let back = if h.active.as_deref() == Some("s1") {
        h.draft.clone()
    } else {
        h.ui.parked_drafts
            .get(&Some("s1".to_string()))
            .map(|(t, _)| t.clone())
            .unwrap_or_default()
    };
    assert_eq!(back, "Hallo Vibe");
}

/// A failed model / thinking step is no safety question (the mode is set):
/// the held prompt still goes out, with a warning.
#[test]
fn config_step_failure_still_sends_prompt_and_warns() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "accept-edits");
    h.current_project = Some(dir.clone());
    h.start_model = Some("mistral-medium-3.5".to_string());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    let result: proto::NewSessionResult = serde_json::from_value(json!({
        "sessionId": "s1",
        "modes": {"currentModeId": "accept-edits", "availableModes": [
            {"id": "accept-edits", "name": "Accept Edits"}]},
        "configOptions": [{"type": "select", "id": "model", "currentValue": "mistral-large-4-0-3",
            "options": [{"value": "mistral-large-4-0-3"}, {"value": "mistral-medium-3.5"}]}]
    }))
    .unwrap();
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result,
    });
    assert_eq!(agent.read_msg()["method"], "session/set_config_option");
    assert_eq!(agent.read_msg()["method"], "session/list");
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::SetConfigOption {
            session_id: "s1".to_string(),
            config_id: "model".to_string(),
        },
        error: rpc::RpcError {
            code: -32602,
            message: "unknown model".to_string(),
            data: None,
        },
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert!(h.pending_mode_prompt.is_none());
    assert!(!h.status.is_empty());
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);
}

/// Vibe answered the prompt, but the answer could not be read: the turn
/// ran and the message was delivered, so it must NOT come back into the
/// composer (sending it again would repeat the turn).
#[test]
fn unreadable_prompt_answer_does_not_restore_the_draft() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.draft = "Lösche die alten Logs".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/prompt");
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: crate::acp::client::parse_error("session/prompt"),
    });
    assert_eq!(h.sessions["s1"].turn, TurnState::Idle, "the turn has ended");
    assert!(
        h.draft.is_empty(),
        "the delivered message stays out of the composer"
    );
    assert!(h.status.contains("delivered"), "{}", h.status);
}

#[test]
fn start_modes_fallback_and_known() {
    let fallback = start_modes(&[]);
    // the fallback list shows the app-side label too: `ask` is "Plan"
    assert_eq!(
        fallback,
        vec![
            ("ask".to_string(), "Plan".to_string()),
            ("accept-edits".to_string(), "Accept Edits".to_string()),
            ("smart-approve".to_string(), "Smart Approve".to_string()),
            ("auto-approve".to_string(), "Auto Approve".to_string()),
        ]
    );
    let known = vec![SessionMode {
        id: "custom".to_string(),
        name: "Eigen".to_string(),
        description: None,
    }];
    assert_eq!(
        start_modes(&known),
        vec![("custom".to_string(), "Eigen".to_string())]
    );
}

#[test]
fn mode_label_maps_ask_to_plan_and_passes_the_rest_through() {
    // the only app-side rename: `ask` shows as "Plan" (the wire id
    // keeps being "ask"); every other id keeps Vibe's own name
    assert_eq!(mode_label("ask", "Ask"), "Plan");
    assert_eq!(mode_label("ask", "Irgendwas"), "Plan");
    assert_eq!(mode_label("accept-edits", "Accept Edits"), "Accept Edits");
    assert_eq!(
        mode_label("claude-delegate", "Claude Delegate"),
        "Claude Delegate"
    );
    assert_eq!(mode_label("custom", "Eigen"), "Eigen");
}

#[test]
fn pickers_hide_delegate_profiles_but_active_mode_keeps_its_label() {
    let mode = |id: &str, name: &str| SessionMode {
        id: id.to_string(),
        name: name.to_string(),
        description: None,
    };
    // wire data as vibe-acp reports it (see tests/fixtures/acp/
    // subagent_session.jsonl): the four defaults plus the profiles
    let modes = ModeState {
        current_mode_id: "claude-delegate-rust".to_string(),
        available_modes: vec![
            mode("ask", "Ask"),
            mode("accept-edits", "Accept Edits"),
            mode("claude-delegate", "Claude Delegate"),
            mode("claude-delegate-rust", "Claude Delegate (Rust)"),
        ],
    };

    // the selectable lists (start screen, settings) drop both
    // profiles, natural future suffixes included, and label `ask`
    // as "Plan"
    assert!(!mode_selectable("claude-delegate"));
    assert!(!mode_selectable("claude-delegate-rust"));
    assert!(mode_selectable("ask"));
    assert!(mode_selectable("accept-edits"));
    assert_eq!(
        start_modes(&modes.available_modes),
        vec![
            ("ask".to_string(), "Plan".to_string()),
            ("accept-edits".to_string(), "Accept Edits".to_string()),
        ]
    );

    // the ACTIVE mode is never hidden: a session started in the hidden
    // profile (from the CLI, say) still renders its Vibe name
    assert_eq!(current_mode_label(&modes), "Claude Delegate (Rust)");
    // and Vibe's own name for a mode that is not in the list at all
    let unknown = ModeState {
        current_mode_id: "claude-delegate".to_string(),
        available_modes: vec![mode("ask", "Ask")],
    };
    assert_eq!(current_mode_label(&unknown), "claude-delegate");
}

/// Live check against the real `vibe-acp` (needs network + a logged-in
/// Vibe): a fresh session with start_mode "ask" must show a permission
/// card for a file write.
/// `cargo test -- --ignored live_permission_card_in_ask_mode`
#[test]
#[ignore = "needs a real vibe-acp with network and a logged-in Vibe"]
fn live_permission_card_in_ask_mode() {
    let dir = std::env::temp_dir().join(format!("acp-live-perm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (client, rx) = AcpClient::spawn("vibe-acp", &[], None).expect("spawn vibe-acp");
    let mut h = Harness::new(vec![dir.clone()], "ask".to_string());
    h.attach(client, rx);
    h.client.as_ref().unwrap().initialize().unwrap();
    let boot = std::time::Instant::now() + Duration::from_secs(60);
    while !matches!(h.conn, Conn::Ready { .. }) {
        h.poll();
        assert!(
            std::time::Instant::now() < boot,
            "vibe-acp never became ready"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    h.current_project = Some(dir.clone());
    h.draft = "Erstelle die Datei probe.txt mit dem Inhalt ok.".to_string();
    h.send();
    // poll until a permission request lands in the session
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    loop {
        h.poll();
        let open = h
            .active
            .as_deref()
            .and_then(|sid| h.sessions.get(sid))
            .map(|v| !v.permissions.is_empty())
            .unwrap_or(false);
        if open {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no permission request within 180 s"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // answer with the first reject_* option
    let (sid, req_id, option_id) = {
        let v = h.sessions.get(h.active.as_ref().unwrap()).unwrap();
        let p = &v.permissions[0];
        let opt = p
            .request
            .options
            .iter()
            .find(|o| {
                matches!(
                    o.kind,
                    proto::PermissionKind::RejectOnce | proto::PermissionKind::RejectAlways
                )
            })
            .unwrap_or(&p.request.options[0]);
        (
            v.session_id.clone(),
            p.request_id.clone(),
            opt.option_id.clone(),
        )
    };
    h.answer_permission(&sid, &req_id, &option_id);
    // wait for the prompt to finish; after a rejection the agent may try
    // another way and ask again, so keep rejecting follow-up requests
    let mut asked = 1;
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    loop {
        h.poll();
        let next = h.sessions.get(&sid).and_then(|v| {
            v.permissions.first().map(|p| {
                let opt = p
                    .request
                    .options
                    .iter()
                    .find(|o| {
                        matches!(
                            o.kind,
                            proto::PermissionKind::RejectOnce | proto::PermissionKind::RejectAlways
                        )
                    })
                    .unwrap_or(&p.request.options[0]);
                (p.request_id.clone(), opt.option_id.clone())
            })
        });
        if let Some((rid, oid)) = next {
            asked += 1;
            h.answer_permission(&sid, &rid, &oid);
        }
        let idle = h
            .active
            .as_deref()
            .and_then(|s| h.sessions.get(s))
            .map(|v| v.turn == TurnState::Idle)
            .unwrap_or(false);
        if idle {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "prompt did not finish within 180 s"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(asked >= 1);
    // the file must not exist: every request was rejected
    assert!(
        !dir.join("probe.txt").exists(),
        "rejected write still happened"
    );
    if let Some(c) = &h.client {
        let _ = c.shutdown();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn permission_request_lands_in_session_and_answer_goes_out() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");

    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });
    assert_eq!(h.sessions["s1"].permissions.len(), 1);

    h.answer_permission("s1", &json!(0), "allow_once");
    assert!(h.sessions["s1"].permissions.is_empty());
    let resp = agent.read_msg();
    assert_eq!(
        resp,
        json!({"jsonrpc": "2.0", "id": 0, "result": {"outcome": {"outcome": "selected", "optionId": "allow_once"}}})
    );
}

/// Loop guard (harness side): repeated identical tool calls arrive as
/// `session/update`s during a running turn, the detection fires at the
/// third one, and the card's stop path ends the turn with
/// `session/cancel`.
#[test]
fn loop_guard_detects_repeated_tool_calls_and_can_stop_the_turn() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.sessions.get_mut("s1").unwrap().begin_turn();
    h.handle_event(AcpEvent::Update {
        session_id: "s1".to_string(),
        update: user_chunk("m1", "Liste die Dateien"),
    });
    let call = |id: &str| -> proto::SessionUpdate {
        serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": "Bash",
            "kind": "execute",
            "status": "completed",
            "rawInput": {"command": "ls"},
        }))
        .unwrap()
    };
    let detected = |h: &Harness| {
        let s = &h.sessions["s1"];
        repeated_tool_call(&s.items, &s.tool_calls).is_some()
    };

    for id in ["t1", "t2"] {
        h.handle_event(AcpEvent::Update {
            session_id: "s1".to_string(),
            update: call(id),
        });
        assert!(!detected(&h), "two repeats are no loop yet");
    }
    h.handle_event(AcpEvent::Update {
        session_id: "s1".to_string(),
        update: call("t3"),
    });
    assert!(detected(&h), "the third identical call is a loop");

    // the card's button calls `h.cancel()`: the turn ends on the wire
    h.cancel();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/cancel");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(h.sessions["s1"].turn, TurnState::Cancelling);
}

/// Finished rounds fold (harness side): a round built from real
/// `session/update`s stays open while the turn runs and folds to its last
/// answer once `session/prompt` finished; a failed prompt does not fold.
#[test]
fn finished_round_folds_after_the_turn_ends() {
    use crate::acp::session::round_key;
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.sessions.get_mut("s1").unwrap().begin_turn();
    let update = |u| AcpEvent::Update {
        session_id: "s1".to_string(),
        update: u,
    };
    h.handle_event(update(user_chunk("m1", "Liste die Dateien")));
    h.handle_event(update(agent_chunk("a1", "Ich schaue nach.")));
    h.handle_event(update(
        serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "t1",
            "title": "Bash",
            "kind": "execute",
            "status": "completed",
        }))
        .unwrap(),
    ));
    h.handle_event(update(agent_chunk("a2", "Fertig.")));

    let folded = |h: &Harness| {
        let rs = h.sessions["s1"].folded_rounds();
        (rs.len(), rs[0].collapsed, rs[0].answer, rs[0].tool_calls)
    };
    // running: the open round stays fully visible
    assert_eq!(folded(&h), (1, false, Some(3), 1));

    let done: proto::PromptResult =
        serde_json::from_value(json!({"stopReason": "end_turn"})).unwrap();
    h.handle_event(AcpEvent::PromptFinished {
        session_id: "s1".to_string(),
        result: done,
    });
    assert_eq!(agent.read_msg()["method"], "session/list");
    // finished: folds to the last answer (index 3), one tool call
    assert_eq!(folded(&h), (1, true, Some(3), 1));
    let s = &h.sessions["s1"];
    let round = &s.folded_rounds()[0];
    assert_eq!(round.summary_range(), 0..3);
    assert_eq!(round_key("s1", round), "s1#round#m1");

    // a second round that fails on the wire stays open; the earlier
    // finished round stays folded
    h.draft = "Zweite Frage".to_string();
    h.send();
    loop {
        let msg = agent.read_msg();
        if msg["method"] != "session/prompt" {
            continue; // e.g. the `_loops/list` refresh after the turn
        }
        assert_eq!(msg["params"]["sessionId"], json!("s1"));
        break;
    }
    // sent, Vibe's echo not there yet: the history does not unfold
    assert_eq!(folded(&h), (1, true, Some(3), 1));
    h.handle_event(update(user_chunk("m2", "Zweite Frage")));
    h.handle_event(update(agent_chunk("a3", "Ich fange an.")));
    let folded2 = |h: &Harness| {
        let rs = h.sessions["s1"].folded_rounds();
        (rs.len(), rs[0].collapsed, rs[1].collapsed)
    };
    assert_eq!(
        folded2(&h),
        (2, true, false),
        "while a turn runs, only its round is open (was: every round)"
    );
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    let s = &h.sessions["s1"];
    assert_eq!(s.turn, TurnState::Idle);
    assert_eq!(s.last_stop_reason, None, "a failed prompt sets no reason");
    let rs = s.folded_rounds();
    assert_eq!(rs.len(), 2);
    // a failed session/prompt does not record a stop reason, so the
    // view alone cannot tell it from a finished turn; the folding rule
    // is `fold_rounds`'s (pure, tested there). Here: both rounds are
    // complete rounds with an answer.
    assert!(rs[0].collapsed && rs[1].collapsed);
}

/// N2: fold states keep binding to their content across a rewind. The
/// new turn occupies the cut positions with new content — an
/// index-based key (the old `<session>#<start>` / `#thought#<index>`)
/// would mark the NEW round's tool group and thought as expanded the
/// moment they appear. Identity keys leave them closed, while the
/// surviving round-1 group keeps its open state.
#[test]
fn fold_states_follow_their_content_across_a_rewind() {
    use crate::harness::ui::transcript::{thought_key, tool_group_key};
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    let update = |u| AcpEvent::Update {
        session_id: "s1".to_string(),
        update: u,
    };
    let tool_call = |id: &str| {
        serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": "Bash",
            "kind": "execute",
            "status": "completed"
        }))
        .unwrap()
    };
    let thought = |id: &str, text: &str| {
        serde_json::from_value(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": text},
            "messageId": id
        }))
        .unwrap()
    };
    // round 1: user m1, tool t1, answer a1
    h.handle_event(update(user_chunk("m1", "erste Frage")));
    h.handle_event(update(tool_call("t1")));
    h.handle_event(update(agent_chunk("a1", "erledigt.")));
    // round 2: user m2, thought th2, tool t2, answer a2
    h.handle_event(update(user_chunk("m2", "zweite Frage")));
    h.handle_event(update(thought("th2", "Ich denke nach.")));
    h.handle_event(update(tool_call("t2")));
    h.handle_event(update(agent_chunk("a2", "auch erledigt.")));

    fn calls_of<'a>(
        v: &'a SessionView,
        group: &'a std::ops::Range<usize>,
    ) -> Vec<&'a proto::ToolCall> {
        v.items[group.clone()]
            .iter()
            .filter_map(|it| match it {
                Item::Tool(id) => v.tool_calls.get(id),
                _ => None,
            })
            .collect()
    }
    // both groups and the thought are open when the rewind happens
    let (round1, round2, round2_pos, round2_thought) = {
        let v = &h.sessions["s1"];
        let groups = crate::acp::session::tool_groups(&v.items);
        assert_eq!(groups.len(), 2, "one tool group per round");
        (
            tool_group_key("s1", groups[0].start, &calls_of(v, &groups[0])),
            tool_group_key("s1", groups[1].start, &calls_of(v, &groups[1])),
            groups[1].start,
            thought_key("s1", Some("th2"), 4),
        )
    };
    assert_eq!(round1, "s1#grp#t1");
    assert_eq!(round2, "s1#grp#t2");
    h.open_groups.insert(round1.clone());
    h.open_groups.insert(round2.clone());
    h.open_groups.insert(round2_thought.clone());

    // the rewind cuts round 2; a new turn fills the same positions
    // (user @3, thought @4, tool @5 — t2's old group position)
    assert!(h.sessions.get_mut("s1").unwrap().rewind_to_message("m2"));
    h.handle_event(update(user_chunk("m3", "noch eine Frage")));
    h.handle_event(update(thought("th3", "Neue Idee.")));
    h.handle_event(update(tool_call("t3")));

    let (new_group, new_pos, new_thought) = {
        let v = &h.sessions["s1"];
        let groups = crate::acp::session::tool_groups(&v.items);
        let group = groups
            .iter()
            .find(|g| g.start >= 3)
            .expect("the new turn brought its own group");
        (
            tool_group_key("s1", group.start, &calls_of(v, group)),
            group.start,
            thought_key("s1", Some("th3"), 4),
        )
    };
    // the new content sits at the old round-2 positions — with index
    // keys their fold states would collide
    assert_eq!(
        new_pos, round2_pos,
        "the scenario really collides positionally"
    );
    assert_ne!(new_group, round2);
    assert!(
        h.open_groups.contains(&round1),
        "the surviving group keeps its state"
    );
    assert!(
        !h.open_groups.contains(&new_group),
        "the new group does not inherit the old round's fold state"
    );
    assert!(
        !h.open_groups.contains(&new_thought),
        "the new thought does not inherit the old one's fold state"
    );
}

#[test]
fn cancel_answers_open_permissions_then_sends_session_cancel() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.sessions.get_mut("s1").unwrap().begin_turn();
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });

    h.cancel();
    let resp = agent.read_msg();
    assert_eq!(
        resp,
        json!({"jsonrpc": "2.0", "id": 0, "result": {"outcome": {"outcome": "cancelled"}}})
    );
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/cancel");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert!(msg.get("id").is_none(), "cancel is a notification");
    assert_eq!(h.sessions["s1"].turn, TurnState::Cancelling);
    assert!(h.sessions["s1"].permissions.is_empty());
}

/// N4: questions whose session has no live view (none, a `child-…`
/// subagent, or a session whose view was dropped) cannot be stopped
/// from their own session — the stop of the active session answers
/// them "cancel" so they do not stay as dead cards. Questions of other
/// sessions with live views stay: their turns are not being stopped —
/// including a `child-…` subagent whose parent view is alive: the
/// parent's own stop will reach it (final audit C).
#[test]
fn cancel_answers_questions_without_a_live_view() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    // s1 is created last, so it is the session the user watches
    create_session(&mut h, &mut agent, &dir, "s2");
    create_session(&mut h, &mut agent, &dir, "s1");
    assert_eq!(h.active.as_deref(), Some("s1"));
    // s2 has a live subagent: its child holds a question of its own
    h.sessions
        .get_mut("s2")
        .unwrap()
        .apply_update(&spawn_with_child("child-2"));
    let mut asked = |sid: Option<&str>, id: i32| {
        h.handle_event(AcpEvent::QuestionAsked {
            request_id: json!(id),
            params: json!({
                "sessionId": sid,
                "requestedSchema": {"properties": {
                    "q0": {"type": "string", "description": "Format?",
                           "oneOf": [{"const": "CSV", "title": "CSV"}]}
                }},
            }),
        });
    };
    asked(Some("child-9"), 7); // subagent: no view at all
    asked(None, 8); // no session id on the request
    asked(Some("s2"), 9); // another live session: none of cancel's business
    asked(Some("child-2"), 10); // subagent of the LIVE s2: s1's stop spares it
    assert_eq!(h.questions.len(), 4);
    // s1 is on screen; its turn is stopped
    h.sessions.get_mut("s1").unwrap().begin_turn();

    h.cancel();
    // both viewless questions were answered "cancel" over the wire
    for id in [7, 8] {
        let resp = agent.read_msg();
        assert_eq!(
            resp,
            json!({"jsonrpc": "2.0", "id": id, "result": {"action": "cancel"}})
        );
    }
    // s2's questions are untouched: still available, nothing on the wire
    // for them — the next line is the session/cancel notification
    assert_eq!(
        h.questions
            .iter()
            .map(|q| q.request_id.clone())
            .collect::<Vec<_>>(),
        vec![json!(9), json!(10)],
        "the live session's question and its live child's stay"
    );
    assert_eq!(h.sessions["s2"].turn, TurnState::Idle, "not stopped");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/cancel");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
}

/// Handoff 2.2: a permission request for a session the harness does
/// not know (a subagent `child-…` id, or a race right after a
/// reconnect) must never be silently dropped: the turn blocks until
/// an answer arrives. The request becomes an answerable orphan card
/// and the answer goes over the wire with the agent's id verbatim.
#[test]
fn permission_request_for_an_unknown_session_is_kept_answerable() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");

    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(3),
        request: permission_request("child-4f2c"),
    });
    assert!(
        h.sessions["s1"].permissions.is_empty(),
        "the request is not stuffed into an unrelated session"
    );
    assert_eq!(h.orphan_permissions.len(), 1, "the request is kept");
    assert_eq!(
        h.orphan_permissions[0].request.session_id, "child-4f2c",
        "the raw session id stays on the card"
    );
    assert!(!h.status.is_empty(), "the arrival shows up in the status");

    // answering needs no session entry — only the request id
    h.answer_orphan_permission(&json!(3), "allow_once");
    assert!(h.orphan_permissions.is_empty());
    let resp = agent.read_msg();
    assert_eq!(
        resp,
        json!({"jsonrpc": "2.0", "id": 3, "result": {"outcome": {"outcome": "selected", "optionId": "allow_once"}}})
    );

    // the orphan card's own cancel path answers "cancelled"
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(4),
        request: permission_request("child-4f2c"),
    });
    h.cancel_orphan_permission(&json!(4));
    assert!(h.orphan_permissions.is_empty());
    let resp = agent.read_msg();
    assert_eq!(
        resp,
        json!({"jsonrpc": "2.0", "id": 4, "result": {"outcome": {"outcome": "cancelled"}}})
    );
}

/// Handoff 2.2: when the unknown session becomes known while its
/// request is still open, the orphan card migrates into that
/// session's view and is answered through the normal session path.
#[test]
fn orphan_permission_migrates_when_the_session_becomes_known() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");

    // the request races ahead of the session/load coming back
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });
    assert_eq!(h.orphan_permissions.len(), 1);
    h.open_session(dir.clone(), "s1");
    assert_eq!(agent.read_msg()["method"], "session/load");
    assert!(h.orphan_permissions.is_empty(), "the card migrated");
    assert_eq!(h.sessions["s1"].permissions.len(), 1);

    h.answer_permission("s1", &json!(0), "reject_once");
    assert!(h.sessions["s1"].permissions.is_empty());
    let resp = agent.read_msg();
    assert_eq!(
        resp,
        json!({"jsonrpc": "2.0", "id": 0, "result": {"outcome": {"outcome": "selected", "optionId": "reject_once"}}})
    );

    // the same migration when a session/new answer hops over the
    // request (reconnect replay order is agent-defined)
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(5),
        request: permission_request("s2"),
    });
    create_session(&mut h, &mut agent, &dir, "s2");
    assert!(h.orphan_permissions.is_empty());
    assert_eq!(h.sessions["s2"].permissions.len(), 1);
}

/// Handoff 2.8.7: a restart kills open permission requests (the new
/// process never saw their ids), so the cards must not just vanish —
/// the status line says the approvals were dropped.
#[test]
fn connection_loss_mentions_dropped_approvals_in_the_status() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(1),
        request: permission_request("child-4f2c"),
    });
    assert_eq!(h.sessions["s1"].permissions.len(), 1);
    assert_eq!(h.orphan_permissions.len(), 1);

    h.handle_event(exited());
    assert!(h.sessions.is_empty() && h.orphan_permissions.is_empty());
    assert!(
        h.status.contains("approvals"),
        "the dropped approvals are mentioned: {}",
        h.status
    );
}

/// An orphaned permission request as `orphan_permissions` holds it.
fn orphan_pending(session_id: &str) -> PendingPermission {
    PendingPermission {
        request_id: json!(0),
        request: permission_request(session_id),
    }
}

/// A completed subagent spawn whose `_meta` carries the child session
/// id (docs/acp.md, "Subagents") — the parent view knows the child.
fn spawn_with_child(child: &str) -> proto::SessionUpdate {
    serde_json::from_value(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "spawn-1",
        "title": "Starting reader",
        "kind": "think",
        "status": "completed",
        "rawInput": {"task": "Lies README.md", "agent": "generic"},
        "_meta": {
            "tool_name": "agents.spawn",
            "effect_kind": "subagent",
            "agent": "generic",
            "task": "Lies README.md",
            "child_session_id": child
        }
    }))
    .unwrap()
}

/// Handoff 2.2, one view (was: the Code/Chat switcher badge, decided
/// per view). A waiting session marks its own workspace; an orphaned
/// request marks the workspace of its known parent session, and no
/// workspace without one (the active session shows it as a card).
#[test]
fn workspace_waiting_permission_attribution() {
    let chat = PathBuf::from("/chat-ws");
    let proj = PathBuf::from("/proj");
    let other = PathBuf::from("/other");
    let mut sessions: HashMap<String, SessionView> = HashMap::new();
    let marks = |sessions: &HashMap<String, SessionView>, orphans: &[PendingPermission]| {
        [&proj, &chat, &other].map(|d| workspace_waiting_permission(sessions, orphans, d))
    };

    // nothing open anywhere
    assert_eq!(marks(&sessions, &[]), [false, false, false]);

    // a project session with a spawned child; nothing waits yet
    let mut code_view = SessionView::new("s1", &proj);
    code_view.begin_turn();
    code_view.apply_update(&spawn_with_child("child-4f2c"));
    sessions.insert("s1".to_string(), code_view);
    assert_eq!(marks(&sessions, &[]), [false, false, false]);
    // the child asks for a permission: attributed through its parent
    // to the project, although its session id is unknown
    assert_eq!(
        marks(&sessions, &[orphan_pending("child-4f2c")]),
        [true, false, false]
    );
    // an orphan without a known parent belongs to no workspace
    assert_eq!(
        marks(&sessions, &[orphan_pending("child-foreign")]),
        [false, false, false]
    );
    // the parent itself asks as well: still only the project
    sessions.get_mut("s1").unwrap().add_permission(
        json!(1),
        serde_json::from_value(json!({
            "sessionId": "s1", "toolCall": {"toolCallId": "t1"}, "options": []
        }))
        .unwrap(),
    );
    assert_eq!(
        marks(&sessions, &[orphan_pending("child-4f2c")]),
        [true, false, false]
    );
    // a waiting chat-workspace session: "Allgemein" as well
    let mut chat_view = SessionView::new("c1", &chat);
    chat_view.add_permission(
        json!(2),
        serde_json::from_value(json!({
            "sessionId": "c1", "toolCall": {"toolCallId": "t2"}, "options": []
        }))
        .unwrap(),
    );
    sessions.insert("c1".to_string(), chat_view);
    assert_eq!(
        marks(&sessions, &[orphan_pending("child-4f2c")]),
        [true, true, false]
    );
}

/// The workspace row's mark (Handoff 2.2; was: the switcher hint for
/// the parked view): set while one of the workspace's sessions waits
/// for a permission or a question, whichever session is on screen, so
/// a request stays visible while its workspace is folded.
#[test]
fn workspace_row_marks_waiting_sessions() {
    let proj = std::env::temp_dir().join(format!("od-hint-{}", std::process::id()));
    std::fs::create_dir_all(&proj).unwrap();
    let ws = proj.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let (mut h, _agent) = setup(vec![proj.clone()], "ask");
    h.set_chat_dir(ws.clone());
    let ws = ws.canonicalize().unwrap();
    let proj = proj.canonicalize().unwrap();
    assert!(!h.workspace_waiting(&ws) && !h.workspace_waiting(&proj));

    // a chat session with an open approval: "Allgemein" is marked
    let mut v = SessionView::new("c1", &ws);
    v.add_permission(json!(0), permission_request("c1"));
    h.sessions.insert("c1".to_string(), v);
    assert!(h.workspace_waiting(&ws));
    assert!(!h.workspace_waiting(&proj), "only its own workspace");
    // on screen or not makes no difference
    h.active = Some("c1".to_string());
    assert!(h.workspace_waiting(&ws));

    // answered: the mark disappears
    h.sessions.get_mut("c1").unwrap().permissions.clear();
    assert!(!h.workspace_waiting(&ws));

    // a question of a project session marks the project
    h.sessions
        .insert("s1".to_string(), SessionView::new("s1", &proj));
    let q = extras::parse_question(
        json!(7),
        &json!({
            "sessionId": "s1",
            "message": "Welche Datei?",
            "requestedSchema": {"type": "object", "properties": {
                "datei": {"type": "string", "title": "Datei"}
            }}
        }),
    )
    .expect("a renderable question");
    h.questions.push(q);
    assert!(h.workspace_waiting(&proj));
    assert!(!h.workspace_waiting(&ws));
    let _ = std::fs::remove_dir_all(&proj);
}

/// The preselected workspace is the one whose newest session was updated
/// last; only candidates count, a tie goes to the earlier one (sidebar
/// order) and undated sessions never decide.
#[test]
fn last_used_workspace_is_the_one_with_the_newest_session() {
    let a = PathBuf::from("/a");
    let b = PathBuf::from("/b");
    let chat = PathBuf::from("/chat");
    let info = |id: &str, at: Option<&str>| -> SessionInfo {
        serde_json::from_value(json!({"sessionId": id, "cwd": "/x", "updatedAt": at})).unwrap()
    };
    let all = [chat.clone(), a.clone(), b.clone()];
    let mut listed: HashMap<PathBuf, Vec<SessionInfo>> = HashMap::new();
    assert_eq!(last_used_workspace(&listed, &all), None, "nothing listed");
    listed.insert(
        a.clone(),
        vec![info("a1", Some("2026-11-07T09:00:00Z")), info("a2", None)],
    );
    assert_eq!(last_used_workspace(&listed, &all), Some(a.clone()));
    listed.insert(b.clone(), vec![info("b1", Some("2026-11-08T08:00:00Z"))]);
    listed.insert(chat.clone(), vec![info("c1", Some("2026-11-06T12:00:00Z"))]);
    assert_eq!(last_used_workspace(&listed, &all), Some(b.clone()));
    // a folder that is no candidate (a removed project) is ignored
    assert_eq!(
        last_used_workspace(&listed, &[chat.clone(), a.clone()]),
        Some(a.clone())
    );
    // a tie goes to the earlier candidate
    listed.insert(chat.clone(), vec![info("c2", Some("2026-11-08T08:00:00Z"))]);
    assert_eq!(last_used_workspace(&listed, &all), Some(chat.clone()));
    let undated: HashMap<PathBuf, Vec<SessionInfo>> = [(a.clone(), vec![info("a3", None)])].into();
    assert_eq!(last_used_workspace(&undated, &all), None);
}

/// At startup the session lists arrive one by one; until the user picks
/// a workspace, the start screen follows the newest session. A choice
/// of the user is never overridden by a later list.
#[test]
fn startup_preselects_the_last_used_workspace_until_the_user_picks() {
    let base = std::env::temp_dir().join(format!("od-last-{}", std::process::id()));
    let (a, b, ws) = (base.join("a"), base.join("b"), base.join("ws"));
    for d in [&a, &b, &ws] {
        std::fs::create_dir_all(d).unwrap();
    }
    let (mut h, _agent) = setup(vec![a.clone(), b.clone()], "ask");
    h.set_chat_dir(ws.clone());
    let ws = ws.canonicalize().unwrap();
    let listed = |dir: &Path, id: &str, at: &str| AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(dir.to_path_buf()),
        result: sessions_listed(json!([{
            "sessionId": id, "cwd": dir.to_string_lossy(), "updatedAt": at
        }])),
    };
    assert_eq!(
        h.current_project.as_deref(),
        Some(a.as_path()),
        "before any list: the first project"
    );
    h.handle_event(listed(&b, "s-b", "2026-11-07T09:00:00Z"));
    assert_eq!(h.current_project.as_deref(), Some(b.as_path()));
    h.handle_event(listed(&ws, "s-c", "2026-11-08T09:00:00Z"));
    assert_eq!(
        h.current_project.as_deref(),
        Some(ws.as_path()),
        "the chat was used last"
    );

    // the user picks a project: a newer list elsewhere changes nothing
    h.select_workspace(a.clone());
    h.handle_event(listed(&b, "s-b2", "2026-11-09T09:00:00Z"));
    assert_eq!(h.current_project.as_deref(), Some(a.as_path()));
    let _ = std::fs::remove_dir_all(&base);
}

/// The chat workspace "Allgemein" is pinned: never a project (an old
/// list entry for its folder goes away, adding it selects it), never
/// removable, and where new sessions start when no project is left.
#[test]
fn chat_workspace_is_pinned_and_never_a_project() {
    let base = std::env::temp_dir().join(format!("od-pin-{}", std::process::id()));
    let (p, ws) = (base.join("p"), base.join("ws"));
    for d in [&p, &ws] {
        std::fs::create_dir_all(d).unwrap();
    }
    let (p, ws_canon) = (p.canonicalize().unwrap(), ws.canonicalize().unwrap());
    let mut h = Harness::new(vec![ws_canon.clone(), p.clone()], "ask".into());
    h.set_chat_dir(ws.clone());
    assert_eq!(h.projects, vec![p.clone()], "shown once, as Allgemein");
    assert_eq!(h.workspaces(), vec![ws_canon.clone(), p.clone()]);

    // adding the chat folder as a project selects "Allgemein" instead
    assert!(h.add_project(&ws_canon.to_string_lossy()));
    assert_eq!(h.projects, vec![p.clone()]);
    assert_eq!(h.current_project.as_deref(), Some(ws_canon.as_path()));

    // it cannot be removed
    h.remove_project(&ws_canon);
    assert_eq!(h.workspaces(), vec![ws_canon.clone(), p.clone()]);

    // the last project in use goes: new sessions start in "Allgemein"
    h.select_workspace(p.clone());
    h.remove_project(&p);
    assert!(h.projects.is_empty());
    assert_eq!(h.current_project.as_deref(), Some(ws_canon.as_path()));

    // and without any project a fresh start begins there, too
    let mut fresh = Harness::new(Vec::new(), "ask".into());
    fresh.set_chat_dir(ws.clone());
    assert_eq!(fresh.current_project.as_deref(), Some(ws_canon.as_path()));
    let _ = std::fs::remove_dir_all(&base);
}

/// N16: a chat folder that cannot be created is reported, and the
/// previous chat dir stays — no half state (new sessions would start
/// in a path that does not exist). The canonicalize fallback continues
/// to apply to folders that do exist.
#[test]
fn chat_dir_error_is_reported_and_keeps_the_previous_dir() {
    let (mut h, _agent) = setup(vec![], "ask");
    let good = std::env::temp_dir().join(format!("od-chat-ok-{}", std::process::id()));
    std::fs::create_dir_all(&good).unwrap();
    h.set_chat_dir(good.clone());
    let good = good.canonicalize().unwrap();
    assert_eq!(h.chat_dir.as_deref(), Some(good.as_path()));
    assert!(h.status.is_empty(), "the good call is quiet: {}", h.status);

    // imposible to create: the parent is a regular file
    let parent = std::env::temp_dir().join(format!("od-chat-file-{}", std::process::id()));
    std::fs::write(&parent, "not a folder").unwrap();
    h.set_chat_dir(parent.join("child"));
    assert!(
        h.status.contains("The chat folder could not be created"),
        "the failure is not swallowed: {}",
        h.status
    );
    assert_eq!(
        h.chat_dir.as_deref(),
        Some(good.as_path()),
        "the chat dir is not switched to a dead path"
    );
    let _ = std::fs::remove_file(&parent);
}

/// Handoff 2.8.7, one view (was: switching views during the reconnect
/// must not let the other view's session take over). The session that
/// was on screen is reloaded and shown in its own workspace, whatever
/// workspace the user picked meanwhile; the other open sessions are not
/// reloaded behind the user's back (a click loads them again). Between
/// the exit and the reload nothing is active: the start screen shows.
#[test]
fn reconnect_reloads_the_session_on_screen_into_its_workspace() {
    let proj = std::env::temp_dir().join(format!("od-sw-{}", std::process::id()));
    std::fs::create_dir_all(&proj).unwrap();
    let ws = proj.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let (mut h, mut agent) = setup(vec![proj.clone()], "ask");
    h.set_chat_dir(ws.clone());
    let ws = ws.canonicalize().unwrap();

    // a project session s1 and, on screen, the chat c1
    h.open_session(proj.clone(), "s1");
    assert_eq!(agent.read_msg()["method"], "session/load");
    h.open_session(ws.clone(), "c1");
    assert_eq!(agent.read_msg()["method"], "session/load");
    assert_eq!(h.active.as_deref(), Some("c1"));

    // vibe-acp dies; the user picks the project while reconnecting
    h.handle_event(exited());
    assert!(matches!(h.conn, Conn::Reconnecting { .. }));
    assert!(h.active.is_none(), "start screen until the reload");
    assert!(h.sessions.is_empty());
    h.select_workspace(proj.clone());

    // the restarted agent becomes ready
    let (a, b) = UnixStream::pair().expect("socket pair");
    a.set_read_timeout(Some(RECV)).unwrap();
    b.set_read_timeout(Some(RECV)).unwrap();
    let (client, rx) = AcpClient::from_streams(
        BufReader::new(a.try_clone().unwrap()),
        a.try_clone().unwrap(),
        None,
    );
    h.attach(client, rx);
    let mut agent2 = Agent {
        reader: BufReader::new(b),
    };
    let init: proto::InitializeResult = serde_json::from_value(json!({
        "protocolVersion": 1,
        "agentCapabilities": {"loadSession": true}
    }))
    .unwrap();
    h.handle_event(AcpEvent::Initialized(init));

    // only the session that was on screen comes back, in its workspace;
    // next is the probe, no second session/load for s1
    let first = agent2.read_msg();
    assert_eq!(first["method"], "session/load");
    assert_eq!(first["params"]["sessionId"], json!("c1"));
    assert_eq!(first["params"]["cwd"], json!(ws.to_string_lossy()));
    let second = agent2.read_msg();
    assert_eq!(second["method"], "session/new", "the probe, not s1");
    assert_eq!(h.active.as_deref(), Some("c1"));
    assert_eq!(h.current_project.as_deref(), Some(ws.as_path()));
    assert!(
        !h.sessions.contains_key("s1"),
        "s1 reloads on its next click"
    );
    let _ = std::fs::remove_dir_all(&proj);
}

#[test]
fn request_failed_auth_sets_login_hint() {
    let (mut h, _agent) = setup(vec![], "ask");
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Initialize,
        error: rpc::RpcError {
            code: rpc::AUTH_REQUIRED,
            message: "authentication required".to_string(),
            data: None,
        },
    });
    assert_eq!(
        h.status,
        "Vibe is not signed in. Please start `vibe` once in a terminal and sign in."
    );
}

#[test]
fn add_project_validation_and_duplicates() {
    let (mut h, _agent) = setup(vec![], "ask");
    let before = h.projects.len();
    assert!(!h.add_project("/definitiv/nicht/vorhanden-xyz-123"));
    assert_eq!(h.projects.len(), before, "list unchanged on error");
    assert!(!h.status.is_empty(), "german error in status");

    let tmp = std::env::temp_dir();
    assert!(h.add_project(&tmp.to_string_lossy()));
    assert_eq!(h.projects.len(), 1);
    let canon = tmp.canonicalize().unwrap_or_else(|_| tmp.clone());
    assert_eq!(h.current_project, Some(canon.clone()));
    assert_eq!(h.projects[0], canon);

    assert!(
        h.add_project(&tmp.to_string_lossy()),
        "duplicate is accepted …"
    );
    assert_eq!(h.projects.len(), 1, "… but not added twice");
}

#[test]
fn format_tokens_cases() {
    assert_eq!(format_tokens(0), "0");
    assert_eq!(format_tokens(950), "950");
    assert_eq!(format_tokens(999), "999");
    assert_eq!(format_tokens(1_000), "1k");
    assert_eq!(format_tokens(121_400), "121k");
    assert_eq!(format_tokens(800_000), "800k");
    assert_eq!(format_tokens(999_999), "999k");
    assert_eq!(format_tokens(1_000_000), "1M");
    assert_eq!(format_tokens(1_250_000), "1.3M");
    assert_eq!(format_tokens(12_000_000), "12M");
}

#[test]
fn stop_reason_notes() {
    assert_eq!(stop_reason_note(&StopReason::EndTurn), None);
    assert_eq!(stop_reason_note(&StopReason::Cancelled), Some("Cancelled"));
    assert_eq!(
        stop_reason_note(&StopReason::MaxTokens),
        Some("Token limit reached")
    );
    assert_eq!(
        stop_reason_note(&StopReason::MaxTurnRequests),
        Some("Step limit reached")
    );
    assert_eq!(stop_reason_note(&StopReason::Refusal), Some("Refused"));
    assert_eq!(
        tr_in(Lang::De, "Token limit reached"),
        "Token-Limit erreicht"
    );
    assert_eq!(stop_reason_note(&StopReason::Other), None);
}

#[test]
fn filter_commands_rules() {
    let cmds: Vec<AvailableCommand> = ["compact", "help", "reload"]
        .iter()
        .map(|n| AvailableCommand {
            name: n.to_string(),
            description: format!("desc {n}"),
            input: None,
        })
        .collect();
    assert!(
        filter_commands(&cmds, "Hallo").is_empty(),
        "no slash -> none"
    );
    assert!(
        filter_commands(&cmds, "/help me").is_empty(),
        "space -> none"
    );
    let r = filter_commands(&cmds, "/co");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].name, "compact");
    let r = filter_commands(&cmds, "/");
    assert_eq!(r.len(), 3, "bare slash lists all");
    let r = filter_commands(&cmds, "/xyz");
    assert!(r.is_empty());
}

/// `session/list` pagination: an answer with `nextCursor` fetches the
/// next page; the page appends instead of replacing the first.
#[test]
fn session_list_pages_are_fetched_and_appended() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    let listed = |h: &Harness| {
        h.listed
            .get(&dir)
            .map(|s| s.iter().map(|i| i.session_id.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    h.handle_event(AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(dir.clone()),
        result: serde_json::from_value(json!({
            "sessions": [{"sessionId": "a", "cwd": dir.to_string_lossy(), "title": "Eins"}],
            "nextCursor": "page-2"
        }))
        .unwrap(),
    });
    assert_eq!(listed(&h), vec!["a"], "the first page is there");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
    assert_eq!(msg["params"]["cursor"], json!("page-2"));
    h.handle_event(AcpEvent::SessionsListed {
        cursor: Some("page-2".to_string()),
        cwd: Some(dir.clone()),
        result: serde_json::from_value(json!({
            "sessions": [{"sessionId": "b", "cwd": dir.to_string_lossy(), "title": "Zwei"}]
        }))
        .unwrap(),
    });
    assert_eq!(
        listed(&h),
        vec!["a", "b"],
        "the second page appends instead of replacing"
    );
    // a later refresh (no cursor) replaces the whole list again
    h.handle_event(AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(dir.clone()),
        result: serde_json::from_value(json!({
            "sessions": [{"sessionId": "c", "cwd": dir.to_string_lossy(), "title": "Drei"}]
        }))
        .unwrap(),
    });
    assert_eq!(listed(&h), vec!["c"]);
}

/// M4: the answer itself says whether it continues its own request —
/// a refresh's first page answering after the page of an older chain
/// replaces the list (the freshest full list wins), and the sessions of
/// a page can never extend a list twice: no duplicates are possible.
#[test]
fn session_list_answers_bind_to_their_request() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    let listed = |h: &Harness| {
        h.listed
            .get(&dir)
            .map(|s| s.iter().map(|i| i.session_id.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    // a paginated chain: page 1 answers, page 2 is requested
    h.handle_event(AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(dir.clone()),
        result: serde_json::from_value(json!({
            "sessions": [{"sessionId": "a", "cwd": dir.to_string_lossy()}],
            "nextCursor": "p2"
        }))
        .unwrap(),
    });
    assert_eq!(listed(&h), vec!["a"]);
    let msg = agent.read_msg();
    assert_eq!(msg["params"]["cursor"], json!("p2"));
    // a parallel refresh is in flight for the same cwd (its first page
    // request is out, too — refresh_list fired it)
    h.refresh_list(&dir);
    // the old chain's page 2 answers FIRST: it extends its own request
    h.handle_event(AcpEvent::SessionsListed {
        cursor: Some("p2".to_string()),
        cwd: Some(dir.clone()),
        result: serde_json::from_value(json!({
            "sessions": [{"sessionId": "b", "cwd": dir.to_string_lossy()}]
        }))
        .unwrap(),
    });
    assert_eq!(listed(&h), vec!["a", "b"]);
    // the refresh's first page answers LAST: it replaces — the old
    // chain's appended page is gone, and no [a, b, b] or [a, b, a] was
    // ever possible (the cursor marker of old confused the two requests)
    h.handle_event(AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(dir.clone()),
        result: serde_json::from_value(json!({
            "sessions": [{"sessionId": "a", "cwd": dir.to_string_lossy()}]
        }))
        .unwrap(),
    });
    assert_eq!(
        listed(&h),
        vec!["a"],
        "the last first page replaces: no duplicates survive"
    );
    // the refresh had more pages: they extend the same chain again
    h.handle_event(AcpEvent::SessionsListed {
        cursor: Some("r2".to_string()),
        cwd: Some(dir.clone()),
        result: serde_json::from_value(json!({
            "sessions": [{"sessionId": "b", "cwd": dir.to_string_lossy()}, {"sessionId": "c", "cwd": dir.to_string_lossy()}]
        }))
        .unwrap(),
    });
    assert_eq!(listed(&h), vec!["a", "b", "c"]);
}

/// Two parallel `session/list` chains of the same cwd interleave their
/// pages: both chains enumerate the same sessions, so the second chain's
/// page delivers sessions the first chain already extended onto the
/// list. Pages merge by session id (final audit A): the list cannot
/// hold a session twice, whatever the arrival order.
#[test]
fn parallel_session_list_pages_merge_without_duplicates() {
    let dir = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    let listed = |h: &Harness| {
        h.listed
            .get(&dir)
            .map(|s| s.iter().map(|i| i.session_id.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let listed_event = |cursor: Option<&str>, next: Option<&str>, ids: &[&str]| {
        let sessions: Vec<_> = ids
            .iter()
            .map(|id| json!({"sessionId": id, "cwd": dir.to_string_lossy()}))
            .collect();
        let mut result = json!({"sessions": sessions});
        if let Some(next) = next {
            result["nextCursor"] = json!(next);
        }
        AcpEvent::SessionsListed {
            cursor: cursor.map(str::to_string),
            cwd: Some(dir.clone()),
            result: serde_json::from_value(result).unwrap(),
        }
    };
    // two chains of the same cwd are in flight (two refreshes); their
    // first pages replace in arrival order, both ask for their page 2
    h.handle_event(listed_event(None, Some("a2"), &["s1"]));
    h.handle_event(listed_event(None, Some("b2"), &["s1"]));
    // chain A's page 2 extends, chain B's page 2 delivers the same
    // sessions: the merge keeps every session exactly once
    h.handle_event(listed_event(Some("a2"), None, &["s2", "s3"]));
    h.handle_event(listed_event(Some("b2"), None, &["s2", "s3"]));
    assert_eq!(
        listed(&h),
        vec!["s1", "s2", "s3"],
        "no chain's page overwrites with duplicates of its sibling"
    );
}

/// Without `promptCapabilities.image` PNG attachments go out as
/// `resource_link`s, never as `image` blocks the agent cannot read.
#[test]
fn png_attachments_follow_the_agents_image_capability() {
    let png = std::env::temp_dir().join(format!("harness-img-{}.png", std::process::id()));
    std::fs::write(&png, "not really a png, size is all that counts here").unwrap();
    let blocks = prompt_blocks("Schau", std::slice::from_ref(&png), false);
    assert!(
        matches!(&blocks[1], PromptBlock::ResourceLink { uri, .. } if uri.starts_with("file://")),
        "without the capability the PNG goes as a link"
    );
    let _ = std::fs::remove_file(&png);
}

#[test]
fn file_uri_percent_encodes_spaces_and_specials() {
    let uri = file_uri(Path::new("/tmp/mein ordner/datei (1).md"));
    assert_eq!(uri, "file:///tmp/mein%20ordner/datei%20%281%29.md");
}

/// A session the agent confirmed closed loses its local view.
#[test]
fn session_closed_drops_the_view() {
    let dir = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    h.sessions
        .insert("s1".to_string(), SessionView::new("s1", dir.clone()));
    h.active = Some("s1".to_string());
    h.handle_event(AcpEvent::SessionClosed {
        session_id: "s1".to_string(),
    });
    assert!(!h.sessions.contains_key("s1"));
    assert!(h.active.is_none(), "a closed session cannot stay on screen");
}

#[test]
fn dropped_files_attach_deduplicated_like_the_plus_picker() {
    let dir = std::env::temp_dir().join(format!("cw-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    let a = dir.join("a.png");
    let b = dir.join("notes.txt");
    std::fs::write(&a, "x").unwrap();
    std::fs::write(&b, "x").unwrap();
    // a drop may list several files, a repeated path attaches only once
    crate::harness::ui::composer::attach_dropped(&mut h, vec![a.clone(), b.clone(), a.clone()]);
    assert_eq!(h.attachments, vec![a.clone(), b.clone()]);
    // and they go out on send exactly like picker attachments
    let blocks = prompt_blocks("hi", &h.attachments, false);
    assert_eq!(blocks.len(), 3, "text first, then both files as links");
    assert!(matches!(blocks[1], PromptBlock::ResourceLink { .. }));
    assert!(matches!(blocks[2], PromptBlock::ResourceLink { .. }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompt_blocks_text_and_resource_links() {
    let f = std::env::temp_dir().join(format!("harness-pb-{}.txt", std::process::id()));
    std::fs::write(&f, "x").unwrap();

    let blocks = prompt_blocks("Hallo", &[], true);
    assert_eq!(blocks, vec![PromptBlock::Text("Hallo".to_string())]);

    let blocks = prompt_blocks("   ", std::slice::from_ref(&f), true);
    assert_eq!(blocks.len(), 1, "blank text is not sent");
    match &blocks[0] {
        PromptBlock::ResourceLink { uri, name } => {
            let abs = f.canonicalize().unwrap();
            assert_eq!(uri.as_str(), format!("file://{}", abs.to_string_lossy()));
            assert_eq!(
                name.as_str(),
                f.file_name().unwrap().to_string_lossy().as_ref()
            );
        }
        other => panic!("expected ResourceLink, got {other:?}"),
    }

    let blocks = prompt_blocks("Text", std::slice::from_ref(&f), true);
    assert_eq!(blocks.len(), 2, "text first, then the file");
    std::fs::remove_file(&f).ok();
}

#[test]
fn png_attachments_become_image_blocks() {
    let dir = std::env::temp_dir().join(format!("harness-png-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("foto.png");
    std::fs::write(&png, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
    let blocks = prompt_blocks("Schau", std::slice::from_ref(&png), true);
    assert_eq!(blocks.len(), 2);
    match &blocks[1] {
        PromptBlock::Image {
            data_base64,
            mime_type,
        } => {
            assert_eq!(mime_type, "image/png");
            assert_eq!(
                data_base64,
                "iVBORwECAw==" // base64 of the 7 header bytes
            );
        }
        other => panic!("expected Image, got {other:?}"),
    }

    // a PNG that is too large stays a link the agent reads itself
    let big = dir.join("big.png");
    std::fs::write(&big, vec![0u8; (MAX_IMAGE_ATTACHMENT_BYTES + 1) as usize]).unwrap();
    match &prompt_blocks("", std::slice::from_ref(&big), true)[0] {
        PromptBlock::ResourceLink { .. } => {}
        other => panic!("expected ResourceLink, got {other:?}"),
    }

    // so do a missing file and a non-PNG extension
    let missing = dir.join("weg.png");
    assert!(matches!(
        prompt_blocks("", std::slice::from_ref(&missing), true)[0],
        PromptBlock::ResourceLink { .. }
    ));
    let txt = dir.join("notiz.txt");
    std::fs::write(&txt, "x").unwrap();
    assert!(matches!(
        prompt_blocks("", std::slice::from_ref(&txt), true)[0],
        PromptBlock::ResourceLink { .. }
    ));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn base64_matches_rfc4648_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
}

#[test]
fn sidebar_search_matches_and_filters() {
    assert!(matches_query("Abendliches Feinarbeiten", ""));
    assert!(matches_query("Abendliches Feinarbeiten", "  "));
    assert!(matches_query("Abendliches Feinarbeiten", "FEIN"));
    assert!(matches_query("Abendliches Feinarbeiten", "arbeiten"));
    assert!(!matches_query("Abendliches Feinarbeiten", "Vermessung"));

    let entries = |titles: &[&str]| -> Vec<SidebarEntry> {
        titles
            .iter()
            .map(|t| SidebarEntry {
                session_id: t.to_string(),
                title: t.to_string(),
            })
            .collect()
    };
    let e = entries(&["Thumbnails", "Schnitt", "Animationen"]);
    // empty query: everything stays
    assert_eq!(filter_entries("YouTube", e.clone(), "").unwrap(), e);
    // folder matches: all sessions stay
    assert_eq!(filter_entries("YouTube", e.clone(), "you").unwrap(), e);
    // only matching session titles stay
    let kept = filter_entries("Werkstatt", e.clone(), "schnitt").unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].title, "Schnitt");
    // nothing matches: the folder is hidden
    assert!(filter_entries("Werkstatt", e, "vermessung").is_none());
}

#[test]
fn project_name_last_component() {
    assert_eq!(project_name(Path::new("/home/x/proj")), "proj");
    assert_eq!(project_name(Path::new("rel/dir")), "dir");
    assert_eq!(project_name(Path::new("/")), "/");
}

#[test]
fn ui_button_click_is_sent_as_prompt_when_idle() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    assert_eq!(h.sessions["s1"].turn, TurnState::Idle);

    h.ui_events.push(UiEvent::Button {
        action: "details".to_string(),
    });
    h.handle_ui_events();
    assert!(h.ui_events.is_empty(), "events are consumed");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "[UI] Button \"details\" clicked."})
    );
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);
}

#[test]
fn ui_form_submit_is_sent_with_values() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");

    h.ui_events.push(UiEvent::Form {
        submit: "save".to_string(),
        values: json!({"name": "Ada", "rate": 2.5}),
    });
    h.handle_ui_events();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({
            "type": "text",
            "text": "[UI] Form \"save\" submitted: {\"name\":\"Ada\",\"rate\":2.5}"
        })
    );
}

/// Handoff 2.8.3: several events of one frame go out as ONE
/// `session/prompt` — a text block per event, in event order — instead
/// of a second parallel prompt on the same session.
#[test]
fn two_ui_events_of_one_frame_go_out_as_one_prompt() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");

    h.ui_events.push(UiEvent::Button {
        action: "details".to_string(),
    });
    h.ui_events.push(UiEvent::Form {
        submit: "save".to_string(),
        values: json!({"name": "Ada"}),
    });
    h.handle_ui_events();
    assert!(h.ui_events.is_empty(), "events are consumed");

    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(
        msg["params"]["prompt"],
        json!([
            {"type": "text", "text": "[UI] Button \"details\" clicked."},
            {"type": "text", "text": "[UI] Form \"save\" submitted: {\"name\":\"Ada\"}"}
        ]),
        "one prompt carries both interactions, in event order"
    );
    assert_eq!(h.sessions["s1"].turn, TurnState::Running);
}

/// Handoff 2.8.4: closing a project closes its sessions — the views,
/// the listed entries and `active` pointing at them — and leaves the
/// other projects untouched.
#[test]
fn remove_project_closes_its_sessions_and_active() {
    let base = std::env::temp_dir().canonicalize().unwrap();
    let a = base.join(format!("od-rm-a-{}", std::process::id()));
    let b = base.join(format!("od-rm-b-{}", std::process::id()));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut h, mut agent) = setup(vec![a.clone(), b.clone()], "ask");
    create_session(&mut h, &mut agent, &a, "s-a");
    create_session(&mut h, &mut agent, &b, "s-b");
    h.handle_event(AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(a.clone()),
        result: sessions_listed(json!([{
            "sessionId": "l-a", "cwd": a.to_string_lossy(), "title": "Liste A"
        }])),
    });
    h.handle_event(AcpEvent::SessionsListed {
        cursor: None,
        cwd: Some(b.clone()),
        result: sessions_listed(json!([{
            "sessionId": "l-b", "cwd": b.to_string_lossy(), "title": "Liste B"
        }])),
    });
    // s-a stays open in the background (was: the parked view's
    // session); the view shows s-b
    assert!(h.sessions.contains_key("s-a"));
    assert_eq!(h.active.as_deref(), Some("s-b"));

    h.remove_project(&a);

    assert_eq!(h.projects, vec![b.clone()]);
    assert!(
        !h.sessions.contains_key("s-a"),
        "views of the removed project"
    );
    assert!(!h.workspaces().contains(&a), "gone from the sidebar");
    assert!(h.sessions.contains_key("s-b"), "the other session stays");
    assert!(!h.listed.contains_key(&a), "list of the removed project");
    assert!(h.listed.contains_key(&b), "the other list stays");
    assert_eq!(
        h.active.as_deref(),
        Some("s-b"),
        "active survives: its session still exists"
    );
    assert_eq!(h.current_project.as_deref(), Some(b.as_path()));

    // removing the project of the visible session takes active with it
    h.remove_project(&b);
    assert!(h.sessions.is_empty());
    assert!(h.active.is_none());
    assert_eq!(h.current_project, None);

    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// Handoff 2.5: every per-interaction state key starts with the owning
/// session id, and a session that goes away takes its entries with it
/// (remove_project here) while another session's state survives.
#[test]
fn remove_project_drops_only_that_projects_ui_state() {
    let base = std::env::temp_dir().canonicalize().unwrap();
    let a = base.join(format!("od-ui-a-{}", std::process::id()));
    let b = base.join(format!("od-ui-b-{}", std::process::id()));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut h, mut agent) = setup(vec![a.clone(), b.clone()], "ask");
    create_session(&mut h, &mut agent, &a, "s-a");
    create_session(&mut h, &mut agent, &b, "s-b");

    // interaction state of both sessions, keyed the way rendering keys
    // it: owner (session id) first
    h.form_values
        .insert(render::field_key("s-a", 1, 0, "rate"), "7.5".into());
    h.form_values
        .insert(render::field_key("s-b", 1, 0, "rate"), "2.5".into());
    let code = "function render(s){return null}";
    h.scripts.insert(
        render::script_key("s-a", 1, 0, code),
        crate::script::ScriptTool::new(code),
    );
    h.scripts.insert(
        render::script_key("s-b", 1, 0, code),
        crate::script::ScriptTool::new(code),
    );
    // "<sid>#<group start>" and "<sid>#<toolCallId>::<path>"
    // (render_tool_group / render_diff_chip)
    h.open_groups.insert("s-a#3".into());
    h.open_groups.insert("s-b#3".into());
    h.diff_open.insert("s-a#t1::f.rs".into());
    h.diff_open.insert("s-b#t2::f.rs".into());
    h.ctx_sizes.insert("s-a".into(), (1, None));
    h.ctx_sizes.insert("s-b".into(), (1, None));

    h.remove_project(&a);

    // the removed project's session state is gone everywhere
    assert!(!h
        .form_values
        .contains_key(&render::field_key("s-a", 1, 0, "rate")));
    assert!(!h
        .scripts
        .contains_key(&render::script_key("s-a", 1, 0, code)));
    assert!(!h.open_groups.contains("s-a#3"));
    assert!(!h.diff_open.contains("s-a#t1::f.rs"));
    assert!(!h.ctx_sizes.contains_key("s-a"));
    // the other project's session keeps its own state
    assert_eq!(
        h.form_values
            .get(&render::field_key("s-b", 1, 0, "rate"))
            .map(String::as_str),
        Some("2.5")
    );
    assert!(h
        .scripts
        .contains_key(&render::script_key("s-b", 1, 0, code)));
    assert!(h.open_groups.contains("s-b#3"));
    assert!(h.diff_open.contains("s-b#t2::f.rs"));
    assert!(h.ctx_sizes.contains_key("s-b"));
    // nothing but s-b's entries is left in any of the maps
    assert!(h.form_values.keys().all(|k| k.starts_with("s-b#")));
    assert!(h.scripts.keys().all(|k| k.starts_with("s-b#")));
    assert!(h.open_groups.iter().all(|k| k.starts_with("s-b#")));
    assert!(h.diff_open.iter().all(|k| k.starts_with("s-b#")));

    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// M7: removing a project really closes its sessions. For every view of
/// the folder the open approvals get the "cancelled" answer, the running
/// turn a `session/cancel`, then `session/close` — all before the local
/// views go, so nothing of the folder keeps running in the background.
#[test]
fn remove_project_cancels_and_closes_the_sessions_of_the_folder() {
    let base = std::env::temp_dir().canonicalize().unwrap();
    let a = base.join(format!("od-close-a-{}", std::process::id()));
    let b = base.join(format!("od-close-b-{}", std::process::id()));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (mut h, mut agent) = setup(vec![a.clone(), b.clone()], "ask");
    // two sessions of project a: one with an open approval, one running
    create_session(&mut h, &mut agent, &a, "s-a1");
    create_session(&mut h, &mut agent, &a, "s-a2");
    create_session(&mut h, &mut agent, &b, "s-b");
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s-a1"),
    });
    h.sessions.get_mut("s-a2").unwrap().begin_turn();

    h.remove_project(&a);

    // five messages reach the agent, grouped per session (the order of
    // the sessions themselves is the map's): each session's approval
    // answer comes first, then cancel, then close
    let mut msgs = Vec::new();
    for _ in 0..5 {
        msgs.push(agent.read_msg());
    }
    let of = |sid: &str, method: &str| {
        msgs.iter()
            .filter(|m| m["method"] == json!(method) && m["params"]["sessionId"] == json!(sid))
            .count()
    };
    assert_eq!(
        of("s-a1", "session/cancel"),
        1,
        "s-a1's running turn is cancelled"
    );
    assert_eq!(of("s-a2", "session/cancel"), 1);
    assert_eq!(of("s-a1", "session/close"), 1);
    assert_eq!(of("s-a2", "session/close"), 1);
    assert_eq!(
        of("s-b", "session/cancel") + of("s-b", "session/close"),
        0,
        "the other project's session is never touched"
    );
    let answer = msgs
        .iter()
        .find(|m| m.get("method").is_none() && m.get("result").is_some())
        .expect("the approval answer goes out");
    assert_eq!(
        answer,
        &json!({"jsonrpc": "2.0", "id": 0, "result": {"outcome": {"outcome": "cancelled"}}}),
        "the orphaned approval is answered, not left hanging"
    );
    let pos = |m: &Value| msgs.iter().position(|x| x == m).unwrap();
    let find = |method: &str, sid: &str| {
        msgs.iter()
            .position(|m| m["method"] == json!(method) && m["params"]["sessionId"] == json!(sid))
            .unwrap_or(usize::MAX)
    };
    assert!(
        find("session/cancel", "s-a1") < find("session/close", "s-a1"),
        "cancel precedes close for s-a1"
    );
    assert!(
        find("session/cancel", "s-a2") < find("session/close", "s-a2"),
        "cancel precedes close for s-a2"
    );
    assert!(
        pos(answer) < find("session/cancel", "s-a1"),
        "the approval is answered before anything is cancelled"
    );
    // both views are gone, the other project's session is untouched
    assert!(!h.sessions.contains_key("s-a1"));
    assert!(!h.sessions.contains_key("s-a2"));
    assert!(h.sessions.contains_key("s-b"));

    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// Handoff 2.5: a failed `session/load` drops only that session's UI
/// state; a lost connection drops it for every session, because no view
/// survives the restart (a ```ui-js tool is evaluated again when its
/// session comes back).
#[test]
fn failed_load_and_connection_loss_drop_session_ui_state() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s-keep");
    let code = "function render(s){return null}";
    h.form_values
        .insert(render::field_key("s-keep", 1, 0, "rate"), "1".into());
    h.ctx_sizes.insert("s-keep".into(), (1, None));
    h.form_values
        .insert(render::field_key("s-gone", 1, 0, "rate"), "2".into());
    h.scripts.insert(
        render::script_key("s-gone", 1, 0, code),
        crate::script::ScriptTool::new(code),
    );
    h.open_groups.insert("s-gone#2".into());
    h.diff_open.insert("s-gone#t1::f.rs".into());

    // session/load fails (deleted session, child id)
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::LoadSession {
            session_id: "s-gone".to_string(),
        },
        error: rpc::RpcError {
            code: -31002,
            message: "A subagent Session cannot be resumed independently".to_string(),
            data: None,
        },
    });
    assert!(
        !h.form_values
            .contains_key(&render::field_key("s-gone", 1, 0, "rate")),
        "the dropped session's form state is gone"
    );
    assert!(h.scripts.is_empty(), "its boa context is gone");
    assert!(!h.open_groups.contains("s-gone#2"));
    assert!(!h.diff_open.contains("s-gone#t1::f.rs"));
    assert!(
        h.form_values
            .contains_key(&render::field_key("s-keep", 1, 0, "rate")),
        "the surviving session keeps its state"
    );

    // the connection dies: no view survives, so no UI state does either
    h.handle_event(exited());
    assert!(h.form_values.is_empty());
    assert!(h.scripts.is_empty());
    assert!(h.open_groups.is_empty());
    assert!(h.diff_open.is_empty());
    assert!(h.ctx_sizes.is_empty());
}

/// N6: a session that ends for good takes its parked draft, its armed
/// UI hint and the prompt not yet echoed back with it — nothing of a
/// dead session id may ghost-restore into a later view.
#[test]
fn a_closed_session_drops_its_parked_draft_and_hint() {
    let dir = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    h.sessions
        .insert("s1".to_string(), SessionView::new("s1", dir.clone()));
    h.ui.parked_drafts
        .insert(Some("s1".to_string()), ("Geparkt".to_string(), Vec::new()));
    h.hint_pending.insert("s1".to_string());
    h.sent_prompts.insert(
        "s1".to_string(),
        vec![PromptBlock::Text("Nicht echoiert".to_string())],
    );

    h.handle_event(AcpEvent::SessionClosed {
        session_id: "s1".to_string(),
    });
    assert!(
        !h.ui.parked_drafts.contains_key(&Some("s1".to_string())),
        "the parked draft does not outlive its session"
    );
    assert!(!h.hint_pending.contains("s1"));
    assert!(!h.sent_prompts.contains_key("s1"));
    assert!(h.sessions.is_empty() && h.active.is_none());
}

/// N6: a prompt whose session view died in between (closed, deleted)
/// has nothing to restore — the late failure parks no ghost draft
/// under the dead id.
#[test]
fn late_prompt_failure_of_a_dropped_session_parks_nothing() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.send_prompt("s1", &[PromptBlock::Text("Verschickt".to_string())]);
    assert!(agent.read_msg()["params"]["sessionId"] == json!("s1"));

    h.handle_event(AcpEvent::SessionClosed {
        session_id: "s1".to_string(),
    });
    assert!(!h.sessions.contains_key("s1"));
    assert!(
        !h.sent_prompts.contains_key("s1"),
        "sent Prompts went with the view"
    );

    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "internal error".to_string(),
            data: None,
        },
    });
    assert!(
        h.ui.parked_drafts.is_empty(),
        "no text parks under the dead session id: {:?}",
        h.ui.parked_drafts
    );
    assert!(h.draft.is_empty(), "the global composer stays untouched");
}

/// N6: a view dropped for a reload (connection lost) is not a session
/// death — its parked draft and hint must survive for the same id that
/// `session/load` brings back.
#[test]
fn connection_loss_keeps_the_parked_drafts_for_the_reload() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.ui.parked_drafts
        .insert(Some("s1".to_string()), ("Getippt".to_string(), Vec::new()));
    h.hint_pending.insert("s1".to_string());

    h.handle_event(exited());
    assert_eq!(
        h.ui.parked_drafts
            .get(&Some("s1".to_string()))
            .map(|(t, _)| t.clone())
            .unwrap_or_default(),
        "Getippt",
        "the reload finds the typed text where it belongs"
    );
    assert!(
        h.hint_pending.contains("s1"),
        "the hint survives until the session prompt arrives"
    );
}

#[test]
fn ui_events_are_dropped_while_a_turn_runs() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.sessions.get_mut("s1").unwrap().begin_turn();

    h.ui_events.push(UiEvent::Button {
        action: "ok".to_string(),
    });
    h.handle_ui_events();
    assert!(h.ui_events.is_empty());
    assert_eq!(
        h.status, "Vibe is still working – interaction discarded.",
        "the user sees that nothing was sent"
    );
    // nothing went out: finish the turn, then the next wire message is
    // the prompt we trigger below
    h.sessions.get_mut("s1").unwrap().turn = TurnState::Idle;
    h.draft = "Hallo".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(
        msg["method"], "session/prompt",
        "no [UI] prompt was sent before"
    );
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "Hallo"})
    );
}

#[test]
fn ui_events_without_session_are_dropped_with_status() {
    let (mut h, _agent) = setup(vec![], "ask");
    h.ui_events.push(UiEvent::Button {
        action: "ok".to_string(),
    });
    h.handle_ui_events();
    assert!(h.ui_events.is_empty());
    assert!(!h.status.is_empty(), "german status explains the drop");
}

#[test]
fn first_prompt_of_a_new_session_carries_the_ui_hint_once() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.ui_hint = true;
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");

    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result("s1"),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": UI_HINT})
    );
    assert_eq!(
        msg["params"]["prompt"][1],
        json!({"type": "text", "text": "Hallo Vibe"})
    );
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");

    // the second prompt of the same session has no hint
    h.sessions.get_mut("s1").unwrap().turn = TurnState::Idle;
    h.draft = "Nochmal".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "Nochmal"})
    );
    assert_eq!(msg["params"]["prompt"].as_array().unwrap().len(), 1);
}

#[test]
fn no_ui_hint_without_the_skill() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.ui_hint = false;
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result("s1"),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "Hallo Vibe"})
    );
}

#[test]
fn loaded_session_gets_no_ui_hint() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.ui_hint = true;
    h.open_session(dir.clone(), "old");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/load");
    h.draft = "Weiter".to_string();
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "Weiter"}),
        "a loaded session is not new: no hint"
    );
}

/// `updatedAt` sorts the sidebar: newest first, sessions without a
/// timestamp last.
/// Pinned sessions sort to the top of their workspace's list, keeping
/// time order within both groups.
/// Clicked paths open in the Artefakte preview; relative ones resolve
/// against the session's workspace (Handoff Ideenliste 5).
/// Worktrees (Handoff Ideenliste 6): the directory naming picks the
/// smallest free counter, and a real `git worktree add` produces a
/// working copy that becomes its own workspace.
#[test]
fn worktree_dir_picks_the_smallest_free_counter() {
    let base = std::env::temp_dir().join(format!("od-wt-base-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let project = base.join("repo");
    assert_eq!(
        Harness::worktree_dir(&base, &project),
        base.join("worktrees").join("repo-1")
    );
    std::fs::create_dir_all(base.join("worktrees").join("repo-1")).unwrap();
    assert_eq!(
        Harness::worktree_dir(&base, &project),
        base.join("worktrees").join("repo-2")
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn create_worktree_adds_the_copy_as_its_own_workspace() {
    let base = std::env::temp_dir().join(format!(
        "od-wt-{}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(crate::harness::watch::git_output(&repo, &["init", "-q"]).is_some());
    std::fs::write(repo.join("datei.txt"), "inhalt").unwrap();
    assert!(crate::harness::watch::git_output(&repo, &["add", "."]).is_some());
    assert!(crate::harness::watch::git_output(
        &repo,
        &[
            // the fixture commits without a global git identity
            "-c",
            "user.name=Catwright",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "init",
            "--allow-empty"
        ]
    )
    .is_some());
    let (mut h, _agent) = setup(vec![], "ask");
    h.current_project = Some(repo.clone());
    h.create_worktree(None);
    assert!(h.worktree_running(), "the checkout runs in the background");
    for _ in 0..1500 {
        h.poll();
        if !h.worktree_running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!h.worktree_running(), "the checkout finished");
    assert!(h.status.contains("Worktree"), "{}", h.status);
    let wt = h
        .projects
        .iter()
        .find(|p| p.to_string_lossy().contains("worktrees"))
        .expect("the worktree is a project now")
        .clone();
    assert!(wt.join("datei.txt").exists(), "the checkout is complete");
    assert_eq!(h.current_project.as_deref(), Some(wt.as_path()));
    assert!(crate::harness::watch::git_output(&wt, &["status"]).is_some());
    // the worktree lands in the real config dir, not below `base`
    let _ = std::fs::remove_dir_all(&wt);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn open_preview_resolves_relative_paths_against_the_session() {
    let dir = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    h.sessions
        .insert("s1".to_string(), SessionView::new("s1", dir.clone()));
    h.open_preview("s1", "src/main.rs");
    assert_eq!(
        h.ui.preview.as_ref().map(|p| p.path.as_str()),
        Some(dir.join("src/main.rs").to_string_lossy().as_ref()),
        "a relative path joins the session's workspace"
    );
    let abs = dir.join("abs.txt");
    h.open_preview("s1", abs.to_string_lossy().as_ref());
    assert_eq!(
        h.ui.preview.as_ref().map(|p| p.path.as_str()),
        Some(abs.to_string_lossy().as_ref())
    );
}

#[test]
fn sidebar_entries_sort_pinned_first() {
    let listed = vec![
        serde_json::from_value(
            json!({"sessionId": "a", "cwd": "/x", "updatedAt": "2026-11-10T10:00:00Z"}),
        )
        .unwrap(),
        serde_json::from_value(
            json!({"sessionId": "b", "cwd": "/x", "updatedAt": "2026-11-10T11:00:00Z"}),
        )
        .unwrap(),
        serde_json::from_value(
            json!({"sessionId": "c", "cwd": "/x", "updatedAt": "2026-11-10T09:00:00Z"}),
        )
        .unwrap(),
    ];
    let pinned: HashSet<String> = ["c".to_string()].into_iter().collect();
    let rows = sidebar_entries(&listed, None, "Neuer Chat", &pinned);
    let ids: Vec<&str> = rows.iter().map(|r| r.session_id.as_str()).collect();
    assert_eq!(ids, ["c", "b", "a"], "the pinned session leads");
    let (mut h, _agent) = setup(vec![], "ask");
    h.toggle_pinned("s1");
    assert!(h.pinned.contains("s1"));
    h.toggle_pinned("s1");
    assert!(h.pinned.is_empty(), "toggling again unpins");
}

#[test]
fn sidebar_entries_sort_by_updated_at() {
    let listed: Vec<SessionInfo> = vec![
        serde_json::from_value(json!({"sessionId": "old", "cwd": "/x", "title": "Alt",
                "updatedAt": "2026-11-01T10:00:00Z"}))
        .unwrap(),
        serde_json::from_value(json!({"sessionId": "new", "cwd": "/x", "title": "Neu",
                "updatedAt": "2026-11-07T22:00:00Z"}))
        .unwrap(),
        serde_json::from_value(json!({"sessionId": "none", "cwd": "/x", "title": "Ohne"})).unwrap(),
    ];
    let rows = sidebar_entries(&listed, None, "Neuer Chat", &HashSet::new());
    assert_eq!(rows[0].session_id, "new");
    assert_eq!(rows[1].session_id, "old");
    assert_eq!(rows[2].session_id, "none");
}

#[test]
fn permission_request_lands_in_the_activity_feed_as_waiting() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });
    let ev = h.activity.last().expect("an activity entry");
    assert_eq!(ev.kind, ActivityKind::Waiting);
    assert_eq!(ev.sid, "s1");
    assert_eq!(ev.request_id, Some(json!(0)), "the feed can answer it");
}

#[test]
fn activity_feed_is_bounded_to_the_newest_100() {
    let (mut h, _agent) = setup(vec![], "ask");
    for i in 0..120 {
        h.push_activity(ActivityKind::Finished, "s", format!("t{i}"), None);
    }
    assert_eq!(h.activity.len(), 100);
    assert_eq!(h.activity[0].text, "t20", "the oldest entries are dropped");
}

#[test]
fn archived_sessions_leave_the_list_and_get_their_own_group() {
    let dir = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    h.listed.insert(
        dir.clone(),
        vec![
            serde_json::from_value(json!({"sessionId": "a", "cwd": dir.to_string_lossy()}))
                .unwrap(),
            serde_json::from_value(json!({"sessionId": "b", "cwd": dir.to_string_lossy()}))
                .unwrap(),
        ],
    );
    h.archive_session("a");
    let rows = h.sidebar_rows(&dir, "New session");
    let ids: Vec<&str> = rows.iter().map(|r| r.session_id.as_str()).collect();
    assert_eq!(ids, ["b"], "archived sessions leave the list");
    let archived = h.sidebar_rows_archived(&dir, "New session");
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].session_id, "a");
    h.unarchive_session("a");
    assert_eq!(h.sidebar_rows(&dir, "New session").len(), 2);
    // archiving clears the unread mark
    h.mark_unread("b");
    assert_eq!(h.session_state("b"), SessionState::Unseen);
    h.archive_session("b");
    assert_eq!(h.session_state("b"), SessionState::Idle);
}

/// `session/fork`: the request carries the source id, the new view
/// shows the source transcript and becomes active.
#[test]
fn fork_copies_the_transcript_into_a_new_active_session() {
    let dir = std::env::temp_dir().join(format!("od-fork-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    let mut v = SessionView::new("s1", dir.clone());
    v.apply_update(&user_chunk("m1", "Frage"));
    v.apply_update(&agent_chunk("m2", "Antwort"));
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());

    h.fork_active_session();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/fork");
    // the field vibe-acp actually accepts (`forkSessionId` was rejected
    // with -32602, verified against Vibe 3.1.0)
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(msg["params"]["cwd"], json!(dir.to_string_lossy()));
    assert!(h.fork_in_flight());

    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::Fork {
            source: "s1".to_string(),
            message_id: None,
        },
        cwd: dir.clone(),
        result: new_session_result("fork-1"),
    });
    assert!(!h.fork_in_flight());
    assert_eq!(h.active.as_deref(), Some("fork-1"));
    let forked = h.sessions.get("fork-1").expect("fork view exists");
    assert_eq!(forked.items.len(), 2, "the transcript carries over");
    assert_eq!(forked.turn, TurnState::Idle);
    // the source stays untouched
    assert!(h.sessions.contains_key("s1"));
}

/// Bug 2 (Handoff 2026-11-09): the fork answer is identified by its
/// origin, not by "the next SessionCreated that happens to arrive".
/// A racing `session/new` of another view must neither steal the fork
/// nor leave a queued prompt stuck forever.
#[test]
fn fork_answer_is_not_stolen_by_a_racing_session_new() {
    let dir = std::env::temp_dir().join(format!("od-fork-race-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    let mut v = SessionView::new("s1", dir.clone());
    v.apply_update(&user_chunk("m1", "Frage"));
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());

    h.fork_active_session();
    assert_eq!(agent.read_msg()["method"], "session/fork");
    assert!(h.fork_in_flight());

    // another session/new answers FIRST: it becomes its own view, the
    // fork stays pending
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result("other-1"),
    });
    assert!(
        h.fork_in_flight(),
        "the racing answer did not consume the fork"
    );
    assert!(h.sessions.contains_key("other-1"));

    // the fork's own answer arrives: the copy lands in its own view
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::Fork {
            source: "s1".to_string(),
            message_id: None,
        },
        cwd: dir.clone(),
        result: new_session_result("fork-1"),
    });
    assert!(!h.fork_in_flight());
    assert_eq!(h.active.as_deref(), Some("fork-1"));
    assert_eq!(
        h.sessions.get("fork-1").unwrap().items.len(),
        1,
        "the transcript carries over"
    );
}

/// Bug 2: `vibe-acp` dying during a fork must not turn the next probe
/// answer into a visible "fork" after the restart.
#[test]
fn connection_lost_clears_the_pending_fork() {
    let dir = std::env::temp_dir().join(format!("od-fork-lost-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    let v = SessionView::new("s1", dir.clone());
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());
    h.fork_active_session();
    assert!(h.fork_in_flight());
    h.handle_event(exited());
    assert!(!h.fork_in_flight(), "the dead fork request is gone");
}

/// A turn in progress refuses the fork; a failed `session/fork` clears
/// the pending marker and leaves the source active.
#[test]
fn fork_guards_and_failure() {
    let dir = std::env::temp_dir().join(format!("od-forkfail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    let mut v = SessionView::new("s1", dir.clone());
    v.begin_turn();
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());
    h.fork_active_session();
    assert!(!h.fork_in_flight(), "a running turn refuses the fork");
    assert!(h.status.contains("answer is running"));

    h.sessions.get_mut("s1").unwrap().turn = TurnState::Idle;
    h.fork_active_session();
    assert!(h.fork_in_flight());
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::ForkSession {
            cwd: dir.clone(),
            fork_session_id: "s1".to_string(),
            message_id: None,
        },
        error: crate::acp::rpc::RpcError {
            code: -32601,
            message: "Method not found".to_string(),
            data: None,
        },
    });
    assert!(!h.fork_in_flight());
    assert!(h.status.contains("Fork failed"));
    assert_eq!(h.active.as_deref(), Some("s1"));
}

/// N1: the fork's answer arrives after its source view was dropped
/// (closed, deleted) — the fork session exists server-side, so it
/// becomes a real, active view instead of the silent new-session path,
/// and the status says why the transcript did not carry over.
#[test]
fn fork_answer_without_the_source_view_opens_an_honest_empty_fork() {
    let dir = std::env::temp_dir().join(format!("od-fork-gone-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    // the fork was sent from "s1", but that view is gone when the
    // answer arrives
    h.fork_pending = Some(("s1".to_string(), None));

    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::Fork {
            source: "s1".to_string(),
            message_id: None,
        },
        cwd: dir.clone(),
        result: new_session_result("fork-1"),
    });
    assert!(!h.fork_in_flight(), "the pending fork is consumed");
    assert!(
        h.sessions.contains_key("fork-1") && h.sessions["fork-1"].items.is_empty(),
        "the fork becomes a visible view — empty, but not absent"
    );
    assert_eq!(h.active.as_deref(), Some("fork-1"), "the fork is active");
    assert!(
        h.status.contains("source session no longer exists"),
        "the status tells why the transcript is empty: {}",
        h.status
    );
    // the fork is not a fresh session: no [Catwright] hint is armed
    assert!(!h.hint_pending.contains("fork-1"));
    assert_eq!(agent.read_msg()["method"], "session/list", "list refreshed");
}

/// A message typed during a running turn waits and goes out by itself
/// when the turn ends.
#[test]
fn busy_send_queues_and_goes_out_after_the_turn() {
    let dir = std::env::temp_dir().join(format!("od-queue-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    let mut v = SessionView::new("s1", dir.clone());
    v.begin_turn();
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());
    h.draft = "Danach bitte das".to_string();
    h.send();
    assert_eq!(h.queue["s1"].len(), 1);
    assert!(h.draft.is_empty(), "the composer is free again");
    h.handle_event(AcpEvent::PromptFinished {
        session_id: "s1".to_string(),
        result: serde_json::from_value(json!({"stopReason": "end_turn"})).unwrap(),
    });
    // session/list, then the extras refresh, then the queued prompt
    let mut prompted = false;
    for _ in 0..6 {
        let m = agent.read_msg();
        if m["method"] == "session/prompt" {
            assert_eq!(m["params"]["prompt"][0]["text"], json!("Danach bitte das"));
            prompted = true;
            break;
        }
    }
    assert!(prompted, "the queued message was sent");
    assert!(h.queue["s1"].is_empty());
}

/// Stop means stop: queued messages return to the composer.
#[test]
fn cancelled_turn_returns_the_queue_to_the_composer() {
    let dir = std::env::temp_dir().join(format!("od-queue2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    let mut v = SessionView::new("s1", dir.clone());
    v.begin_turn();
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());
    h.draft = "eins".to_string();
    h.send();
    h.draft = "zwei".to_string();
    h.send();
    h.handle_event(AcpEvent::PromptFinished {
        session_id: "s1".to_string(),
        result: serde_json::from_value(json!({"stopReason": "cancelled"})).unwrap(),
    });
    assert_eq!(h.draft, "eins\n\nzwei");
    assert!(!h.queue.contains_key("s1"));
}

/// N8: `run_command` waits like `send` while a prompt is held back for
/// the start mode — no second prompt may run in parallel. The command
/// is reported, not sent and not queued.
#[test]
fn run_command_waits_while_the_start_mode_prompt_is_held() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.draft = "Hallo Vibe".to_string();
    h.send();
    assert_eq!(agent.read_msg()["method"], "session/new");
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result_with_modes("s1", "accept-edits"),
    });
    assert_eq!(
        agent.read_msg()["method"],
        "session/set_mode",
        "the setup step went out first"
    );
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/list");
    assert!(h.pending_mode_prompt.is_some(), "the first prompt is held");

    h.run_command("/compact");
    assert_eq!(
        h.status, "Vibe is still working – please wait or stop it.",
        "the command is refused with the same words as `send`"
    );
    assert!(
        h.pending_mode_prompt.is_some(),
        "the held prompt is untouched"
    );
    assert!(!h.queue.contains_key("s1"), "nothing was queued silently");
    assert_eq!(h.sessions["s1"].turn, TurnState::Idle, "no prompt was sent");
    // nothing went over the wire for the command: the next message is
    // the prompt the held setup releases
    h.handle_event(AcpEvent::ModeSet {
        session_id: "s1".to_string(),
        mode_id: "ask".to_string(),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["prompt"][0]["text"], json!("Hallo Vibe"));
}

/// N8, the guard's neighbor: a running turn still queues the command —
/// the refusal only applies to the paths where a parallel prompt could
/// actually start (session/new in flight, prompt held for the mode).
#[test]
fn run_command_queues_while_the_turn_runs() {
    let dir = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    let mut v = SessionView::new("s1", dir.clone());
    v.begin_turn();
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());
    h.run_command("/compact");
    assert_eq!(
        h.queue["s1"]
            .iter()
            .map(|q| q.text.clone())
            .collect::<Vec<_>>(),
        vec!["/compact".to_string()],
        "the command is the next message of the running session"
    );
    assert!(h.status.is_empty(), "no refusal while the turn runs");
}

#[test]
fn rewind_preview_opens_the_dialog_and_rewind_truncates() {
    let dir = std::env::temp_dir().join(format!("od-rewind-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    let mut v = SessionView::new("s1", dir.clone());
    v.apply_update(&user_chunk("m1", "Erste Frage"));
    v.apply_update(&agent_chunk("a1", "Antwort"));
    v.apply_update(&user_chunk("m2", "Zweite Frage"));
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());

    h.begin_rewind("s1", "m2");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "_rewind/preview");
    assert_eq!(msg["params"]["messageId"], json!("m2"));
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::RewindPreview {
            session_id: "s1".to_string(),
            message_id: "m2".to_string(),
        },
        result: json!({"paths": ["/w/a.txt"]}),
    });
    assert_eq!(h.ui.rewind.as_ref().unwrap().paths, vec!["/w/a.txt"]);

    h.confirm_rewind("s1", "m2", true);
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "_rewind/to");
    assert_eq!(msg["params"]["restoreFiles"], json!(true));
    h.handle_event(AcpEvent::ExtResult {
            kind: ExtKind::RewindTo {
                session_id: "s1".to_string(),
                message_id: "m2".to_string(),
                restore_files: true,
            },
            result: json!({"messageContent": "Zweite Frage", "restoreErrors": [], "restoredPaths": ["/w/a.txt"]}),
        });
    assert_eq!(h.sessions["s1"].items.len(), 2);
    assert_eq!(
        h.draft, "Zweite Frage",
        "the message comes back to the composer"
    );
    assert!(h.status.contains("1 file"));
}

#[test]
fn delete_removes_the_session_everywhere() {
    let dir = std::env::temp_dir().join(format!("od-del-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.sessions
        .insert("s1".to_string(), SessionView::new("s1", dir.clone()));
    h.active = Some("s1".to_string());
    h.listed.insert(
        dir.clone(),
        serde_json::from_value(json!([{"sessionId": "s1", "cwd": dir.to_string_lossy()}])).unwrap(),
    );
    h.delete_session("s1", &dir);
    assert_eq!(agent.read_msg()["method"], "_session/delete");
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::DeleteSession {
            session_id: "s1".to_string(),
            cwd: dir.clone(),
        },
        result: json!({}),
    });
    assert!(!h.sessions.contains_key("s1"));
    assert_eq!(h.active, None);
    assert!(h.listed[&dir].is_empty());
}

#[test]
fn a_running_session_cannot_be_deleted() {
    let (mut h, _agent) = setup(vec![], "ask");
    let mut v = SessionView::new("s1", "/w");
    v.begin_turn();
    h.sessions.insert("s1".to_string(), v);
    h.delete_session("s1", Path::new("/w"));
    assert!(h.status.contains("not possible"));
    assert!(h.sessions.contains_key("s1"));
}

#[test]
fn failed_probe_does_not_leave_the_account_loading() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.set_chat_dir(dir.clone());
    h.load_account();
    assert_eq!(agent.read_msg()["method"], "session/new");
    assert!(h.account_loading());
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::NewSession {
            cwd: dir,
            probe: true,
        },
        error: crate::acp::rpc::RpcError {
            code: -32603,
            message: "boom".to_string(),
            data: None,
        },
    });
    assert!(!h.account_loading());
}

/// A writer whose first `fails` lines are swallowed with a broken-pipe
/// error — everything after goes to the stream. Lets a test fail
/// single requests while later ones still reach the agent.
struct FlakyWriter {
    stream: std::os::unix::net::UnixStream,
    fails: usize,
}

impl std::io::Write for FlakyWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.fails > 0 {
            self.fails -= 1;
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "injected send failure",
            ));
        }
        self.stream.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

/// N9: both account reads of the probe fail to send (the wire broke
/// between the session creation and the reads): `account_pending`
/// stays zero and nothing would ever answer — the probe session is
/// closed right away instead of waiting for an `account_answered`
/// that cannot come (it would leak server-side).
#[test]
fn probe_with_failed_account_reads_closes_the_probe_immediately() {
    let dir = std::env::temp_dir();
    let (a, b) = UnixStream::pair().expect("socket pair");
    a.set_read_timeout(Some(RECV)).unwrap();
    b.set_read_timeout(Some(RECV)).unwrap();
    let (client, rx) = AcpClient::from_streams(
        BufReader::new(a.try_clone().unwrap()),
        FlakyWriter {
            stream: a,
            fails: 2,
        },
        None,
    );
    let mut h = Harness::new(vec![], "ask".to_string());
    h.attach(client, rx);
    let mut agent = Agent {
        reader: BufReader::new(b),
    };
    h.account_after_probe = true;

    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::Probe,
        cwd: dir.clone(),
        result: new_session_result("probe"),
    });
    assert!(
        h.account_probe_session.is_none(),
        "no probe waits for answers that cannot arrive"
    );
    assert!(!h.account_loading());
    let close = agent.read_msg();
    assert_eq!(
        close["method"], "session/close",
        "the close is the next (and only) line on the wire"
    );
    assert_eq!(close["params"]["sessionId"], json!("probe"));
}

#[test]
fn retry_note_ends_when_output_flows_again() {
    let (mut h, _agent) = setup(vec![], "ask");
    h.sessions
        .insert("s1".to_string(), SessionView::new("s1", "/w"));
    h.handle_event(AcpEvent::Retrying {
        session_id: "s1".to_string(),
        category: "rate_limited".to_string(),
        detail: String::new(),
    });
    assert!(h.retrying.contains_key("s1"));
    h.handle_event(AcpEvent::Update {
        session_id: "s1".to_string(),
        update: agent_chunk("a1", "weiter"),
    });
    assert!(!h.retrying.contains_key("s1"));
}

#[test]
fn question_is_kept_and_answered_with_the_form_content() {
    let (mut h, mut agent) = setup(vec![], "ask");
    h.handle_event(AcpEvent::QuestionAsked {
        request_id: json!(9),
        params: json!({"sessionId": "s1", "requestedSchema": {"properties": {
            "q0": {"type": "string", "description": "Format?",
                   "oneOf": [{"const": "CSV", "title": "CSV"}]}
        }}}),
    });
    assert_eq!(h.questions.len(), 1);
    h.answer_question(0, true);
    assert_eq!(h.questions.len(), 1, "unanswered: nothing is sent");
    h.questions[0]
        .answers
        .entry("q0".to_string())
        .or_default()
        .selected = vec!["CSV".to_string()];
    h.answer_question(0, true);
    assert!(h.questions.is_empty());
    let msg = agent.read_msg();
    assert_eq!(msg["id"], json!(9));
    assert_eq!(
        msg["result"],
        json!({"action": "accept", "content": {"q0": "CSV"}})
    );
}

#[test]
fn unrenderable_question_is_declined_not_left_hanging() {
    let (mut h, mut agent) = setup(vec![], "ask");
    h.handle_event(AcpEvent::QuestionAsked {
        request_id: json!(4),
        params: json!({"message": "x"}),
    });
    assert!(h.questions.is_empty());
    let msg = agent.read_msg();
    assert_eq!(msg["result"], json!({"action": "decline"}));
    assert!(h.status.contains("question"));
}

#[test]
fn session_state_prefers_waiting_over_running() {
    let (mut h, _agent) = setup(vec![], "ask");
    let mut v = SessionView::new("s1", "/w");
    v.begin_turn();
    h.sessions.insert("s1".to_string(), v);
    assert_eq!(h.session_state("s1"), SessionState::Running);
    h.handle_event(AcpEvent::QuestionAsked {
        request_id: json!(1),
        params: json!({"sessionId": "s1", "requestedSchema": {"properties": {
                "q0": {"type": "string", "description": "?", "oneOf": []}}}}),
    });
    assert_eq!(h.session_state("s1"), SessionState::Waiting);
    assert_eq!(h.session_state("unknown"), SessionState::Idle);
}

#[test]
fn turn_finished_in_the_background_is_marked_unseen() {
    let (mut h, _agent) = setup(vec![], "ask");
    h.sessions
        .insert("s2".to_string(), SessionView::new("s2", "/w"));
    h.active = Some("s1".to_string());
    h.handle_event(AcpEvent::PromptFinished {
        session_id: "s2".to_string(),
        result: serde_json::from_value(json!({"stopReason": "end_turn"})).unwrap(),
    });
    assert_eq!(h.session_state("s2"), SessionState::Unseen);
    h.open_session(PathBuf::from("/w"), "s2");
    assert_eq!(h.session_state("s2"), SessionState::Idle);
}

#[test]
fn trust_comes_from_the_session_meta_and_the_decision_updates_it() {
    let dir = std::env::temp_dir().join(format!("od-trust-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    let result = serde_json::from_value(json!({"sessionId": "s1", "_meta": {"workspace_trust": {
        "status": "untrusted",
        "details": {"detectedFiles": ["AGENTS.md"], "availableDecisions": ["trust_cwd", "decline"]}
    }}}))
    .unwrap();
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result,
    });
    assert!(h.trust[&dir].needs_decision());
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::TrustDecision {
            session_id: "s1".to_string(),
            cwd: dir.clone(),
            decision: "trust_cwd".to_string(),
        },
        result: json!({"trust_status": "trusted", "details": null}),
    });
    assert!(h.trust[&dir].trusted);
    assert!(h.status.contains("trusted"));
}

/// Background refreshes stay quiet when they fail; user actions speak up.
#[test]
fn failed_background_refresh_is_silent_user_action_is_not() {
    let (mut h, _agent) = setup(vec![], "ask");
    let err = crate::acp::rpc::RpcError {
        code: -32601,
        message: "Method not found".to_string(),
        data: None,
    };
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Ext(ExtKind::ReviewState {
            session_id: "s".to_string(),
        }),
        error: err.clone(),
    });
    assert!(h.status.is_empty());
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Ext(ExtKind::DeleteSession {
            session_id: "s".to_string(),
            cwd: PathBuf::from("/w"),
        }),
        error: err,
    });
    assert!(h.status.contains("Delete failed"));
}

#[test]
fn the_hint_is_never_shown_as_text_or_title() {
    let echoed = format!("{UI_HINT}\n\nEchte Frage");
    assert_eq!(without_ui_hint(&echoed), "Echte Frage");
    assert_eq!(without_ui_hint("Nur Text"), "Nur Text");
    assert_eq!(display_title(&echoed, "Neu"), "Echte Frage");
    assert_eq!(display_title(UI_HINT, "Neu"), "Neu");
    // Vibe cuts long first messages: a title that stops inside the hint
    let cut = format!("{}…", &UI_HINT[..30]);
    assert_eq!(display_title(&cut, "Neu"), "Neu");
    assert_eq!(display_title("Mein Titel", "Neu"), "Mein Titel");
    assert!(UI_HINT.starts_with("[Catwright]"));
}

#[test]
fn image_descriptions_never_show_up_as_message_or_title() {
    // a normal attachment: typed text stays, the description becomes a block
    let echoed = "Welches Tier?\n<image alias=\"pasted-image.png\">\nEin Hund im Park.\n</image>";
    let (visible, blocks) = split_image_blocks(echoed);
    assert_eq!(visible, "Welches Tier?");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].alias, "pasted-image.png");
    assert_eq!(blocks[0].description, "Ein Hund im Park.");
    // several blocks: everything between them stays visible
    let (visible, blocks) = split_image_blocks(
        "a <image alias=\"x.png\">Eins</image> b <image alias=\"y.png\">Zwei</image>",
    );
    assert_eq!(visible, "a  b");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[1].alias, "y.png");
    assert_eq!(blocks[1].description, "Zwei");
    // without an alias the fallback name is "image"
    let (visible, blocks) = split_image_blocks("x <image >ganz ohne Namen</image>");
    assert_eq!(visible, "x");
    assert_eq!(blocks[0].alias, "image");
    // an unclosed block (the description is still streaming) hides the rest
    let (visible, blocks) = split_image_blocks("Foto <image alias=\"x.png\">Halb");
    assert_eq!(visible, "Foto");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].description, "Halb");
    // no block: nothing changes
    let (visible, blocks) = split_image_blocks("Ganz normale Frage");
    assert_eq!(visible, "Ganz normale Frage");
    assert!(blocks.is_empty());
    // titles come from the visible part only, also without hint
    assert_eq!(
        derive_title("<image alias=\"x.png\">Nur ein Bild</image>"),
        None
    );
}

/// `_review/state` with per-turn scopes: the files land in the session's
/// rounds, a stale turn id does not.
#[test]
fn review_scopes_are_assigned_to_the_rounds_of_the_transcript() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    for (id, text) in [("m1", "erste Frage"), ("m2", "zweite Frage")] {
        h.handle_event(AcpEvent::Update {
            session_id: "s1".to_string(),
            update: user_chunk(id, text),
        });
        h.handle_event(AcpEvent::Update {
            session_id: "s1".to_string(),
            update: agent_chunk(&format!("a{id}"), "Antwort"),
        });
    }
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::ReviewState {
            session_id: "s1".to_string(),
        },
        result: json!({
            "files": [
                {"path": "/w/a.rs", "status": "modified", "regions": [{"decision": "pending"}]}
            ],
            "scopes": [
                {"owner": {"turnId": "m1"}, "files": ["/w/a.rs"]},
                {"owner": {"turnId": "m2"}, "files": ["/w/b.rs", "/w/c.rs"]},
                // a turn the transcript no longer has (e.g. cut by a rewind)
                {"owner": {"turnId": "m9"}, "files": ["/w/old.rs"]}
            ]
        }),
    });
    let rounds = crate::acp::session::rounds(&h.sessions["s1"].items);
    assert_eq!(rounds.len(), 2);
    let scope_of = |turn: &str| -> Vec<String> {
        h.review_scopes["s1"]
            .iter()
            .filter(|s| s.turn_id.as_deref() == Some(turn))
            .flat_map(|s| s.files.clone())
            .collect()
    };
    assert_eq!(scope_of("m1"), vec!["/w/a.rs"]);
    assert_eq!(scope_of("m2"), vec!["/w/b.rs", "/w/c.rs"]);
    assert_eq!(rounds[0].user_message_id.as_deref(), Some("m1"));
    assert_eq!(rounds[1].user_message_id.as_deref(), Some("m2"));
    // the undecided list is unchanged
    assert_eq!(h.review["s1"].len(), 1);
}

/// After a rewind the cut rounds are gone; their scopes must stay
/// readable as "without a round" instead of disappearing.
#[test]
fn after_a_rewind_the_cut_rounds_changes_have_no_round_left() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    for (id, text) in [("m1", "erste Frage"), ("m2", "zweite Frage")] {
        h.handle_event(AcpEvent::Update {
            session_id: "s1".to_string(),
            update: user_chunk(id, text),
        });
    }
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::ReviewState {
            session_id: "s1".to_string(),
        },
        result: json!({"files": [], "scopes": [
            {"owner": {"turnId": "m1"}, "files": ["/w/a.rs"]},
            {"owner": {"turnId": "m2"}, "files": ["/w/b.rs"]}
        ]}),
    });
    assert!(h.sessions.get_mut("s1").unwrap().rewind_to_message("m2"));
    let rounds = crate::acp::session::rounds(&h.sessions["s1"].items);
    let ids: Vec<_> = rounds
        .iter()
        .filter_map(|r| r.user_message_id.clone())
        .collect();
    assert_eq!(ids, vec!["m1"]);
    // the scopes are not touched by the rewind: m2's file stays in the
    // state, its round is gone (the transcript shows it as unassigned)
    assert!(h.review_scopes["s1"]
        .iter()
        .any(|s| s.turn_id.as_deref() == Some("m2") && s.files == vec!["/w/b.rs"]));
}

/// A session loaded with `session/load` gets its review state requested
/// again; an empty answer (old session) leaves both lists empty.
#[test]
fn a_loaded_session_asks_for_its_review_state_again() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.sessions
        .insert("s1".to_string(), SessionView::new("s1", dir.clone()));
    let loaded: proto::LoadSessionResult = serde_json::from_value(json!({})).unwrap();
    h.handle_event(AcpEvent::SessionLoaded {
        session_id: "s1".to_string(),
        result: loaded,
    });
    let mut methods = Vec::new();
    for _ in 0..2 {
        methods.push(
            agent.read_msg()["method"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        );
    }

    assert!(methods.contains(&"_loops/list".to_string()));
    assert!(methods.contains(&"_review/state".to_string()));
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::ReviewState {
            session_id: "s1".to_string(),
        },
        result: json!({"files": [], "scopes": []}),
    });
    assert!(h.review["s1"].is_empty());
    assert!(h.review_scopes["s1"].is_empty());
}

/// A click loads the turn diff once; the answer lands in the session's
/// map, a failure is kept as a note.
#[test]
fn turn_diff_is_requested_once_and_its_answer_is_kept() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.load_turn_diff("s1", "m1", "/w/a.rs");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "_review/turnDiff");
    assert_eq!(
        msg["params"],
        json!({"sessionId": "s1", "path": "/w/a.rs", "owner": {"turnId": "m1"}})
    );
    // a second click while the answer is in flight sends nothing
    h.load_turn_diff("s1", "m1", "/w/a.rs");
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::ReviewTurnDiff {
            session_id: "s1".to_string(),
            path: "/w/a.rs".to_string(),
            turn_id: "m1".to_string(),
        },
        result: json!({"status": "modified", "baseline": "alt", "current": "neu"}),
    });
    let view = h.turn_diffs["s1"].get("m1", "/w/a.rs").unwrap();
    match view {
        TurnDiffView::Loaded(d) => {
            assert_eq!(d.baseline.as_deref(), Some("alt"));
            assert_eq!(d.current.as_deref(), Some("neu"));
        }
        other => panic!("unexpected {other:?}"),
    }
    // a failed request is answered in the row, not only in the status
    h.load_turn_diff("s1", "m2", "/w/b.rs");
    assert_eq!(agent.read_msg()["method"], "_review/turnDiff");
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Ext(ExtKind::ReviewTurnDiff {
            session_id: "s1".to_string(),
            path: "/w/b.rs".to_string(),
            turn_id: "m2".to_string(),
        }),
        error: crate::acp::rpc::RpcError {
            code: -32602,
            message: "Unbekannter Turn".to_string(),
            data: None,
        },
    });
    match h.turn_diffs["s1"].get("m2", "/w/b.rs").unwrap() {
        TurnDiffView::Failed(note) => assert!(note.contains("Unbekannter Turn")),
        other => panic!("unexpected {other:?}"),
    }
    // a new review state drops the loaded diffs (baselines may move)
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::ReviewState {
            session_id: "s1".to_string(),
        },
        result: json!({"files": [], "scopes": []}),
    });
    assert!(h.turn_diffs["s1"].get("m1", "/w/a.rs").is_none());
}

#[test]
fn retry_notes_name_the_reason() {
    assert_eq!(
        retry_note("rate_limited", "429"),
        "Retrying: rate limit reached"
    );
    assert_eq!(retry_note("unknown", "Boom"), "Retrying: Boom");
    assert_eq!(
        tr_in(Lang::De, "Retrying: {reason}"),
        "Neuer Versuch: {reason}"
    );
    assert_eq!(tr_in(Lang::De, "rate limit reached"), "Ratenlimit erreicht");
}

/// The full harness UI rendered headless (egui `Context::run`): the user's
/// live crash report on this machine — typing „Schreibe eine Zeile mit dem
/// Wort Größe" — comes back as a render repro: the draft, the echoed user
/// message, the sidebar title and a fenced rust code block with multi-byte
/// text in every tokenizer position must all draw without panicking.
#[test]
fn the_full_ui_renders_multi_byte_text_without_panicking() {
    let dir = std::env::temp_dir();
    let (mut h, _agent) = setup(vec![dir.clone()], "ask");
    h.draft = "Schreibe eine Zeile mit dem Wort Größe".to_string();
    let mut v = SessionView::new("s1", dir.clone());
    v.apply_update(&user_chunk("m1", "Schreibe eine Zeile mit dem Wort Größe"));
    let code = concat!(
        "```rust\n",
        "let text = \"Größe\"; // die Größe des Textes\n",
        "fn calc_größe(x: f64) -> f64 {\n",
        "    let größe = x;\n",
        "    größe * größe // Größe ✓\n",
        "}\n",
        "```"
    );
    v.apply_update(&agent_chunk("m2", code));
    h.sessions.insert("s1".to_string(), v);
    h.active = Some("s1".to_string());

    let ctx = egui::Context::default();
    // the app installs its fonts on creation (main.rs); a bare Context
    // cannot draw the UI without them
    crate::fonts::install_fallback_fonts(&ctx);
    let text = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1360.0, 860.0),
        )),
        ..Default::default()
    };
    // three frames: the transcript renders the code block, the sidebar the
    // umlaut title, the composer the draft
    for _ in 0..3 {
        let _ = ctx.run(text.clone(), |ctx| {
            let _ = crate::harness::ui::show(&mut h, ctx);
        });
    }
    // the session and its items survive the frames
    assert_eq!(h.sessions.len(), 1);
    assert!(
        h.sessions["s1"].items.len() >= 2,
        "the user and the agent items are still there"
    );
}

/// A failing checkout leaves no orphan branch behind and says why.
#[test]
fn failed_worktree_is_cleaned_up_and_explained() {
    let base = std::env::temp_dir().join(format!(
        "od-wtf-{}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    // a repository without any commit: there is no HEAD to check out
    assert!(crate::harness::watch::git_output(&repo, &["init", "-q"]).is_some());
    let target = base.join("wt").join("repo-1");
    let err = make_worktree(&repo, &target, None).expect_err("no HEAD, no worktree");
    assert!(!err.is_empty());
    let branches = crate::harness::watch::git_output(&repo, &["branch", "--list"]).unwrap();
    assert!(!branches.contains("catwright/"), "{branches}");
    assert!(!target.exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// Headless render of the new views: the frames run in a plain egui context
/// (no window) and the painted text shapes are read back, so the test proves
/// the new widgets are really laid out, not just that the code runs.
fn render_texts(h: &mut Harness, frames: usize) -> Vec<String> {
    let ctx = egui::Context::default();
    // the app installs its fonts on creation (main.rs); a bare Context
    // cannot draw the UI without them
    crate::fonts::install_fallback_fonts(&ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1440.0, 900.0),
        )),
        ..Default::default()
    };
    let mut texts = Vec::new();
    for _ in 0..frames {
        let out = ctx.run(input.clone(), |ctx| {
            let _ = crate::harness::ui::show(h, ctx);
        });
        for clipped in &out.shapes {
            let galley = match &clipped.shape {
                egui::Shape::Text(t) => &t.galley,
                _ => continue,
            };
            texts.push(galley.text().to_string());
        }
    }
    texts
}

#[test]
fn new_views_render_headless_with_their_widgets() {
    let dir = std::env::temp_dir().join(format!("od-render-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("analysis.py");
    std::fs::write(&file, "a = 1\nb = 2\n").unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    let update = |u| AcpEvent::Update {
        session_id: "s1".to_string(),
        update: u,
    };
    h.handle_event(update(user_chunk("m1", "Add the stats table")));
    let tool_call = serde_json::from_value(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "t1",
        "title": "Writing analysis.py",
        "kind": "edit",
        "status": "completed",
        "locations": [{"path": file.to_string_lossy()}],
        "content": [{"type": "diff", "path": file.to_string_lossy(),
                     "oldText": "a = 1\n", "newText": "a = 1\nb = 2\nc = 3\n"}]
    }))
    .unwrap();
    h.handle_event(update(tool_call));
    h.handle_event(update(agent_chunk("a1", "Table added.")));
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });
    h.active = Some("s1".to_string());

    // 1) the activity feed: the waiting entry with its actions (section
    // headers are mono labels, so they render uppercased)
    h.ui.activity_open = true;
    let all = render_texts(&mut h, 3).join("\n");
    assert!(all.contains("Activity"), "nav row renders:\n{all}");
    assert!(all.contains("WAITING FOR YOU"), "feed section:\n{all}");
    assert!(all.contains("Permission request"), "feed entry:\n{all}");
    assert!(all.contains("Open"), "feed action:\n{all}");

    // 2) the session with an open diff, one comment and the preview pane
    h.ui.activity_open = false;
    // unfold the round so the tool group (and its diff chip) renders
    let rounds = h.sessions["s1"].folded_rounds();
    let rkey = crate::acp::session::round_key("s1", &rounds[0]);
    h.open_groups.insert(rkey);
    let key = format!("s1#t1::{}", file.to_string_lossy());
    h.diff_open.insert(key.clone());
    h.ui.diff_comments.insert(
        key,
        vec![crate::harness::state::DiffComment {
            line: 3,
            text: "mark the outlier".to_string(),
        }],
    );
    h.ui.preview = Some(crate::harness::state::PreviewState {
        path: file.to_string_lossy().into_owned(),
        width: 380.0,
        mode: crate::harness::state::PreviewMode::Code,
    });
    let all = render_texts(&mut h, 3).join("\n");
    assert!(
        all.contains("analysis.py"),
        "diff chip and preview header:\n{all}"
    );
    assert!(all.contains("c = 3"), "the open diff body:\n{all}");
    assert!(all.contains("Send to Vibe"), "comment action:\n{all}");
    assert!(all.contains("L3: mark the outlier"), "comment row:\n{all}");
    assert!(
        all.contains("1 comment on this file"),
        "comment count:\n{all}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Render the feature states to PNGs when CATWRIGHT_SHOT_DIR is set, so a
/// change can be looked at before it is committed (AGENTS.md "Seeing the UI").
#[test]
fn feature_screenshots() {
    let dir = std::env::temp_dir().join(format!("od-shots-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("analysis.py");
    std::fs::write(&file, "a = 1\nb = 2\n").unwrap();
    // a real git repo, so the preview pane's Diff mode has something to show
    crate::harness::watch::git_output(&dir, &["init", "-q"]);
    crate::harness::watch::git_output(&dir, &["add", "analysis.py"]);
    crate::harness::watch::git_output(
        &dir,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "v1",
        ],
    );
    std::fs::write(&file, "a = 1\nb = 2\nc = 3\nd = 4\n").unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    let update = |u| AcpEvent::Update {
        session_id: "s1".to_string(),
        update: u,
    };
    h.handle_event(update(user_chunk("m1", "Add the stats table")));
    let tool_call = serde_json::from_value(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "t1",
        "title": "Writing analysis.py",
        "kind": "edit",
        "status": "completed",
        "locations": [{"path": file.to_string_lossy()}],
        "content": [{"type": "diff", "path": file.to_string_lossy(),
                     "oldText": "a = 1\n", "newText": "a = 1\nb = 2\nc = 3\nd = 4\n"}]
    }))
    .unwrap();
    h.handle_event(update(tool_call));
    h.handle_event(update(agent_chunk("a1", "Table added.")));
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });
    h.active = Some("s1".to_string());
    let rounds = h.sessions["s1"].folded_rounds();
    h.open_groups
        .insert(crate::acp::session::round_key("s1", &rounds[0]));
    let key = format!("s1#t1::{}", file.to_string_lossy());
    h.diff_open.insert(key.clone());
    h.ui.diff_comments.insert(
        key.clone(),
        vec![crate::harness::state::DiffComment {
            line: 3,
            text: "mark the outlier".to_string(),
        }],
    );

    let shot = |h: &mut Harness, name: &str| {
        let img = crate::softrender::render_ui([1440, 900], 4, |ctx| {
            let _ = crate::harness::ui::show(h, ctx);
            crate::harness::ui::show_dialogs(h, ctx);
        });
        // the frame is not blank
        let bg = crate::theme::BG;
        assert!(
            img.pixels().any(|p| p.0[..3] != [bg.r(), bg.g(), bg.b()]),
            "{name}: something was painted"
        );
        if let Ok(out) = std::env::var("CATWRIGHT_SHOT_DIR") {
            let p = std::path::Path::new(&out).join(format!("{name}.png"));
            img.save(&p).unwrap();
        }
    };

    shot(&mut h, "01-diff-comments");
    h.ui.comment_edit = Some(crate::harness::state::CommentEdit {
        key: key.clone(),
        line: 4,
        text: String::new(),
    });
    shot(&mut h, "02-comment-editor");
    h.ui.comment_edit = None;
    h.ui.preview = Some(crate::harness::state::PreviewState {
        path: file.to_string_lossy().into_owned(),
        width: 380.0,
        mode: crate::harness::state::PreviewMode::Code,
    });
    shot(&mut h, "03-preview-pane");
    h.ui.preview.as_mut().unwrap().mode = crate::harness::state::PreviewMode::Diff;
    shot(&mut h, "03b-preview-diff");
    // a comment in the preview's diff, plus the editor on another line
    let pkey = format!("preview::{}", file.to_string_lossy());
    h.ui.diff_comments.insert(
        pkey.clone(),
        vec![crate::harness::state::DiffComment {
            line: 3,
            text: "use a constant".to_string(),
        }],
    );
    h.ui.comment_edit = Some(crate::harness::state::CommentEdit {
        key: pkey,
        line: 4,
        text: String::new(),
    });
    shot(&mut h, "03c-preview-diff-comments");
    h.ui.comment_edit = None;
    h.ui.preview = None;
    // the header's "N changed" menu (the git state arrives in the background)
    let mut tries = 0;
    while h.watcher.git(&dir).is_none_or(|g| g.changed == 0) && tries < 100 {
        h.watcher.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
        tries += 1;
    }
    let popup = crate::harness::ui::header::changed_files_popup(&dir);
    let img = crate::softrender::render_ui([1440, 900], 4, |ctx| {
        ctx.memory_mut(|m| m.open_popup(popup));
        let _ = crate::harness::ui::show(&mut h, ctx);
    });
    if let Ok(out) = std::env::var("CATWRIGHT_SHOT_DIR") {
        img.save(std::path::Path::new(&out).join("03d-changed-files.png"))
            .unwrap();
    }
    h.ui.activity_open = true;
    shot(&mut h, "04-activity");
    h.ui.activity_open = false;
    // a failed session shows FAILED in the sidebar
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -31001,
            message: "rate limited".to_string(),
            data: None,
        },
    });
    shot(&mut h, "06-sidebar-failed");
    // a running turn at each detail level
    h.sessions.get_mut("s1").unwrap().begin_turn();
    for level in ["compact", "standard", "detailed", "verbose"] {
        h.ui.transcript_detail = level.to_string();
        shot(&mut h, &format!("05-detail-{level}"));
    }
    // file chips: an agent answer naming files of the workspace (and one
    // that does not exist, which stays plain text)
    h.ui.transcript_detail = "standard".to_string();
    h.handle_event(update(agent_chunk(
        "a2",
        "Updated `analysis.py` — see analysis.py for the table; `missing.py` is not here.",
    )));
    shot(&mut h, "07-file-chips");
    // a folded round with its detail panel open (tool calls, status, duration)
    {
        let rounds = h.sessions["s1"].folded_rounds();
        let rk = crate::acp::session::round_key("s1", &rounds[0]);
        h.open_groups.insert(format!("detail::{rk}"));
        shot(&mut h, "12-round-detail");
        h.open_groups.remove(&format!("detail::{rk}"));
    }
    // drag-over hint on the composer (hovered files injected into the input)
    {
        let h = &mut h;
        let img = crate::softrender::render_ui([1440, 900], 4, |ctx| {
            ctx.input_mut(|i| {
                i.raw.hovered_files.push(egui::HoveredFile {
                    path: Some(std::path::PathBuf::from("/tmp/demo.png")),
                    mime: "image/png".to_string(),
                });
            });
            let _ = crate::harness::ui::show(h, ctx);
        });
        assert!(img.pixels().any(|p| p.0[..3]
            != [
                crate::theme::BG.r(),
                crate::theme::BG.g(),
                crate::theme::BG.b()
            ]));
        if let Ok(out) = std::env::var("CATWRIGHT_SHOT_DIR") {
            img.save(std::path::Path::new(&out).join("08-drop-hint.png"))
                .unwrap();
        }
    }
    // routines card with two loops and its "Delete all" button, then the
    // confirmation dialog for _loops/clear
    h.loops.insert(
        "s1".to_string(),
        vec![
            crate::harness::extras::LoopInfo {
                id: "l1".to_string(),
                prompt: "Daily summary\nSummarise the day.".to_string(),
                interval_seconds: Some(86400),
                cron: None,
                next_fire_at: crate::harness::extras::now_epoch() as f64 + 3600.0,
            },
            crate::harness::extras::LoopInfo {
                id: "l2".to_string(),
                prompt: "Weekly report\nReport on Monday.".to_string(),
                interval_seconds: None,
                cron: Some("0 9 * * 1".to_string()),
                next_fire_at: crate::harness::extras::now_epoch() as f64 + 90000.0,
            },
        ],
    );
    h.ui.panel_sections.insert("routines", true);
    shot(&mut h, "09-routines");
    h.ui.confirm = Some(crate::harness::state::Confirm::ClearRoutines {
        session_id: "s1".to_string(),
        count: 2,
    });
    shot(&mut h, "10-confirm-clear-routines");
    h.ui.confirm = None;
    // the command palette (Ctrl K) with a file query: project files listed
    h.ui.palette = Some(crate::harness::state::PaletteState {
        query: "analysis".to_string(),
        selected: 0,
        scroll_to_selected: false,
    });
    shot(&mut h, "11-palette-files");
    h.ui.palette = None;
    // the composer's microphone (Settings → General → Voice dictation):
    // idle, and recording in ORANGE
    h.voice_dictation = true;
    shot(&mut h, "15-dictation-idle");
    h.dictation = Dictation::Recording;
    shot(&mut h, "15b-dictation-recording");
    h.dictation = Dictation::Idle;
    h.voice_dictation = false;
    // a worktree of the project shows in the sidebar; the delete dialog warns
    {
        let wt = std::env::temp_dir().join(format!("cw-wt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&wt);
        crate::harness::watch::git_output(
            &dir,
            &[
                "worktree",
                "add",
                "-b",
                "catwright/demo",
                &wt.to_string_lossy(),
                "HEAD",
            ],
        );
        h.watcher.invalidate_git(&dir); // re-read now, not after the 6 s interval
        let mut tries = 0;
        while h.worktrees_of(&dir).is_empty() && tries < 100 {
            h.watcher.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
            tries += 1;
        }
        shot(&mut h, "13-worktrees");
        let w = h
            .worktrees_of(&dir)
            .into_iter()
            .next()
            .expect("one worktree");
        std::fs::write(w.path.join("dirty.txt"), "x").unwrap();
        h.ui.worktree_delete = Some(crate::harness::state::WorktreeDelete {
            project: dir.clone(),
            path: w.path.clone(),
            branch: w.branch.clone(),
            dirty: Harness::worktree_dirty(&w.path),
            delete_branch: false,
        });
        shot(&mut h, "14-worktree-delete");
        h.ui.worktree_delete = None;
        crate::harness::watch::git_output(
            &dir,
            &["worktree", "remove", "--force", &wt.to_string_lossy()],
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_turn_marks_the_session_failed_until_the_next_turn_or_open() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -31001,
            message: "rate limited".to_string(),
            data: None,
        },
    });
    assert_eq!(h.session_state("s1"), SessionState::Failed);
    let ev = h.activity.last().unwrap();
    assert_eq!(ev.kind, ActivityKind::Failed);
    assert!(ev.text.contains("rate limited"));
    // opening the session clears the mark
    h.open_session(dir.clone(), "s1");
    assert_eq!(h.session_state("s1"), SessionState::Idle);
    // and so does the next prompt
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::Prompt {
            session_id: "s1".to_string(),
        },
        error: rpc::RpcError {
            code: -32603,
            message: "boom".to_string(),
            data: None,
        },
    });
    assert_eq!(h.session_state("s1"), SessionState::Failed);
    h.sessions.get_mut("s1").unwrap().begin_turn();
    h.send_prompt("s1", &[proto::PromptBlock::Text("again".to_string())]);
    let _ = agent.read_msg();
    assert_eq!(
        h.session_state("s1"),
        SessionState::Running,
        "a new turn clears the failed mark"
    );
}

#[test]
fn compact_round_keeps_the_changed_file_card() {
    // compact folds a running round's tool calls into one line, but the
    // changed files stay visible (the setting promises it for every level)
    let dir = std::env::temp_dir().join(format!("od-compact-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("analysis.py");
    std::fs::write(&file, "a = 1\nb = 2\n").unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    let update = |u| AcpEvent::Update {
        session_id: "s1".to_string(),
        update: u,
    };
    h.handle_event(update(user_chunk("m1", "Add the stats table")));
    let edit = serde_json::from_value(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "t1",
        "title": "Writing analysis.py",
        "kind": "edit",
        "status": "completed",
        "locations": [{"path": file.to_string_lossy()}],
        "content": [{"type": "diff", "path": file.to_string_lossy(),
                     "oldText": "a = 1\n", "newText": "a = 1\nb = 2\n"}]
    }))
    .unwrap();
    h.handle_event(update(edit));
    h.handle_event(update(agent_chunk("a1", "Table added.")));
    h.sessions.get_mut("s1").unwrap().begin_turn();
    h.active = Some("s1".to_string());

    h.ui.transcript_detail = "compact".to_string();
    // the right panel lists changed files too; only the transcript counts
    h.ui.panel_closed = true;
    let texts = render_texts(&mut h, 3);
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with(&format!("{} ·", tr("Working")))),
        "the summary line:\n{texts:#?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("Writing analysis.py")),
        "the tool call itself is folded away:\n{texts:#?}"
    );
    assert!(
        texts.iter().any(|t| t == "analysis.py"),
        "the changed-file card stays:\n{texts:#?}"
    );
}

#[test]
fn clear_loops_sends_loops_clear_and_refreshes_the_list() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1"); // consumes session/list
    h.clear_loops("s1");
    let msg = agent.read_msg();
    assert_eq!(msg["method"], json!("_loops/clear"));
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    // the answer refreshes the list
    h.handle_event(AcpEvent::ExtResult {
        kind: ExtKind::LoopsClear {
            session_id: "s1".to_string(),
        },
        result: json!({}),
    });
    let msg = agent.read_msg();
    assert_eq!(msg["method"], json!("_loops/list"));
}

#[test]
fn desktop_notifications_queue_only_when_unfocused_and_opted_in() {
    let dir = std::env::temp_dir().join(format!("cw-notify-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1"); // consumes session/list
    let done: proto::PromptResult =
        serde_json::from_value(json!({"stopReason": "end_turn"})).unwrap();
    let mut finish = |h: &mut Harness| {
        h.handle_event(AcpEvent::PromptFinished {
            session_id: "s1".to_string(),
            result: done.clone(),
        });
        agent.read_msg(); // the list refresh after the turn
    };

    // opted in, window unfocused: a finished turn queues a notification that
    // names the project only (never title, prompt or answer)
    h.notify_events = true;
    h.window_focused = false;
    finish(&mut h);
    let notes = h.take_pending_notifications();
    assert_eq!(notes.len(), 1, "one notification per finished turn");
    assert!(
        notes[0].contains("Vibe is done in"),
        "generic text: {}",
        notes[0]
    );
    assert!(
        notes[0].contains(&project_name(&dir)),
        "names the project: {}",
        notes[0]
    );

    // window focused: nothing is queued
    h.window_focused = true;
    finish(&mut h);
    assert!(h.take_pending_notifications().is_empty());

    // opted out: nothing is queued even when unfocused
    h.notify_events = false;
    h.window_focused = false;
    finish(&mut h);
    assert!(h.take_pending_notifications().is_empty());

    // a waiting approval queues the approval text
    h.notify_events = true;
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: permission_request("s1"),
    });
    let notes = h.take_pending_notifications();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("approval"), "approval text: {}", notes[0]);
    assert!(notes[0].contains(&project_name(&dir)));

    // a question waits for the user just like an approval
    h.handle_event(AcpEvent::QuestionAsked {
        request_id: json!(9),
        params: json!({"sessionId": "s1", "requestedSchema": {"properties": {
            "q0": {"type": "string", "description": "Format?",
                   "oneOf": [{"const": "CSV", "title": "CSV"}]}
        }}}),
    });
    let notes = h.take_pending_notifications();
    assert_eq!(notes.len(), 1, "a question notifies");
    assert!(notes[0].contains("question"), "question text: {}", notes[0]);
    assert!(notes[0].contains(&project_name(&dir)));

    // an approval of a session this client does not know (a subagent's)
    // still notifies, without a project name
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(1),
        request: permission_request("child-of-s1"),
    });
    assert_eq!(
        h.take_pending_notifications(),
        vec!["Vibe needs your approval".to_string()],
        "unknown session: generic approval text"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn palette_lists_project_files_and_enter_opens_the_preview() {
    let dir = std::env::temp_dir().join(format!("cw-pal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("analysis.py"), "x").unwrap();
    std::fs::write(dir.join("src").join("main.rs"), "x").unwrap();
    crate::harness::watch::git_output(&dir, &["init", "-q"]);
    crate::harness::watch::git_output(&dir, &["add", "."]);
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.active = Some("s1".to_string());
    // the watcher lists the project's files in the background
    let mut tries = 0;
    while h.watcher.files(&dir).is_empty() && tries < 100 {
        h.watcher.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
        tries += 1;
    }
    use crate::harness::ui::palette::{apply, entries, Action};
    let file_hits = |list: &[crate::harness::ui::palette::Entry]| -> Vec<String> {
        list.iter()
            .filter_map(|e| match &e.action {
                Action::File(_, rel) => Some(rel.clone()),
                _ => None,
            })
            .collect()
    };
    // without a query the palette keeps to actions and sessions
    let all = entries(&mut h, "");
    assert!(file_hits(&all).is_empty(), "files: {:?}", file_hits(&all));
    // a query lists the matching project files
    let files = file_hits(&entries(&mut h, "a"));
    assert!(files.iter().any(|f| f == "analysis.py"), "files: {files:?}");
    assert!(files.iter().any(|f| f == "src/main.rs"), "files: {files:?}");
    // fuzzy query narrows them
    let hits = entries(&mut h, "main");
    assert!(
        hits.iter()
            .any(|e| matches!(&e.action, Action::File(_, r) if r == "src/main.rs")),
        "fuzzy hit for 'main'"
    );
    // Enter on a file opens the preview pane with the resolved path
    let mut out = crate::harness::ui::HarnessAction::None;
    apply(
        &mut h,
        Action::File(dir.clone(), "analysis.py".to_string()),
        &mut out,
    );
    let preview = h.ui.preview.expect("preview opens on Enter");
    assert!(
        preview.path.ends_with("analysis.py"),
        "path: {}",
        preview.path
    );
    assert_eq!(preview.mode, crate::harness::state::PreviewMode::Code);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tool_call_arrival_timestamps_drive_the_round_duration() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1"); // consumes session/list
    let update = |u| AcpEvent::Update {
        session_id: "s1".to_string(),
        update: u,
    };
    let call = serde_json::from_value(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "t1",
        "title": "Writing analysis.py",
        "kind": "edit",
        "status": "in_progress",
    }))
    .unwrap();
    h.handle_event(update(call));
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    let tc = &h.sessions["s1"].tool_calls["t1"];
    assert!(tc.first_seen > 0, "first arrival stamped");
    assert!(tc.last_update >= tc.first_seen);
    // still running: duration measures up to now
    assert_eq!(tc.duration_secs(now()), Some(now() - tc.first_seen));
    let upd = serde_json::from_value(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "t1",
        "status": "completed",
    }))
    .unwrap();
    h.handle_event(update(upd));
    let tc = &h.sessions["s1"].tool_calls["t1"];
    assert_eq!(tc.status, crate::acp::protocol::ToolStatus::Completed);
    assert!(tc.last_update >= tc.first_seen, "last update stamped");
    assert!(tc.duration_secs(now() + 60) >= tc.duration_secs(now()));
}

#[test]
fn worktree_porcelain_parsing_marks_the_main_worktree() {
    let out = "worktree /home/u/proj\nHEAD abc123\nbranch refs/heads/main\n\nworktree /cfg/worktrees/proj-1\nHEAD def456\nbranch refs/heads/catwright/proj-1\n";
    let wts = crate::harness::watch::parse_worktrees(out);
    assert_eq!(wts.len(), 2);
    assert!(wts[0].main, "the first entry is the main worktree");
    assert!(!wts[1].main);
    assert_eq!(wts[0].branch, "main");
    assert_eq!(wts[1].branch, "catwright/proj-1");
    assert_eq!(
        wts[1].path,
        std::path::PathBuf::from("/cfg/worktrees/proj-1")
    );
}

#[test]
fn delete_worktree_removes_it_and_the_branch_only_when_asked() {
    use crate::harness::watch::run_git;
    let base = std::env::temp_dir().join(format!("cw-wtdel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    run_git(
        &repo,
        &["init", "-q", "-b", "main"],
        Duration::from_secs(30),
    )
    .unwrap();
    std::fs::write(repo.join("f.txt"), "x").unwrap();
    run_git(&repo, &["add", "."], Duration::from_secs(30)).unwrap();
    run_git(
        &repo,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "c1",
        ],
        Duration::from_secs(30),
    )
    .unwrap();
    let wt = base.join("wt1");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "wt-branch",
            &wt.to_string_lossy(),
            "HEAD",
        ],
        Duration::from_secs(60),
    )
    .unwrap();
    assert!(wt.exists());

    let (mut h, _agent) = setup(vec![repo.clone()], "ask");
    // without the branch option: worktree gone, branch stays
    h.delete_worktree(&repo, &wt, "wt-branch", false);
    let mut tries = 0;
    while (wt.exists() || !h.status.contains("Worktree deleted.")) && tries < 200 {
        h.poll();
        std::thread::sleep(std::time::Duration::from_millis(25));
        tries += 1;
    }
    assert!(!wt.exists(), "worktree removed");
    let branches = run_git(&repo, &["branch", "--list"], Duration::from_secs(30)).unwrap();
    assert!(
        branches.contains("wt-branch"),
        "branch kept by default: {branches}"
    );

    // with the option: branch deleted too
    let wt2 = base.join("wt2");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "wt-branch2",
            &wt2.to_string_lossy(),
            "HEAD",
        ],
        Duration::from_secs(60),
    )
    .unwrap();
    h.delete_worktree(&repo, &wt2, "wt-branch2", true);
    let mut tries = 0;
    while (wt2.exists() || !h.status.contains("Worktree deleted.")) && tries < 200 {
        h.poll();
        std::thread::sleep(std::time::Duration::from_millis(25));
        tries += 1;
    }
    assert!(!wt2.exists());
    let branches = run_git(&repo, &["branch", "--list"], Duration::from_secs(30)).unwrap();
    assert!(
        !branches.contains("wt-branch2"),
        "branch deleted on request: {branches}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// A start ref names where the worktree's own branch begins: `main`, which
/// is checked out in the main worktree, must work (git refuses to check
/// out one branch twice), and the new branch is `catwright/<name>`.
#[test]
fn worktree_with_a_start_ref_gets_its_own_branch_from_that_ref() {
    let base = std::env::temp_dir().join(format!(
        "od-wtr-{}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| crate::harness::watch::git_output(&repo, args);
    assert!(git(&["init", "-q", "-b", "main"]).is_some());
    let commit = |msg: &str| {
        git(&[
            "-c",
            "user.name=Catwright",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            msg,
            "--allow-empty",
        ])
    };
    assert!(commit("first").is_some());
    let first = git(&["rev-parse", "HEAD"]).unwrap().trim().to_string();
    assert!(commit("second").is_some());
    // the branch checked out in the main worktree as start ref
    let target = base.join("wt").join("repo-1");
    make_worktree(&repo, &target, Some("main")).expect("worktree from main");
    let wt = |args: &[&str]| crate::harness::watch::git_output(&target, args);
    assert_eq!(
        wt(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap().trim(),
        "catwright/repo-1"
    );
    // a commit as start ref: the new branch starts there
    let target2 = base.join("wt").join("repo-2");
    make_worktree(&repo, &target2, Some(&first)).expect("worktree from a commit");
    assert_eq!(
        crate::harness::watch::git_output(&target2, &["rev-parse", "HEAD"])
            .unwrap()
            .trim(),
        first
    );
    // option-like input never reaches git
    let target3 = base.join("wt").join("repo-3");
    assert!(make_worktree(&repo, &target3, Some("--detach")).is_err());
    assert!(!target3.exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// The history a `session/load` replays carries no timing: its tool calls
/// get no client-side timestamps, so the per-round view shows no made-up
/// duration. Calls of a live turn afterwards are timed again.
#[test]
fn replayed_tool_calls_have_no_duration() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.open_session(dir.clone(), "s1");
    assert_eq!(agent.read_msg()["method"], "session/load");
    assert!(h.sessions["s1"].replaying, "the load replays the history");
    let update = |u| AcpEvent::Update {
        session_id: "s1".to_string(),
        update: u,
    };
    let call = |id: &str, status: &str| {
        serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": "Reading analysis.py",
            "kind": "read",
            "status": status,
        }))
        .unwrap()
    };
    h.handle_event(update(call("old", "completed")));
    h.handle_event(AcpEvent::SessionLoaded {
        session_id: "s1".to_string(),
        result: serde_json::from_value(json!({})).unwrap(),
    });
    assert!(!h.sessions["s1"].replaying);
    let old = &h.sessions["s1"].tool_calls["old"];
    assert_eq!(old.first_seen, 0);
    assert_eq!(old.duration_secs(u64::MAX), None, "no duration for history");
    // a later update of the replayed call does not invent a start either
    let upd = serde_json::from_value(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "old",
        "status": "completed",
    }))
    .unwrap();
    h.handle_event(update(upd));
    assert_eq!(h.sessions["s1"].tool_calls["old"].duration_secs(0), None);
    // a live call is timed
    h.handle_event(update(call("new", "in_progress")));
    let new = &h.sessions["s1"].tool_calls["new"];
    assert!(new.first_seen > 0);
    assert!(new.duration_secs(new.first_seen + 3) == Some(3));
}

/// A waiting feed entry leaves the feed (and the badge) once its approval
/// is answered — it no longer waits for anything.
#[test]
fn answered_approval_leaves_the_waiting_feed() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(7),
        request: permission_request("s1"),
    });
    let waiting: Vec<_> = h
        .activity
        .iter()
        .filter(|e| e.kind == ActivityKind::Waiting)
        .cloned()
        .collect();
    assert_eq!(waiting.len(), 1);
    assert!(h.activity_shown(&waiting[0]));
    let before = h.activity_unseen();
    assert!(before >= 1);
    h.answer_permission("s1", &json!(7), "allow_once");
    assert!(
        !h.activity_shown(&waiting[0]),
        "answered: no longer waiting"
    );
    assert_eq!(h.activity_unseen(), before - 1, "the badge drops it too");
}

/// The feed keeps the newest 100 entries; the read marker moves with the
/// dropped front, so new entries still count as unseen.
#[test]
fn activity_read_marker_survives_the_bound() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    for _ in 0..100 {
        h.push_activity(ActivityKind::Finished, "s1", "done".to_string(), None);
    }
    h.ui.activity_seen = h.activity.len();
    assert_eq!(h.activity_unseen(), 0);
    h.push_activity(ActivityKind::Finished, "s1", "new".to_string(), None);
    assert_eq!(h.activity.len(), 100);
    assert_eq!(h.activity_unseen(), 1, "the new entry is unseen");
}

/// The header's "N changed" menu lists git's changed paths — also for a
/// project that is a sub-folder of its repository — and a file opens as its
/// diff against HEAD in the preview pane, where it can be commented like a
/// transcript diff and the comments go to the composer.
#[test]
fn changed_files_open_as_a_commentable_preview_diff() {
    let repo = std::env::temp_dir().join(format!("cw-changed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    let sub = repo.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let file = sub.join("analysis.py");
    std::fs::write(&file, "old_line = 1\n").unwrap();
    let git = |args: &[&str]| crate::harness::watch::git_output(&repo, args);
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "v1",
    ]);
    std::fs::write(&file, "new_line = 2\n").unwrap();
    std::fs::write(repo.join("top.txt"), "x").unwrap();

    let (mut h, mut agent) = setup(vec![sub.clone()], "ask");
    create_session(&mut h, &mut agent, &sub, "s1");
    h.active = Some("s1".to_string());
    let mut tries = 0;
    while h.watcher.git(&sub).is_none_or(|g| g.changed == 0) && tries < 100 {
        h.watcher.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
        tries += 1;
    }
    let info = h.watcher.git(&sub).cloned().expect("git state arrives");
    assert_eq!(info.changed, 2, "{info:?}");

    // the open menu: the file relative to the project, the one above it
    // relative to the repository
    let popup = crate::harness::ui::header::changed_files_popup(&sub);
    let ctx = egui::Context::default();
    crate::fonts::install_fallback_fonts(&ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1440.0, 900.0),
        )),
        ..Default::default()
    };
    let mut texts = Vec::new();
    for _ in 0..3 {
        ctx.memory_mut(|m| m.open_popup(popup));
        let out = ctx.run(input.clone(), |ctx| {
            let _ = crate::harness::ui::show(&mut h, ctx);
        });
        for clipped in &out.shapes {
            if let egui::Shape::Text(t) = &clipped.shape {
                texts.push(t.galley.text().to_string());
            }
        }
    }
    let all = texts.join("\n");
    assert!(all.contains("2 changed"), "the header count:\n{all}");
    assert!(all.contains("analysis.py"), "the project file:\n{all}");
    assert!(
        all.contains("top.txt"),
        "the file above the project:\n{all}"
    );

    // a click on the file: its diff against HEAD (found from the sub-folder,
    // so the old line shows as removed) with a comment
    h.open_preview_diff(&file);
    let key = format!("preview::{}", file.to_string_lossy());
    h.ui.diff_comments.insert(
        key.clone(),
        vec![crate::harness::state::DiffComment {
            line: 1,
            text: "rename it back".to_string(),
        }],
    );
    let all = render_texts(&mut h, 3).join("\n");
    assert!(all.contains("old_line = 1"), "HEAD's version:\n{all}");
    assert!(all.contains("new_line = 2"), "the working tree:\n{all}");
    assert!(all.contains("L1: rename it back"), "comment row:\n{all}");
    assert!(all.contains("Send to Vibe"), "comment action:\n{all}");

    crate::harness::ui::transcript::send_comments(&mut h, &key, "analysis.py");
    assert!(
        h.draft.contains("analysis.py") && h.draft.contains("L1: rename it back"),
        "the composer draft: {}",
        h.draft
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// vibe-acp sends only an id with a permission request — the call's entry
/// id, or just the tool's name when it has none. A card whose call the
/// session has not seen shows that id and says that no details came,
/// instead of an empty card; once the call arrives it shows the call.
#[test]
fn permission_card_without_a_known_call_is_not_empty() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.active = Some("s1".to_string());
    h.handle_event(AcpEvent::PermissionRequested {
        request_id: json!(0),
        request: serde_json::from_value(json!({
            "sessionId": "s1",
            "toolCall": {"toolCallId": "bash"},
            "options": [
                {"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"},
                {"optionId": "reject_once", "name": "Deny", "kind": "reject_once"}
            ]
        }))
        .unwrap(),
    });
    let all = render_texts(&mut h, 3).join("\n");
    assert!(all.contains("Run action"), "headline:\n{all}");
    assert!(all.contains("bash"), "the tool Vibe named:\n{all}");
    assert!(all.contains("Vibe sent no details"), "the note:\n{all}");
    assert!(all.contains("Allow once"), "still answerable:\n{all}");

    // the call arrives: the card shows it, the note goes
    h.handle_event(AcpEvent::Update {
        session_id: "s1".to_string(),
        update: serde_json::from_value(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "bash",
            "title": "ls -la",
            "kind": "execute",
            "status": "pending",
            "rawInput": {"command": "ls -la"}
        }))
        .unwrap(),
    });
    let all = render_texts(&mut h, 3).join("\n");
    assert!(all.contains("$ ls -la"), "the real call:\n{all}");
    assert!(!all.contains("Vibe sent no details"), "no note:\n{all}");
}

// ---------------------------------------------------------------------------
// Voice dictation: Vibe records and transcribes, Catwright only asks
// (`_voice/*`, docs/acp.md)
// ---------------------------------------------------------------------------

/// A harness with one open session and the microphone setting on.
fn dictation_harness() -> (Harness, Agent) {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    create_session(&mut h, &mut agent, &dir, "s1");
    h.active = Some("s1".to_string());
    h.voice_dictation = true;
    (h, agent)
}

/// The click starts Vibe's transcription (with the UI language), its answer
/// makes the state Recording, and a second click stops it.
#[test]
fn dictation_starts_records_and_stops() {
    let (mut h, mut agent) = dictation_harness();
    h.toggle_dictation();
    assert_eq!(h.dictation, Dictation::Starting);
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "_voice/transcribeStart");
    assert_eq!(msg["params"]["language"], json!(crate::i18n::lang().code()));
    h.handle_event(AcpEvent::VoiceStarted {
        ok: true,
        error: None,
    });
    assert_eq!(h.dictation, Dictation::Recording);

    h.toggle_dictation();
    assert_eq!(h.dictation, Dictation::Stopping);
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "_voice/transcribeStop");
    assert_eq!(msg["params"], json!({}));
    h.handle_event(AcpEvent::VoiceStopped {
        ok: true,
        error: None,
    });
    assert_eq!(h.dictation, Dictation::Idle);
    assert!(h.status.is_empty(), "no error to show: {}", h.status);
}

/// A failed start goes back to Idle and shows Vibe's own text.
#[test]
fn a_failed_start_keeps_vibes_text() {
    let (mut h, mut agent) = dictation_harness();
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "_voice/transcribeStart");
    h.handle_event(AcpEvent::VoiceStarted {
        ok: false,
        error: Some("No audio input device found".into()),
    });
    assert_eq!(h.dictation, Dictation::Idle);
    assert_eq!(h.status, "No audio input device found");
}

/// Deltas append; the first one of a dictation is separated from the typed
/// draft by exactly one space.
#[test]
fn dictation_deltas_append_with_one_space() {
    let (mut h, mut agent) = dictation_harness();
    h.draft = "Bitte".to_string();
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "_voice/transcribeStart");
    h.handle_event(AcpEvent::VoiceStarted {
        ok: true,
        error: None,
    });
    for piece in ["die ", "Tests ", "laufen"] {
        h.handle_event(AcpEvent::VoiceDelta {
            text: Some(piece.into()),
            error: None,
        });
    }
    assert_eq!(h.draft, "Bitte die Tests laufen");

    // a draft that already ends in whitespace gets no second space
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "_voice/transcribeStop");
    h.handle_event(AcpEvent::VoiceStopped {
        ok: true,
        error: None,
    });
    h.draft = "Bitte ".to_string();
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "_voice/transcribeStart");
    h.handle_event(AcpEvent::VoiceStarted {
        ok: true,
        error: None,
    });
    h.handle_event(AcpEvent::VoiceDelta {
        text: Some("weiter".into()),
        error: None,
    });
    assert_eq!(h.draft, "Bitte weiter");

    // an empty draft gets no leading space
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "_voice/transcribeStop");
    h.handle_event(AcpEvent::VoiceStopped {
        ok: true,
        error: None,
    });
    h.draft.clear();
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "_voice/transcribeStart");
    h.handle_event(AcpEvent::VoiceStarted {
        ok: true,
        error: None,
    });
    h.handle_event(AcpEvent::VoiceDelta {
        text: Some("Hallo".into()),
        error: None,
    });
    assert_eq!(h.draft, "Hallo");
}

/// An error delta ends the dictation; what was recognised stays.
#[test]
fn an_error_delta_ends_the_dictation() {
    let (mut h, mut agent) = dictation_harness();
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "_voice/transcribeStart");
    h.handle_event(AcpEvent::VoiceStarted {
        ok: true,
        error: None,
    });
    h.handle_event(AcpEvent::VoiceDelta {
        text: Some("Hallo".into()),
        error: None,
    });
    h.handle_event(AcpEvent::VoiceDelta {
        text: None,
        error: Some("Audio backend is unavailable: no device".into()),
    });
    assert_eq!(h.dictation, Dictation::Idle);
    assert_eq!(h.status, "Audio backend is unavailable: no device");
    assert_eq!(h.draft, "Hallo", "the recognised text stays");
}

/// Sending a message aborts the dictation first; a late start answer must
/// not revive it.
#[test]
fn sending_a_message_cancels_the_dictation() {
    let (mut h, mut agent) = dictation_harness();
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "_voice/transcribeStart");
    h.handle_event(AcpEvent::VoiceStarted {
        ok: true,
        error: None,
    });
    h.handle_event(AcpEvent::VoiceDelta {
        text: Some("Hallo".into()),
        error: None,
    });
    h.draft.push_str(" Vibe");
    h.send();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "_voice/transcribeCancel");
    assert_eq!(msg["params"], json!({}));
    assert_eq!(h.dictation, Dictation::Idle);
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(
        msg["params"]["prompt"][0],
        json!({"type": "text", "text": "Hallo Vibe"})
    );
    // the start answer arrives after the cancel: it is stale
    h.handle_event(AcpEvent::VoiceStarted {
        ok: true,
        error: None,
    });
    assert_eq!(h.dictation, Dictation::Idle);
}

/// The microphone button exists only while the setting is on, and its
/// tooltip says what a click does.
#[test]
fn the_microphone_button_follows_the_setting() {
    let (mut h, _agent) = dictation_harness();
    h.voice_dictation = false;
    assert!(
        crate::harness::ui::composer::dictation_button(&h).is_none(),
        "the setting is off: no microphone"
    );
    h.voice_dictation = true;
    let (icon, tooltip, color) = crate::harness::ui::composer::dictation_button(&h).unwrap();
    assert_eq!(icon, crate::icons::Icon::Mic);
    assert_eq!(tooltip, "Dictate");
    assert_eq!(color, crate::theme::MUTED);
    h.dictation = Dictation::Recording;
    let (icon, tooltip, color) = crate::harness::ui::composer::dictation_button(&h).unwrap();
    assert_eq!(icon, crate::icons::Icon::Mic);
    assert_eq!(tooltip, "Stop dictation");
    assert_eq!(color, crate::theme::ORANGE, "recording is orange");
}

/// The composer really paints the microphone: the frame with the setting on
/// differs from the one with it off (and the frame itself is stable).
#[test]
fn the_microphone_is_painted_only_with_the_setting_on() {
    let (mut h, _agent) = dictation_harness();
    h.voice_dictation = false;
    // no focus request: the composer's frame colour would differ between
    // the first and the second render
    h.ui.focus_composer = false;
    let shot = |h: &mut Harness| {
        crate::softrender::render_ui([1360, 860], 3, |ctx| {
            let _ = crate::harness::ui::show(h, ctx);
        })
    };
    let off = shot(&mut h);
    let off_again = shot(&mut h);
    assert_eq!(
        off.as_raw(),
        off_again.as_raw(),
        "the same state renders the same frame"
    );
    h.voice_dictation = true;
    let on = shot(&mut h);
    assert_ne!(
        off.as_raw(),
        on.as_raw(),
        "the microphone button is painted"
    );
}

/// On the start screen no session is open in vibe-acp, and Vibe's voice
/// needs one: the microphone starts the project's session first and the
/// transcription once it is there.
#[test]
fn dictation_on_the_start_screen_starts_the_session_first() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.voice_dictation = true;
    h.toggle_dictation();
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/new");
    assert_eq!(msg["params"]["cwd"], json!(dir.to_string_lossy()));
    assert_eq!(h.dictation, Dictation::Starting);
    // a second click while the session is coming does nothing
    h.toggle_dictation();
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result("s1"),
    });
    assert_eq!(h.active.as_deref(), Some("s1"));
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "_voice/transcribeStart");
    assert_eq!(agent.read_msg()["method"], "session/list");
    h.handle_event(AcpEvent::VoiceStarted {
        ok: true,
        error: None,
    });
    assert_eq!(h.dictation, Dictation::Recording);
}

/// A message sent while the microphone's session is still coming goes into
/// that session: no second session/new.
#[test]
fn sending_while_the_dictation_session_is_coming_uses_it() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.voice_dictation = true;
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "session/new");
    h.draft = "Hallo Vibe".into();
    h.send();
    assert_eq!(h.dictation, Dictation::Idle);
    assert!(h.queued.is_some(), "the message waits for the session");
    h.handle_event(AcpEvent::SessionCreated {
        origin: SessionOrigin::New,
        cwd: dir.clone(),
        result: new_session_result("s1"),
    });
    // the message goes out, no transcription starts (the dictation ended)
    let msg = agent.read_msg();
    assert_eq!(msg["method"], "session/prompt");
    assert_eq!(msg["params"]["sessionId"], json!("s1"));
    assert_eq!(agent.read_msg()["method"], "session/list");
}

/// The microphone's session/new fails: the dictation is over.
#[test]
fn a_failed_dictation_session_ends_the_dictation() {
    let dir = std::env::temp_dir();
    let (mut h, mut agent) = setup(vec![dir.clone()], "ask");
    h.current_project = Some(dir.clone());
    h.voice_dictation = true;
    h.toggle_dictation();
    assert_eq!(agent.read_msg()["method"], "session/new");
    h.handle_event(AcpEvent::RequestFailed {
        request: RequestKind::NewSession {
            cwd: dir.clone(),
            probe: false,
        },
        error: rpc::RpcError {
            code: -32603,
            message: "boom".into(),
            data: None,
        },
    });
    assert_eq!(h.dictation, Dictation::Idle);
}
