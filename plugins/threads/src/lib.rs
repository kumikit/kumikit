//! `kumikit.threads`: first-party agent tools for scheduling wake-ups, inspecting and
//! messaging threads, managing queues, and suggesting thread titles (ADR 0030).
//!
//! Built solely on the public `kumikit-tool` SDK (`kumikit:tool/plugin@0.1.0`).

pub mod format;

use std::time::{SystemTime, UNIX_EPOCH};

use kumikit_tool::turn::SteerStatus;
use kumikit_tool::{Error, ErrorCode, Plugin, Tool, ToolCall, ToolResult, client, threads, turn};
use serde_json::{Value, json};

struct Threads;

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn req_str<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

fn opt_str<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn calling_workspace(call: &ToolCall) -> Result<String, Error> {
    let raw = client::get_thread(&call.thread)?;
    let snap: Value = serde_json::from_str(&raw)
        .map_err(|e| Error::new(ErrorCode::Internal, format!("invalid thread snapshot: {e}")))?;
    snap.pointer("/thread/workspace")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::new(ErrorCode::Internal, "thread snapshot missing workspace"))
}

fn handle_wake_me(call: &ToolCall, input: &Value) -> Result<ToolResult, Error> {
    let text = match req_str(input, "text") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    let trigger = match format::resolve_wake_trigger(input, now_unix_secs()) {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    match threads::defer(&call.thread, text, &trigger.to_string()) {
        Ok(raw) => {
            let entry: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            let id = entry.get("id").and_then(Value::as_str).unwrap_or("?");
            let kind = trigger
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("trigger");
            Ok(ToolResult {
                text: format!("Scheduled wake-up {id} ({kind}) on thread {}.", call.thread),
                structured: Some(raw),
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

fn handle_list_threads(call: &ToolCall, input: &Value) -> Result<ToolResult, Error> {
    let ws = match opt_str(input, "workspace") {
        Some(w) => w.to_owned(),
        None => match calling_workspace(call) {
            Ok(w) => w,
            Err(e) => return Ok(ToolResult::failed(e.message)),
        },
    };
    match client::list_threads(&ws) {
        Ok(list) => {
            let mut text = String::new();
            let mut items = Vec::with_capacity(list.len());
            for t in &list {
                let marker = if t.id == call.thread {
                    " (this thread)"
                } else {
                    ""
                };
                let title = t.title.as_deref().unwrap_or("(untitled)");
                text.push_str(&format!("- {} [{}] {}{}\n", t.id, t.status, title, marker));
                items.push(json!({
                    "id": t.id,
                    "workspace": t.workspace,
                    "title": t.title,
                    "provider": t.provider,
                    "model": t.model,
                    "status": t.status,
                }));
            }
            if text.is_empty() {
                text.push_str("No threads in workspace.");
            }
            Ok(ToolResult {
                text,
                structured: Some(json!({ "workspace": ws, "threads": items }).to_string()),
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

fn handle_read_thread(input: &Value) -> Result<ToolResult, Error> {
    let thread = match req_str(input, "thread") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    let after = input.get("after").and_then(Value::as_u64).unwrap_or(0);
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 200) as u32;
    match threads::read_events(thread, after, limit) {
        Ok(raw) => {
            let events: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
            let (text, last_seq, truncated) = format::format_events_page(&events);
            Ok(ToolResult {
                text,
                structured: Some(
                    json!({
                        "thread": thread,
                        "count": events.len(),
                        "nextAfter": last_seq.unwrap_or(after),
                        "truncated": truncated,
                    })
                    .to_string(),
                ),
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

fn handle_list_queue(call: &ToolCall, input: &Value) -> Result<ToolResult, Error> {
    let thread = opt_str(input, "thread").unwrap_or(&call.thread);
    match threads::list_queue(thread) {
        Ok(raw) => {
            let entries: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
            Ok(ToolResult {
                text: format::format_queue(&entries),
                structured: Some(json!({ "thread": thread, "entries": entries }).to_string()),
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

fn handle_suggest_title(call: &ToolCall, input: &Value) -> Result<ToolResult, Error> {
    let thread = opt_str(input, "thread").unwrap_or(&call.thread);
    let title = match req_str(input, "title") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    match threads::suggest_title(thread, title) {
        Ok(applied) => {
            let text = if applied {
                format!("Updated title of {thread} to \"{title}\".")
            } else {
                format!("Kept existing user-set title on {thread}.")
            };
            Ok(ToolResult {
                text,
                structured: Some(json!({ "thread": thread, "applied": applied }).to_string()),
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

fn handle_message_thread(call: &ToolCall, input: &Value) -> Result<ToolResult, Error> {
    let thread = match req_str(input, "thread") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    let text = match req_str(input, "text") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    if thread == call.thread {
        return Ok(ToolResult::failed(
            "message_thread targets another thread; use wake_me for the calling thread",
        ));
    }
    match client::send(thread, text) {
        Ok(turn_id) => Ok(ToolResult {
            text: format!("Sent message to {thread} (turn {turn_id})."),
            structured: Some(json!({ "thread": thread, "turn": turn_id }).to_string()),
            is_error: false,
        }),
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

fn handle_queue_message(input: &Value) -> Result<ToolResult, Error> {
    let thread = match req_str(input, "thread") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    let text = match req_str(input, "text") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    if let Some(trigger) = input.get("trigger") {
        if !trigger.is_object() {
            return Ok(ToolResult::failed("trigger must be an object"));
        }
        return match threads::defer(thread, text, &trigger.to_string()) {
            Ok(raw) => Ok(ToolResult {
                text: format!("Deferred message on {thread}."),
                structured: Some(raw),
                is_error: false,
            }),
            Err(e) => Ok(ToolResult::failed(e.message)),
        };
    }
    match client::send(thread, text) {
        Ok(id) => Ok(ToolResult {
            text: format!("Queued message on {thread} ({id})."),
            structured: Some(json!({ "thread": thread, "id": id }).to_string()),
            is_error: false,
        }),
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

fn handle_steer_thread(input: &Value) -> Result<ToolResult, Error> {
    let thread = match req_str(input, "thread") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    let text = match req_str(input, "text") {
        Ok(t) => t,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    match turn::steer(thread, text) {
        Ok(res) => {
            let status = match res.status {
                SteerStatus::Accepted => "accepted",
                SteerStatus::Queued => "queued",
            };
            Ok(ToolResult {
                text: format!("Steer on {thread}: {status}."),
                structured: Some(
                    json!({
                        "thread": thread,
                        "status": status,
                        "turn": res.turn,
                        "entry": res.entry,
                    })
                    .to_string(),
                ),
                is_error: false,
            })
        }
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

fn handle_cancel_deferred(call: &ToolCall, input: &Value) -> Result<ToolResult, Error> {
    let thread = opt_str(input, "thread").unwrap_or(&call.thread);
    let entry = match req_str(input, "entry") {
        Ok(e) => e,
        Err(msg) => return Ok(ToolResult::failed(msg)),
    };
    match threads::remove_queued(thread, entry) {
        Ok(()) => Ok(ToolResult {
            text: format!("Removed entry {entry} from {thread}."),
            structured: Some(json!({ "thread": thread, "entry": entry }).to_string()),
            is_error: false,
        }),
        Err(e) => Ok(ToolResult::failed(e.message)),
    }
}

impl Plugin for Threads {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}
}

impl Tool for Threads {
    fn call(call: ToolCall) -> Result<ToolResult, Error> {
        let input: Value = serde_json::from_str(&call.input).unwrap_or_else(|_| json!({}));
        match call.name.as_str() {
            "wake_me" => handle_wake_me(&call, &input),
            "list_threads" => handle_list_threads(&call, &input),
            "read_thread" => handle_read_thread(&input),
            "list_queue" => handle_list_queue(&call, &input),
            "suggest_title" => handle_suggest_title(&call, &input),
            "message_thread" => handle_message_thread(&call, &input),
            "queue_message" => handle_queue_message(&input),
            "steer_thread" => handle_steer_thread(&input),
            "cancel_deferred" => handle_cancel_deferred(&call, &input),
            other => Err(Error::new(
                ErrorCode::NotFound,
                format!("unknown tool: {other}"),
            )),
        }
    }
}

kumikit_tool::export_tool!(Threads);
