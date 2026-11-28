//! Pure trigger parsing, UTC RFC 3339 formatting, and bounded thread/queue formatting.
//! Kept free of WASM host calls so native `cargo test` covers it directly.

use serde_json::{Value, json};

/// Maximum text size returned by `read_thread`.
pub const MAX_READ_BYTES: usize = 32 * 1024;

/// Formats a Unix timestamp in whole seconds as UTC RFC 3339 (`YYYY-MM-DDTHH:MM:SS.000Z`).
/// Uses Howard Hinnant's civil-from-days algorithm so no calendar crate is needed.
pub fn format_utc_rfc3339(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);

    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}.000Z")
}

/// Resolves the `wake_me` arguments into a wire `QueueTrigger` JSON value.
/// `now_unix_secs` is the current UTC Unix timestamp in seconds (injected for tests).
pub fn resolve_wake_trigger(input: &Value, now_unix_secs: i64) -> Result<Value, String> {
    if let Some(trigger) = input.get("trigger") {
        if !trigger.is_object() {
            return Err("trigger must be an object".into());
        }
        return Ok(trigger.clone());
    }
    if let Some(minutes) = input.get("inMinutes").and_then(Value::as_i64) {
        if !(1..=10_080).contains(&minutes) {
            return Err("inMinutes must be between 1 and 10080".into());
        }
        let target = now_unix_secs
            .checked_add(minutes * 60)
            .ok_or_else(|| "timestamp overflow".to_owned())?;
        return Ok(json!({ "kind": "at", "time": format_utc_rfc3339(target) }));
    }
    if let Some(at) = input.get("at").and_then(Value::as_str) {
        if at.trim().is_empty() {
            return Err("at must be a non-empty RFC 3339 timestamp".into());
        }
        return Ok(json!({ "kind": "at", "time": at.trim() }));
    }
    if let Some(thread) = input.get("afterThread").and_then(Value::as_str) {
        if thread.trim().is_empty() {
            return Err("afterThread must be a non-empty thread id".into());
        }
        let only_on_success = input
            .get("onlyOnSuccess")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        return Ok(json!({
            "kind": "after_turn",
            "thread": thread.trim(),
            "onlyOnSuccess": only_on_success,
        }));
    }
    if let Some(argv) = string_array(input.get("afterCommand"))? {
        let mut obj = json!({
            "kind": "after_command",
            "argv": argv,
            "timeoutS": input.get("timeoutS").and_then(Value::as_u64).unwrap_or(300),
        });
        if let Some(cwd) = input.get("cwd").and_then(Value::as_str)
            && !cwd.trim().is_empty()
        {
            obj["cwd"] = json!(cwd.trim());
        }
        return Ok(obj);
    }
    if let Some(argv) = string_array(input.get("pollCommand"))? {
        let mut obj = json!({
            "kind": "poll_command",
            "argv": argv,
            "everyS": input.get("everyS").and_then(Value::as_u64).unwrap_or(60),
            "maxTries": input.get("maxTries").and_then(Value::as_u64).unwrap_or(30),
            "timeoutS": input.get("timeoutS").and_then(Value::as_u64).unwrap_or(300),
        });
        if let Some(cwd) = input.get("cwd").and_then(Value::as_str)
            && !cwd.trim().is_empty()
        {
            obj["cwd"] = json!(cwd.trim());
        }
        return Ok(obj);
    }
    Err("specify one of: trigger, inMinutes, at, afterThread, afterCommand, or pollCommand".into())
}

fn string_array(val: Option<&Value>) -> Result<Option<Vec<String>>, String> {
    let Some(v) = val else {
        return Ok(None);
    };
    let Some(arr) = v.as_array() else {
        return Err("command argv must be an array of strings".into());
    };
    if arr.is_empty() {
        return Err("command argv must not be empty".into());
    }
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(s) = item.as_str() else {
            return Err("command argv entries must be strings".into());
        };
        out.push(s.to_owned());
    }
    Ok(Some(out))
}

/// Formats a page of thread events returned by `threads::read_events` into bounded text
/// (capped at [`MAX_READ_BYTES`]) and returns `(text, last_seq, truncated)`.
pub fn format_events_page(events: &[Value]) -> (String, Option<u64>, bool) {
    let mut out = String::new();
    let mut last_seq: Option<u64> = None;
    let mut truncated = false;

    for ev in events {
        let seq = ev.get("seq").and_then(Value::as_u64);
        let kind = ev.get("kind").and_then(Value::as_str).unwrap_or("");
        let data = ev.get("data").unwrap_or(&Value::Null);

        let line = match kind {
            "turn.started" => {
                let text = data
                    .pointer("/input/text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let provider = data.get("provider").and_then(Value::as_str).unwrap_or("");
                if provider.is_empty() {
                    Some(format!("[user] {text}"))
                } else {
                    Some(format!("[user -> {provider}] {text}"))
                }
            }
            "input.steered" => {
                let text = data
                    .pointer("/input/text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Some(format!("[steered] {text}"))
            }
            "text.done" => {
                let text = data.get("text").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() {
                    None
                } else {
                    Some(format!("[assistant] {text}"))
                }
            }
            "tool.started" => {
                let name = data.get("name").and_then(Value::as_str).unwrap_or("tool");
                Some(format!("[tool start] {name}"))
            }
            "tool.finished" => {
                let ok = data.get("ok").and_then(Value::as_bool).unwrap_or(true);
                let call = data.get("call").and_then(Value::as_str).unwrap_or("");
                Some(format!(
                    "[tool {}] {call}",
                    if ok { "ok" } else { "failed" }
                ))
            }
            "turn.ended" => {
                let outcome = data
                    .get("outcome")
                    .and_then(Value::as_str)
                    .unwrap_or("done");
                let err = data.get("error").and_then(Value::as_str);
                match err {
                    Some(e) if !e.is_empty() => Some(format!("[turn ended: {outcome}] {e}")),
                    _ => Some(format!("[turn ended: {outcome}]")),
                }
            }
            "thread.switched" => {
                let from = data
                    .pointer("/from/provider")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let to = data
                    .pointer("/to/provider")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                Some(format!("[switched provider] {from} -> {to}"))
            }
            _ => None,
        };

        if let Some(s) = seq {
            last_seq = Some(s);
        }
        let Some(line) = line else {
            continue;
        };
        let prefix = match seq {
            Some(s) => format!("#{s} {line}\n"),
            None => format!("{line}\n"),
        };
        if out.len() + prefix.len() > MAX_READ_BYTES {
            truncated = true;
            break;
        }
        out.push_str(&prefix);
    }

    if out.is_empty() {
        out.push_str("No readable events in this range.");
    } else if truncated {
        out.push_str("[truncated at 32 KiB; pass after=<nextAfter> to continue]\n");
    }
    (out, last_seq, truncated)
}

/// Formats queue entries returned by `threads::list_queue`.
pub fn format_queue(entries: &[Value]) -> String {
    if entries.is_empty() {
        return "Queue is empty.".into();
    }
    let mut out = String::new();
    for entry in entries {
        let id = entry.get("id").and_then(Value::as_str).unwrap_or("?");
        let text = entry
            .pointer("/input/text")
            .and_then(Value::as_str)
            .unwrap_or("");
        let trigger = match entry.get("trigger") {
            Some(t) if t.is_object() => {
                let kind = t.get("kind").and_then(Value::as_str).unwrap_or("trigger");
                format!(" [{kind}]")
            }
            _ => String::new(),
        };
        out.push_str(&format!("- {id}{trigger}: {text}\n"));
    }
    out
}

/// Reads `start_thread`'s `options`: an object of model settings by id, each a string
/// (`{ "effort": "medium" }`). Absent means none.
pub fn parse_model_options(input: &Value) -> Result<Vec<(String, String)>, String> {
    match input.get("options") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| match v.as_str() {
                Some(s) if !s.trim().is_empty() => Ok((k.clone(), s.trim().to_owned())),
                _ => Err(format!("options.{k} must be a non-empty string")),
            })
            .collect(),
        Some(_) => Err("options must be an object of setting id to value".into()),
    }
}

/// Reads `start_thread`'s `permissionMode`: `plan`, `standard` or `accept_edits`.
pub fn parse_permission_mode(input: &Value) -> Result<Option<String>, String> {
    match input.get("permissionMode") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(m)) if ["plan", "standard", "accept_edits"].contains(&m.as_str()) => {
            Ok(Some(m.clone()))
        }
        Some(_) => Err("permissionMode must be plan, standard or accept_edits".into()),
    }
}

/// One provider's models as text lines: id, label, and each setting with its values and
/// default (`effort: low, medium, high (default medium)`). The first model is the default.
pub fn format_models(provider: &str, name: &str, models: &[Value]) -> String {
    let mut out = format!("{name} (provider {provider})\n");
    if models.is_empty() {
        out.push_str("  (no models)\n");
    }
    for (i, m) in models.iter().enumerate() {
        let id = m.get("id").and_then(Value::as_str).unwrap_or("?");
        let label = m.get("label").and_then(Value::as_str).unwrap_or(id);
        let default = if i == 0 { ", default" } else { "" };
        out.push_str(&format!("  - {id} ({label}{default})\n"));
        for o in m
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let oid = o.get("id").and_then(Value::as_str).unwrap_or("?");
            let values: Vec<&str> = o
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.get("value").and_then(Value::as_str))
                .collect();
            let dflt = o
                .get("default")
                .and_then(Value::as_str)
                .map(|d| format!(" (default {d})"))
                .unwrap_or_default();
            out.push_str(&format!("      {oid}: {}{dflt}\n", values.join(", ")));
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_rfc3339_correctly() {
        assert_eq!(format_utc_rfc3339(0), "1970-01-01T00:00:00.000Z");
        // 2026-11-24T12:34:56Z
        assert_eq!(
            format_utc_rfc3339(1_795_523_696),
            "2026-11-24T12:34:56.000Z"
        );
        // Leap day 2024-02-29T00:00:00Z
        assert_eq!(
            format_utc_rfc3339(1_709_164_800),
            "2024-02-29T00:00:00.000Z"
        );
    }

    #[test]
    fn resolves_wake_triggers() {
        let t = resolve_wake_trigger(&json!({ "text": "ping", "inMinutes": 15 }), 0).unwrap();
        assert_eq!(
            t,
            json!({ "kind": "at", "time": "1970-01-01T00:15:00.000Z" })
        );

        let t2 = resolve_wake_trigger(
            &json!({ "text": "next", "afterThread": "T2", "onlyOnSuccess": true }),
            0,
        )
        .unwrap();
        assert_eq!(
            t2,
            json!({ "kind": "after_turn", "thread": "T2", "onlyOnSuccess": true })
        );

        let t3 = resolve_wake_trigger(
            &json!({ "text": "done", "afterCommand": ["cargo", "test"], "timeoutS": 60 }),
            0,
        )
        .unwrap();
        assert_eq!(
            t3,
            json!({ "kind": "after_command", "argv": ["cargo", "test"], "timeoutS": 60 })
        );

        let t4 = resolve_wake_trigger(
            &json!({ "text": "ci", "pollCommand": ["gh", "run", "view"], "everyS": 30, "maxTries": 5 }),
            0,
        )
        .unwrap();
        assert_eq!(
            t4,
            json!({ "kind": "poll_command", "argv": ["gh", "run", "view"], "everyS": 30, "maxTries": 5, "timeoutS": 300 })
        );

        assert!(resolve_wake_trigger(&json!({ "text": "missing" }), 0).is_err());
    }

    #[test]
    fn bounds_events_page_at_32_kib() {
        let big = "a".repeat(20_000);
        let events = vec![
            json!({ "seq": 1, "kind": "text.done", "data": { "text": big } }),
            json!({ "seq": 2, "kind": "text.done", "data": { "text": big } }),
        ];
        let (text, last_seq, truncated) = format_events_page(&events);
        assert!(truncated);
        assert_eq!(last_seq, Some(2));
        assert!(text.contains("truncated at 32 KiB"));
    }

    #[test]
    fn model_options_are_strings_by_id() {
        assert_eq!(parse_model_options(&json!({})), Ok(vec![]));
        assert_eq!(
            parse_model_options(&json!({ "options": { "effort": " medium " } })),
            Ok(vec![("effort".into(), "medium".into())])
        );
        assert!(parse_model_options(&json!({ "options": { "effort": 3 } })).is_err());
        assert!(parse_model_options(&json!({ "options": { "effort": "" } })).is_err());
        assert!(parse_model_options(&json!({ "options": ["effort"] })).is_err());
    }

    #[test]
    fn permission_mode_excludes_full_access() {
        assert_eq!(parse_permission_mode(&json!({})), Ok(None));
        assert_eq!(
            parse_permission_mode(&json!({ "permissionMode": "plan" })),
            Ok(Some("plan".into()))
        );
        assert!(parse_permission_mode(&json!({ "permissionMode": "full_access" })).is_err());
        assert!(parse_permission_mode(&json!({ "permissionMode": "yolo" })).is_err());
    }

    #[test]
    fn models_list_their_settings_and_default() {
        let models = vec![
            json!({ "id": "neon", "label": "Neon", "options": [{
                "id": "effort", "label": "Effort",
                "values": [{ "value": "low", "label": "Low" }, { "value": "medium", "label": "Medium" }],
                "default": "low"
            }] }),
            json!({ "id": "lite", "label": "Lite" }),
        ];
        let text = format_models("acme/gem", "Gem", &models);
        assert_eq!(
            text,
            "Gem (provider acme/gem)\n  - neon (Neon, default)\n      effort: low, medium (default low)\n  - lite (Lite)\n"
        );
        assert!(format_models("a/b", "B", &[]).contains("(no models)"));
    }
}
