//! An Agent Client Protocol client without I/O (<https://agentclientprotocol.com>).
//!
//! [`Session`] takes lines from the agent ([`Session::on_line`]) and calls from the
//! plugin ([`Session::start`], [`Session::prompt`], [`Session::cancel`],
//! [`Session::decide`]) and returns [`Action`]s: lines to write, events to emit,
//! approvals to ask for, and the end of a phase. The WASM glue performs the actions;
//! tests replay recorded transcripts through it natively.
//!
//! Only the subset of ACP this plugin needs is modeled, as JSON values, so unknown
//! fields and update kinds never break parsing.

use std::collections::HashMap;

use serde_json::{Value, json};

/// The ACP version this client speaks.
pub const PROTOCOL_VERSION: u64 = 1;

/// An event for the running turn, in the kernel's vocabulary.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// `text.delta`
    TextDelta {
        /// Text.
        text: String,
    },
    /// `text.done`
    TextDone {
        /// The whole message.
        text: String,
    },
    /// `reasoning.delta`
    ReasoningDelta {
        /// Text.
        text: String,
    },
    /// `tool.started`
    ToolStarted {
        /// Call id.
        call: String,
        /// Tool name.
        name: String,
        /// Input.
        input: Value,
    },
    /// `tool.output`
    ToolOutput {
        /// Call id.
        call: String,
        /// Output text.
        chunk: String,
    },
    /// `tool.finished`
    ToolFinished {
        /// Call id.
        call: String,
        /// Whether it succeeded.
        ok: bool,
        /// Result.
        result: Option<Value>,
    },
    /// `file.changed`
    FileChanged {
        /// Path relative to the workspace root.
        path: String,
        /// Unified diff.
        diff: Option<String>,
    },
    /// `usage`
    Usage {
        /// Input tokens.
        input_tokens: u64,
        /// Output tokens.
        output_tokens: u64,
        /// Cost in US dollars.
        cost: Option<f64>,
    },
    /// `plugin.event`
    Plugin {
        /// Kind, `acp.<update>`.
        kind: String,
        /// The update.
        data: Value,
    },
}

/// The user's answer to an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Allow once.
    Allow,
    /// Allow this kind from now on.
    AllowAlways,
    /// Deny.
    Deny,
    /// The turn was cancelled before an answer.
    Cancelled,
}

/// Something the glue must do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Write this line (without newline) to the agent's stdin.
    Write(String),
    /// Emit an event.
    Emit(Event),
    /// Ask the user, then call [`Session::decide`] with `request`.
    Approve {
        /// The agent's request id.
        request: Value,
        /// Approval kind.
        kind: String,
        /// One-line summary.
        summary: String,
        /// Details.
        detail: Value,
    },
    /// `start` finished: the session is ready for prompts.
    Ready {
        /// The agent's session id.
        session: String,
    },
    /// The current prompt finished.
    TurnDone,
    /// The current phase failed.
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Initializing,
    Opening,
    SettingMode,
    Ready,
    Prompting,
}

#[derive(Debug, Default)]
struct Tool {
    started: bool,
    finished: bool,
    title: String,
    input: Value,
    diffs: Vec<(String, Option<String>, String)>,
    /// Text content already emitted. ACP updates replace the whole content list, so the
    /// same item arrives again with each update.
    emitted: Vec<String>,
}

/// Options of a session.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Working directory (the workspace root).
    pub cwd: String,
    /// The agent's session id to resume, if any.
    pub resume: Option<String>,
    /// Session mode to set after opening (`session/set_mode`).
    pub mode: Option<String>,
}

/// One ACP session.
#[derive(Debug)]
pub struct Session {
    options: Options,
    phase: Phase,
    next_id: u64,
    /// Our request ids and what they were for.
    pending: HashMap<u64, &'static str>,
    session: Option<String>,
    load_supported: bool,
    text: String,
    tools: HashMap<String, Tool>,
    tool_order: Vec<String>,
    cost: Option<f64>,
}

fn update_text(update: &Value) -> Option<String> {
    let c = &update["content"];
    (c["type"] == "text").then(|| c["text"].as_str().unwrap_or_default().to_owned())
}

/// A path relative to `root`, or the path itself if it is elsewhere.
fn relative(root: &str, path: &str) -> String {
    let root = root.trim_end_matches('/');
    path.strip_prefix(root)
        .map(|p| p.trim_start_matches('/').to_owned())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| path.to_owned())
}

/// A whole-file unified diff from old and new text.
pub fn unified_diff(path: &str, old: Option<&str>, new: &str) -> String {
    let old_lines: Vec<&str> = old.map(|o| o.lines().collect()).unwrap_or_default();
    let new_lines: Vec<&str> = new.lines().collect();
    let from = if old.is_some() {
        format!("a/{path}")
    } else {
        "/dev/null".to_owned()
    };
    let mut d = format!(
        "--- {from}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        usize::from(!old_lines.is_empty()),
        old_lines.len(),
        usize::from(!new_lines.is_empty()),
        new_lines.len()
    );
    for l in &old_lines {
        d.push('-');
        d.push_str(l);
        d.push('\n');
    }
    for l in &new_lines {
        d.push('+');
        d.push_str(l);
        d.push('\n');
    }
    d
}

impl Session {
    /// A session not yet started.
    pub fn new(options: Options) -> Self {
        Session {
            options,
            phase: Phase::Idle,
            next_id: 0,
            pending: HashMap::new(),
            session: None,
            load_supported: false,
            text: String::new(),
            tools: HashMap::new(),
            tool_order: Vec::new(),
            cost: None,
        }
    }

    /// The agent's session id, once open.
    pub fn id(&self) -> Option<&str> {
        self.session.as_deref()
    }

    fn request(&mut self, method: &'static str, params: Value) -> Action {
        self.next_id += 1;
        let id = self.next_id;
        self.pending.insert(id, method);
        Action::Write(
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        )
    }

    /// Starts the session: `initialize`, then `session/new` or `session/load`, then the
    /// mode if one is configured. Ends with [`Action::Ready`] or [`Action::Failed`].
    pub fn start(&mut self) -> Vec<Action> {
        self.phase = Phase::Initializing;
        vec![self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false},
                    "terminal": false
                }
            }),
        )]
    }

    /// Sends a prompt. Ends with [`Action::TurnDone`] or [`Action::Failed`].
    pub fn prompt(&mut self, text: &str) -> Vec<Action> {
        let Some(session) = self.session.clone() else {
            return vec![Action::Failed("the session is not open".into())];
        };
        self.phase = Phase::Prompting;
        self.text.clear();
        self.tools.clear();
        self.tool_order.clear();
        self.cost = None;
        vec![self.request(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
        )]
    }

    /// Asks the agent to stop the running prompt (`session/cancel`); the prompt then
    /// ends with stop reason `cancelled`.
    pub fn cancel(&mut self) -> Vec<Action> {
        match (&self.session, self.phase) {
            (Some(s), Phase::Prompting) => vec![Action::Write(
                json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": s}})
                    .to_string(),
            )],
            _ => Vec::new(),
        }
    }

    /// Answers a permission request with the option matching `decision`.
    pub fn decide(&mut self, request: &Value, options: &Value, decision: Decision) -> Vec<Action> {
        let pick = |kinds: &[&str]| {
            options.as_array().and_then(|opts| {
                kinds.iter().find_map(|k| {
                    opts.iter()
                        .find(|o| o["kind"] == *k)
                        .and_then(|o| o["optionId"].as_str())
                        .map(str::to_owned)
                })
            })
        };
        let chosen = match decision {
            Decision::Allow => pick(&["allow_once", "allow_always"]),
            Decision::AllowAlways => pick(&["allow_always", "allow_once"]),
            Decision::Deny => pick(&["reject_once", "reject_always"]),
            Decision::Cancelled => None,
        };
        let outcome = match chosen {
            Some(id) => json!({"outcome": "selected", "optionId": id}),
            None => json!({"outcome": "cancelled"}),
        };
        vec![Action::Write(
            json!({"jsonrpc": "2.0", "id": request, "result": {"outcome": outcome}}).to_string(),
        )]
    }

    /// Handles one line from the agent.
    pub fn on_line(&mut self, line: &str) -> Vec<Action> {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            // Not JSON-RPC (a log line on stdout); ignore it.
            return Vec::new();
        };
        match (msg.get("method").and_then(Value::as_str), msg.get("id")) {
            (Some(method), Some(id)) => self.on_request(method, id.clone(), &msg["params"]),
            (Some("session/update"), None) => self.on_update(&msg["params"]["update"]),
            (Some(_), None) => Vec::new(),
            (None, Some(id)) => self.on_response(id, &msg),
            (None, None) => Vec::new(),
        }
    }

    fn on_request(&mut self, method: &str, id: Value, params: &Value) -> Vec<Action> {
        if method == "session/request_permission" && self.phase == Phase::Prompting {
            let call = &params["toolCall"];
            let mut out = Vec::new();
            if let Some(cid) = call["toolCallId"].as_str() {
                out.extend(self.tool_update(cid, call));
            }
            let summary = call["title"]
                .as_str()
                .or_else(|| call["name"].as_str())
                .unwrap_or("Use a tool")
                .to_owned();
            out.push(Action::Approve {
                request: id,
                kind: "tool".into(),
                summary,
                detail: json!({
                    "toolCall": {
                        "toolCallId": call["toolCallId"],
                        "kind": call["kind"],
                        "rawInput": call["rawInput"],
                        "locations": call["locations"],
                    },
                    "options": params["options"],
                }),
            });
            return out;
        }
        // We offered no file-system or terminal capabilities.
        vec![Action::Write(
            json!({"jsonrpc": "2.0", "id": id,
                   "error": {"code": -32601, "message": format!("{method} is not supported")}})
            .to_string(),
        )]
    }

    fn on_response(&mut self, id: &Value, msg: &Value) -> Vec<Action> {
        let Some(method) = id.as_u64().and_then(|i| self.pending.remove(&i)) else {
            return Vec::new();
        };
        if let Some(err) = msg.get("error") {
            let message = err["message"].as_str().unwrap_or("agent error").to_owned();
            self.phase = if self.session.is_some() {
                Phase::Ready
            } else {
                Phase::Idle
            };
            return vec![Action::Failed(format!("{method}: {message}"))];
        }
        let result = &msg["result"];
        match method {
            "initialize" => {
                self.load_supported = result["agentCapabilities"]["loadSession"] == true;
                let mcp: Vec<Value> = Vec::new();
                match self.options.resume.clone() {
                    Some(s) if self.load_supported => {
                        self.phase = Phase::Opening;
                        self.session = Some(s.clone());
                        vec![self.request(
                            "session/load",
                            json!({"sessionId": s, "cwd": self.options.cwd, "mcpServers": mcp}),
                        )]
                    }
                    _ => {
                        self.phase = Phase::Opening;
                        vec![self.request(
                            "session/new",
                            json!({"cwd": self.options.cwd, "mcpServers": mcp}),
                        )]
                    }
                }
            }
            "session/new" | "session/load" => {
                if let Some(s) = result["sessionId"].as_str() {
                    self.session = Some(s.to_owned());
                }
                let Some(session) = self.session.clone() else {
                    return vec![Action::Failed("the agent returned no session id".into())];
                };
                match self.options.mode.clone() {
                    Some(mode) => {
                        self.phase = Phase::SettingMode;
                        vec![self.request(
                            "session/set_mode",
                            json!({"sessionId": session, "modeId": mode}),
                        )]
                    }
                    None => {
                        self.phase = Phase::Ready;
                        vec![Action::Ready { session }]
                    }
                }
            }
            "session/set_mode" => {
                self.phase = Phase::Ready;
                vec![Action::Ready {
                    session: self.session.clone().unwrap_or_default(),
                }]
            }
            "session/prompt" => self.turn_done(result),
            _ => Vec::new(),
        }
    }

    fn turn_done(&mut self, result: &Value) -> Vec<Action> {
        self.phase = Phase::Ready;
        let mut out = Vec::new();
        // Tools the agent never finished (a cancelled turn) end as failed.
        for cid in std::mem::take(&mut self.tool_order) {
            if let Some(t) = self.tools.get(&cid)
                && t.started
                && !t.finished
            {
                out.push(Action::Emit(Event::ToolFinished {
                    call: cid,
                    ok: false,
                    result: None,
                }));
            }
        }
        if !self.text.is_empty() {
            out.push(Action::Emit(Event::TextDone {
                text: std::mem::take(&mut self.text),
            }));
        }
        let usage = &result["usage"];
        if usage.is_object() || self.cost.is_some() {
            out.push(Action::Emit(Event::Usage {
                input_tokens: usage["inputTokens"].as_u64().unwrap_or(0),
                output_tokens: usage["outputTokens"].as_u64().unwrap_or(0),
                cost: self.cost,
            }));
        }
        out.push(Action::TurnDone);
        out
    }

    fn on_update(&mut self, u: &Value) -> Vec<Action> {
        // Updates outside a prompt (history replayed by `session/load`, notices) are not
        // part of a turn.
        if self.phase != Phase::Prompting {
            return Vec::new();
        }
        match u["sessionUpdate"].as_str().unwrap_or_default() {
            "agent_message_chunk" => match update_text(u) {
                Some(text) => {
                    self.text.push_str(&text);
                    vec![Action::Emit(Event::TextDelta { text })]
                }
                None => Vec::new(),
            },
            "agent_thought_chunk" => update_text(u)
                .map(|text| vec![Action::Emit(Event::ReasoningDelta { text })])
                .unwrap_or_default(),
            "tool_call" | "tool_call_update" => match u["toolCallId"].as_str() {
                Some(cid) => {
                    let cid = cid.to_owned();
                    self.tool_update(&cid, u)
                }
                None => Vec::new(),
            },
            "usage_update" => {
                if let Some(c) = u["cost"]["amount"].as_f64()
                    && u["cost"]["currency"] == "USD"
                {
                    self.cost = Some(c);
                }
                Vec::new()
            }
            "plan" => vec![Action::Emit(Event::Plugin {
                kind: "acp.plan".into(),
                data: u.clone(),
            })],
            // Session bookkeeping, not turn content.
            "available_commands_update"
            | "session_info_update"
            | "config_option_update"
            | "current_mode_update"
            | "user_message_chunk" => Vec::new(),
            other => vec![Action::Emit(Event::Plugin {
                kind: format!("acp.{other}"),
                data: u.clone(),
            })],
        }
    }

    /// Folds a `tool_call` or `tool_call_update` (or a permission request's tool call)
    /// into the tool's state and returns the events it causes.
    fn tool_update(&mut self, cid: &str, u: &Value) -> Vec<Action> {
        if !self.tools.contains_key(cid) {
            self.tool_order.push(cid.to_owned());
        }
        let root = self.options.cwd.clone();
        let tool = self.tools.entry(cid.to_owned()).or_default();
        if let Some(t) = u["title"].as_str() {
            tool.title = t.to_owned();
        }
        if u["rawInput"].as_object().is_some_and(|o| !o.is_empty()) {
            tool.input = u["rawInput"].clone();
        }
        let mut out = Vec::new();
        let status = u["status"].as_str();
        // Start once the input is known, or when the call moves past `pending`.
        let ready =
            !tool.input.is_null() || matches!(status, Some("in_progress" | "completed" | "failed"));
        if !tool.started && ready {
            tool.started = true;
            let name = if tool.title.is_empty() {
                u["kind"].as_str().unwrap_or("tool").to_owned()
            } else {
                tool.title.clone()
            };
            out.push(Action::Emit(Event::ToolStarted {
                call: cid.to_owned(),
                name,
                input: if tool.input.is_null() {
                    json!({})
                } else {
                    tool.input.clone()
                },
            }));
        }
        if let Some(items) = u["content"].as_array() {
            for item in items {
                match item["type"].as_str() {
                    Some("content") if item["content"]["type"] == "text" => {
                        if tool.started
                            && let Some(t) = item["content"]["text"].as_str()
                            && !tool.emitted.iter().any(|e| e == t)
                        {
                            tool.emitted.push(t.to_owned());
                            out.push(Action::Emit(Event::ToolOutput {
                                call: cid.to_owned(),
                                chunk: t.to_owned(),
                            }));
                        }
                    }
                    Some("diff") => {
                        let path = item["path"].as_str().unwrap_or_default().to_owned();
                        let old = item["oldText"].as_str().map(str::to_owned);
                        let new = item["newText"].as_str().unwrap_or_default().to_owned();
                        tool.diffs.retain(|(p, _, _)| p != &path);
                        tool.diffs.push((path, old, new));
                    }
                    _ => {}
                }
            }
        }
        if tool.started && !tool.finished && matches!(status, Some("completed" | "failed")) {
            tool.finished = true;
            let ok = status == Some("completed");
            if ok {
                for (path, old, new) in std::mem::take(&mut tool.diffs) {
                    let rel = relative(&root, &path);
                    let diff = unified_diff(&rel, old.as_deref(), &new);
                    out.push(Action::Emit(Event::FileChanged {
                        path: rel,
                        diff: Some(diff),
                    }));
                }
            }
            let result = match &u["rawOutput"] {
                Value::Null => None,
                v => Some(v.clone()),
            };
            out.push(Action::Emit(Event::ToolFinished {
                call: cid.to_owned(),
                ok,
                result,
            }));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_are_whole_file_unified_diffs() {
        assert_eq!(
            unified_diff("hello.txt", None, "hi\n"),
            "--- /dev/null\n+++ b/hello.txt\n@@ -0,0 +1,1 @@\n+hi\n"
        );
        assert_eq!(
            unified_diff("a.txt", Some("x\n"), "y\n"),
            "--- a/a.txt\n+++ b/a.txt\n@@ -1,1 +1,1 @@\n-x\n+y\n"
        );
    }

    #[test]
    fn paths_are_relative_to_the_workspace() {
        assert_eq!(relative("/workspace", "/workspace/a/b.txt"), "a/b.txt");
        assert_eq!(relative("/workspace/", "/workspace/a"), "a");
        assert_eq!(relative("/workspace", "/etc/x"), "/etc/x");
    }

    #[test]
    fn unsupported_agent_requests_get_an_error() {
        let mut s = Session::new(Options::default());
        let out = s.on_line(r#"{"jsonrpc":"2.0","id":7,"method":"fs/read_text_file","params":{}}"#);
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}")
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["error"]["code"], -32601);
    }

    #[test]
    fn resume_uses_session_load_when_the_agent_supports_it() {
        let mut s = Session::new(Options {
            cwd: "/w".into(),
            resume: Some("old".into()),
            mode: None,
        });
        s.start();
        let out = s.on_line(
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}"#,
        );
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert!(
            line.contains("session/load") && line.contains("\"old\""),
            "{line}"
        );
        // History replayed during load is not turn content.
        let replayed = s.on_line(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"old","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old"}}}}"#,
        );
        assert!(replayed.is_empty());
        let out = s.on_line(r#"{"jsonrpc":"2.0","id":2,"result":null}"#);
        assert_eq!(
            out,
            [Action::Ready {
                session: "old".into()
            }]
        );
    }
}
