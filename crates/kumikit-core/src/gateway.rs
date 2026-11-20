//! The tool gateway's MCP server (`docs/spec/03-kernel.md#tool-gateway`, ADR 0025). A
//! local connection whose first message is a valid `gateway.attach` carries MCP from then
//! on, for that token's provider session: `initialize`, `ping`, `tools/list`,
//! `tools/call`, and `notifications/tools/list_changed` when plugins change.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use kumikit_kernel::gateway::GatewaySession;
use kumikit_plugin_host::lifecycle::{PluginHost, ToolInfo};
use kumikit_protocol::event::ToolSource;
use kumikit_protocol::model::Resolution;
use serde_json::{Value, json};
use tokio::sync::{Semaphore, mpsc};

use crate::messages::{MessageReader, Received};

/// MCP protocol versions this server speaks, newest first.
pub const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
/// Calls one session may have in flight.
pub const IN_FLIGHT: usize = 4;
/// Largest input and result.
pub const MAX_SIZE: usize = 1 << 20;
/// Wall time of one call.
pub const CALL_TIME: Duration = Duration::from_secs(300);

static CALLS: AtomicU64 = AtomicU64::new(1);

fn reply(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error(id: &Value, code: i64, message: impl Into<String>) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
        .to_string()
}

fn tool_error(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": true })
}

fn listing(tools: &[ToolInfo]) -> Value {
    json!({
        "tools": tools.iter().map(|t| json!({
            "name": t.mcp_name,
            "title": t.title,
            "description": t.description,
            "inputSchema": t.input_schema,
            "annotations": { "readOnlyHint": !t.write },
        })).collect::<Vec<_>>()
    })
}

/// Serves MCP for `session` until the agent disconnects.
pub async fn serve<R: MessageReader>(
    host: PluginHost,
    session: GatewaySession,
    mut reader: R,
    tx: mpsc::Sender<String>,
) {
    let limit = Arc::new(Semaphore::new(IN_FLIGHT));
    let mut changes = host.changes();
    changes.mark_unchanged();
    let announce = {
        let tx = tx.clone();
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let note =
                    json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" });
                if tx.send(note.to_string()).await.is_err() {
                    return;
                }
            }
        })
    };
    tracing::debug!(thread = %session.thread, "tool gateway attached");
    loop {
        let message = match reader.recv().await {
            Ok(Received::Message(m)) if m.is_empty() => continue,
            Ok(Received::Message(m)) => m,
            Ok(Received::Eof) | Ok(Received::Refused(_)) | Err(_) => break,
        };
        let Ok(value) = serde_json::from_slice::<Value>(&message) else {
            let _ = tx.send(error(&Value::Null, -32700, "not JSON")).await;
            continue;
        };
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(id) = value.get("id").cloned() else {
            // Notifications (`initialized`, `cancelled`) need no answer.
            continue;
        };
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        tracing::debug!(thread = %session.thread, %method, "gateway request");
        match method.as_str() {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .filter(|v| VERSIONS.contains(v))
                    .unwrap_or(VERSIONS[0]);
                let result = json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": true } },
                    "serverInfo": { "name": "kumikit", "version": env!("CARGO_PKG_VERSION") },
                });
                let _ = tx.send(reply(&id, result)).await;
            }
            "ping" => {
                let _ = tx.send(reply(&id, json!({}))).await;
            }
            "tools/list" => {
                let _ = tx.send(reply(&id, listing(&host.tools()))).await;
            }
            "tools/call" => {
                let (host, session, tx, limit) = (
                    host.clone(),
                    session.clone(),
                    tx.clone(),
                    Arc::clone(&limit),
                );
                tokio::spawn(async move {
                    let Ok(_permit) = limit.acquire_owned().await else {
                        return;
                    };
                    let result = call(&host, &session, &params).await;
                    let _ = tx.send(reply(&id, result)).await;
                });
            }
            other => {
                let _ = tx
                    .send(error(&id, -32601, format!("no method {other}")))
                    .await;
            }
        }
    }
    announce.abort();
    tracing::debug!(thread = %session.thread, "tool gateway detached");
}

/// One `tools/call`: checks, logs, asks when needed, runs. Returns the MCP result.
async fn call(host: &PluginHost, session: &GatewaySession, params: &Value) -> Value {
    let kernel = host.manager().kernel().clone();
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let Some(tool) = host.tools().into_iter().find(|t| t.mcp_name == name) else {
        return tool_error(format!("no tool {name}"));
    };
    let Some(turn) = kernel.running_turn(&session.thread) else {
        return tool_error("no turn is running in this thread, so its tools cannot be called");
    };
    if input.to_string().len() > MAX_SIZE {
        return tool_error("the input is larger than 1 MiB");
    }
    match jsonschema::validator_for(&tool.input_schema) {
        Ok(v) => {
            let problems: Vec<String> = v.iter_errors(&input).map(|e| e.to_string()).collect();
            if !problems.is_empty() {
                return tool_error(format!("invalid arguments: {}", problems.join("; ")));
            }
        }
        Err(e) => return tool_error(format!("the tool's schema is broken: {e}")),
    }
    let call_id = format!("gateway-{}", CALLS.fetch_add(1, Ordering::Relaxed));
    let source = ToolSource {
        plugin: tool.plugin.clone(),
        tool: tool.name.clone(),
    };
    if let Err(e) = kernel
        .gateway_tool_started(
            &session.thread,
            &turn,
            &call_id,
            &tool.mcp_name,
            input.clone(),
            source,
        )
        .await
    {
        return tool_error(format!("cannot record the call: {e}"));
    }
    let finish = |ok: bool, text: String, result: Option<Value>| {
        let kernel = kernel.clone();
        let (thread, turn, call_id) = (session.thread.clone(), turn.clone(), call_id.clone());
        async move {
            let _ = kernel
                .gateway_tool_finished(&thread, &turn, &call_id, Some(text), ok, result)
                .await;
        }
    };
    if tool.write {
        let always = kernel
            .gateway_always_allowed(&session.thread, &tool.mcp_name)
            .await
            .unwrap_or(false);
        if !always {
            let detail = json!({ "tool": tool.mcp_name, "plugin": tool.plugin, "input": input });
            let decision = kernel
                .gateway_request_approval(
                    &session.thread,
                    &turn,
                    format!("Run `{}`", tool.mcp_name),
                    detail,
                )
                .await;
            match decision {
                Ok(Resolution::Allow | Resolution::AllowAlways) => {}
                Ok(_) => {
                    let why = "the user did not allow this call".to_owned();
                    finish(false, why.clone(), Some(json!({ "denied": true }))).await;
                    return tool_error(why);
                }
                Err(e) => {
                    let why = format!("cannot ask for approval: {e}");
                    finish(false, why.clone(), None).await;
                    return tool_error(why);
                }
            }
        }
    }
    let ran = tokio::time::timeout(
        CALL_TIME,
        host.call_tool(
            &tool.plugin,
            &tool.name,
            &input,
            &session.thread.0,
            &session.workspace_root,
        ),
    )
    .await;
    let outcome = match ran {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            let why = e.to_string();
            finish(false, why.clone(), None).await;
            return tool_error(why);
        }
        Err(_) => {
            let why = "the tool took longer than 5 minutes".to_owned();
            finish(false, why.clone(), None).await;
            return tool_error(why);
        }
    };
    let mut text = outcome.text;
    if text.len() > MAX_SIZE {
        let mut cut = MAX_SIZE;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n[cut: the result was longer than 1 MiB]");
    }
    let structured: Option<Value> = outcome
        .structured
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .filter(|v: &Value| v.to_string().len() <= MAX_SIZE);
    finish(!outcome.is_error, text.clone(), structured.clone()).await;
    let mut result =
        json!({ "content": [{ "type": "text", "text": text }], "isError": outcome.is_error });
    if let Some(s) = structured {
        result["structuredContent"] = s;
    }
    result
}
