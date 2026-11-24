//! What the desktop client needs from the core (protocol 0.3): providers and models, the
//! workspace topic, installing from a folder, `core.shutdown` and bundled plugins.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use std::ffi::OsStr;

use serde_json::{Value, json};
use support::{
    Client, Core, DEADLINE, Dirs, PROBE, PROBE_MANIFEST, install, kkp, probe_kkp, provider_acp_kkp,
};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

#[tokio::test(flavor = "multi_thread")]
async fn providers_are_listed_with_names_and_models() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        assert_eq!(
            c.call("provider.list", json!({})).await,
            json!({"providers": []})
        );
        install(&mut c, &probe_kkp(&dirs)).await;
        let list = c.call("provider.list", json!({})).await;
        assert_eq!(
            list,
            json!({"providers": [{"id": PROBE, "name": "Probe", "plugin": "test.probe"}]})
        );
        let models = c.call("provider.models", json!({"provider": PROBE})).await;
        let ids: Vec<&str> = models["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["probe", "echo", "tool-approval", "fail"]);
        let missing = c
            .request("provider.models", json!({"provider": "test.nope/x"}))
            .await;
        assert_eq!(missing["error"]["data"]["code"], "not_found");
        core.stop().await;
    })
    .await;
}

/// Reads `events.event` notifications until one of `kind` arrives (inclusive).
async fn workspace_events_until(c: &mut Client, kind: &str) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        let e = c.event().await;
        let done = e["kind"] == kind;
        out.push(e);
        if done {
            return out;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_topic_follows_thread_changes() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        let ws = c
            .call("workspace.open", json!({"path": dirs.workspace()}))
            .await["workspace"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let topic = format!("workspace:{ws}");

        let cursor = c
            .request("events.subscribe", json!({"topic": topic, "after": 3}))
            .await;
        assert_eq!(cursor["error"]["data"]["code"], "invalid_params");
        let unknown = c
            .request(
                "events.subscribe",
                json!({"topic": "workspace:nope", "after": 0}),
            )
            .await;
        assert_eq!(unknown["error"]["data"]["code"], "not_found");

        // A second client watches; the first one acts.
        let mut watcher = Client::hello(&dirs.address).await;
        let head = watcher
            .call("events.subscribe", json!({"topic": topic, "after": 0}))
            .await;
        assert_eq!(head, json!({"head": 0}));

        let thread = c
            .call(
                "thread.create",
                json!({"workspace": ws, "provider": PROBE, "model": "echo"}),
            )
            .await["thread"]
            .clone();
        let id = thread["id"].as_str().unwrap().to_owned();
        c.call(
            "thread.send",
            json!({"thread": id, "input": {"text": "hi"}}),
        )
        .await;
        let mut events = workspace_events_until(&mut watcher, "thread.created").await;
        assert_eq!(events[0]["data"]["thread"], thread);
        // Running, then idle again; text events cause no workspace event.
        loop {
            let e = watcher.event().await;
            let idle = e["data"]["thread"]["status"] == "idle";
            events.push(e);
            if idle {
                break;
            }
        }
        let summary: Vec<(String, String)> = events
            .iter()
            .map(|e| {
                (
                    e["kind"].as_str().unwrap().to_owned(),
                    e["data"]["thread"]["status"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("thread.created".to_owned(), "idle".to_owned()),
                ("thread.updated".to_owned(), "running".to_owned()),
                ("thread.updated".to_owned(), "idle".to_owned()),
            ]
        );
        let seqs: Vec<u64> = events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, [1, 2, 3]);
        assert!(events.iter().all(|e| e["topic"] == topic));

        c.call("thread.archive", json!({"thread": id})).await;
        let archived = workspace_events_until(&mut watcher, "thread.archived").await;
        assert_eq!(archived.len(), 1, "{archived:?}");
        assert_eq!(archived[0]["data"]["thread"]["status"], "archived");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_with_a_manifest_installs_like_its_package() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        // `kkp` leaves the source folder next to the package.
        let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
        kkp(&dirs, "probe", PROBE_MANIFEST, &[("plugin.wasm", &wasm)]);
        let folder = dirs.root().join("probe-src");
        let installed = c
            .call("plugin.install", json!({"path": folder.to_str().unwrap()}))
            .await;
        assert_eq!(installed["plugin"]["id"], "test.probe");
        assert_eq!(installed["plugin"]["state"], "installed");
        assert_eq!(
            installed["plugin"]["requested"]["spawn"],
            json!(["fake-acp-agent"])
        );

        let empty = dirs.root().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let bad = c
            .request("plugin.install", json!({"path": empty.to_str().unwrap()}))
            .await;
        assert_eq!(bad["error"]["data"]["code"], "invalid_params");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_answers_then_stops_the_core() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        assert_eq!(c.call("core.shutdown", json!({})).await, json!({}));
        assert!(core.wait().await, "the core exits cleanly");
        // The socket is gone, so a new core can start on it.
        let core = Core::start(&dirs).await;
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bundled_plugins_are_seeded_once() {
    within(async {
        let dirs = Dirs::new();
        let bundle = dirs.root().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        // Seeded in file-name order: the probe is installed before provider-acp is
        // enabled, so once provider-acp shows as enabled both are done.
        std::fs::copy(probe_kkp(&dirs), bundle.join("a-probe.kkp")).unwrap();
        std::fs::copy(provider_acp_kkp(&dirs), bundle.join("b-provider-acp.kkp")).unwrap();
        std::fs::write(bundle.join("notes.txt"), "not a package").unwrap();
        let args = [OsStr::new("--bundled-plugins"), bundle.as_os_str()];

        let (core, _) = Core::start_with_args(&dirs, None, &args).await;
        // provider-acp asks for nothing, so it is enabled; the probe waits for a grant.
        // Seeding runs after `ready`: it is either done at hello, or announced later.
        let mut c = Client::connect(&dirs.address).await;
        let hello = c
            .call(
                "core.hello",
                json!({"protocol":"0.7", "client": {"name": "t", "version": "0"}}),
            )
            .await;
        if hello["plugins"].as_array().unwrap().is_empty() {
            let note = c.notification("core.pluginsChanged").await;
            assert_eq!(note["plugins"][0]["id"], "kumikit.provider-acp");
        }
        let states = |list: &Value| -> Vec<(String, String)> {
            list["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p["id"].as_str().unwrap().to_owned(),
                        p["state"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        };
        let list = c.call("plugin.list", json!({})).await;
        assert_eq!(
            states(&list),
            [
                ("kumikit.provider-acp".to_owned(), "enabled".to_owned()),
                ("test.probe".to_owned(), "installed".to_owned()),
            ]
        );

        // Removed plugins stay removed across restarts.
        c.call(
            "plugin.uninstall",
            json!({"plugin": "kumikit.provider-acp"}),
        )
        .await;
        core.stop().await;
        let (core, _) = Core::start_with_args(&dirs, None, &args).await;
        let mut c = Client::hello(&dirs.address).await;
        let list = c.call("plugin.list", json!({})).await;
        assert_eq!(
            states(&list),
            [("test.probe".to_owned(), "installed".to_owned())]
        );
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newer_bundled_version_replaces_the_bundled_copy() {
    within(async {
        let dirs = Dirs::new();
        let bundle = dirs.root().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
        let version = |v: &str, extra: &str| {
            let manifest = PROBE_MANIFEST
                .replace("version = \"0.1.0\"", &format!("version = \"{v}\""))
                .replace(
                    "spawn = [\"fake-acp-agent\"]",
                    &format!("spawn = [\"fake-acp-agent\"]\n{extra}"),
                );
            kkp(
                &dirs,
                &format!("probe-{v}"),
                &manifest,
                &[("plugin.wasm", &wasm)],
            )
        };
        let args = [OsStr::new("--bundled-plugins"), bundle.as_os_str()];

        // 0.1.0 is seeded and the user grants it. Seeding runs in file-name order and
        // provider-acp (seeded last) is enabled, so its announcement means both are done.
        std::fs::copy(version("0.1.0", ""), bundle.join("probe.kkp")).unwrap();
        std::fs::copy(provider_acp_kkp(&dirs), bundle.join("z-acp.kkp")).unwrap();
        let (core, _) = Core::start_with_args(&dirs, None, &args).await;
        let mut c = Client::hello(&dirs.address).await;
        let mut list = c.call("plugin.list", json!({})).await;
        while list["plugins"].as_array().unwrap().len() < 2 {
            c.notification("core.pluginsChanged").await;
            list = c.call("plugin.list", json!({})).await;
        }
        let plugin = |list: &Value| {
            list["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == "test.probe")
                .unwrap()
                .clone()
        };
        let requested = plugin(&list)["requested"].clone();
        c.call(
            "plugin.enable",
            json!({"plugin": "test.probe", "grant": requested}),
        )
        .await;
        core.stop().await;

        // 0.2.0 asks for the same: it replaces 0.1.0 and stays enabled with the grant.
        std::fs::copy(version("0.2.0", ""), bundle.join("probe.kkp")).unwrap();
        let (core, _) = Core::start_with_args(&dirs, None, &args).await;
        let mut c = Client::hello(&dirs.address).await;
        // The update disables, replaces and enables again, so the list can show 0.2.0
        // still disabled in between: wait for the end state, or for the changes to stop.
        let mut p = plugin(&c.call("plugin.list", json!({})).await);
        while !(p["version"] == "0.2.0" && p["state"] == "enabled") {
            let quiet = std::time::Duration::from_secs(10);
            if tokio::time::timeout(quiet, c.notification("core.pluginsChanged"))
                .await
                .is_err()
            {
                break;
            }
            p = plugin(&c.call("plugin.list", json!({})).await);
        }
        assert_eq!(p["version"], "0.2.0", "{p}");
        assert_eq!(p["state"], "enabled", "{p}");
        assert_eq!(p["source"], "bundled", "{p}");
        core.stop().await;

        // 0.3.0 asks for a secret too: it is installed but waits for a new grant.
        std::fs::copy(
            version("0.3.0", "secrets = [\"probe-token\"]"),
            bundle.join("probe.kkp"),
        )
        .unwrap();
        let (core, _) = Core::start_with_args(&dirs, None, &args).await;
        let mut c = Client::hello(&dirs.address).await;
        let mut p = plugin(&c.call("plugin.list", json!({})).await);
        while p["version"] != "0.3.0" {
            c.notification("core.pluginsChanged").await;
            p = plugin(&c.call("plugin.list", json!({})).await);
        }
        assert_ne!(p["state"], "enabled", "{p}");
        assert!(p.get("granted").is_none_or(Value::is_null), "{p}");
        core.stop().await;
    })
    .await;
}
