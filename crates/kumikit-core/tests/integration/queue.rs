//! The thread queue over the protocol (ADR 0027), against the real binary and the probe
//! provider: messages sent during a turn are queued, held after Stop, edited, reordered,
//! sent now and resumed.

use std::future::Future;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::support::{Client, Core, DEADLINE, Dirs, install, kinds, probe_kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

/// Where `fake-acp-agent` (which `hang` starts) lives.
fn bin_dir() -> PathBuf {
    Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
        .parent()
        .unwrap()
        .to_owned()
}

/// A probe thread, subscribed from the start.
async fn thread(c: &mut Client, dirs: &Dirs) -> String {
    install(c, &probe_kkp(dirs)).await;
    let ws = c
        .call("workspace.open", json!({ "path": dirs.workspace() }))
        .await["workspace"]["id"]
        .clone();
    let thread = c
        .call(
            "thread.create",
            json!({ "workspace": ws, "provider": "test.probe/probe", "model": "probe" }),
        )
        .await["thread"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    c.call(
        "events.subscribe",
        json!({ "topic": format!("thread:{thread}"), "after": 0 }),
    )
    .await;
    thread
}

async fn send(c: &mut Client, thread: &str, text: &str) -> Value {
    c.call(
        "thread.send",
        json!({ "thread": thread, "input": { "text": text } }),
    )
    .await
}

fn queue_texts(snapshot: &Value) -> Vec<String> {
    snapshot["queue"]
        .as_array()
        .map(|q| {
            q.iter()
                .map(|e| e["input"]["text"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_holds_the_queue_and_resume_runs_it_in_its_new_order() {
    within(async {
        let dirs = Dirs::new();
        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        let thread = thread(&mut c, &dirs).await;

        send(&mut c, &thread, "hang").await;
        c.events_until("turn.started").await;
        let first = send(&mut c, &thread, "kv").await;
        let second = send(&mut c, &thread, "kv typo").await;
        assert_eq!(first["queued"], true, "{first}");
        let (a, b) = (first["entry"].clone(), second["entry"].clone());

        let edited = c
            .call(
                "thread.queue.update",
                json!({ "thread": thread, "entry": b, "input": { "text": "kv" } }),
            )
            .await;
        assert_eq!(edited["entry"]["input"]["text"], "kv");
        c.call(
            "thread.queue.move",
            json!({ "thread": thread, "entry": b, "before": a }),
        )
        .await;
        let missing = c
            .request(
                "thread.queue.remove",
                json!({ "thread": thread, "entry": "01JCQE00000000000000000009" }),
            )
            .await;
        assert_eq!(missing["error"]["data"]["code"], "not_found", "{missing}");

        c.call("thread.cancel", json!({ "thread": thread })).await;
        let events = c.events_until("queue.held").await;
        assert_eq!(events.last().unwrap()["data"]["reason"], "stopped");
        let snap = c.call("thread.get", json!({ "thread": thread })).await;
        assert_eq!(queue_texts(&snap), ["kv", "kv"]);
        assert_eq!(snap["queue"][0]["id"], b, "moved to the front");
        assert_eq!(snap["queueHold"], "stopped");
        assert_eq!(snap["thread"]["status"], "idle");

        c.call("thread.queue.resume", json!({ "thread": thread }))
            .await;
        let mut events = c.events_until("turn.ended").await;
        events.extend(c.events_until("turn.ended").await);
        let started: Vec<&Value> = events
            .iter()
            .filter(|e| e["kind"] == "turn.started")
            .collect();
        assert_eq!(started[0]["data"]["entry"], b);
        assert_eq!(started[1]["data"]["entry"], a);
        assert_eq!(kinds(&events)[0], "queue.resumed");

        let snap = c.call("thread.get", json!({ "thread": thread })).await;
        assert!(snap.get("queue").is_none(), "{snap}");
        assert!(snap.get("queueHold").is_none(), "{snap}");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn send_now_stops_the_running_turn() {
    within(async {
        let dirs = Dirs::new();
        let (core, _) = Core::start_with_path(&dirs, Some(&bin_dir())).await;
        let mut c = Client::hello(&dirs.address).await;
        let thread = thread(&mut c, &dirs).await;

        send(&mut c, &thread, "hang").await;
        c.events_until("turn.started").await;
        let queued = send(&mut c, &thread, "kv").await;
        let now = c
            .call(
                "thread.queue.sendNow",
                json!({ "thread": thread, "entry": queued["entry"] }),
            )
            .await;
        assert_eq!(now["queued"], true);
        assert_eq!(now["turn"], queued["turn"]);

        let ended = c.events_until("turn.ended").await;
        assert_eq!(ended.last().unwrap()["data"]["outcome"], "cancelled");
        let events = c.events_until("turn.ended").await;
        assert_eq!(events[0]["kind"], "turn.started");
        assert_eq!(events[0]["data"]["turn"], queued["turn"]);
        assert_eq!(events.last().unwrap()["data"]["outcome"], "done");
        core.stop().await;
    })
    .await;
}
