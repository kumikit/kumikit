//! Forks (`thread.fork`), edit-and-resend (`beforeTurn`), and side threads
//! (`thread.create { parent, side: true }`).

use std::future::Future;

use serde_json::json;

use crate::support::{Client, Core, DEADLINE, Dirs, install, probe_kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_at_and_before_turn_and_side_thread_context() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;

        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]["id"]
            .clone();
        let main = c
            .call(
                "thread.create",
                json!({
                    "workspace": ws,
                    "provider": "test.probe/probe",
                    "model": "echo",
                    "title": "Main task"
                }),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        c.call(
            "events.subscribe",
            json!({ "topic": format!("thread:{main}"), "after": 0 }),
        )
        .await;

        // Run two turns on the main thread.
        let t1 = c
            .call(
                "thread.send",
                json!({ "thread": main, "input": { "text": "alpha fact" } }),
            )
            .await["turn"]
            .as_str()
            .unwrap()
            .to_owned();
        c.events_until("turn.ended").await;

        let t2 = c
            .call(
                "thread.send",
                json!({ "thread": main, "input": { "text": "beta mistake" } }),
            )
            .await["turn"]
            .as_str()
            .unwrap()
            .to_owned();
        c.events_until("turn.ended").await;

        // 1. Fork before t2 (edit-and-resend): excludes t2, keeps t1, leaves source unchanged.
        let edit_fork = c
            .call(
                "thread.fork",
                json!({ "thread": main, "beforeTurn": t2, "title": "Edited branch" }),
            )
            .await["thread"]
            .clone();
        let edit_id = edit_fork["id"].as_str().unwrap().to_owned();
        assert_eq!(edit_fork["forkedFrom"]["thread"], main);
        assert_eq!(edit_fork["forkedFrom"]["turn"], t2);

        let edit_snap = c.call("thread.get", json!({ "thread": edit_id })).await;
        let edit_turns = edit_snap["turns"].as_array().unwrap();
        assert_eq!(edit_turns.len(), 1);
        assert_eq!(edit_turns[0]["id"], t1);

        // Source still has both turns.
        let main_snap = c.call("thread.get", json!({ "thread": main })).await;
        assert_eq!(main_snap["turns"].as_array().unwrap().len(), 2);

        // Sending on the fork delivers handoff context containing "alpha fact".
        c.call(
            "events.subscribe",
            json!({ "topic": format!("thread:{edit_id}"), "after": edit_snap["seq"] }),
        )
        .await;
        c.call(
            "thread.send",
            json!({ "thread": edit_id, "input": { "text": "gamma retry" } }),
        )
        .await;
        let evs = c.events_until("turn.ended").await;
        assert!(
            evs.iter().any(|e| e["kind"] == "handoff.delivered"),
            "fork's first turn should deliver handoff: {evs:?}"
        );
        let reply = evs.iter().find(|e| e["kind"] == "text.done").unwrap()["data"]["text"]
            .as_str()
            .unwrap();
        assert!(reply.contains("alpha fact"), "{reply}");
        assert!(reply.contains("gamma retry"), "{reply}");

        // 2. Side thread: hidden from main thread.list, shown with includeSide: true,
        // receives unseen parent context without touching main's queue.
        let side = c
            .call(
                "thread.create",
                json!({
                    "workspace": ws,
                    "parent": main,
                    "side": true,
                    "title": "Side question"
                }),
            )
            .await["thread"]
            .clone();
        let side_id = side["id"].as_str().unwrap().to_owned();
        assert_eq!(side["parent"], main);
        assert_eq!(side["side"], true);

        let visible = c.call("thread.list", json!({ "workspace": ws })).await["threads"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            !visible.iter().any(|t| t["id"] == side_id),
            "side thread must be hidden from default thread.list"
        );
        let with_side = c
            .call(
                "thread.list",
                json!({ "workspace": ws, "includeSide": true }),
            )
            .await["threads"]
            .as_array()
            .unwrap()
            .clone();
        assert!(with_side.iter().any(|t| t["id"] == side_id));

        c.call(
            "events.subscribe",
            json!({ "topic": format!("thread:{side_id}"), "after": 0 }),
        )
        .await;
        c.call(
            "thread.send",
            json!({ "thread": side_id, "input": { "text": "quick side question" } }),
        )
        .await;
        let side_evs = c.events_until("turn.ended").await;
        let side_reply =
            side_evs.iter().find(|e| e["kind"] == "text.done").unwrap()["data"]["text"]
                .as_str()
                .unwrap();
        assert!(side_reply.contains("[Main thread context]"), "{side_reply}");
        assert!(side_reply.contains("alpha fact"), "{side_reply}");
        assert!(side_reply.contains("quick side question"), "{side_reply}");

        // Parent's queue and turns were not touched by the side thread.
        let main_after = c.call("thread.get", json!({ "thread": main })).await;
        assert_eq!(main_after["turns"].as_array().unwrap().len(), 2);
        assert!(
            main_after
                .get("queue")
                .and_then(|v| v.as_array())
                .is_none_or(Vec::is_empty)
        );

        core.stop().await;
    })
    .await;
}
