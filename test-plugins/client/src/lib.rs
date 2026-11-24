//! Test plugin: each command calls one `kumikit:host/client` function with the command's
//! JSON args and returns the result as JSON in the message, or the host's error
//! unchanged, so tests see its code. Built only with the public SDK.
//!
//! - `workspaces`, `providers`
//! - `threads { workspace }`, `get { thread }`, `create { workspace, provider?, model? }`
//! - `send { thread, text }`, `cancel { thread }`, `respond { approval, decision }`
//! - `follow { topic, after, count }`: subscribes and waits (on the pollable) until
//!   `count` events arrived, then returns their kinds.
//! - `timer-set { key, cron, job }`: stores `job` (`{ workspace, provider, model, text }`)
//!   and sets the schedule; when it fires, the plugin starts a thread in that workspace
//!   with that text and remembers the due time. `timer-delete { key }`, `timers`,
//!   `fired` (keys and due times so far).

use kumikit_plugin::client::{self, Decision};
use kumikit_plugin::timers::{self, Missed, Schedule};
use kumikit_plugin::turn::{self, SteerStatus};
use kumikit_plugin::{CallContext, CommandResult, Error, ErrorCode, Plugin, kv, poll};
use serde_json::{Value, json};

struct Client;

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(ErrorCode::InvalidArgument, format!("missing {key}")))
}

fn opt(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn thread_json(t: &client::Thread) -> Value {
    json!({ "id": t.id, "workspace": t.workspace, "status": t.status, "provider": t.provider })
}

fn run(id: &str, args: &Value) -> Result<Value, Error> {
    Ok(match id {
        "workspaces" => Value::from_iter(
            client::list_workspaces()?
                .into_iter()
                .map(|w| json!({ "id": w.id, "name": w.name })),
        ),
        "providers" => Value::from_iter(client::list_providers()?.into_iter().map(|p| json!(p.id))),
        "threads" => Value::from_iter(
            client::list_threads(arg(args, "workspace")?)?
                .iter()
                .map(thread_json),
        ),
        "get" => serde_json::from_str(&client::get_thread(arg(args, "thread")?)?)
            .map_err(|e| Error::new(ErrorCode::Internal, e.to_string()))?,
        "create" => thread_json(&client::create_thread(
            arg(args, "workspace")?,
            opt(args, "provider").as_deref(),
            opt(args, "model").as_deref(),
            None,
        )?),
        "send" => json!(client::send(arg(args, "thread")?, arg(args, "text")?)?),
        "steer" => {
            let r = turn::steer(arg(args, "thread")?, arg(args, "text")?)?;
            let status = match r.status {
                SteerStatus::Accepted => "accepted",
                SteerStatus::Queued => "queued",
            };
            json!({ "status": status, "turn": r.turn, "entry": r.entry })
        }
        "cancel" => {
            client::cancel(arg(args, "thread")?)?;
            Value::Null
        }
        "respond" => {
            let decision = match arg(args, "decision")? {
                "allow" => Decision::Allow,
                _ => Decision::Deny,
            };
            client::respond_approval(arg(args, "approval")?, decision)?;
            Value::Null
        }
        "follow" => {
            let after = args.get("after").and_then(Value::as_u64).unwrap_or(0);
            let count = args.get("count").and_then(Value::as_u64).unwrap_or(1) as usize;
            let sub = client::subscribe(arg(args, "topic")?, after)?;
            let mut kinds = Vec::new();
            while kinds.len() < count {
                let ready = sub.subscribe();
                poll::poll(&[&ready]);
                for e in sub.next(64)? {
                    kinds.push(json!(e.kind));
                }
            }
            Value::Array(kinds)
        }
        "timer-set" => {
            let key = arg(args, "key")?;
            let job = args.get("job").cloned().unwrap_or(Value::Null);
            kv::set(&format!("job:{key}"), job.to_string().as_bytes())?;
            json!(timers::set(
                key,
                &Schedule {
                    cron: arg(args, "cron")?.to_owned(),
                    time_zone: opt(args, "zone"),
                    description: format!("test job {key}"),
                    missed: Missed::Skip,
                }
            )?)
        }
        "timer-delete" => {
            timers::delete(arg(args, "key")?)?;
            Value::Null
        }
        "timers" => Value::from_iter(timers::list()?.into_iter().map(|i| json!(i.key))),
        "fired" => {
            let mut out = Vec::new();
            for k in kv::list("fired:")? {
                let due = kv::get(&k)?.unwrap_or_default();
                out.push(json!([
                    k.trim_start_matches("fired:"),
                    String::from_utf8_lossy(&due)
                ]));
            }
            Value::Array(out)
        }
        other => {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("no command {other}"),
            ));
        }
    })
}

/// A schedule fired: start its job's thread, then remember that it ran.
fn fire(key: &str, due: &str) -> Result<(), Error> {
    if let Some(job) = kv::get(&format!("job:{key}"))? {
        let job: Value = serde_json::from_slice(&job).unwrap_or(Value::Null);
        if let Ok(workspace) = arg(&job, "workspace") {
            let t = client::create_thread(
                workspace,
                opt(&job, "provider").as_deref(),
                opt(&job, "model").as_deref(),
                Some(&format!("Scheduled: {key}")),
            )?;
            client::send(&t.id, arg(&job, "text")?)?;
        }
    }
    kv::set(&format!("fired:{key}"), due.as_bytes())
}

impl Plugin for Client {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}

    fn fire(key: String, due: String) -> Result<(), Error> {
        fire(&key, &due)
    }

    fn run_command(id: String, args: String, _ctx: CallContext) -> Result<CommandResult, Error> {
        let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
        let out = run(&id, &args)?;
        Ok(CommandResult {
            message: Some(out.to_string()),
            view: None,
        })
    }
}

kumikit_plugin::export_plugin!(Client);
