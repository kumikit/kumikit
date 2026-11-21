//! Recorded real agent output, replayed through the adapter (AGENTS.md, Tests).
//!
//! Each `transcripts/<name>.jsonl` is a scrubbed recording of a real ACP agent (see
//! `transcripts/README.md`). The test drives [`Session`] as the plugin would: every
//! message the session writes must equal the recorded client message, and the events it
//! emits must equal `expected/<name>.json`. `KUMIKIT_BLESS=1` rewrites the expected
//! files.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use kumikit_provider_acp::acp::{Action, Decision, Options, Session};
use serde_json::{Value, json};

struct Replay {
    session: Session,
    writes: VecDeque<String>,
    approvals: HashMap<String, Value>,
    log: Vec<Value>,
}

impl Replay {
    fn take(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Write(line) => self.writes.push_back(line),
                Action::Emit(e) => self.log.push(serde_json::to_value(e).unwrap()),
                Action::Approve {
                    request,
                    kind,
                    summary,
                    detail,
                } => {
                    self.log
                        .push(json!({"approve": {"kind": kind, "summary": summary}}));
                    self.approvals
                        .insert(request.to_string(), detail["options"].clone());
                }
                Action::Ready { session } => self.log.push(json!({"ready": session})),
                Action::TurnDone => self.log.push(json!("turn_done")),
                Action::Failed(m) => self.log.push(json!({"failed": m})),
            }
        }
    }
}

fn replay(name: &str, mode: Option<&str>) -> Vec<Value> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let text =
        std::fs::read_to_string(dir.join("transcripts").join(format!("{name}.jsonl"))).unwrap();
    let mut r = Replay {
        session: Session::new(Options {
            cwd: "/workspace".into(),
            mode: mode.map(str::to_owned),
            ..Options::default()
        }),
        writes: VecDeque::new(),
        approvals: HashMap::new(),
        log: Vec::new(),
    };
    let start = r.session.start();
    r.take(start);
    for (i, line) in text.lines().enumerate() {
        let entry: Value = serde_json::from_str(line).unwrap();
        let msg = &entry["message"];
        if entry["from"] == "agent" {
            let actions = r.session.on_line(&msg.to_string());
            r.take(actions);
            continue;
        }
        // The client side: trigger what the plugin would do, then compare.
        match msg["method"].as_str() {
            Some("session/prompt") => {
                let text = msg["params"]["prompt"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let actions = r.session.prompt(&text);
                r.take(actions);
            }
            Some("session/cancel") => {
                let actions = r.session.cancel();
                r.take(actions);
            }
            Some(_) => {}
            None => {
                // An answer to a permission request: decide as the recorded user did.
                let options = r.approvals[&msg["id"].to_string()].clone();
                let chosen = msg["result"]["outcome"]["optionId"].as_str().unwrap();
                let kind = options
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|o| o["optionId"] == chosen)
                    .unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let decision = match kind.as_str() {
                    "allow_once" => Decision::Allow,
                    "allow_always" => Decision::AllowAlways,
                    _ => Decision::Deny,
                };
                let actions = r.session.decide(&msg["id"], &options, decision);
                r.take(actions);
            }
        }
        let written = r
            .writes
            .pop_front()
            .unwrap_or_else(|| panic!("{name}[{i}]: the session wrote nothing; expected {msg}"));
        let written: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(&written, msg, "{name}[{i}]");
    }
    assert!(
        r.writes.is_empty(),
        "{name}: unexpected writes {:?}",
        r.writes
    );
    r.log
}

fn check(name: &str, mode: Option<&str>) {
    let log = replay(name, mode);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/expected")
        .join(format!("{name}.json"));
    let actual = serde_json::to_string_pretty(&log).unwrap() + "\n";
    if std::env::var_os("KUMIKIT_BLESS").is_some() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; run with KUMIKIT_BLESS=1", path.display()));
    assert_eq!(
        actual, expected,
        "events of {name} changed; run with KUMIKIT_BLESS=1 to accept"
    );
}

#[test]
fn claude_text_reply() {
    check("claude-text", None);
}

#[test]
fn claude_file_edit_approved() {
    check("claude-tool-allow", Some("default"));
}

#[test]
fn claude_command_denied() {
    check("claude-tool-deny", Some("default"));
}

#[test]
fn claude_read_only_command_needs_no_approval() {
    check("claude-tool-readonly", Some("default"));
}

#[test]
fn claude_cancelled_mid_answer() {
    check("claude-cancel", None);
}
