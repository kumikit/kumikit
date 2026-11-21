//! A test helper program, started by tests through the core's process supervisor.
//!
//! Modes (first argument):
//! - `tree`: starts `hang` as a child that shares this process's stdout, prints
//!   `ready`, then hangs. Stdout reaches EOF only when both processes are gone, which is
//!   how tests see that a whole process tree was killed.
//! - `hang`: blocks forever.
//! - `env`: prints its environment as `NAME=value` lines, sorted, and exits.
//! - `cwd`: prints its working directory and exits.
//! - `cat`: copies stdin to stdout until EOF.
//! - `stderr-flood <bytes>`: writes that many bytes to stderr, then `done` to stdout.
//! - `sleep <ms>`: waits that long, then prints `done` (for tests about time spent in
//!   the host, not for synchronizing them).
//! - `record <transcript> -- <agent> [args…]`: runs a real ACP agent and passes stdio
//!   through, writing every line with its direction to `<transcript>` (JSON lines,
//!   `{"from": "client" | "agent", "message": …}`). Used to record fixtures.
//! - `replay <transcript>`: plays the agent side of a transcript. Before each recorded
//!   client message it reads one from stdin and checks it has the same method (or is a
//!   response to the same request); then it writes the agent messages that followed.
//!   Client request ids are mapped, so responses carry the ids the client used, and the
//!   recorded workspace path `/workspace` becomes this process's working directory, as
//!   a real agent would report it. A mismatch is reported on stderr and ends the
//!   process with a failure.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{self, Read, Write};
use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("tree") => tree(),
        Some("hang") => hang(),
        Some("env") => {
            let mut vars: Vec<(String, String)> = std::env::vars().collect();
            vars.sort();
            for (k, v) in vars {
                println!("{k}={v}");
            }
            ExitCode::SUCCESS
        }
        Some("cwd") => match std::env::current_dir() {
            Ok(d) => {
                println!("{}", d.display());
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e.to_string()),
        },
        Some("cat") => {
            let mut buf = Vec::new();
            if let Err(e) = io::stdin().read_to_end(&mut buf) {
                return fail(&e.to_string());
            }
            let _ = io::stdout().write_all(&buf);
            ExitCode::SUCCESS
        }
        Some("stderr-flood") => {
            let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let chunk = vec![b'x'; 64 * 1024];
            let mut left = n;
            let mut err = io::stderr().lock();
            while left > 0 {
                let k = left.min(chunk.len());
                if err.write_all(&chunk[..k]).is_err() {
                    break;
                }
                left -= k;
            }
            drop(err);
            println!("done");
            ExitCode::SUCCESS
        }
        Some("sleep") => {
            let ms: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            println!("done");
            ExitCode::SUCCESS
        }
        Some("record") => match (args.get(1), args.get(2).map(String::as_str)) {
            (Some(out), Some("--")) if args.len() > 3 => record(out, &args[3..]),
            _ => fail("usage: fake-acp-agent record <transcript> -- <agent> [args…]"),
        },
        Some("replay") => match args.get(1) {
            Some(path) => replay(path, &args[2..]),
            None => fail("usage: fake-acp-agent replay <transcript>"),
        },
        _ => fail(
            "usage: fake-acp-agent <tree|hang|env|cwd|cat|stderr-flood N|sleep MS|record|replay>",
        ),
    }
}

fn tree() -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return fail(&e.to_string()),
    };
    // The child inherits stdout, so it keeps the pipe open as long as it lives.
    if let Err(e) = Command::new(exe).arg("hang").stdin(Stdio::null()).spawn() {
        return fail(&e.to_string());
    }
    println!("ready");
    let _ = io::stdout().flush();
    hang()
}

fn hang() -> ExitCode {
    loop {
        std::thread::park();
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("fake-acp-agent: {message}");
    ExitCode::FAILURE
}

fn record(out: &str, agent: &[String]) -> ExitCode {
    use std::io::BufRead;
    use std::sync::{Arc, Mutex};

    let file = match std::fs::File::create(out) {
        Ok(f) => Arc::new(Mutex::new(f)),
        Err(e) => return fail(&format!("{out}: {e}")),
    };
    let mut child = match Command::new(&agent[0])
        .args(&agent[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return fail(&format!("{}: {e}", agent[0])),
    };
    let log = |file: &Arc<Mutex<std::fs::File>>, from: &str, line: &str| {
        let message: serde_json::Value =
            serde_json::from_str(line).unwrap_or(serde_json::Value::String(line.to_owned()));
        let entry = serde_json::json!({ "from": from, "message": message });
        if let Ok(mut f) = file.lock() {
            let _ = writeln!(f, "{entry}");
        }
    };
    let (Some(mut to_agent), Some(from_agent)) = (child.stdin.take(), child.stdout.take()) else {
        return fail("no pipes");
    };
    let f = Arc::clone(&file);
    let pump = std::thread::spawn(move || {
        let mut out = io::stdout();
        for line in io::BufReader::new(from_agent).lines().map_while(Result::ok) {
            log(&f, "agent", &line);
            if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
                break;
            }
        }
    });
    for line in io::stdin().lock().lines().map_while(Result::ok) {
        log(&file, "client", &line);
        if writeln!(to_agent, "{line}")
            .and_then(|()| to_agent.flush())
            .is_err()
        {
            break;
        }
    }
    drop(to_agent);
    let _ = child.wait();
    let _ = pump.join();
    ExitCode::SUCCESS
}

fn mcp_servers_from_args(extra: &[String]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut iter = extra.iter();
    while let Some(arg) = iter.next() {
        let json_str = if arg == "--mcp-config" {
            iter.next().map(String::as_str)
        } else {
            arg.strip_prefix("--mcp-config=")
        };
        if let Some(raw) = json_str
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(raw)
            && let Some(servers) = v.get("mcpServers").and_then(|s| s.as_object())
        {
            for (name, cfg) in servers {
                let mut env = Vec::new();
                if let Some(map) = cfg.get("env").and_then(|e| e.as_object()) {
                    for (k, val) in map {
                        env.push(serde_json::json!({ "name": k, "value": val }));
                    }
                }
                out.push(serde_json::json!({
                    "name": name,
                    "command": cfg["command"],
                    "args": cfg["args"],
                    "env": env,
                }));
            }
        }
    }
    out
}

fn replay(path: &str, extra: &[String]) -> ExitCode {
    use std::collections::HashMap;
    use std::io::BufRead;

    use serde_json::Value;

    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return fail(&format!("{path}: {e}")),
    };
    let entries: Vec<Value> = match text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
    {
        Ok(e) => e,
        Err(e) => return fail(&format!("{path}: {e}")),
    };
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut out = io::stdout();
    // Recorded client request id -> the id the client used now (ACP JSON-RPC and
    // Claude Code stream-json control_request ids).
    let mut ids: HashMap<String, Value> = HashMap::new();
    let mut req_ids: HashMap<String, String> = HashMap::new();
    let cwd = std::env::current_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/workspace".to_owned());
    // The MCP servers the client gave in `session/new` or `--mcp-config` (the tool gateway).
    let mut mcp_servers: Vec<Value> = mcp_servers_from_args(extra);
    for (i, entry) in entries.iter().enumerate() {
        let message = &entry["message"];
        if entry["from"] == "mcp" {
            // Not recorded traffic: a step that calls a tool through the first MCP server
            // the client gave, and says the result as an agent message chunk.
            let Some(server) = mcp_servers.first() else {
                return fail(&format!("entry {i}: the client gave no MCP server"));
            };
            let text = match mcp_call(server, &entry["call"]) {
                Ok(t) => t,
                Err(e) => return fail(&format!("entry {i}: {e}")),
            };
            let chunk = if entry["format"].as_str() == Some("stream-json") {
                serde_json::json!({
                    "type": "stream_event",
                    "event": {
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": { "type": "text_delta", "text": text }
                    }
                })
            } else {
                serde_json::json!({ "jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": entry["session"], "update": { "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text } } } })
            };
            if writeln!(out, "{chunk}").and_then(|()| out.flush()).is_err() {
                return ExitCode::SUCCESS;
            }
            continue;
        }
        if entry["from"] == "client" {
            let Some(Ok(line)) = lines.next() else {
                return fail(&format!("entry {i}: the client closed stdin early"));
            };
            let got: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => return fail(&format!("entry {i}: not JSON: {e}")),
            };
            let same = if message.get("jsonrpc").is_some() {
                match message.get("method") {
                    Some(m) => got.get("method") == Some(m),
                    // A response to an agent request: same id, and a result or an error.
                    None => {
                        got.get("method").is_none()
                            && got.get("id") == message.get("id")
                            && (got.get("result").is_some() || got.get("error").is_some())
                    }
                }
            } else {
                // Claude Code stream-json message: match type and control subtypes/ids.
                let same_type = got.get("type") == message.get("type");
                match message["type"].as_str() {
                    Some("control_request") => {
                        same_type && got["request"]["subtype"] == message["request"]["subtype"]
                    }
                    Some("control_response") => {
                        same_type
                            && got["response"]["subtype"] == message["response"]["subtype"]
                            && got["response"]["request_id"] == message["response"]["request_id"]
                    }
                    _ => same_type,
                }
            };
            if !same {
                return fail(&format!("entry {i}: expected {message}, got {got}"));
            }
            if message.get("method").is_some()
                && let (Some(rec), Some(now)) = (message.get("id"), got.get("id"))
            {
                ids.insert(rec.to_string(), now.clone());
            }
            if message["type"].as_str() == Some("control_request")
                && let (Some(rec), Some(now)) =
                    (message["request_id"].as_str(), got["request_id"].as_str())
            {
                req_ids.insert(rec.to_owned(), now.to_owned());
            }
            if got.get("method").and_then(Value::as_str) == Some("session/new") {
                mcp_servers = got["params"]["mcpServers"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
            }
        } else {
            let mut message = message.clone();
            rewrite_paths(&mut message, &cwd);
            // A response to a client request takes the id the client used.
            if message.get("jsonrpc").is_some()
                && message.get("method").is_none()
                && let Some(id) = message.get("id").map(Value::to_string)
                && let Some(now) = ids.get(&id)
            {
                message["id"] = now.clone();
            }
            if message["type"].as_str() == Some("control_response")
                && let Some(id) = message["response"]["request_id"].as_str()
                && let Some(now) = req_ids.get(id)
            {
                message["response"]["request_id"] = Value::String(now.clone());
            }
            if writeln!(out, "{message}")
                .and_then(|()| out.flush())
                .is_err()
            {
                return ExitCode::SUCCESS;
            }
        }
    }
    // Like a real agent, stay until the client closes stdin.
    for _ in lines {}
    ExitCode::SUCCESS
}

/// Replaces the recorded workspace path in every string of `v` with `cwd`.
fn rewrite_paths(v: &mut serde_json::Value, cwd: &str) {
    match v {
        serde_json::Value::String(s) if s.contains("/workspace") => {
            *s = s.replace("/workspace", cwd);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|i| rewrite_paths(i, cwd)),
        serde_json::Value::Object(map) => map.values_mut().for_each(|i| rewrite_paths(i, cwd)),
        _ => {}
    }
}

/// Starts a stdio MCP server (ACP form: `{ command, args, env: [{ name, value }] }`),
/// initializes it, calls one tool and returns the text of its result (`ERROR: …` when the
/// result is an error).
fn mcp_call(server: &serde_json::Value, call: &serde_json::Value) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(server["command"].as_str().ok_or("no command")?);
    for a in server["args"].as_array().into_iter().flatten() {
        cmd.arg(a.as_str().unwrap_or_default());
    }
    for e in server["env"].as_array().into_iter().flatten() {
        cmd.env(
            e["name"].as_str().unwrap_or_default(),
            e["value"].as_str().unwrap_or_default(),
        );
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("starting the MCP server: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("no stdout")?).lines();
    let mut ask = |id: u64,
                   method: &str,
                   params: serde_json::Value|
     -> Result<serde_json::Value, String> {
        let req =
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(stdin, "{req}").map_err(|e| e.to_string())?;
        loop {
            let line = lines
                .next()
                .ok_or("the MCP server closed")?
                .map_err(|e| e.to_string())?;
            let v: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            if v["id"] == serde_json::json!(id) {
                return Ok(v);
            }
        }
    };
    ask(
        1,
        "initialize",
        serde_json::json!({ "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": { "name": "fake-acp-agent", "version": "0" } }),
    )?;
    let r = ask(2, "tools/call", call.clone())?;
    drop(child.kill());
    let text = r["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    Ok(if r["result"]["isError"] == true {
        format!("ERROR: {text}")
    } else {
        text
    })
}
