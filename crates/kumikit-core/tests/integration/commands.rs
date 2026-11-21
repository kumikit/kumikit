//! Plugin methods (`<plugin id>/<name>`) and core commands (`command.*`) against the real
//! binary (`docs/spec/02-protocol.md#methods-added-in-06`): only declared methods are
//! routed, `workspace` calls act for their workspace, and errors keep their codes.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs, PROBE_MANIFEST, install, kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

fn probe_with_methods(dirs: &Dirs) -> std::path::PathBuf {
    let manifest = PROBE_MANIFEST.replace(
        "[contributes]",
        "[contributes]\n\
         methods = [{ name = \"echo\" }, { name = \"cwd\", scope = \"workspace\" }, { name = \"cwd-core\" }, { name = \"fail\" }]\n\
         commands = [{ id = \"greet\", title = \"Greet\" }, { id = \"greet-submit\", title = \"Submit\", scope = \"workspace\", when = \"never\" }]",
    );
    let wasm = std::fs::read(kumikit_test_plugins::PROBE_WASM).unwrap();
    kkp(dirs, "probe-methods", &manifest, &[("plugin.wasm", &wasm)])
}

async fn code(c: &mut Client, method: &str, params: Value) -> String {
    let r = c.request(method, params).await;
    r["error"]["data"]["code"]
        .as_str()
        .unwrap_or("none")
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn declared_methods_are_routed_and_act_for_their_workspace() {
    within(async {
        let dirs = Dirs::new();
        let bin = std::path::Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
            .parent()
            .unwrap()
            .to_owned();
        let (_core, _) = Core::start_with_path(&dirs, Some(&bin)).await;
        let mut c = Client::hello(&dirs.address).await;
        let plugin = install(&mut c, &probe_with_methods(&dirs)).await;
        assert_eq!(
            plugin["contributes"]["methods"],
            json!(["echo", "cwd", "cwd-core", "fail"])
        );

        assert_eq!(
            c.call("test.probe/echo", json!({ "a": [1, 2] })).await,
            json!({ "a": [1, 2] })
        );
        assert_eq!(
            code(&mut c, "test.probe/undeclared", json!({})).await,
            "method_not_found"
        );
        assert_eq!(
            code(&mut c, "test.other/echo", json!({})).await,
            "method_not_found"
        );
        assert_eq!(
            code(&mut c, "test.probe/fail", json!({ "code": "not_found" })).await,
            "not_found"
        );

        // A workspace method needs a workspace, runs there, and knows which one.
        assert_eq!(
            code(&mut c, "test.probe/cwd", json!({})).await,
            "invalid_params"
        );
        assert_eq!(
            code(&mut c, "test.probe/cwd", json!({ "workspace": "nope" })).await,
            "not_found"
        );
        let ws = c
            .call("workspace.open", json!({ "path": dirs.workspace() }))
            .await["workspace"]
            .clone();
        let r = c
            .call("test.probe/cwd", json!({ "workspace": ws["id"] }))
            .await;
        assert_eq!(r["workspace"], ws["id"]);
        assert_eq!(
            std::fs::canonicalize(r["cwd"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(ws["root"].as_str().unwrap()).unwrap()
        );
        // A core method acts for no workspace, so it cannot spawn.
        assert_eq!(
            code(&mut c, "test.probe/cwd-core", json!({})).await,
            "permission_denied"
        );
        // A closed workspace is refused.
        c.call("workspace.close", json!({ "workspace": ws["id"] }))
            .await;
        assert_eq!(
            code(&mut c, "test.probe/cwd", json!({ "workspace": ws["id"] })).await,
            "conflict"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_are_listed_and_run() {
    within(async {
        let dirs = Dirs::new();
        let (_core, _) = Core::start_with_path(&dirs, None).await;
        let mut c = Client::hello(&dirs.address).await;
        assert_eq!(c.call("command.list", json!({})).await, json!({ "commands": [] }));
        install(&mut c, &probe_with_methods(&dirs)).await;
        assert_eq!(
            c.call("command.list", json!({})).await["commands"],
            json!([
                { "plugin": "test.probe", "id": "greet", "title": "Greet", "scope": "core" },
                { "plugin": "test.probe", "id": "greet-submit", "title": "Submit", "scope": "workspace", "when": "never" }
            ])
        );
        let r = c.call("command.run", json!({ "plugin": "test.probe", "command": "greet" })).await;
        assert_eq!(r["message"], "hello from the probe");
        assert_eq!(r["view"]["model"], "form");
        assert_eq!(r["view"]["submit"], "greet-submit");

        let ws = c.call("workspace.open", json!({ "path": dirs.workspace() })).await["workspace"].clone();
        let r = c
            .call(
                "command.run",
                json!({ "plugin": "test.probe", "command": "greet-submit", "args": { "name": "Ada" }, "workspace": ws["id"] }),
            )
            .await;
        assert_eq!(r, json!({ "message": "submitted {\"name\":\"Ada\"}" }));
        assert_eq!(
            code(&mut c, "command.run", json!({ "plugin": "test.probe", "command": "greet-submit" })).await,
            "invalid_params"
        );
        assert_eq!(
            code(&mut c, "command.run", json!({ "plugin": "test.probe", "command": "nope" })).await,
            "not_found"
        );
        assert_eq!(
            code(&mut c, "command.run", json!({ "plugin": "test.none", "command": "greet" })).await,
            "not_found"
        );
    })
    .await;
}
