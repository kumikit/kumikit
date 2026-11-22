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

/// A model reported by an ACP agent in `session/new` (`configOptions` or `models`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredModel {
    /// Model id (`value` in `configOptions`, or `modelId` in `models.availableModels`).
    pub id: String,
    /// Human-readable display label.
    pub label: String,
}

/// Permission mode requested for the session (`SessionConfig.options["permission-mode"]`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PermissionMode {
    /// Ask before edits and commands (default).
    #[default]
    Standard,
    /// Auto-approve file edits; ask before shell commands.
    AcceptEdits,
    /// Read-only planning and exploration.
    Plan,
    /// Auto-approve edits and commands without asking.
    FullAccess,
}

impl PermissionMode {
    /// Parses the wire value (`standard`, `accept_edits`, `plan`, `full_access`).
    pub fn parse(value: &str) -> Self {
        match value {
            "accept_edits" => PermissionMode::AcceptEdits,
            "plan" => PermissionMode::Plan,
            "full_access" => PermissionMode::FullAccess,
            _ => PermissionMode::Standard,
        }
    }
}

/// An attachment provided alongside a user prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// Display filename.
    pub name: String,
    /// MIME type (e.g. `image/png`, `text/plain`).
    pub mime_type: String,
    /// Host path on disk where the file was materialized.
    pub host_path: String,
    /// Base64-encoded bytes (present for images).
    pub data: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Initializing,
    Authenticating,
    Opening,
    SettingModel,
    SettingMode,
    SettingPermissions,
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
    /// Model id to select via ACP (`session/set_config_option` or `session/set_model`)
    /// after opening, when not `"default"` and different from the agent's current model.
    pub model: Option<String>,
    /// Session mode to set after opening (`session/set_mode`).
    pub mode: Option<String>,
    /// Requested permission mode from the thread.
    pub permission_mode: PermissionMode,
    /// ACP authentication method id to call with `authenticate` after `initialize`.
    pub auth_method: Option<String>,
    /// The tool gateway, passed to the agent as a stdio MCP server
    /// (`docs/spec/03-kernel.md#tool-gateway`).
    pub mcp: Option<McpStdio>,
    /// Stop as soon as `session/new` answers (used by `list-models` probing; skips
    /// `session/set_model` and `session/set_mode`).
    pub probe_only: bool,
}

/// A stdio MCP server for the agent to start (ACP `McpServer` stdio form).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpStdio {
    /// Its name.
    pub name: String,
    /// Absolute path of the program.
    pub command: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Environment variables (they hold a token: never logged).
    pub env: Vec<(String, String)>,
}

impl McpStdio {
    /// The ACP form: `{ name, command, args, env: [{ name, value }] }`.
    pub fn to_acp(&self) -> Value {
        json!({
            "name": self.name,
            "command": self.command,
            "args": self.args,
            "env": self.env.iter().map(|(k, v)| json!({ "name": k, "value": v })).collect::<Vec<_>>(),
        })
    }
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
    image_supported: bool,
    /// Models reported by the agent in `session/new` (first is the default).
    discovered_models: Vec<DiscoveredModel>,
    /// Config option id for model selection (`session/set_config_option`), if advertised.
    model_config_id: Option<String>,
    /// Current model reported by the agent in `session/new`.
    current_model: Option<String>,
    /// Whether `session/new` advertised `models.availableModels` (`session/set_model`).
    has_models_api: bool,
    /// Mode ids advertised in `modes.availableModes` or `configOptions[category="mode"]`.
    available_modes: Vec<String>,
    /// Current mode id reported by the agent in `session/new`.
    current_mode: Option<String>,
    /// Whether `session/new` advertised `modes.availableModes` (`session/set_mode`).
    has_modes_api: bool,
    /// Config option id for mode selection (`session/set_config_option`), if advertised.
    mode_config_id: Option<String>,
    /// Optional `(configId, allowValue)` for a `permissions` config option (e.g. `allow_all` -> `on`).
    allow_all_config: Option<(String, String)>,
    /// True once a native `accept-edits` mode was selected on the agent.
    native_accept_edits: bool,
    text: String,
    tools: HashMap<String, Tool>,
    tool_order: Vec<String>,
    cost: Option<f64>,
}

/// Extracts available models from an ACP `session/new` result.
///
/// Prefers `configOptions` with `category == "model"` (or `id == "model"`), falling back
/// to `models.availableModels`. Reorders the list so `"default"` (if present) or the
/// agent's `currentValue`/`currentModelId` comes first.
pub fn extract_models(result: &Value) -> Vec<DiscoveredModel> {
    parse_models_meta(result).0
}

/// Returns `(models, config_option_id, current_model, has_models_api)`.
fn parse_models_meta(
    result: &Value,
) -> (Vec<DiscoveredModel>, Option<String>, Option<String>, bool) {
    let has_models_api = result["models"]["availableModels"].is_array();
    let mut models = Vec::new();
    let mut config_id = None;
    let mut current = None;

    if let Some(opts) = result["configOptions"].as_array()
        && let Some(entry) = opts
            .iter()
            .find(|o| o["category"] == "model" || o["id"] == "model")
    {
        config_id = entry["id"].as_str().map(str::to_owned);
        current = entry["currentValue"].as_str().map(str::to_owned);
        if let Some(items) = entry["options"].as_array() {
            collect_config_options(items, &mut models);
        }
    }

    if models.is_empty()
        && let Some(avail) = result["models"]["availableModels"].as_array()
    {
        current = result["models"]["currentModelId"]
            .as_str()
            .map(str::to_owned);
        for item in avail {
            let Some(id) = item["modelId"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            let label = item["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(id)
                .to_owned();
            if !models.iter().any(|m| m.id == id) {
                models.push(DiscoveredModel {
                    id: id.to_owned(),
                    label,
                });
            }
        }
    }

    // Put "default" first if present; otherwise put the agent's current model first so
    // `models[0]` always represents the agent's default choice.
    if let Some(pos) = models.iter().position(|m| m.id == "default") {
        if pos > 0 {
            let item = models.remove(pos);
            models.insert(0, item);
        }
    } else if let Some(cur) = current.as_deref()
        && let Some(pos) = models.iter().position(|m| m.id == cur)
        && pos > 0
    {
        let item = models.remove(pos);
        models.insert(0, item);
    }

    (models, config_id, current, has_models_api)
}

#[derive(Default)]
struct ModesMeta {
    available: Vec<String>,
    current: Option<String>,
    has_modes_api: bool,
    mode_config_id: Option<String>,
    allow_all_config: Option<(String, String)>,
}

fn parse_modes_meta(result: &Value) -> ModesMeta {
    let mut meta = ModesMeta::default();
    if let Some(avail) = result["modes"]["availableModes"].as_array() {
        meta.has_modes_api = true;
        meta.current = result["modes"]["currentModeId"].as_str().map(str::to_owned);
        for item in avail {
            if let Some(id) = item["id"].as_str().filter(|s| !s.is_empty())
                && !meta.available.iter().any(|m| m == id)
            {
                meta.available.push(id.to_owned());
            }
        }
    }
    if let Some(opts) = result["configOptions"].as_array() {
        if let Some(entry) = opts
            .iter()
            .find(|o| o["category"] == "mode" || o["id"] == "mode")
        {
            meta.mode_config_id = entry["id"].as_str().map(str::to_owned);
            if meta.current.is_none() {
                meta.current = entry["currentValue"].as_str().map(str::to_owned);
            }
            if meta.available.is_empty()
                && let Some(items) = entry["options"].as_array()
            {
                for item in items {
                    if let Some(v) = item["value"].as_str().filter(|s| !s.is_empty())
                        && !meta.available.iter().any(|m| m == v)
                    {
                        meta.available.push(v.to_owned());
                    }
                }
            }
        }
        if let Some(perm) = opts.iter().find(|o| o["category"] == "permissions")
            && let Some(cid) = perm["id"].as_str().filter(|s| !s.is_empty())
            && let Some(items) = perm["options"].as_array()
        {
            for item in items {
                if let Some(val) = item["value"].as_str()
                    && matches!(
                        val.to_ascii_lowercase().as_str(),
                        "on" | "true" | "allow" | "allow_all"
                    )
                {
                    meta.allow_all_config = Some((cid.to_owned(), val.to_owned()));
                    break;
                }
            }
        }
    }
    meta
}

fn mode_suffix(id: &str) -> String {
    let tail = id
        .rsplit_once('#')
        .map(|(_, s)| s)
        .or_else(|| id.rsplit_once('/').map(|(_, s)| s))
        .unwrap_or(id);
    tail.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn pick_mode(available: &[String], candidates: &[&str]) -> Option<String> {
    for want in candidates {
        if let Some(found) = available.iter().find(|m| mode_suffix(m) == *want) {
            return Some(found.clone());
        }
    }
    None
}

fn collect_config_options(items: &[Value], out: &mut Vec<DiscoveredModel>) {
    for item in items {
        if let Some(group_items) = item["options"].as_array() {
            collect_config_options(group_items, out);
            continue;
        }
        let Some(id) = item["value"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let label = item["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(id)
            .to_owned();
        if !out.iter().any(|m| m.id == id) {
            out.push(DiscoveredModel {
                id: id.to_owned(),
                label,
            });
        }
    }
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
            image_supported: false,
            discovered_models: Vec::new(),
            model_config_id: None,
            current_model: None,
            has_models_api: false,
            available_modes: Vec::new(),
            current_mode: None,
            has_modes_api: false,
            mode_config_id: None,
            allow_all_config: None,
            native_accept_edits: false,
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

    /// Models reported by the agent during `session/new`, if any.
    pub fn discovered_models(&self) -> &[DiscoveredModel] {
        &self.discovered_models
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
        self.prompt_with_attachments(text, &[])
    }

    /// Sends a prompt with optional attachments (images as ACP `image` blocks when the
    /// agent advertises `promptCapabilities.image`, otherwise file-path text references).
    pub fn prompt_with_attachments(
        &mut self,
        text: &str,
        attachments: &[Attachment],
    ) -> Vec<Action> {
        let Some(session) = self.session.clone() else {
            return vec![Action::Failed("the session is not open".into())];
        };
        self.phase = Phase::Prompting;
        self.text.clear();
        self.tools.clear();
        self.tool_order.clear();
        self.cost = None;
        let mut blocks = Vec::new();
        if !text.is_empty() || attachments.is_empty() {
            blocks.push(json!({"type": "text", "text": text}));
        }
        let mut file_notes = Vec::new();
        for a in attachments {
            if self.image_supported
                && a.mime_type.starts_with("image/")
                && let Some(data) = &a.data
            {
                blocks.push(json!({
                    "type": "image",
                    "mimeType": a.mime_type,
                    "data": data,
                }));
            } else {
                file_notes.push(format!(
                    "Attached file: {} ({}) at {}",
                    a.name, a.mime_type, a.host_path
                ));
            }
        }
        if !file_notes.is_empty() {
            blocks.push(json!({"type": "text", "text": file_notes.join("\n")}));
        }
        vec![self.request(
            "session/prompt",
            json!({"sessionId": session, "prompt": blocks}),
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
            if self.options.permission_mode == PermissionMode::FullAccess {
                out.extend(self.decide(&id, &params["options"], Decision::AllowAlways));
                return out;
            }
            if self.options.permission_mode == PermissionMode::AcceptEdits
                && !self.native_accept_edits
                && matches!(
                    call["kind"].as_str(),
                    Some("edit" | "write" | "delete" | "move")
                )
            {
                out.extend(self.decide(&id, &params["options"], Decision::Allow));
                return out;
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

    fn open_session(&mut self) -> Vec<Action> {
        let mcp: Vec<Value> = self.options.mcp.iter().map(McpStdio::to_acp).collect();
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
                self.image_supported =
                    result["agentCapabilities"]["promptCapabilities"]["image"] == true;
                if let Some(method_id) = self.options.auth_method.clone() {
                    self.phase = Phase::Authenticating;
                    return vec![self.request("authenticate", json!({"methodId": method_id}))];
                }
                self.open_session()
            }
            "authenticate" => self.open_session(),
            "session/new" | "session/load" => {
                if let Some(s) = result["sessionId"].as_str() {
                    self.session = Some(s.to_owned());
                }
                let Some(session) = self.session.clone() else {
                    return vec![Action::Failed("the agent returned no session id".into())];
                };
                let (models, config_id, current, has_models_api) = parse_models_meta(result);
                if !models.is_empty() {
                    self.discovered_models = models;
                }
                self.model_config_id = config_id;
                self.current_model = current;
                self.has_models_api = has_models_api;

                let modes = parse_modes_meta(result);
                self.available_modes = modes.available;
                self.current_mode = modes.current;
                self.has_modes_api = modes.has_modes_api;
                self.mode_config_id = modes.mode_config_id;
                self.allow_all_config = modes.allow_all_config;

                if self.options.probe_only {
                    self.phase = Phase::Ready;
                    return vec![Action::Ready { session }];
                }
                self.maybe_set_model_or_mode(session)
            }
            "session/set_config_option" | "session/set_model" => {
                let session = self.session.clone().unwrap_or_default();
                match self.phase {
                    Phase::SettingModel => self.maybe_set_mode(session),
                    Phase::SettingMode => self.maybe_set_permissions(session),
                    Phase::SettingPermissions => {
                        self.phase = Phase::Ready;
                        vec![Action::Ready { session }]
                    }
                    _ => {
                        self.phase = Phase::Ready;
                        vec![Action::Ready { session }]
                    }
                }
            }
            "session/set_mode" => {
                let session = self.session.clone().unwrap_or_default();
                self.maybe_set_permissions(session)
            }
            "session/prompt" => self.turn_done(result),
            _ => Vec::new(),
        }
    }

    fn maybe_set_model_or_mode(&mut self, session: String) -> Vec<Action> {
        if let Some(model) = self.options.model.clone()
            && !model.is_empty()
            && model != "default"
            && self.current_model.as_deref() != Some(model.as_str())
        {
            if let Some(config_id) = self.model_config_id.clone() {
                self.phase = Phase::SettingModel;
                return vec![self.request(
                    "session/set_config_option",
                    json!({"sessionId": session, "configId": config_id, "value": model}),
                )];
            }
            if self.has_models_api {
                self.phase = Phase::SettingModel;
                return vec![self.request(
                    "session/set_model",
                    json!({"sessionId": session, "modelId": model}),
                )];
            }
        }
        self.maybe_set_mode(session)
    }

    fn maybe_set_mode(&mut self, session: String) -> Vec<Action> {
        let (target, explicit_manifest) = match self.options.permission_mode {
            PermissionMode::Standard => (self.options.mode.clone(), true),
            PermissionMode::AcceptEdits => {
                if let Some(m) = pick_mode(
                    &self.available_modes,
                    &["acceptedits", "autoedit", "autoedits"],
                ) {
                    self.native_accept_edits = true;
                    (Some(m), false)
                } else {
                    (self.options.mode.clone(), true)
                }
            }
            PermissionMode::Plan => (
                pick_mode(&self.available_modes, &["plan", "planmode"]),
                false,
            ),
            PermissionMode::FullAccess => (
                pick_mode(
                    &self.available_modes,
                    &[
                        "bypasspermissions",
                        "fullaccess",
                        "autopilot",
                        "auto",
                        "yolo",
                    ],
                ),
                false,
            ),
        };

        if let Some(mode) = target {
            let already_active =
                !explicit_manifest && self.current_mode.as_deref() == Some(mode.as_str());
            if !already_active {
                self.phase = Phase::SettingMode;
                if self.has_modes_api || explicit_manifest || self.mode_config_id.is_none() {
                    return vec![self.request(
                        "session/set_mode",
                        json!({"sessionId": session, "modeId": mode}),
                    )];
                }
                if let Some(config_id) = self.mode_config_id.clone() {
                    return vec![self.request(
                        "session/set_config_option",
                        json!({"sessionId": session, "configId": config_id, "value": mode}),
                    )];
                }
            }
        }
        self.maybe_set_permissions(session)
    }

    fn maybe_set_permissions(&mut self, session: String) -> Vec<Action> {
        if self.options.permission_mode == PermissionMode::FullAccess
            && let Some((config_id, value)) = self.allow_all_config.clone()
        {
            self.phase = Phase::SettingPermissions;
            return vec![self.request(
                "session/set_config_option",
                json!({"sessionId": session, "configId": config_id, "value": value}),
            )];
        }
        self.phase = Phase::Ready;
        vec![Action::Ready { session }]
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
    fn the_gateway_is_passed_as_a_stdio_mcp_server() {
        let mut s = Session::new(Options {
            cwd: "/w".into(),
            mcp: Some(McpStdio {
                name: "kumikit".into(),
                command: "/usr/bin/kumikit".into(),
                args: vec!["mcp".into()],
                env: vec![("KUMIKIT_MCP_TOKEN".into(), "t0k".into())],
            }),
            ..Options::default()
        });
        s.start();
        let out = s.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}")
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "session/new");
        assert_eq!(
            v["params"]["mcpServers"],
            json!([{ "name": "kumikit", "command": "/usr/bin/kumikit", "args": ["mcp"],
                     "env": [{ "name": "KUMIKIT_MCP_TOKEN", "value": "t0k" }] }])
        );
    }

    #[test]
    fn resume_uses_session_load_when_the_agent_supports_it() {
        let mut s = Session::new(Options {
            cwd: "/w".into(),
            resume: Some("old".into()),
            ..Options::default()
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

    #[test]
    fn authenticates_before_opening_session_when_configured() {
        let mut s = Session::new(Options {
            cwd: "/w".into(),
            auth_method: Some("login_method".into()),
            ..Options::default()
        });
        s.start();
        let out = s.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}")
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "authenticate");
        assert_eq!(v["params"], json!({"methodId": "login_method"}));

        let out = s.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#);
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}")
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "session/new");
        let out = s.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{"sessionId":"s1"}}"#);
        assert_eq!(
            out,
            [Action::Ready {
                session: "s1".into()
            }]
        );
    }

    #[test]
    fn extracts_models_from_config_options_and_puts_current_or_default_first() {
        // When `currentValue` is non-default and "default" is not in the list, `currentValue`
        // moves to index 0.
        let r = json!({
            "sessionId": "s1",
            "configOptions": [
                {"id": "mode", "category": "mode", "options": [{"value": "build", "name": "Build"}]},
                {
                    "id": "model",
                    "category": "model",
                    "currentValue": "opencode/tidepool",
                    "options": [
                        {"value": "deepseek/v4", "name": "DeepSeek V4"},
                        {"value": "opencode/tidepool", "name": "Tidepool"}
                    ]
                }
            ]
        });
        let models = extract_models(&r);
        assert_eq!(
            models,
            vec![
                DiscoveredModel {
                    id: "opencode/tidepool".into(),
                    label: "Tidepool".into()
                },
                DiscoveredModel {
                    id: "deepseek/v4".into(),
                    label: "DeepSeek V4".into()
                },
            ]
        );

        // When "default" is present, it comes first even if currentValue is something else.
        let r2 = json!({
            "sessionId": "s1",
            "configOptions": [{
                "id": "model",
                "category": "model",
                "currentValue": "haiku",
                "options": [
                    {"value": "opus", "name": "Opus 5.5"},
                    {"value": "default", "name": "Default (recommended)"},
                    {"value": "haiku", "name": "Haiku 5.5"}
                ]
            }]
        });
        assert_eq!(extract_models(&r2)[0].id, "default");

        // Falls back to `models.availableModels` when `configOptions` has no model entry.
        let r3 = json!({
            "sessionId": "s1",
            "models": {
                "currentModelId": "m2",
                "availableModels": [
                    {"modelId": "m1", "name": "Model One"},
                    {"modelId": "m2", "name": "Model Two"}
                ]
            }
        });
        assert_eq!(
            extract_models(&r3),
            vec![
                DiscoveredModel {
                    id: "m2".into(),
                    label: "Model Two".into()
                },
                DiscoveredModel {
                    id: "m1".into(),
                    label: "Model One".into()
                },
            ]
        );
    }

    #[test]
    fn sets_selected_model_via_config_option_or_set_model_and_skips_when_already_active() {
        // Different model with configOptions -> sends session/set_config_option then session/set_mode.
        let mut s = Session::new(Options {
            cwd: "/w".into(),
            model: Some("claude-opus-5-5".into()),
            mode: Some("default".into()),
            ..Options::default()
        });
        s.start();
        s.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let out = s.on_line(
            r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","configOptions":[{"id":"model","category":"model","currentValue":"gemini-4","options":[{"value":"gemini-4","name":"Gemini 4"},{"value":"claude-opus-5-5","name":"Claude Opus 5.5"}]}]}}"#,
        );
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}");
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "session/set_config_option");
        assert_eq!(
            v["params"],
            json!({"sessionId": "s1", "configId": "model", "value": "claude-opus-5-5"})
        );
        let out = s.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#);
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}");
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "session/set_mode");
        let out = s.on_line(r#"{"jsonrpc":"2.0","id":4,"result":{}}"#);
        assert_eq!(
            out,
            [Action::Ready {
                session: "s1".into()
            }]
        );

        // Same model as currentValue -> skips set_config_option and goes straight to Ready.
        let mut s2 = Session::new(Options {
            cwd: "/w".into(),
            model: Some("gemini-4".into()),
            ..Options::default()
        });
        s2.start();
        s2.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let out = s2.on_line(
            r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","configOptions":[{"id":"model","category":"model","currentValue":"gemini-4","options":[{"value":"gemini-4","name":"Gemini 4"}]}]}}"#,
        );
        assert_eq!(
            out,
            [Action::Ready {
                session: "s1".into()
            }]
        );

        // Agent with only `models.availableModels` -> uses `session/set_model`.
        let mut s3 = Session::new(Options {
            cwd: "/w".into(),
            model: Some("m2".into()),
            ..Options::default()
        });
        s3.start();
        s3.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let out = s3.on_line(
            r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","models":{"currentModelId":"m1","availableModels":[{"modelId":"m1","name":"One"},{"modelId":"m2","name":"Two"}]}}}"#,
        );
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}");
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "session/set_model");
        assert_eq!(v["params"], json!({"sessionId": "s1", "modelId": "m2"}));
        let out = s3.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#);
        assert_eq!(
            out,
            [Action::Ready {
                session: "s1".into()
            }]
        );
    }

    #[test]
    fn probe_only_stops_at_session_new_and_records_discovered_models() {
        let mut s = Session::new(Options {
            cwd: "/probe".into(),
            mode: Some("default".into()),
            probe_only: true,
            ..Options::default()
        });
        s.start();
        s.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let out = s.on_line(
            r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","configOptions":[{"id":"model","category":"model","currentValue":"m1","options":[{"value":"m1","name":"Model 1"}]}]}}"#,
        );
        assert_eq!(
            out,
            [Action::Ready {
                session: "s1".into()
            }]
        );
        assert_eq!(s.discovered_models().len(), 1);
        assert_eq!(s.discovered_models()[0].id, "m1");
    }

    #[test]
    fn permission_mode_selects_native_mode_and_auto_approves_in_full_access() {
        // Plan mode maps to advertised `plan` via session/set_config_option when modes api is absent.
        let mut s = Session::new(Options {
            cwd: "/w".into(),
            permission_mode: PermissionMode::Plan,
            ..Options::default()
        });
        s.start();
        s.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let out = s.on_line(
            r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","configOptions":[{"id":"mode","category":"mode","currentValue":"build","options":[{"value":"build","name":"Build"},{"value":"plan","name":"Plan"}]}]}}"#,
        );
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}");
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "session/set_config_option");
        assert_eq!(
            v["params"],
            json!({"sessionId": "s1", "configId": "mode", "value": "plan"})
        );
        assert_eq!(
            s.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#),
            [Action::Ready {
                session: "s1".into()
            }]
        );

        // Full access sets allow_all permission config option and auto-approves requests.
        let mut s2 = Session::new(Options {
            cwd: "/w".into(),
            permission_mode: PermissionMode::FullAccess,
            ..Options::default()
        });
        s2.start();
        s2.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let out = s2.on_line(
            r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","modes":{"availableModes":[{"id":"https://agentclientprotocol.com/protocol/session-modes#agent"},{"id":"https://agentclientprotocol.com/protocol/session-modes#autopilot"}],"currentModeId":"https://agentclientprotocol.com/protocol/session-modes#agent"},"configOptions":[{"id":"allow_all","category":"permissions","currentValue":"off","options":[{"value":"on","name":"On"},{"value":"off","name":"Off"}]}]}}"#,
        );
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}");
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "session/set_mode");
        assert_eq!(
            v["params"]["modeId"],
            "https://agentclientprotocol.com/protocol/session-modes#autopilot"
        );
        let out = s2.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#);
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}");
        };
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["method"], "session/set_config_option");
        assert_eq!(
            v["params"],
            json!({"sessionId": "s1", "configId": "allow_all", "value": "on"})
        );
        assert_eq!(
            s2.on_line(r#"{"jsonrpc":"2.0","id":4,"result":{}}"#),
            [Action::Ready {
                session: "s1".into()
            }]
        );

        // Even if the agent still requests permission in FullAccess, it is auto-approved.
        s2.prompt("go");
        let perm = s2.on_line(
            r#"{"jsonrpc":"2.0","id":99,"method":"session/request_permission","params":{"toolCall":{"toolCallId":"c1","title":"Run shell","kind":"execute"},"options":[{"optionId":"yes","kind":"allow_always"}]}}"#,
        );
        assert!(
            perm.iter()
                .any(|a| matches!(a, Action::Write(l) if l.contains("\"yes\"")))
        );
        assert!(!perm.iter().any(|a| matches!(a, Action::Approve { .. })));
    }

    #[test]
    fn multimodal_prompt_uses_image_block_when_supported_and_fallback_for_files() {
        let mut s = Session::new(Options {
            cwd: "/w".into(),
            ..Options::default()
        });
        s.start();
        s.on_line(
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":true}}}}"#,
        );
        s.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);

        let out = s.prompt_with_attachments(
            "check this",
            &[
                Attachment {
                    name: "shot.png".into(),
                    mime_type: "image/png".into(),
                    host_path: "/tmp/0-shot.png".into(),
                    data: Some("iVBORw0KGgo=".into()),
                },
                Attachment {
                    name: "notes.txt".into(),
                    mime_type: "text/plain".into(),
                    host_path: "/tmp/1-notes.txt".into(),
                    data: None,
                },
            ],
        );
        let [Action::Write(line)] = out.as_slice() else {
            panic!("{out:?}");
        };
        let v: Value = serde_json::from_str(line).unwrap();
        let blocks = v["params"]["prompt"].as_array().unwrap();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0], json!({"type": "text", "text": "check this"}));
        assert_eq!(
            blocks[1],
            json!({"type": "image", "mimeType": "image/png", "data": "iVBORw0KGgo="})
        );
        assert!(
            blocks[2]["text"]
                .as_str()
                .unwrap()
                .contains("/tmp/1-notes.txt")
        );
    }
}
