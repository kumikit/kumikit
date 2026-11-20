//! `kumikit:host/client` against the real binary (`docs/spec/03-kernel.md#client-interface`):
//! every operation needs its `permissions.client` entry and a workspace the plugin may
//! act in; what the plugin sends and answers is recorded as the plugin's.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, install, kkp, probe_kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

fn client_kkp(dirs: &Dirs, manifest: &str) -> std::path::PathBuf {
    let wasm = std::fs::read(kumikit_test_plugins::CLIENT_WASM).unwrap();
    kkp(dirs, "client", manifest, &[("plugin.wasm", &wasm)])
}

/// Runs a client-plugin command: `Ok(result)` or `Err(error code)`.
async fn run(c: &mut Client, command: &str, args: Value) -> Result<Value, String> {
    let r = c
        .request(
            "command.run",
            json!({ "plugin": "test.client", "command": command, "args": args }),
        )
        .await;
    match r.get("error") {
        Some(e) => Err(e["data"]["code"].as_str().unwrap_or("?").to_owned()),
        None => Ok(serde_json::from_str(r["result"]["message"].as_str().unwrap()).unwrap()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_plugin_acts_only_where_it_may_and_is_recorded() {
    within(async {
        let dirs = Dirs::new();
        let _core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        install(
            &mut c,
            &client_kkp(&dirs, kumikit_test_plugins::CLIENT_MANIFEST),
        )
        .await;
        let w1 = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]["id"]
            .clone();
        let other = dirs.root().join("other");
        std::fs::create_dir_all(&other).unwrap();
        let w2 = c
            .call("workspace.open", json!({ "path": other.to_str().unwrap() }))
            .await["workspace"]["id"]
            .clone();

        // Nothing is linked yet: it sees no workspace and may not act in one.
        assert_eq!(run(&mut c, "workspaces", json!({})).await, Ok(json!([])));
        assert_eq!(
            run(&mut c, "threads", json!({ "workspace": w1 })).await,
            Err("permission_denied".into())
        );
        c.call(
            "plugin.setWorkspaces",
            json!({ "plugin": "test.client", "workspaces": [w1] }),
        )
        .await;
        assert_eq!(
            c.call("plugin.workspaces", json!({ "plugin": "test.client" }))
                .await["workspaces"],
            json!([w1])
        );
        assert_eq!(
            run(&mut c, "workspaces", json!({})).await.unwrap()[0]["id"],
            w1
        );
        assert_eq!(
            run(
                &mut c,
                "create",
                json!({ "workspace": w2, "provider": "test.probe/probe" })
            )
            .await,
            Err("permission_denied".into())
        );

        // In w1 it creates a thread, sends to it and answers its approval.
        let t = run(
            &mut c,
            "create",
            json!({ "workspace": w1, "provider": "test.probe/probe", "model": "tool-approval" }),
        )
        .await
        .unwrap();
        let thread = t["id"].as_str().unwrap().to_owned();
        c.call(
            "events.subscribe",
            json!({ "topic": format!("thread:{thread}"), "after": 0 }),
        )
        .await;
        run(
            &mut c,
            "send",
            json!({ "thread": thread, "text": "list files" }),
        )
        .await
        .unwrap();
        let events = c.events_until("approval.requested").await;
        assert_eq!(
            events[0]["data"]["from"],
            json!({ "plugin": "test.client" })
        );
        let approval = events.last().unwrap()["data"]["approval"]["id"].clone();
        run(
            &mut c,
            "respond",
            json!({ "approval": approval, "decision": "allow" }),
        )
        .await
        .unwrap();
        let events = c.events_until("turn.ended").await;
        let resolved = events
            .iter()
            .find(|e| e["kind"] == "approval.resolved")
            .unwrap();
        assert_eq!(resolved["data"]["plugin"], "test.client");
        assert_eq!(events.last().unwrap()["data"]["outcome"], "done");

        // It reads the thread and follows its log through a pollable subscription.
        let snapshot = run(&mut c, "get", json!({ "thread": thread }))
            .await
            .unwrap();
        assert_eq!(snapshot["thread"]["id"], thread.as_str());
        let kinds = run(
            &mut c,
            "follow",
            json!({ "topic": format!("thread:{thread}"), "after": 0, "count": 3 }),
        )
        .await
        .unwrap();
        assert_eq!(kinds[0], "turn.started");

        // Unlinked again, the same thread is out of reach.
        c.call(
            "plugin.setWorkspaces",
            json!({ "plugin": "test.client", "workspaces": [] }),
        )
        .await;
        assert_eq!(
            run(&mut c, "get", json!({ "thread": thread })).await,
            Err("permission_denied".into())
        );
        assert_eq!(
            run(
                &mut c,
                "follow",
                json!({ "topic": format!("thread:{thread}") })
            )
            .await,
            Err("permission_denied".into())
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_operation_needs_its_grant() {
    within(async {
        let dirs = Dirs::new();
        let _core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        let manifest = kumikit_test_plugins::CLIENT_MANIFEST.replace(
            "client = [\"workspaces.list\", \"threads.read\", \"threads.create\", \"threads.send\", \"threads.cancel\", \"approvals.respond\"]\nworkspaces = \"linked\"",
            "client = [\"workspaces.list\", \"threads.read\"]\nworkspaces = \"all\"",
        );
        install(&mut c, &client_kkp(&dirs, &manifest)).await;
        let w = c.call("workspace.open", json!({ "path": dirs.workspace() })).await["workspace"]["id"].clone();
        let thread = c
            .call("thread.create", json!({ "workspace": w, "provider": "test.probe/probe", "model": "echo" }))
            .await["thread"]["id"]
            .clone();
        // `all` reaches every workspace without links; reads are granted.
        assert_eq!(run(&mut c, "threads", json!({ "workspace": w })).await.unwrap()[0]["id"], thread);
        for (command, args) in [
            ("providers", json!({})),
            ("create", json!({ "workspace": w })),
            ("send", json!({ "thread": thread, "text": "hi" })),
            ("cancel", json!({ "thread": thread })),
            ("respond", json!({ "approval": "01JCAPNOPE", "decision": "allow" })),
        ] {
            assert_eq!(run(&mut c, command, args).await, Err("permission_denied".into()), "{command}");
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn client_operations_without_workspaces_are_refused_at_install() {
    within(async {
        let dirs = Dirs::new();
        let _core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        let manifest =
            kumikit_test_plugins::CLIENT_MANIFEST.replace("workspaces = \"linked\"\n", "");
        let r = c
            .request(
                "plugin.install",
                json!({ "path": client_kkp(&dirs, &manifest).to_str().unwrap() }),
            )
            .await;
        assert!(r.to_string().contains("need permissions.workspaces"), "{r}");
        let manifest = kumikit_test_plugins::CLIENT_MANIFEST
            .replace("\"threads.cancel\"", "\"threads.delete\"");
        let r = c
            .request(
                "plugin.install",
                json!({ "path": client_kkp(&dirs, &manifest).to_str().unwrap() }),
            )
            .await;
        assert!(r.to_string().contains("threads.delete"), "{r}");
    })
    .await;
}
