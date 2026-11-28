//! `kumikit:host/client` against the real binary (`docs/spec/03-kernel.md#client-interface`):
//! every operation needs its `permissions.client` entry and a workspace the plugin may
//! act in; what the plugin sends and answers is recorded as the plugin's.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

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

        // It reads the thread, defers/lists/removes a queued entry, suggests a title, and follows its log.
        let snapshot = run(&mut c, "get", json!({ "thread": thread }))
            .await
            .unwrap();
        assert_eq!(snapshot["thread"]["id"], thread.as_str());
        let ev_page = run(
            &mut c,
            "read-events",
            json!({ "thread": thread, "after": 0, "limit": 20 }),
        )
        .await
        .unwrap();
        assert!(!ev_page.as_array().unwrap().is_empty());
        let deferred = run(
            &mut c,
            "defer",
            json!({
                "thread": thread,
                "text": "check later",
                "trigger": { "kind": "at", "time": "2099-01-01T00:00:00.000Z" }
            }),
        )
        .await
        .unwrap();
        let entry_id = deferred["id"].as_str().unwrap().to_owned();
        let queue = run(&mut c, "list-queue", json!({ "thread": thread }))
            .await
            .unwrap();
        assert_eq!(queue.as_array().unwrap().len(), 1);
        run(
            &mut c,
            "remove-queued",
            json!({ "thread": thread, "entry": entry_id }),
        )
        .await
        .unwrap();
        assert_eq!(
            run(&mut c, "list-queue", json!({ "thread": thread }))
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            0
        );
        // Agent title suggestion applies until the user renames the thread.
        assert_eq!(
            run(
                &mut c,
                "suggest-title",
                json!({ "thread": thread, "title": "Agent title" })
            )
            .await
            .unwrap()["applied"],
            json!(true)
        );
        c.call(
            "thread.update",
            json!({ "thread": thread, "title": "User title" }),
        )
        .await;
        assert_eq!(
            run(
                &mut c,
                "suggest-title",
                json!({ "thread": thread, "title": "Ignored title" })
            )
            .await
            .unwrap()["applied"],
            json!(false)
        );
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

/// `kumikit:threads@0.2.0` (ADR 0034): models with settings, threads created with
/// settings, and permission modes bounded by `full_access` and the origin thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_plugin_creates_threads_with_settings_and_bounded_modes() {
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
        let w = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]["id"]
            .clone();
        let other = dirs.root().join("other");
        std::fs::create_dir_all(&other).unwrap();
        let w2 = c
            .call("workspace.open", json!({ "path": other.to_str().unwrap() }))
            .await["workspace"]["id"]
            .clone();
        c.call(
            "plugin.setWorkspaces",
            json!({ "plugin": "test.client", "workspaces": [w, w2] }),
        )
        .await;
        let mode = |snap: &Value| snap["thread"]["permissionMode"].clone();

        // Models come with their settings.
        let models = run(&mut c, "models", json!({ "provider": "test.probe/probe" }))
            .await
            .unwrap();
        assert_eq!(models[0]["id"], "probe");
        assert_eq!(models[0]["options"][0]["id"], "effort");

        // A thread with a model setting; an unknown value is an invalid argument.
        let t = run(
            &mut c,
            "create-with",
            json!({
                "workspace": w,
                "provider": "test.probe/probe",
                "model": "probe",
                "options": { "effort": "high" },
                "title": "Review"
            }),
        )
        .await
        .unwrap();
        let snap = run(&mut c, "get", json!({ "thread": t["id"] })).await.unwrap();
        assert_eq!(snap["thread"]["options"], json!({ "effort": "high" }));
        assert_eq!(snap["thread"]["title"], "Review");
        assert_eq!(mode(&snap), Value::Null, "standard is the default");
        assert_eq!(
            run(
                &mut c,
                "create-with",
                json!({
                    "workspace": w,
                    "provider": "test.probe/probe",
                    "model": "probe",
                    "options": { "effort": "extreme" }
                }),
            )
            .await,
            Err("invalid_params".into())
        );

        // Never full_access, even without an origin.
        assert_eq!(
            run(
                &mut c,
                "create-with",
                json!({ "workspace": w, "provider": "test.probe/probe", "permissionMode": "full_access" }),
            )
            .await,
            Err("permission_denied".into())
        );

        // A plan origin: the default follows it and a looser mode is refused.
        let origin = c
            .call(
                "thread.create",
                json!({ "workspace": w, "provider": "test.probe/probe", "model": "echo", "permissionMode": "plan" }),
            )
            .await["thread"]["id"]
            .clone();
        let child = run(
            &mut c,
            "create-with",
            json!({ "workspace": w, "provider": "test.probe/probe", "origin": origin }),
        )
        .await
        .unwrap();
        let snap = run(&mut c, "get", json!({ "thread": child["id"] })).await.unwrap();
        assert_eq!(mode(&snap), "plan");
        // ADR 0036: a thread started for an origin is a side thread of it, so it is hidden
        // from the sidebar's list and only shows with includeSide.
        assert_eq!(snap["thread"]["parent"], origin);
        assert_eq!(snap["thread"]["side"], true);
        let plain = c.call("thread.list", json!({ "workspace": w })).await["threads"].clone();
        assert!(!plain.to_string().contains(child["id"].as_str().unwrap()));
        let with_side = c
            .call("thread.list", json!({ "workspace": w, "includeSide": true }))
            .await["threads"]
            .clone();
        assert!(with_side.to_string().contains(child["id"].as_str().unwrap()));
        // Without an origin, a thread has no parent and stays in the sidebar.
        let plain_thread = run(
            &mut c,
            "create-with",
            json!({ "workspace": w, "provider": "test.probe/probe" }),
        )
        .await
        .unwrap();
        let snap = run(&mut c, "get", json!({ "thread": plain_thread["id"] }))
            .await
            .unwrap();
        assert_eq!(snap["thread"]["parent"], Value::Null);
        assert_eq!(snap["thread"]["side"], Value::Null);
        assert_eq!(
            run(
                &mut c,
                "create-with",
                json!({ "workspace": w, "provider": "test.probe/probe", "origin": origin, "permissionMode": "standard" }),
            )
            .await,
            Err("permission_denied".into())
        );

        // The origin must be in the workspace the thread is created in.
        assert_eq!(
            run(
                &mut c,
                "create-with",
                json!({ "workspace": w2, "provider": "test.probe/probe", "origin": origin }),
            )
            .await,
            Err("permission_denied".into())
        );

        // ADR 0036: with no mode asked for, a side thread of an accept_edits origin gets
        // standard. The kernel would inherit accept_edits if the host left the mode out.
        let loose = c
            .call(
                "thread.create",
                json!({ "workspace": w, "provider": "test.probe/probe", "model": "echo", "permissionMode": "accept_edits" }),
            )
            .await["thread"]["id"]
            .clone();
        let child = run(
            &mut c,
            "create-with",
            json!({ "workspace": w, "provider": "test.probe/probe", "origin": loose }),
        )
        .await
        .unwrap();
        let snap = run(&mut c, "get", json!({ "thread": child["id"] })).await.unwrap();
        // The kernel stores standard as no mode, so the snapshot shows null.
        assert_eq!(mode(&snap), Value::Null);
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
            "client = [\"workspaces.list\", \"threads.read\", \"threads.create\", \"threads.send\", \"threads.steer\", \"threads.defer\", \"threads.queue\", \"threads.title.suggest\", \"threads.cancel\", \"approvals.respond\"]\nworkspaces = \"linked\"",
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
            ("models", json!({ "provider": "test.probe/probe" })),
            ("create-with", json!({ "workspace": w })),
            ("send", json!({ "thread": thread, "text": "hi" })),
            ("steer", json!({ "thread": thread, "text": "go left" })),
            (
                "defer",
                json!({
                    "thread": thread,
                    "text": "later",
                    "trigger": { "kind": "at", "time": "2099-01-01T00:00:00.000Z" }
                }),
            ),
            ("list-queue", json!({ "thread": thread })),
            ("remove-queued", json!({ "thread": thread, "entry": "01JCAPNOPE" })),
            ("suggest-title", json!({ "thread": thread, "title": "Nope" })),
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
