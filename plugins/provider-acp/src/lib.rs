//! `kumikit.provider-acp`: runs any agent that speaks the Agent Client Protocol, as a
//! provider backend for the `acp` key (`docs/spec/05-providers.md`). Other plugins
//! declare agents with a manifest entry and no code:
//!
//! ```toml
//! providers = [{ id = "claude", name = "Claude Code", acp = { command = "claude-agent-acp" } }]
//! ```
//!
//! Each session runs in its own instance (ADR 0010), so the session lives in a
//! thread-local. [`acp`] is the protocol logic, free of I/O and tested natively; this
//! file performs its actions with the host imports.

pub mod acp;
pub mod config;

use std::cell::RefCell;

use kumikit_provider::{
    Child, Decision, Error, ErrorCode, Event, Model, Plugin, Provider, ProviderRef, SessionConfig,
    SpawnRequest, UserInput, events, process, take_line,
};

use crate::acp::{Action, Options, Session};
use crate::config::Config;

struct Acp;

struct State {
    session: Session,
    child: Child,
    buf: Vec<u8>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn err(code: ErrorCode, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

fn is_cancelled(e: &Error) -> bool {
    e.code == ErrorCode::Cancelled
}

fn wit_event(e: acp::Event) -> Event {
    use acp::Event as A;
    let json = |v: &serde_json::Value| v.to_string();
    match e {
        A::TextDelta { text } => Event::TextDelta(text),
        A::TextDone { text } => Event::TextDone(text),
        A::ReasoningDelta { text } => Event::ReasoningDelta(text),
        A::ToolStarted { call, name, input } => Event::ToolStarted(events::ToolStarted {
            call,
            name,
            input: json(&input),
        }),
        A::ToolOutput { call, chunk } => Event::ToolOutput(events::ToolOutput { call, chunk }),
        A::ToolFinished { call, ok, result } => Event::ToolFinished(events::ToolFinished {
            call,
            ok,
            result: result.as_ref().map(json),
        }),
        A::FileChanged { path, diff } => Event::FileChanged(events::FileChanged { path, diff }),
        A::Usage {
            input_tokens,
            output_tokens,
            cost,
        } => Event::Usage(events::Usage {
            input_tokens,
            output_tokens,
            cost,
        }),
        A::Plugin { kind, data } => Event::Plugin(events::PluginEvent {
            kind,
            data: json(&data),
        }),
    }
}

/// What ended a run of actions.
enum Stop {
    Ready(String),
    TurnDone,
}

impl State {
    fn read_line(&mut self) -> Result<Option<String>, Error> {
        loop {
            if let Some(line) = take_line(&mut self.buf, false) {
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            match self.child.read_stdout(64 * 1024)? {
                Some(chunk) => self.buf.extend_from_slice(&chunk),
                None => {
                    return Ok(take_line(&mut self.buf, true)
                        .map(|l| String::from_utf8_lossy(&l).into_owned()));
                }
            }
        }
    }

    /// Performs actions, reading the agent's output when none are left, until the
    /// session is ready or the turn is done. A cancel (from any host call) becomes ACP
    /// `session/cancel`; the turn then ends when the agent says so.
    fn run(&mut self, mut queue: Vec<Action>) -> Result<Stop, Error> {
        queue.reverse();
        let mut cancelled = false;
        loop {
            let Some(action) = queue.pop() else {
                let line = match self.read_line() {
                    Ok(Some(l)) => l,
                    Ok(None) => return Err(err(ErrorCode::Io, "the agent exited")),
                    Err(e) if is_cancelled(&e) => {
                        cancelled = true;
                        queue.extend(self.session.cancel().into_iter().rev());
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                queue.extend(self.session.on_line(&line).into_iter().rev());
                continue;
            };
            match action {
                Action::Write(mut line) => {
                    line.push('\n');
                    self.child.write_stdin(line.as_bytes())?;
                }
                Action::Emit(e) => match events::emit(&wit_event(e)) {
                    Ok(()) => {}
                    // After a cancel the turn is over for the kernel; finish quietly.
                    Err(_) if cancelled => {}
                    Err(e) if is_cancelled(&e) => {
                        cancelled = true;
                        queue.extend(self.session.cancel().into_iter().rev());
                    }
                    Err(e) => return Err(e),
                },
                Action::Approve {
                    request,
                    kind,
                    summary,
                    detail,
                } => {
                    let options = detail["options"].clone();
                    let decision = if cancelled {
                        acp::Decision::Cancelled
                    } else {
                        match events::request_approval(&kind, &summary, Some(&detail.to_string())) {
                            Ok(Decision::Allow) => acp::Decision::Allow,
                            Ok(Decision::AllowAlways) => acp::Decision::AllowAlways,
                            Ok(Decision::Deny) => acp::Decision::Deny,
                            Err(e) if is_cancelled(&e) => {
                                cancelled = true;
                                queue.extend(self.session.cancel().into_iter().rev());
                                acp::Decision::Cancelled
                            }
                            Err(e) => return Err(e),
                        }
                    };
                    queue.extend(
                        self.session
                            .decide(&request, &options, decision)
                            .into_iter()
                            .rev(),
                    );
                }
                Action::Ready { session } => return Ok(Stop::Ready(session)),
                Action::TurnDone => return Ok(Stop::TurnDone),
                Action::Failed(m) => return Err(err(ErrorCode::Internal, m)),
            }
        }
    }
}

fn config(provider: &ProviderRef) -> Result<Config, Error> {
    let text = provider.config.as_deref().ok_or_else(|| {
        err(
            ErrorCode::InvalidArgument,
            "the provider entry has no `acp` table",
        )
    })?;
    Config::parse(text).map_err(|e| err(ErrorCode::InvalidArgument, e))
}

impl Plugin for Acp {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {
        STATE.with(|s| {
            if let Some(state) = s.borrow_mut().take() {
                state.child.kill();
            }
        });
    }
}

impl Provider for Acp {
    fn list_models(provider: ProviderRef) -> Result<Vec<Model>, Error> {
        Ok(config(&provider)?
            .models()
            .into_iter()
            .map(|m| Model {
                id: m.id,
                label: m.label,
                capabilities: m.capabilities,
            })
            .collect())
    }

    fn start_session(provider: ProviderRef, cfg: SessionConfig) -> Result<String, Error> {
        let config = config(&provider)?;
        let child = process::spawn(&SpawnRequest {
            program: config.command.clone(),
            args: config.args_for(&cfg.model),
            cwd: None,
            env: config.env.clone().into_iter().collect(),
            secret_env: config.secret_env.clone().into_iter().collect(),
        })?;
        let mut state = State {
            session: Session::new(Options {
                cwd: cfg.workspace_root,
                resume: cfg.resume,
                mode: config.mode.clone(),
                mcp: cfg.mcp_server.map(|m| acp::McpStdio {
                    name: m.name,
                    command: m.command,
                    args: m.args,
                    env: m.env,
                }),
            }),
            child,
            buf: Vec::new(),
        };
        let start = state.session.start();
        let id = match state.run(start)? {
            Stop::Ready(id) => id,
            Stop::TurnDone => return Err(err(ErrorCode::Internal, "unexpected turn end")),
        };
        STATE.with(|s| *s.borrow_mut() = Some(state));
        Ok(id)
    }

    fn send(_session: String, input: UserInput) -> Result<(), Error> {
        STATE.with(|s| {
            let mut guard = s.borrow_mut();
            let state = guard
                .as_mut()
                .ok_or_else(|| err(ErrorCode::Conflict, "no session is open"))?;
            let prompt = state.session.prompt(&input.text);
            state.run(prompt).map(|_| ())
        })
    }

    fn end_session(_session: String) {
        STATE.with(|s| {
            if let Some(state) = s.borrow_mut().take() {
                state.child.close_stdin();
                state.child.kill();
            }
        });
    }
}

kumikit_provider::export_provider!(Acp);
