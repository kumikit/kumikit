//! Deferred messages over the protocol (ADR 0027): `thread.defer` with `after_turn`,
//! `after_command`, cycle refusal, and `thread.queue.sendNow` on a deferred entry.

use std::future::Future;

use serde_json::json;

use crate::support::{Client, Core, DEADLINE, Dirs, install, probe_kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_defer_after_turn_command_and_send_now() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;

        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]["id"]
            .clone();
        let t1 = c
            .call(
                "thread.create",
                json!({ "workspace": ws, "provider": "test.probe/probe", "model": "echo" }),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let t2 = c
            .call(
                "thread.create",
                json!({ "workspace": ws, "provider": "test.probe/probe", "model": "echo" }),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        c.call(
            "events.subscribe",
            json!({ "topic": format!("thread:{t1}"), "after": 0 }),
        )
        .await;

        // Defer on t1 waiting for t2 to finish.
        let deferred = c
            .call(
                "thread.defer",
                json!({
                    "thread": t1,
                    "input": { "text": "t2 finished" },
                    "trigger": { "kind": "after_turn", "thread": t2, "onlyOnSuccess": true }
                }),
            )
            .await;
        assert_eq!(deferred["entry"]["trigger"]["kind"], "after_turn");

        // Loop back from t2 to t1 is refused.
        let loop_err = c
            .request(
                "thread.defer",
                json!({
                    "thread": t2,
                    "input": { "text": "loop" },
                    "trigger": { "kind": "after_turn", "thread": t1 }
                }),
            )
            .await;
        assert_eq!(loop_err["error"]["data"]["code"], "invalid_params");

        // Run a turn on t2 -> triggers t1.
        c.call(
            "thread.send",
            json!({ "thread": t2, "input": { "text": "do work" } }),
        )
        .await;
        let evs = c.events_until("turn.ended").await;
        let started = evs.iter().find(|e| e["kind"] == "turn.started").unwrap();
        assert!(
            started["data"]["input"]["text"]
                .as_str()
                .unwrap()
                .contains("t2 finished")
        );
        assert_eq!(started["data"]["from"]["deferred"], deferred["entry"]["id"]);

        // `after_command` runs under the supervisor and appends exit code + output.
        let argv = if cfg!(windows) {
            json!(["cmd", "/C", "echo defer-ok"])
        } else {
            json!(["sh", "-c", "echo defer-ok"])
        };
        c.call(
            "thread.defer",
            json!({
                "thread": t1,
                "input": { "text": "command done" },
                "trigger": { "kind": "after_command", "argv": argv, "timeoutS": 30 }
            }),
        )
        .await;
        let evs2 = c.events_until("turn.ended").await;
        let started2 = evs2.iter().find(|e| e["kind"] == "turn.started").unwrap();
        let body = started2["data"]["input"]["text"].as_str().unwrap();
        assert!(body.contains("command done"), "{body}");
        assert!(body.contains("exited 0"), "{body}");
        assert!(body.contains("defer-ok"), "{body}");

        // Future `at` entry can be triggered early via `thread.queue.sendNow`.
        let future = c
            .call(
                "thread.defer",
                json!({
                    "thread": t1,
                    "input": { "text": "run early" },
                    "trigger": { "kind": "at", "time": "2099-01-01T00:00:00.000Z" }
                }),
            )
            .await;
        let entry_id = future["entry"]["id"].as_str().unwrap();
        c.call(
            "thread.queue.sendNow",
            json!({ "thread": t1, "entry": entry_id }),
        )
        .await;
        let evs3 = c.events_until("turn.ended").await;
        assert!(
            evs3.iter()
                .any(|e| e["kind"] == "turn.started" && e["data"]["input"]["text"] == "run early")
        );

        core.stop().await;
    })
    .await;
}
