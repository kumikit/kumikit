//! Schedules against the real binary (`docs/spec/03-kernel.md#schedules`): a plugin sets
//! one, the user sees, runs, pauses and removes it, and a run starts a turn recorded as
//! the plugin's. Timing rules (due, missed, skipped, DST) are kernel tests with explicit
//! times; nothing here waits for a clock.

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

fn client_kkp(dirs: &Dirs, name: &str, manifest: &str) -> std::path::PathBuf {
    let wasm = std::fs::read(kumikit_test_plugins::CLIENT_WASM).unwrap();
    kkp(dirs, name, manifest, &[("plugin.wasm", &wasm)])
}

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
async fn a_schedule_is_listed_runs_a_turn_and_can_be_paused_and_removed() {
    within(async {
        let dirs = Dirs::new();
        let _core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        let manifest = kumikit_test_plugins::CLIENT_MANIFEST.replace("workspaces = \"linked\"", "workspaces = \"all\"");
        install(&mut c, &client_kkp(&dirs, "client", &manifest)).await;
        let ws = c.call("workspace.open", json!({ "path": dirs.workspace() })).await["workspace"]["id"].clone();

        let next = run(
            &mut c,
            "timer-set",
            json!({ "key": "morning", "cron": "0 9 * * *", "zone": "UTC",
                    "job": { "workspace": ws, "provider": "test.probe/probe", "model": "echo", "text": "good morning" } }),
        )
        .await
        .unwrap();
        assert!(next.as_str().unwrap().ends_with("T09:00:00Z"), "{next}");
        let list = c.call("schedule.list", json!({})).await["schedules"].clone();
        assert_eq!(list[0]["plugin"], "test.client");
        assert_eq!(list[0]["key"], "morning");
        assert_eq!(list[0]["description"], "test job morning");
        assert_eq!(list[0]["source"], "plugin");
        assert_eq!(list[0]["nextRun"], next);
        assert_eq!(run(&mut c, "timers", json!({})).await.unwrap(), json!(["morning"]));

        // "Run now": the plugin starts a thread and the turn says who sent it.
        c.call("events.subscribe", json!({ "topic": format!("workspace:{}", ws.as_str().unwrap()), "after": 0 })).await;
        c.call("schedule.run", json!({ "plugin": "test.client", "key": "morning" })).await;
        let created = c.events_until("thread.created").await;
        let thread = created.last().unwrap()["data"]["thread"]["id"].as_str().unwrap().to_owned();
        c.call("events.subscribe", json!({ "topic": format!("thread:{thread}"), "after": 0 })).await;
        let events = c.events_until("turn.ended").await;
        // The workspace topic's events arrive on the same connection: pick the thread's.
        let started = events
            .iter()
            .find(|e| e["kind"] == "turn.started" && e["topic"] == format!("thread:{thread}"))
            .expect("turn.started");
        assert_eq!(started["data"]["from"], json!({ "plugin": "test.client" }));
        assert_eq!(started["data"]["input"]["text"], "good morning");
        // The run is recorded once its call returns.
        loop {
            let fired = run(&mut c, "fired", json!({})).await.unwrap();
            let last = c.call("schedule.list", json!({})).await["schedules"][0].clone();
            if fired.as_array().unwrap().len() == 1 && last["lastOutcome"] == "ran" {
                break;
            }
            tokio::task::yield_now().await;
        }

        // Paused, it keeps its row; removed, it is gone.
        c.call("schedule.setEnabled", json!({ "plugin": "test.client", "key": "morning", "enabled": false })).await;
        assert_eq!(c.call("schedule.list", json!({})).await["schedules"][0]["enabled"], false);
        c.call("schedule.delete", json!({ "plugin": "test.client", "key": "morning" })).await;
        assert_eq!(c.call("schedule.list", json!({})).await["schedules"], json!([]));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn schedules_need_their_permission_and_manifest_ones_stay() {
    within(async {
        let dirs = Dirs::new();
        let _core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        // Without `schedules`, timers are refused.
        let manifest = kumikit_test_plugins::CLIENT_MANIFEST.replace("schedules = true\n", "");
        install(&mut c, &client_kkp(&dirs, "plain", &manifest)).await;
        assert_eq!(
            run(&mut c, "timer-set", json!({ "key": "k", "cron": "@daily" })).await,
            Err("permission_denied".into())
        );
        c.call(
            "plugin.uninstall",
            json!({ "plugin": "test.client", "deleteData": true }),
        )
        .await;

        // A manifest trigger needs the permission at install.
        let manifest = kumikit_test_plugins::CLIENT_MANIFEST
            .replace("schedules = true\n", "")
            .replace(
                "[permissions]",
                "[activation]\non = [\"schedule:@daily\"]\n[permissions]",
            );
        let r = c
            .request(
                "plugin.install",
                json!({ "path": client_kkp(&dirs, "bad", &manifest).to_str().unwrap() }),
            )
            .await;
        assert!(r.to_string().contains("need permissions.schedules"), "{r}");

        // With it, the trigger is a manifest schedule: listed, pausable, not deletable,
        // and gone with the plugin.
        let manifest = kumikit_test_plugins::CLIENT_MANIFEST.replace(
            "[permissions]",
            "[activation]\non = [\"schedule:30 6 * * 1\"]\n[permissions]",
        );
        install(&mut c, &client_kkp(&dirs, "triggered", &manifest)).await;
        let list = c.call("schedule.list", json!({})).await["schedules"].clone();
        assert_eq!(list[0]["key"], "manifest:0");
        assert_eq!(list[0]["source"], "manifest");
        assert_eq!(list[0]["cron"], "30 6 * * 1");
        let r = c
            .request(
                "schedule.delete",
                json!({ "plugin": "test.client", "key": "manifest:0" }),
            )
            .await;
        assert_eq!(r["error"]["data"]["code"], "conflict");
        assert_eq!(
            run(&mut c, "timer-delete", json!({ "key": "manifest:0" })).await,
            Err("invalid_params".into())
        );
        c.call(
            "plugin.uninstall",
            json!({ "plugin": "test.client", "deleteData": false }),
        )
        .await;
        assert_eq!(
            c.call("schedule.list", json!({})).await["schedules"],
            json!([])
        );
    })
    .await;
}
