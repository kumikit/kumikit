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
//! - `spawn-secret <var> <secret>`: starts `fake-acp-agent env` with a secret variable.
//! - `secret <name>`: reads a secret.
//! - `kv`: a set/get/list/delete round trip in its own store.
//! - `fs-own`: writes and reads a file in its data directory.
//! - `fs-escape <path>`: tries to read a file outside the sandbox.
//! - `socket`: tries to open a TCP connection.
//! - `trap`: panics. `spin`: loops forever. `oom <mib>`: allocates that much.
//! - `hang`: starts `fake-acp-agent hang` and blocks reading its stdout (until cancel).

use kumikit_provider::{
    Error, ErrorCode, Event, Lifecycle, Model, Provider, ProviderRef, SessionConfig, SpawnRequest,
    UserInput, events, kv, process, secrets,
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

impl Lifecycle for Probe {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}
}

impl Provider for Probe {
    fn list_models(_provider: ProviderRef) -> Result<Vec<Model>, Error> {
        Ok(["probe", "echo", "tool-approval", "fail"]
            .map(|id| Model {
                id: id.into(),
                label: id.into(),
                capabilities: Vec::new(),
            })
            .to_vec())
    }

    fn start_session(_provider: ProviderRef, config: SessionConfig) -> Result<String, Error> {
        Ok(format!("{}:{}", config.model, config.thread))
    }

    fn send(session: String, input: UserInput) -> Result<(), Error> {
        match model_of(&session) {
            "echo" => echo(&input.text),
            "tool-approval" => tool_approval(),
            "fail" => {
                emit(Event::TextDelta("about to fail".into()))?;
                Err(fail(ErrorCode::Internal, "scripted failure"))
            }
            _ => probe(&input.text),
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

fn tool_approval() -> Result<(), Error> {
    emit(Event::TextDelta("Running the tool.".into()))?;
    emit(Event::ToolStarted(events::ToolStarted {
        call: "call-1".into(),
        name: "shell".into(),
        input: r#"{"command":"ls"}"#.into(),
    }))?;
    let decision = events::request_approval("tool", "Run `ls`", Some(r#"{"command":"ls"}"#))?;
    let ok = matches!(
        decision,
        events::Decision::Allow | events::Decision::AllowAlways
    );
    if ok {
        emit(Event::ToolOutput(events::ToolOutput {
            call: "call-1".into(),
            chunk: "README.md\n".into(),
        }))?;
    }
    emit(Event::ToolFinished(events::ToolFinished {
        call: "call-1".into(),
        ok,
        result: None,
    }))?;
    let text = if ok { "Done." } else { "The tool was denied." };
    emit(Event::TextDone(text.into()))?;
    emit(Event::Usage(events::Usage {
        input_tokens: 3,
        output_tokens: 2,
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

fn probe(command: &str) -> Result<(), Error> {
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.as_slice() {
        ["spawn", program, rest @ ..] => {
            let mut req = request(program, &["cwd"]);
            req.cwd = rest.first().map(|c| (*c).to_owned());
            // Report only when a turn is running (host tests call `send` without one).
            spawn(req, false)
        }
        ["run", args @ ..] => spawn(request("fake-acp-agent", args), false),
        ["spawn-secret", var, secret] => {
            let mut req = request("fake-acp-agent", &["env"]);
            req.secret_env = vec![((*var).to_owned(), (*secret).to_owned())];
            spawn(req, false)
        }
        ["secret", name] => secrets::get(name).map(|_| ()),
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
        _ => Err(fail(
            ErrorCode::InvalidArgument,
            format!("unknown probe command `{command}`"),
        )),
    }
}

kumikit_provider::export_provider!(Probe);
