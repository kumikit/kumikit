//! `plugin.*`, `core.hello` plugins and `core.pluginsChanged`, against the real binary.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use serde_json::json;
use support::{Client, Core, DEADLINE, Dirs, PROBE, install, probe_kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

#[tokio::test(flavor = "multi_thread")]
async fn installing_and_disabling_is_announced_to_every_client() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut watcher = Client::hello(&dirs.address).await;
        let mut admin = Client::hello(&dirs.address).await;

        let hello = admin
            .call("core.hello", json!({"protocol":"0.11", "client": {"name": "t", "version": "0"}}))
            .await;
        assert_eq!(hello["plugins"], json!([]));
        assert!(hello["capabilities"].as_array().unwrap().contains(&json!("plugins")));

        let installed = admin
            .call("plugin.install", json!({"path": probe_kkp(&dirs).to_str().unwrap()}))
            .await;
        assert_eq!(installed["plugin"]["state"], "installed");
        assert_eq!(installed["plugin"]["signed"], false);
        assert_eq!(installed["plugin"]["requested"]["spawn"], json!(["fake-acp-agent"]));

        // Permissions are granted as a whole.
        let wrong = admin
            .request(
                "plugin.enable",
                json!({"plugin": "test.probe", "grant": {"spawn": [], "network": [], "files": "none", "secrets": [], "tools": false}}),
            )
            .await;
        assert_eq!(wrong["error"]["data"]["code"], "conflict");

        let grant = installed["plugin"]["requested"].clone();
        let enabled = admin
            .call("plugin.enable", json!({"plugin": "test.probe", "grant": grant}))
            .await;
        assert_eq!(enabled["plugin"]["state"], "enabled");
        assert_eq!(enabled["plugin"]["contributes"]["providers"], json!([PROBE]));

        let note = watcher.notification("core.pluginsChanged").await;
        assert_eq!(note["plugins"][0]["id"], "test.probe");
        assert_eq!(note["plugins"][0]["contributes"]["providers"], json!([PROBE]));

        let list = admin.call("plugin.list", json!({})).await;
        assert_eq!(list["plugins"].as_array().unwrap().len(), 1);

        admin
            .call("plugin.disable", json!({"plugin": "test.probe"}))
            .await;
        let note = watcher.notification("core.pluginsChanged").await;
        assert_eq!(note["plugins"], json!([]));

        admin
            .call("plugin.uninstall", json!({"plugin": "test.probe", "deleteData": true}))
            .await;
        let list = admin.call("plugin.list", json!({})).await;
        assert_eq!(list["plugins"], json!([]));
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn enabled_plugins_survive_a_restart_and_appear_in_hello() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        install(&mut c, &probe_kkp(&dirs)).await;
        drop(c);
        core.stop().await;

        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        let hello = c
            .call(
                "core.hello",
                json!({"protocol":"0.11", "client": {"name": "t", "version": "0"}}),
            )
            .await;
        assert_eq!(
            hello["plugins"][0]["contributes"]["providers"],
            json!([PROBE])
        );
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_packages_are_invalid_params() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        let r = c
            .request("plugin.install", json!({"path": "relative.kkp"}))
            .await;
        assert_eq!(r["error"]["data"]["code"], "invalid_params");
        let junk = dirs.root().join("junk.kkp");
        std::fs::write(&junk, b"not a package").unwrap();
        let r = c
            .request("plugin.install", json!({"path": junk.to_str().unwrap()}))
            .await;
        assert_eq!(r["error"]["data"]["code"], "invalid_params", "{r}");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_package_can_be_installed_from_its_bytes() {
    use base64::Engine;
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        let bytes = std::fs::read(probe_kkp(&dirs)).unwrap();
        let package = base64::engine::general_purpose::STANDARD.encode(bytes);
        let installed = c.call("plugin.install", json!({"package": package})).await;
        assert_eq!(installed["plugin"]["id"], "test.probe");
        assert_eq!(installed["plugin"]["state"], "installed");

        // The same rules as a file: not base64, not a package, both or neither given.
        for params in [
            json!({"package": "not base64!"}),
            json!({"package": base64::engine::general_purpose::STANDARD.encode(b"junk")}),
            json!({"package": "", "path": "/x.kkp"}),
            json!({}),
        ] {
            let r = c.request("plugin.install", params.clone()).await;
            assert_eq!(
                r["error"]["data"]["code"], "invalid_params",
                "{params}: {r}"
            );
        }
        core.stop().await;
    })
    .await;
}
