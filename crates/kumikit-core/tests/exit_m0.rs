//! M0 exit criteria (`docs/spec/12-roadmap.md#m0--protocol-and-kernel`): a client creates
//! a thread, sends a message, receives streamed events, disconnects mid-turn, reconnects
//! from its cursor and sees no gap or duplicate. Runs the real `kumikit` binary.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, kinds, record, seqs};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

/// Opens the test workspace and creates a thread with `model`; returns its topic.
async fn thread(c: &mut Client, dirs: &Dirs, model: &str) -> (String, String) {
    let ws = c
        .call("workspace.open", json!({"path": dirs.workspace()}))
        .await;
    let t = c
        .call(
            "thread.create",
            json!({"workspace": ws["workspace"]["id"], "model": model}),
        )
        .await;
    let id = t["thread"]["id"].as_str().unwrap().to_owned();
    (id.clone(), format!("thread:{id}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_from_cursor_sees_no_gap_or_duplicate() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;

        // Client A starts a turn and follows it until the agent waits for an approval.
        let mut a = Client::hello(&dirs.address).await;
        let (thread, topic) = thread(&mut a, &dirs, "tool-approval").await;
        let sub = a
            .call("events.subscribe", json!({"topic": topic, "after": 0}))
            .await;
        assert_eq!(sub["head"], 0);
        let sent = a
            .call(
                "thread.send",
                json!({"thread": thread, "input": {"text": "list files"}}),
            )
            .await;
        assert_eq!(sent["queued"], false);
        let before = a.events_until("approval.requested").await;
        assert_eq!(
            kinds(&before),
            [
                "turn.started",
                "text.delta",
                "tool.started",
                "approval.requested"
            ]
        );
        let cursor = before.last().unwrap()["seq"].as_u64().unwrap();
        let approval = before.last().unwrap()["data"]["approval"]["id"].clone();
        record("a-before-disconnect", &a.transcript);
        drop(a); // disconnect mid-turn

        // Client B answers while A is away, so the turn produces events A has not seen.
        let mut b = Client::hello(&dirs.address).await;
        b.call(
            "thread.respondApproval",
            json!({"approval": approval, "decision": "allow"}),
        )
        .await;
        b.call("events.subscribe", json!({"topic": topic, "after": cursor}))
            .await;
        let rest = b.events_until("turn.ended").await;
        let end = rest.last().unwrap()["seq"].as_u64().unwrap();
        assert_eq!(rest.last().unwrap()["data"]["outcome"], "done");
        record("b-answers", &b.transcript);
        drop(b);

        // A reconnects from its cursor: exactly the events after it, once each.
        let mut a = Client::hello(&dirs.address).await;
        let sub = a
            .call("events.subscribe", json!({"topic": topic, "after": cursor}))
            .await;
        assert_eq!(sub["head"], end);
        let after = a.events_until("turn.ended").await;
        assert_eq!(seqs(&after), ((cursor + 1)..=end).collect::<Vec<_>>());
        assert_eq!(after, rest, "both clients saw the same events");
        assert_eq!(
            kinds(&after),
            [
                "approval.resolved",
                "tool.output",
                "tool.finished",
                "text.delta",
                "text.done",
                "usage",
                "turn.ended"
            ]
        );

        // The whole transcript is contiguous from 1 and agrees with the folded snapshot.
        let all: Vec<Value> = before.into_iter().chain(after).collect();
        assert_eq!(seqs(&all), (1..=end).collect::<Vec<_>>());
        let snap = a.call("thread.get", json!({"thread": thread})).await;
        assert_eq!(snap["seq"], end);
        assert_eq!(snap["thread"]["status"], "idle");
        assert_eq!(snap["pendingApprovals"], json!([]));
        assert_eq!(snap["turns"][0]["outcome"], "done");
        assert!(a.no_buffered_events(), "no event after turn.ended");
        record("a-after-reconnect", &a.transcript);

        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribers_joining_mid_stream_get_replay_then_live() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut sender = Client::hello(&dirs.address).await;
        let (thread, topic) = thread(&mut sender, &dirs, "echo").await;
        let words: Vec<String> = (0..300).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");

        let mut watchers = Vec::new();
        for _ in 0..3 {
            watchers.push(Client::hello(&dirs.address).await);
        }
        sender
            .call(
                "thread.send",
                json!({"thread": thread, "input": {"text": text}}),
            )
            .await;
        // Subscribe while the turn streams, from different cursors.
        let cursors = [0u64, 5, 50];
        let mut tasks = Vec::new();
        for (mut w, after) in watchers.into_iter().zip(cursors) {
            let topic = topic.clone();
            tasks.push(tokio::spawn(async move {
                w.call("events.subscribe", json!({"topic": topic, "after": after}))
                    .await;
                (after, w.events_until("turn.ended").await)
            }));
        }
        // 1 turn.started + 300 deltas + text.done + usage + turn.ended.
        let end = 304;
        for t in tasks {
            let (after, events) = t.await.unwrap();
            assert_eq!(seqs(&events), ((after + 1)..=end).collect::<Vec<_>>());
        }
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn core_restart_between_disconnect_and_reconnect() {
    for graceful in [true, false] {
        within(async {
            let dirs = Dirs::new();
            let core = Core::start(&dirs).await;
            let mut a = Client::hello(&dirs.address).await;
            let (thread, topic) = thread(&mut a, &dirs, "tool-approval").await;
            a.call("events.subscribe", json!({"topic": topic, "after": 0}))
                .await;
            a.call(
                "thread.send",
                json!({"thread": thread, "input": {"text": "go"}}),
            )
            .await;
            let before = a.events_until("approval.requested").await;
            let cursor = before.last().unwrap()["seq"].as_u64().unwrap();
            drop(a);
            if graceful {
                core.stop().await;
            } else {
                core.crash().await;
            }

            let core = Core::start(&dirs).await;
            let mut a = Client::hello(&dirs.address).await;
            a.call("events.subscribe", json!({"topic": topic, "after": cursor}))
                .await;
            let after = a.events_until("turn.ended").await;
            assert_eq!(seqs(&after), [cursor + 1, cursor + 2]);
            assert_eq!(after[0]["data"]["decision"], "cancelled");
            assert_eq!(after[1]["data"]["outcome"], "failed");
            assert_eq!(after[1]["data"]["error"], "core_restarted");
            let snap = a.call("thread.get", json!({"thread": thread})).await;
            assert_eq!(snap["thread"]["status"], "failed");
            core.stop().await;
        })
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn second_core_on_the_same_address_is_refused() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
            .args(["serve", "--socket", &dirs.address, "--data-dir"])
            .arg(dirs.data())
            .output()
            .await
            .unwrap();
        assert!(!out.status.success());
        assert!(
            out.stdout.is_empty(),
            "the second core must not report ready"
        );
        core.stop().await;
    })
    .await;
}
