//! Core plugins from a registry (10-distribution, Install, Updates, Lockfiles): a fixture
//! registry on loopback, signed with keys made here, and a real core.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use std::path::PathBuf;

use kumikit_plugin_host::package::SigningKey;
use kumikit_test_registry_fixture::{Fixture, Server, serve};
use serde_json::{Value, json};
use support::{Client, Core, DEADLINE, Dirs};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

/// The probe test plugin as `acme.probe`, at `version`, asking for `extra` permissions
/// lines on top of its `spawn`.
fn probe_src(root: &std::path::Path, version: &str, extra: &str) -> PathBuf {
    let dir = root.join(format!("src-{version}-{}", extra.len()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        format!(
            "id = \"acme.probe\"\nname = \"Probe\"\nversion = \"{version}\"\nkind = [\"provider\"]\n\
             api = \"^0.2\"\nrepository = \"https://example.com/acme/probe\"\n\
             [runtime]\nwasm = \"plugin.wasm\"\nworlds = [\"kumikit:provider/plugin@0.2.0\"]\n\
             [permissions]\nspawn = [\"fake-acp-agent\"]\n{extra}\n\
             [contributes]\nproviders = [{{ id = \"probe\", name = \"Probe\" }}]\n"
        ),
    )
    .unwrap();
    std::fs::copy(kumikit_test_plugins::PROBE_WASM, dir.join("plugin.wasm")).unwrap();
    dir
}

struct Registry {
    fixture: Fixture,
    server: Server,
    key: SigningKey,
    root: PathBuf,
}

async fn registry(dirs: &Dirs, name: &str) -> Registry {
    let root = dirs.root().join(name);
    let www = root.join("www");
    std::fs::create_dir_all(&www).unwrap();
    let server = serve(www).await;
    let fixture = Fixture::new(&root, &server.url);
    let (key, _) = SigningKey::generate().unwrap();
    fixture.publisher("acme", &key, &["acme"]);
    fixture.publish(&probe_src(&root, "0.1.0", ""), Some(&key));
    fixture.starter(&["acme.probe"]);
    fixture.build_index();
    Registry {
        fixture,
        server,
        key,
        root,
    }
}

async fn core(dirs: &Dirs, r: &Registry) -> Core {
    let url = r.fixture.index_url();
    let agent = std::path::Path::new(kumikit_test_plugins::FAKE_ACP_AGENT)
        .parent()
        .unwrap()
        .to_owned();
    Core::start_with_env(
        dirs,
        Some(&agent),
        &[],
        &[("KUMIKIT_DEFAULT_REGISTRY", &url)],
    )
    .await
    .0
}

fn code(r: &Value) -> &str {
    r["error"]["data"]["code"].as_str().unwrap_or("")
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_makes_no_registry_request() {
    within(async {
        let dirs = Dirs::new();
        let r = registry(&dirs, "reg").await;
        let core = core(&dirs, &r).await;
        let mut c = Client::hello(&dirs.address).await;
        let hello = c
            .call(
                "core.hello",
                json!({"protocol":"0.13", "client": {"name": "t", "version": "0"}}),
            )
            .await;
        assert!(
            hello["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("registry"))
        );
        c.call("plugin.list", json!({})).await;
        let list = c.call("registry.list", json!({})).await;
        assert_eq!(list["registries"][0]["url"], r.fixture.index_url());
        assert_eq!(
            r.server.hits(),
            0,
            "nothing was fetched without being asked"
        );
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn install_by_id_checks_and_records_the_publisher() {
    within(async {
        let dirs = Dirs::new();
        let r = registry(&dirs, "reg").await;
        let core = core(&dirs, &r).await;
        let mut c = Client::hello(&dirs.address).await;

        let found = c.call("registry.search", json!({"query": "probe"})).await;
        let hit = &found["results"][0];
        assert_eq!(hit["id"], "acme.probe");
        assert_eq!(hit["key"], r.key.public().to_string());
        assert_eq!(hit["starter"], true);
        assert_eq!(hit["requested"]["spawn"], json!(["fake-acp-agent"]));

        let installed = c
            .call("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;
        let p = &installed["plugin"];
        assert_eq!(p["signed"], true);
        assert_eq!(p["source"], "registry");
        assert_eq!(p["publisher"]["key"], r.key.public().to_string());
        assert_eq!(p["publisher"]["registry"], r.fixture.index_url());
        c.call(
            "plugin.enable",
            json!({"plugin": "acme.probe", "grant": p["requested"]}),
        )
        .await;

        let lock = c.call("plugin.lock", json!({})).await["lock"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(lock.contains("id = \"acme.probe\""), "{lock}");
        assert!(lock.contains(&r.key.public().to_string()), "{lock}");
        assert!(lock.contains("state = \"enabled\""), "{lock}");
        let on_disk = std::fs::read_to_string(dirs.data().join("plugins.lock")).unwrap();
        assert_eq!(on_disk, lock);

        let again = c
            .request("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;
        assert_eq!(code(&again), "conflict");
        let missing = c
            .request(
                "plugin.install",
                json!({"registry": {"id": "acme.nothing"}}),
            )
            .await;
        assert_eq!(code(&missing), "not_found");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lying_or_tampered_registry_installs_nothing() {
    within(async {
        let dirs = Dirs::new();
        let r = registry(&dirs, "reg").await;
        let core = core(&dirs, &r).await;
        let mut c = Client::hello(&dirs.address).await;
        // The hash in the index does not match the download.
        r.fixture
            .edit_plugin("acme.probe", |f| f.versions[0].sha256 = "0".repeat(64));
        r.fixture.build_index();
        let e = c
            .request("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;
        assert_eq!(code(&e), "untrusted", "{e}");

        // The entry hides a permission the package asks for.
        r.fixture
            .publish(&probe_src(&r.root, "0.1.0", ""), Some(&r.key));
        r.fixture.edit_plugin("acme.probe", |f| {
            f.versions[0].permissions.as_mut().unwrap().spawn.clear();
        });
        r.fixture.build_index();
        let e = c
            .request("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;
        assert_eq!(code(&e), "untrusted", "{e}");
        assert!(
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("permissions"),
            "{e}"
        );

        // Signed by someone other than the publisher.
        let (other, _) = SigningKey::generate().unwrap();
        r.fixture
            .publish(&probe_src(&r.root, "0.1.0", ""), Some(&other));
        r.fixture.build_index();
        let e = c
            .request("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;
        assert_eq!(code(&e), "untrusted", "{e}");
        assert!(
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("publisher's key"),
            "{e}"
        );

        // Not signed at all.
        r.fixture.publish(&probe_src(&r.root, "0.1.0", ""), None);
        r.fixture.build_index();
        let e = c
            .request("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;
        assert_eq!(code(&e), "untrusted", "{e}");

        assert_eq!(c.call("plugin.list", json!({})).await["plugins"], json!([]));
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn updates_are_proposed_and_wider_ones_wait_for_a_grant() {
    within(async {
        let dirs = Dirs::new();
        let r = registry(&dirs, "reg").await;
        let core = core(&dirs, &r).await;
        let mut c = Client::hello(&dirs.address).await;
        let p = c
            .call("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;
        c.call(
            "plugin.enable",
            json!({"plugin": "acme.probe", "grant": p["plugin"]["requested"]}),
        )
        .await;
        // A same-permission update is offered, and nothing happens until it is applied.
        r.fixture.publish(&probe_src(&r.root, "0.2.0", ""), Some(&r.key));
        r.fixture.build_index();
        let updates = c.call("plugin.updates", json!({"refresh": true})).await;
        assert_eq!(
            updates["updates"],
            json!([{"plugin": "acme.probe", "from": "0.1.0", "to": "0.2.0",
                    "registry": r.fixture.index_url(),
                    "requested": {"spawn": ["fake-acp-agent"], "network": [], "files": "none", "secrets": [], "tools": false},
                    "widens": false}])
        );
        let list = c.call("plugin.list", json!({})).await;
        assert_eq!(list["plugins"][0]["version"], "0.1.0", "updates are never silent");
        let updated = c
            .call("plugin.update", json!({"plugin": "acme.probe", "version": "0.2.0"}))
            .await;
        assert_eq!(updated["plugin"]["version"], "0.2.0");
        assert_eq!(updated["plugin"]["state"], "enabled");
        assert!(!dirs.data().join("plugins/acme.probe/0.1.0").exists());

        // A wider update needs the new permissions granted.
        r.fixture.publish(
            &probe_src(&r.root, "0.3.0", "secrets = [\"acme-token\"]"),
            Some(&r.key),
        );
        r.fixture.build_index();
        let updates = c.call("plugin.updates", json!({"refresh": true})).await;
        assert_eq!(updates["updates"][0]["widens"], true);
        let requested = updates["updates"][0]["requested"].clone();
        let e = c
            .request("plugin.update", json!({"plugin": "acme.probe", "version": "0.3.0"}))
            .await;
        assert_eq!(code(&e), "conflict", "{e}");
        let list = c.call("plugin.list", json!({})).await;
        assert_eq!(list["plugins"][0]["version"], "0.2.0", "it stays on the old version");
        let updated = c
            .call(
                "plugin.update",
                json!({"plugin": "acme.probe", "version": "0.3.0", "grant": requested}),
            )
            .await;
        assert_eq!(updated["plugin"]["version"], "0.3.0");
        assert_eq!(updated["plugin"]["granted"]["secrets"], json!(["acme-token"]));

        // Never a lower version.
        let e = c
            .request("plugin.update", json!({"plugin": "acme.probe", "version": "0.2.0"}))
            .await;
        assert_eq!(code(&e), "conflict", "{e}");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_key_or_another_registry_cannot_take_over_updates() {
    within(async {
        let dirs = Dirs::new();
        let r = registry(&dirs, "reg").await;
        let core = core(&dirs, &r).await;
        let mut c = Client::hello(&dirs.address).await;
        c.call("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;

        // A second registry offers a higher version under its own key: not an update.
        let other = registry(&dirs, "other").await;
        other
            .fixture
            .publish(&probe_src(&other.root, "9.0.0", ""), Some(&other.key));
        other.fixture.build_index();
        c.call("registry.add", json!({"url": other.fixture.index_url()}))
            .await;
        let updates = c.call("plugin.updates", json!({"refresh": true})).await;
        assert_eq!(updates["updates"], json!([]), "{updates}");

        // The first registry swaps the publisher's key and offers 0.2.0 signed with it.
        let (new_key, _) = SigningKey::generate().unwrap();
        r.fixture.publisher("acme", &new_key, &["acme"]);
        r.fixture
            .publish(&probe_src(&r.root, "0.2.0", ""), Some(&new_key));
        r.fixture.build_index();
        let e = c
            .request(
                "plugin.update",
                json!({"plugin": "acme.probe", "version": "0.2.0"}),
            )
            .await;
        assert_eq!(code(&e), "untrusted", "{e}");
        assert!(
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("remove the plugin and install it again"),
            "{e}"
        );
        let list = c.call("plugin.list", json!({})).await;
        assert_eq!(list["plugins"][0]["version"], "0.1.0");
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_waits_for_a_running_turn() {
    within(async {
        let dirs = Dirs::new();
        let r = registry(&dirs, "reg").await;
        let core = core(&dirs, &r).await;
        let mut c = Client::hello(&dirs.address).await;
        let p = c
            .call("plugin.install", json!({"registry": {"id": "acme.probe"}}))
            .await;
        c.call(
            "plugin.enable",
            json!({"plugin": "acme.probe", "grant": p["plugin"]["requested"]}),
        )
        .await;
        let ws = c
            .call("workspace.open", json!({"path": dirs.workspace()}))
            .await["workspace"]["id"]
            .clone();
        let thread = c
            .call(
                "thread.create",
                json!({"workspace": ws, "provider": "acme.probe/probe", "model": "probe"}),
            )
            .await["thread"]["id"]
            .clone();
        c.call(
            "events.subscribe",
            json!({"topic": format!("thread:{}", thread.as_str().unwrap()), "after": 0}),
        )
        .await;
        c.call(
            "thread.send",
            json!({"thread": thread, "input": {"text": "hang"}}),
        )
        .await;
        c.events_until("turn.started").await;

        r.fixture
            .publish(&probe_src(&r.root, "0.2.0", ""), Some(&r.key));
        r.fixture.build_index();
        let e = c
            .request(
                "plugin.update",
                json!({"plugin": "acme.probe", "version": "0.2.0"}),
            )
            .await;
        assert_eq!(code(&e), "conflict", "{e}");
        assert!(
            e["error"]["message"].as_str().unwrap().contains("turn"),
            "{e}"
        );

        c.call("thread.cancel", json!({"thread": thread})).await;
        c.events_until("turn.ended").await;
        let updated = c
            .call(
                "plugin.update",
                json!({"plugin": "acme.probe", "version": "0.2.0"}),
            )
            .await;
        assert_eq!(updated["plugin"]["version"], "0.2.0");
        core.stop().await;
    })
    .await;
}

/// Runs `kumikit plugin <args>` against `dirs` (no core running: the CLI opens the data
/// directory), with the fixture as the default registry.
async fn cli(dirs: &Dirs, r: &Registry, args: &[&str]) -> (bool, String) {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
        .arg("plugin")
        .args(args)
        .arg("--socket")
        .arg(&dirs.address)
        .arg("--data-dir")
        .arg(dirs.data())
        .env("KUMIKIT_DEFAULT_REGISTRY", r.fixture.index_url())
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_adds_by_id_updates_and_moves_a_setup_with_a_lockfile() {
    within(async {
        let dirs = Dirs::new();
        let r = registry(&dirs, "reg").await;

        let (ok, out) = cli(&dirs, &r, &["search", "probe"]).await;
        assert!(ok && out.contains("acme.probe 0.1.0"), "{out}");
        assert!(
            out.contains("can start the `fake-acp-agent` program"),
            "{out}"
        );

        // Without a terminal and without --yes, nothing is granted.
        let (ok, out) = cli(&dirs, &r, &["add", "acme.probe"]).await;
        assert!(ok, "{out}");
        assert!(out.contains("the publisher's key in the registry"), "{out}");
        assert!(out.contains("installed but not enabled"), "{out}");
        let (ok, out) = cli(&dirs, &r, &["add", "acme.probe", "--yes"]).await;
        assert!(ok && out.contains("Enabled acme.probe"), "{out}");

        // A wider update: --yes alone does not apply it.
        r.fixture.publish(
            &probe_src(&r.root, "0.2.0", "secrets = [\"acme-token\"]"),
            Some(&r.key),
        );
        r.fixture.build_index();
        let (ok, out) = cli(&dirs, &r, &["update", "--yes"]).await;
        assert!(ok, "{out}");
        assert!(out.contains("acme.probe 0.1.0 → 0.2.0"), "{out}");
        assert!(out.contains("can read the secret `acme-token`"), "{out}");
        assert!(out.contains("Not updated"), "{out}");
        let (ok, out) = cli(&dirs, &r, &["update", "--allow-wider"]).await;
        assert!(ok && out.contains("Updated acme.probe to 0.2.0"), "{out}");
        let (_, out) = cli(&dirs, &r, &["update"]).await;
        assert!(out.contains("Everything is up to date"), "{out}");

        // Export, then import into another, fresh core.
        let lock = dirs.root().join("exported.lock");
        let (ok, out) = cli(&dirs, &r, &["lock", "export", "-o", lock.to_str().unwrap()]).await;
        assert!(ok, "{out}");
        let fresh = Dirs::new();
        let (ok, out) = cli(
            &fresh,
            &r,
            &["lock", "import", lock.to_str().unwrap(), "--yes"],
        )
        .await;
        assert!(ok, "{out}");
        assert!(out.contains("Installed acme.probe 0.2.0"), "{out}");
        assert!(out.contains("Enabled acme.probe"), "{out}");
        let (_, out) = cli(&fresh, &r, &["list"]).await;
        assert!(out.contains("acme.probe 0.2.0  enabled"), "{out}");

        // A lockfile whose hash does not match installs nothing.
        let text = std::fs::read_to_string(&lock).unwrap();
        let sha = text
            .lines()
            .find_map(|l| l.strip_prefix("sha256 = \""))
            .unwrap()
            .trim_end_matches('"')
            .to_owned();
        std::fs::write(&lock, text.replace(&sha, &"0".repeat(64))).unwrap();
        let other = Dirs::new();
        let (_, out) = cli(
            &other,
            &r,
            &["lock", "import", lock.to_str().unwrap(), "--yes"],
        )
        .await;
        assert!(
            out.contains("not installed") && out.contains("lockfile"),
            "{out}"
        );
        let (_, out) = cli(&other, &r, &["list"]).await;
        assert!(out.contains("No plugins are installed"), "{out}");

        // A registry not configured here is not added by an import.
        let elsewhere = text.replace(&r.fixture.index_url(), "http://127.0.0.1:9/index.json");
        std::fs::write(&lock, elsewhere).unwrap();
        let third = Dirs::new();
        let (_, out) = cli(
            &third,
            &r,
            &["lock", "import", lock.to_str().unwrap(), "--yes"],
        )
        .await;
        assert!(out.contains("is not configured here"), "{out}");
        let (_, out) = cli(&third, &r, &["registry", "list"]).await;
        assert!(!out.contains("127.0.0.1:9"), "{out}");
    })
    .await;
}
