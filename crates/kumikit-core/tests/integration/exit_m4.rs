//! M4's exit criteria (12-roadmap) on the core: a plugin published to the registry by a
//! third party installs on a fresh machine, with its permissions shown and enforced.
//!
//! Everything a third party and a registry do is simulated here, on loopback: a publisher
//! "acme" makes a key with `kumikit plugin keygen`, signs the probe test plugin with it
//! (`kumikit plugin pack --key`), and adds it to a fixture registry whose checker
//! (`kumikit-registry check`, the registry's CI) passes. A fresh core then installs it by
//! id with `kumikit plugin add`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use std::path::{Path, PathBuf};

use kumikit_registry::fetch::Fetcher;
use kumikit_registry_tool::{CheckOptions, check};
use kumikit_test_registry_fixture::{Fixture, serve};
use serde_json::json;
use support::{Client, Core, DEADLINE, Dirs};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

async fn kumikit(dirs: &Dirs, index: &str, args: &[&str]) -> (bool, String) {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
        .args(args)
        .arg("--socket")
        .arg(&dirs.address)
        .arg("--data-dir")
        .arg(dirs.data())
        .env("KUMIKIT_DEFAULT_REGISTRY", index)
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

/// The publisher's plugin source: the probe component with acme's manifest.
fn source(root: &Path, version: &str, extra: &str) -> PathBuf {
    let dir = root.join(format!("acme-probe-{version}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        format!(
            "id = \"acme.probe\"\nname = \"Acme Probe\"\nversion = \"{version}\"\n\
             description = \"A third-party provider.\"\nlicense = \"MIT\"\n\
             repository = \"https://example.com/acme/probe\"\nkind = [\"provider\"]\napi = \"^0.2\"\n\
             [runtime]\nwasm = \"plugin.wasm\"\nworlds = [\"kumikit:provider/plugin@0.2.0\"]\n\
             [permissions]\nspawn = [\"fake-acp-agent\"]\n{extra}\n\
             [contributes]\nproviders = [{{ id = \"probe\", name = \"Acme Probe\" }}]\n"
        ),
    )
    .unwrap();
    std::fs::copy(kumikit_test_plugins::PROBE_WASM, dir.join("plugin.wasm")).unwrap();
    dir
}

/// Sends a message to the probe and returns its `turn.ended` data.
async fn turn(client: &mut Client, thread: &str, text: &str) -> serde_json::Value {
    client
        .call(
            "thread.send",
            json!({"thread": thread, "input": {"text": text}}),
        )
        .await;
    let events = client.events_until("turn.ended").await;
    events.last().unwrap()["data"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_third_party_plugin_installs_by_id_with_its_permissions_shown_and_enforced() {
    within(async {
        let publisher = Dirs::new();
        let root = publisher.root().to_owned();
        let www = root.join("www");
        std::fs::create_dir_all(&www).unwrap();
        let server = serve(www).await;
        let fixture = Fixture::new(&root, &server.url);
        let index = fixture.index_url();

        // 1. The publisher: a key from `kumikit plugin keygen`, a package signed with it.
        let key_file = root.join("keys").join("acme.key");
        let (ok, out) = kumikit(&publisher, &index, &["plugin", "keygen", "--out", key_file.to_str().unwrap()]).await;
        assert!(ok, "{out}");
        let key = kumikit_plugin_host::package::SigningKey::from_pem(
            &std::fs::read_to_string(&key_file).unwrap(),
        )
        .unwrap();
        fixture.publisher("acme", &key, &["acme-bot"]);
        let packed = root.join("acme.probe-0.1.0.kkp");
        let (ok, out) = kumikit(
            &publisher,
            &index,
            &[
                "plugin", "pack", source(&root, "0.1.0", "").to_str().unwrap(),
                "-o", packed.to_str().unwrap(),
                "--key", key_file.to_str().unwrap(),
            ],
        )
        .await;
        assert!(ok && out.contains(&key.public().to_string()), "{out}");
        // The registry entry is made from that package, as a publisher's pull request.
        let src = root.join("unpacked");
        std::fs::create_dir_all(&src).unwrap();
        kumikit_plugin_host::package::unpack(&packed, &src).unwrap();
        std::fs::remove_file(src.join("SIGNATURE")).unwrap();
        fixture.publish(&src, Some(&key));
        fixture.starter(&["acme.probe"]);
        // The registry's CI accepts it, then builds the index.
        let errors = check(
            &fixture.repo,
            None,
            &CheckOptions { author: Some("acme-bot".into()), require_early: true },
            &Fetcher::default(),
        )
        .await;
        assert!(errors.is_empty(), "{errors:#?}");
        fixture.build_index();
        let checked = server.hits();

        // 2. A fresh machine: a new data directory, the fixture as its registry. Starting
        // the core contacts nothing.
        let machine = Dirs::new();
        let agent = Path::new(kumikit_test_plugins::FAKE_ACP_AGENT).parent().unwrap().to_owned();
        let (core, _) = Core::start_with_env(
            &machine,
            Some(&agent),
            &[],
            &[("KUMIKIT_DEFAULT_REGISTRY", &index)],
        )
        .await;
        let mut client = Client::hello(&machine.address).await;
        assert_eq!(server.hits(), checked, "a fresh core contacts no registry by itself");

        // 3. Install it by id: who made it and what it may do are shown first.
        let (ok, out) = kumikit(&machine, &index, &["plugin", "add", "acme.probe", "--yes"]).await;
        assert!(ok, "{out}");
        for shown in [
            "Acme Probe 0.1.0 (acme.probe)",
            "Publisher: acme",
            "Repository: https://example.com/acme/probe",
            "the publisher's key in the registry",
            "can start the `fake-acp-agent` program, which runs with your full user access",
            "Enabled acme.probe.",
        ] {
            assert!(out.contains(shown), "missing {shown:?} in:\n{out}");
        }
        let list = client.call("plugin.list", json!({})).await;
        let p = &list["plugins"][0];
        assert_eq!(p["id"], "acme.probe");
        assert_eq!(p["signed"], true);
        assert_eq!(p["publisher"]["key"], key.public().to_string());
        assert_eq!(p["source"], "registry");

        // 4. Enforced: what was granted works; anything else is permission denied.
        let ws = client.call("workspace.open", json!({"path": machine.workspace()})).await;
        let thread = client
            .call(
                "thread.create",
                json!({"workspace": ws["workspace"]["id"], "provider": "acme.probe/probe", "model": "probe"}),
            )
            .await["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        client
            .call("events.subscribe", json!({"topic": format!("thread:{thread}"), "after": 0}))
            .await;
        let granted = turn(&mut client, &thread, "run env").await;
        assert_eq!(granted["outcome"], "done", "{granted}");
        let secret = turn(&mut client, &thread, "secret anthropic-api-key").await;
        assert_eq!(secret["outcome"], "failed", "{secret}");
        let error = secret["error"].as_str().unwrap();
        assert_eq!(
            error, "permission_denied: secret `anthropic-api-key` is not in permissions.secrets",
            "a host call outside the grant fails with permission_denied"
        );
        let shell = turn(&mut client, &thread, "spawn sh").await;
        assert_eq!(shell["outcome"], "failed", "{shell}");
        assert!(
            shell["error"].as_str().unwrap().starts_with("permission_denied: "),
            "{shell}"
        );

        // 5. The lockfile records it.
        let lock = client.call("plugin.lock", json!({})).await["lock"].as_str().unwrap().to_owned();
        for part in ["id = \"acme.probe\"", "version = \"0.1.0\"", &key.public().to_string(), "fake-acp-agent"] {
            assert!(lock.contains(part), "missing {part} in:\n{lock}");
        }

        // 6. Later versions: another key is refused; wider permissions wait for approval.
        let (other, _) = kumikit_plugin_host::package::SigningKey::generate().unwrap();
        fixture.publish(&source(&root, "0.2.0", ""), Some(&other));
        fixture.build_index();
        let (_, out) = kumikit(&machine, &index, &["plugin", "update", "--yes"]).await;
        assert!(out.contains("Not updated") && out.contains("not by the publisher's key"), "{out}");
        fixture.publish(&source(&root, "0.2.0", "secrets = [\"acme-token\"]"), Some(&key));
        fixture.build_index();
        let (_, out) = kumikit(&machine, &index, &["plugin", "update", "--yes"]).await;
        assert!(out.contains("asks for more permissions") && out.contains("Not updated"), "{out}");
        let list = client.call("plugin.list", json!({})).await;
        assert_eq!(list["plugins"][0]["version"], "0.1.0", "it stays on the approved version");

        core.stop().await;
    })
    .await;
}
