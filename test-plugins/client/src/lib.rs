//! Test plugin: each command calls one `kumikit:host/client` function with the command's
//! JSON args and returns the result as JSON in the message, or the host's error
//! unchanged, so tests see its code. Built only with the public SDK.
//!
//! - `workspaces`, `providers`
//! - `threads { workspace }`, `get { thread }`, `create { workspace, provider?, model? }`
//! - `send { thread, text }`, `cancel { thread }`, `respond { approval, decision }`
//! - `follow { topic, after, count }`: subscribes and waits (on the pollable) until
//!   `count` events arrived, then returns their kinds.

use kumikit_plugin::client::{self, Decision};
use kumikit_plugin::{CallContext, CommandResult, Error, ErrorCode, Plugin, poll};
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
        other => {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("no command {other}"),
            ));
        }
    })
}

impl Plugin for Client {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}

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
