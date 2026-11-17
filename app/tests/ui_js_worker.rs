//! End-to-end test of the `--ui-js-worker` mode against the real binary
//! (`script.rs`): one JSON request in, one JSON response out. The
//! in-process engine, the protocol and the timeout/kill logic have their
//! unit tests in `src/script.rs`; this file proves the CLI wiring.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn run_worker(request: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_catwright"))
        .arg("--ui-js-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the binary");
    {
        let mut stdin = child.stdin.take().expect("stdin piped");
        stdin
            .write_all(format!("{request}\n").as_bytes())
            .expect("write the request");
    }
    let out = child.wait_with_output().expect("worker exits");
    assert!(out.status.success(), "worker exit: {:?}", out.status);
    serde_json::from_slice(&out.stdout).expect("response JSON")
}

#[test]
fn worker_renders_nodes_and_seeds_state_from_init() {
    let resp = run_worker(json!({
        "source": "function init(){ return {n: 2}; }\n\
                   function render(s){ return {type:'text', text:'n=' + s.n}; }",
        "state": {},
        "init": true,
        "action": null
    }));
    assert_eq!(resp["ok"], json!(true));
    assert_eq!(resp["state"], json!({"n": 2}));
    assert_eq!(resp["nodes"], json!({"type": "text", "text": "n=2"}));
}

#[test]
fn worker_runs_actions_locally() {
    let resp = run_worker(json!({
        "source": "function render(){ return {type:'divider'}; }\n\
                   function onAction(a, s){ if (a === 'x') { s.done = true; return s; } }",
        "state": {"other": 1},
        "init": false,
        "action": "x"
    }));
    assert_eq!(resp["ok"], json!(true));
    assert_eq!(resp["outcome"], json!("local"));
    assert_eq!(resp["state"], json!({"other": 1, "done": true}));
}

#[test]
fn worker_reports_script_errors() {
    let resp = run_worker(json!({
        "source": "function render(){ while(true){} }",
        "state": {},
        "init": true,
        "action": null
    }));
    assert_eq!(resp["ok"], json!(false));
    // the child has no UI language: its errors are English msgids on
    // the wire; the parent translates known ones for display
    assert!(resp["error"].as_str().unwrap().contains("loop limit"));
}

/// M11 (Wirecap review): a legal answer carries `nodes` PLUS the echoed
/// `state`, so it can exceed the request budget (~389 KiB) while staying
/// inside the response budget (592 KiB) — proven here end to end through
/// the real worker. The capped pipe used to cut such an answer off, the
/// worker died of EPIPE mid-write and the parent reported "no valid
/// answer" for a feature that was working.
#[test]
fn worker_answer_with_payloads_beyond_the_request_budget_arrives() {
    let chunk = "x".repeat(105_000);
    let resp = run_worker(json!({
        "source": "function render(s){ return {type:'text', text: s.a + s.b}; }",
        "state": { "a": chunk, "b": chunk },
        "init": false,
        "action": null
    }));
    assert_eq!(resp["ok"], json!(true), "{resp:?}");
    // nodes (210_000 text bytes + envelope) plus the 210_000-byte state
    // echo ≈ 420 KiB in total: above the request budget (398_528), well
    // below the response budget (606_208)
    assert_eq!(resp["nodes"]["text"].as_str().map(str::len), Some(210_000));
    assert_eq!(resp["state"]["a"].as_str().map(str::len), Some(105_000));
}
