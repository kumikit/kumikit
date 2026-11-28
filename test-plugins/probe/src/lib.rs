//! Test plugin: a deterministic provider for host and kernel tests. Built only with the
//! public SDK.
//!
//! The model picks a turn scenario (no timers, no randomness):
//! - `echo`: streams the input back word by word, then `text.done` and `usage`.
//! - `tool-approval`: announces a tool call, asks for approval and waits; on allow it
//!   reports canned tool output, on deny a failed call; then `text.done` and `usage`.
//! - `fail`: one delta, then the turn fails.
//! - `probe` (the first model): the message is a command that exercises one host
//!   capability; the turn returns the host's error unchanged, so tests see its code.
//!
//! Probe commands (words separated by spaces):
//! - `spawn <program> [cwd]`: starts it, closes stdin, returns its stdout as `text.done`
//!   when a turn is running.
//! - `run <args…>`: starts `fake-acp-agent <args…>` and waits for it.
//! - `report <program>`: starts `<program> cwd` in a turn and replies with its output.
//! - `spawn-secret <var> <secret>`: starts `fake-acp-agent env` with a secret variable.
//! - `secret <name>`: reads a secret. `log-secret <name>`: reads one and logs its value.
//! - `settings`: replies with its settings (`kumikit:host/settings`).
//! - `mcp-info <thread>`: replies with the session's tool gateway record, or `null`.
//! - `session-options <thread>`: replies with the `key=value` options the session started
//!   with. `set-options`: calls `kumikit:models/options.set-options` outside `list-models`.
//! - `http <url>`: GETs it and replies `<status> <body>` (`wasi:http`).
//! - `kv`: a set/get/list/delete round trip in its own store.
//! - `fs-own`: writes and reads a file in its data directory.
//! - `fs-escape <path>`: tries to read a file outside the sandbox.
//! - `socket`: tries to open a TCP connection.
//! - `trap`: panics. `spin`: loops forever. `oom <mib>`: allocates that much.
//! - `hang`: starts `fake-acp-agent hang` and blocks reading its stdout (until cancel).
//! - `steer`: like `hang`, but through `kumikit:turn/input`: when a steering message
//!   arrives it replies `steered: <text>` as `text.done` and ends the turn.
//! - `ignore-cancel`: like `hang`, but keeps reading after `cancelled`; it returns only
//!   when the child's stdout closes (the host killed it after the grace period).
//! - `limits <percent>`: reports a plan limits snapshot with one window at that percent
//!   (`kumikit:usage/report`). `usage-models`: reports usage by model.
//!
//! It also exports the limits reader (`kumikit:usage/probe`): `read-limits` returns one
//! window whose percent counts the reads, kept in its key-value store.

use kumikit_provider::usage::{self, LimitWindow, Limits, LimitsReader, ModelUsage};
use kumikit_provider::{
    CallContext, CommandResult, Error, ErrorCode, Event, Model, Plugin, Provider, ProviderRef,
    SessionConfig, SpawnRequest, UserInput, events, kv, log, process, secrets, settings,
};

struct Probe;

/// The session id encodes the model, so `send` knows the scenario.
fn model_of(session: &str) -> &str {
    session.split_once(':').map_or("probe", |(m, _)| m)
}

fn fail(code: ErrorCode, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

fn emit(e: Event) -> Result<(), Error> {
    events::emit(&e)
}

impl Plugin for Probe {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}

    /// Methods: `echo` (core) returns its params; `cwd` (workspace) and `cwd-core` (core)
    /// run `fake-acp-agent cwd` and return its output as `{ "cwd": … }`; `fail` returns a
    /// guest error with the code named in `params.code`.
    fn handle(method: String, params: String, ctx: CallContext) -> Result<String, Error> {
        match method.as_str() {
            "echo" => Ok(params),
            "cwd" | "cwd-core" => {
                let child = process::spawn(&request("fake-acp-agent", &["cwd"]))?;
                child.close_stdin();
                let mut out = Vec::new();
                while let Some(chunk) = child.read_stdout(64 * 1024)? {
                    out.extend(chunk);
                }
                child.wait()?;
                let cwd = String::from_utf8_lossy(&out).trim().to_owned();
                let ws = ctx.workspace.map(|w| w.id).unwrap_or_default();
                Ok(format!(r#"{{"cwd":{cwd:?},"workspace":{ws:?}}}"#))
            }
            "fail" => Err(fail(
                if params.contains("not_found") {
                    ErrorCode::NotFound
                } else {
                    ErrorCode::Internal
                },
                "scripted",
            )),
            _ => Err(fail(ErrorCode::NotFound, format!("no method {method}"))),
        }
    }

    /// Commands: `greet` shows a message and a form whose submit runs `greet-submit`,
    /// which answers with the submitted args.
    fn run_command(id: String, args: String, _ctx: CallContext) -> Result<CommandResult, Error> {
        match id.as_str() {
            "greet" => Ok(CommandResult {
                message: Some("hello from the probe".into()),
                view: Some(
                    r#"{"model":"form","schema":{"type":"object","properties":{"name":{"type":"string","title":"Name"}}},"values":{},"submit":"greet-submit"}"#.into(),
                ),
            }),
            "greet-submit" => Ok(CommandResult {
                message: Some(format!("submitted {args}")),
                view: None,
            }),
            _ => Err(fail(ErrorCode::NotFound, format!("no command {id}"))),
        }
    }
}

impl Provider for Probe {
    fn list_models(_provider: ProviderRef) -> Result<Vec<Model>, Error> {
        // The probe model has an effort setting (ADR 0033).
        kumikit_provider::models::set_options(
            "probe",
            &[kumikit_provider::models::ModelOption {
                id: "effort".into(),
                label: "Effort".into(),
                kind: Some("effort".into()),
                values: ["low", "high"]
                    .map(|v| kumikit_provider::models::OptionValue {
                        value: v.into(),
                        label: v.into(),
                        description: None,
                    })
                    .to_vec(),
                default: Some("low".into()),
            }],
        )?;
        Ok(["probe", "echo", "tool-approval", "fail"]
            .map(|id| Model {
                id: id.into(),
                label: id.into(),
                // The probe model can be steered (the `steer` command).
                capabilities: if id == "probe" {
                    vec!["steer".into()]
                } else {
                    Vec::new()
                },
            })
            .to_vec())
    }

    fn start_session(_provider: ProviderRef, config: SessionConfig) -> Result<String, Error> {
        // Kept for `mcp-info`: the tool gateway this session got, if any.
        let mcp = match &config.mcp_server {
            Some(m) => {
                let env: Vec<String> = m.env.iter().map(|(k, v)| format!("{k:?}:{v:?}")).collect();
                let args: Vec<String> = m.args.iter().map(|a| format!("{a:?}")).collect();
                format!(
                    r#"{{"name":{:?},"command":{:?},"args":[{}],"env":{{{}}}}}"#,
                    m.name,
                    m.command,
                    args.join(","),
                    env.join(",")
                )
            }
            None => "null".into(),
        };
        kv::set(&format!("mcp:{}", config.thread), mcp.as_bytes())?;
        // Kept for `session-options`: the options this session started with.
        let options: Vec<String> = config
            .options
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        kv::set(
            &format!("options:{}", config.thread),
            options.join(",").as_bytes(),
        )?;
        Ok(format!("{}:{}", config.model, config.thread))
    }

    fn send(session: String, input: UserInput) -> Result<(), Error> {
        match model_of(&session) {
            "echo" => echo(strip_title_hint(&input.text)),
            "tool-approval" => tool_approval(),
            "fail" => {
                emit(Event::TextDelta("about to fail".into()))?;
                Err(fail(ErrorCode::Internal, "scripted failure"))
            }
            _ => probe(strip_side_context(strip_title_hint(&input.text))),
        }
    }

    fn end_session(_session: String) {}
}

fn echo(text: &str) -> Result<(), Error> {
    for word in text.split_inclusive(' ') {
        emit(Event::TextDelta(word.to_owned()))?;
    }
    emit(Event::TextDone(text.to_owned()))?;
    let words = text.split_whitespace().count() as u64;
    emit(Event::Usage(events::Usage {
        input_tokens: words,
        output_tokens: words,
        cost: None,
    }))
}

/// The M0 scripted provider's `tool-approval` scenario, event for event.
fn tool_approval() -> Result<(), Error> {
    let input = r#"{"command":"ls"}"#;
    emit(Event::TextDelta("I'll list the files.".into()))?;
    emit(Event::ToolStarted(events::ToolStarted {
        call: "call-1".into(),
        name: "shell".into(),
        input: input.into(),
    }))?;
    let decision = events::request_approval("tool", "Run `ls` in the workspace", Some(input))?;
    let text = match decision {
        events::Decision::Allow | events::Decision::AllowAlways => {
            emit(Event::ToolOutput(events::ToolOutput {
                call: "call-1".into(),
                chunk: "README.md\nsrc\n".into(),
            }))?;
            emit(Event::ToolFinished(events::ToolFinished {
                call: "call-1".into(),
                ok: true,
                result: Some(r#"{"exitCode":0}"#.into()),
            }))?;
            "The workspace has README.md and src."
        }
        events::Decision::Deny => {
            emit(Event::ToolFinished(events::ToolFinished {
                call: "call-1".into(),
                ok: false,
                result: Some(r#"{"denied":true}"#.into()),
            }))?;
            "The tool call was denied."
        }
    };
    emit(Event::TextDelta(format!(" {text}")))?;
    emit(Event::TextDone(format!("I'll list the files. {text}")))?;
    emit(Event::Usage(events::Usage {
        input_tokens: 12,
        output_tokens: 9,
        cost: None,
    }))
}

fn spawn(req: SpawnRequest, report: bool) -> Result<(), Error> {
    let child = process::spawn(&req)?;
    child.close_stdin();
    let mut out = Vec::new();
    while let Some(chunk) = child.read_stdout(64 * 1024)? {
        out.extend(chunk);
    }
    child.wait()?;
    if report {
        emit(Event::TextDone(String::from_utf8_lossy(&out).into_owned()))?;
    }
    Ok(())
}

fn request(program: &str, args: &[&str]) -> SpawnRequest {
    SpawnRequest {
        program: program.into(),
        args: args.iter().map(|s| (*s).to_owned()).collect(),
        cwd: None,
        env: Vec::new(),
        secret_env: Vec::new(),
    }
}

/// The kernel puts a title hint before the first message of an untitled thread, followed
/// by a blank line (spec 03, Threads). The scenarios read only the user's own message.
fn strip_title_hint(text: &str) -> &str {
    if text.starts_with("[Thread title]") {
        text.split_once("\n\n").map_or(text, |(_, rest)| rest)
    } else {
        text
    }
}

/// Side threads prepend the main thread's context to their first turn. Probe commands
/// read only the user's message after the closing marker.
fn strip_side_context(text: &str) -> &str {
    const END: &str = "[End of main thread context — answer the side question below]\n\n";
    if text.starts_with("[Main thread context]") {
        text.split_once(END).map_or(text, |(_, rest)| rest)
    } else {
        text
    }
}

fn limits(percent: f32) -> Limits {
    Limits {
        plan: Some("Probe plan".into()),
        account: None,
        account_url: None,
        windows: vec![LimitWindow {
            id: "five-hour".into(),
            label: "5 hours".into(),
            group: None,
            used_percent: percent,
            resets_at: Some("2026-11-15T15:00:00.000Z".into()),
            model: None,
        }],
    }
}

impl LimitsReader for Probe {
    fn read_limits(_provider: ProviderRef) -> Result<Option<Limits>, Error> {
        let reads = kv::get("limit-reads")?
            .and_then(|v| String::from_utf8(v).ok())
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        kv::set("limit-reads", reads.to_string().as_bytes())?;
        Ok(Some(limits(reads as f32 * 10.0)))
    }
}

kumikit_provider::export_limits_reader!(Probe);

fn probe(command: &str) -> Result<(), Error> {
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.as_slice() {
        ["limits", percent] => {
            let percent: f32 = percent
                .parse()
                .map_err(|_| fail(ErrorCode::InvalidArgument, "not a number"))?;
            usage::emit_limits(&limits(percent))?;
            emit(Event::TextDone("reported".into()))
        }
        ["limits-invalid", kind] => {
            let bad = match *kind {
                "nan" => limits(f32::NAN),
                "reset" => {
                    let mut l = limits(10.0);
                    l.windows[0].resets_at = Some("not-a-timestamp".into());
                    l
                }
                "too-many" => {
                    let mut l = limits(10.0);
                    l.windows = (0..33)
                        .map(|i| LimitWindow {
                            id: format!("w-{i}"),
                            label: format!("Window {i}"),
                            group: None,
                            used_percent: 10.0,
                            resets_at: None,
                            model: None,
                        })
                        .collect();
                    l
                }
                other => return Err(fail(ErrorCode::InvalidArgument, format!("unknown {other}"))),
            };
            usage::emit_limits(&bad)?;
            emit(Event::TextDone("reported".into()))
        }
        ["usage-models"] => {
            let m = |model: &str, input, cached| ModelUsage {
                model: model.into(),
                input_tokens: input,
                output_tokens: 3,
                cached_input_tokens: cached,
                cost: Some(0.5),
            };
            usage::emit_turn_usage(&[m("probe", 100, 60), m("probe-small", 10, 0)])?;
            emit(Event::TextDone("reported".into()))
        }
        ["spawn", program, rest @ ..] => {
            let mut req = request(program, &["cwd"]);
            req.cwd = rest.first().map(|c| (*c).to_owned());
            // Report only when a turn is running (host tests call `send` without one).
            spawn(req, false)
        }
        ["run", args @ ..] => spawn(request("fake-acp-agent", args), false),
        // Like `spawn`, inside a turn: the program's output is the reply.
        ["report", program] => spawn(request(program, &["cwd"]), true),
        ["spawn-secret", var, secret] => {
            let mut req = request("fake-acp-agent", &["env"]);
            req.secret_env = vec![((*var).to_owned(), (*secret).to_owned())];
            spawn(req, false)
        }
        ["secret", name] => secrets::get(name).map(|_| ()),
        // GETs a URL and replies `<status> <body>`; a refusal is `permission_denied`.
        ["http", url] => {
            match kumikit_provider::http::send(&kumikit_provider::http::Request::get(*url)) {
                Ok(r) => emit(Event::TextDone(format!(
                    "{} {}",
                    r.status,
                    String::from_utf8_lossy(&r.body)
                ))),
                Err(e) if e.denied => Err(fail(ErrorCode::PermissionDenied, format!("http: {e}"))),
                Err(e) => Err(fail(ErrorCode::Io, format!("http: {e}"))),
            }
        }
        // Replies with the session's tool gateway record (JSON), or `null`.
        ["mcp-info", thread] => {
            let v = kv::get(&format!("mcp:{thread}"))?.unwrap_or_else(|| b"null".to_vec());
            emit(Event::TextDone(String::from_utf8_lossy(&v).into_owned()))
        }
        // Replies with the plugin's own settings (`kumikit:host/settings`).
        ["settings"] => emit(Event::TextDone(settings::get()?)),
        // Logs a secret's value, so a test can prove the core's log redacts it.
        ["log-secret", name] => {
            let value = secrets::get(name)?.unwrap_or_default();
            log::log(log::Level::Warn, &format!("probe read {value}"));
            Ok(())
        }
        ["session-options", thread] => {
            let v = kv::get(&format!("options:{thread}"))?.unwrap_or_default();
            emit(Event::TextDone(String::from_utf8_lossy(&v).into_owned()))
        }
        ["set-options"] => {
            // Outside list-models this must fail with invalid-argument.
            kumikit_provider::models::set_options("probe", &[])?;
            emit(Event::TextDone("set".into()))
        }
        ["kv"] => {
            kv::set("a/1", b"one")?;
            kv::set("a/2", b"two")?;
            kv::set("b", b"three")?;
            let got = kv::get("a/1")?;
            let keys = kv::list("a/")?;
            kv::delete("a/1")?;
            let gone = kv::get("a/1")?;
            if got.as_deref() == Some(&b"one"[..]) && keys == ["a/1", "a/2"] && gone.is_none() {
                Ok(())
            } else {
                Err(fail(
                    ErrorCode::Internal,
                    format!("kv mismatch: {got:?} {keys:?} {gone:?}"),
                ))
            }
        }
        ["fs-own"] => std::fs::write("/data/probe.txt", b"hello")
            .and_then(|()| std::fs::read("/data/probe.txt"))
            .map_err(|e| fail(ErrorCode::Io, format!("fs: {e}")))
            .and_then(|b| {
                if b == b"hello" {
                    Ok(())
                } else {
                    Err(fail(ErrorCode::Internal, "fs: wrong contents"))
                }
            }),
        ["fs-escape", path] => match std::fs::read(path) {
            Ok(_) => Err(fail(
                ErrorCode::Internal,
                format!("read {path} outside the sandbox"),
            )),
            Err(e) => Err(fail(ErrorCode::PermissionDenied, format!("fs: {e}"))),
        },
        ["socket"] => match std::net::TcpStream::connect("127.0.0.1:9") {
            Ok(_) => Err(fail(ErrorCode::Internal, "opened a socket")),
            Err(e) => Err(fail(ErrorCode::PermissionDenied, format!("socket: {e}"))),
        },
        ["trap"] => panic!("probe trap"),
        ["spin"] => {
            let mut n: u64 = 0;
            loop {
                n = std::hint::black_box(n.wrapping_add(1));
            }
        }
        ["oom", mib] => {
            let mib: usize = mib.parse().unwrap_or(1024);
            let mut blocks: Vec<Vec<u8>> = Vec::new();
            for _ in 0..mib {
                blocks.push(vec![1u8; 1 << 20]);
            }
            Err(fail(
                ErrorCode::Internal,
                format!("allocated {} MiB", blocks.len()),
            ))
        }
        ["hang"] => {
            let child = process::spawn(&request("fake-acp-agent", &["hang"]))?;
            // Blocks until the host cancels the turn (or the child dies).
            child.read_stdout(1).map(|_| ())
        }
        ["steer"] => {
            let child = process::spawn(&request("fake-acp-agent", &["hang"]))?;
            loop {
                match kumikit_provider::steering::read_stdout(&child, 1)? {
                    kumikit_provider::steering::Read::Input(input) => {
                        emit(Event::TextDone(format!("steered: {}", input.text)))?;
                        return Ok(());
                    }
                    kumikit_provider::steering::Read::Eof => return Ok(()),
                    kumikit_provider::steering::Read::Stdout(_) => {}
                }
            }
        }
        ["ignore-cancel"] => {
            let child = process::spawn(&request("fake-acp-agent", &["hang"]))?;
            loop {
                match child.read_stdout(1) {
                    Ok(None) => return Ok(()),
                    Ok(Some(_)) => {}
                    Err(e) if e.code == ErrorCode::Cancelled => {}
                    Err(e) => return Err(e),
                }
            }
        }
        _ => Err(fail(
            ErrorCode::InvalidArgument,
            format!("unknown probe command `{command}`"),
        )),
    }
}

kumikit_provider::export_provider!(Probe);
