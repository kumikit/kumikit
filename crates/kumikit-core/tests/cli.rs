//! `kumikit plugin add/list/remove/pack`, against a running core and on a stopped core's
//! data directory. Stdin is never a terminal here, so nothing is granted without `--yes`.

// Integration-test crate: clippy's allow-*-in-tests does not cover helper functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::process::Stdio;

use serde_json::json;
use support::{Client, Core, DEADLINE, Dirs, PROBE, PROBE_MANIFEST, probe_kkp};

async fn within<F: Future<Output = ()>>(f: F) {
    tokio::time::timeout(DEADLINE, f)
        .await
        .expect("test exceeded its deadline");
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

async fn kumikit(dirs: &Dirs, args: &[&str]) -> Run {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
        .arg("plugin")
        .args(args)
        .arg("--socket")
        .arg(&dirs.address)
        .arg("--data-dir")
        .arg(dirs.data())
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    Run {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn add_on_a_stopped_core_then_serve_offers_the_provider() {
    within(async {
        let dirs = Dirs::new();
        let file = probe_kkp(&dirs);
        let r = kumikit(&dirs, &["add", file.to_str().unwrap(), "--yes"]).await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);
        assert!(r.stdout.contains("not signed"), "{}", r.stdout);
        assert!(
            r.stdout.contains(
                "can start the `fake-acp-agent` program, which runs with your full user access"
            ),
            "{}",
            r.stdout
        );
        assert!(
            r.stdout.contains(&format!("Provider: {PROBE}")),
            "{}",
            r.stdout
        );

        let r = kumikit(&dirs, &["list"]).await;
        assert!(
            r.stdout.contains("test.probe 0.1.0  enabled"),
            "{}",
            r.stdout
        );

        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        let hello = c
            .call(
                "core.hello",
                json!({"protocol": "0.5", "client": {"name": "t", "version": "0"}}),
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
async fn add_on_a_running_core_is_announced() {
    within(async {
        let dirs = Dirs::new();
        let core = Core::start(&dirs).await;
        let mut watcher = Client::hello(&dirs.address).await;
        let file = probe_kkp(&dirs);
        let r = kumikit(&dirs, &["add", file.to_str().unwrap(), "--yes"]).await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);
        let note = watcher.notification("core.pluginsChanged").await;
        assert_eq!(
            note["plugins"][0]["contributes"]["providers"],
            json!([PROBE])
        );
        let r = kumikit(&dirs, &["list"]).await;
        assert!(
            r.stdout.contains("provider test.probe/probe"),
            "{}",
            r.stdout
        );
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_terminal_or_yes_nothing_is_granted_until_asked_again() {
    within(async {
        let dirs = Dirs::new();
        let file = probe_kkp(&dirs);
        let r = kumikit(&dirs, &["add", file.to_str().unwrap()]).await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);
        assert!(
            r.stdout.contains("installed but not enabled"),
            "{}",
            r.stdout
        );
        let r = kumikit(&dirs, &["list"]).await;
        assert!(r.stdout.contains("installed (not enabled)"), "{}", r.stdout);

        // Adding again finishes the grant.
        let r = kumikit(&dirs, &["add", file.to_str().unwrap(), "--yes"]).await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);
        assert!(r.stdout.contains("Enabled test.probe"), "{}", r.stdout);
        let r = kumikit(&dirs, &["add", file.to_str().unwrap(), "--yes"]).await;
        assert!(
            r.stdout.contains("already installed and enabled"),
            "{}",
            r.stdout
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_keeps_data_unless_told_to_delete_it() {
    within(async {
        let dirs = Dirs::new();
        let file = probe_kkp(&dirs);
        let data = dirs.data().join("plugin-data").join("test.probe");
        kumikit(&dirs, &["add", file.to_str().unwrap(), "--yes"]).await;
        std::fs::create_dir_all(&data).unwrap();
        let r = kumikit(&dirs, &["remove", "test.probe"]).await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);
        assert!(r.stdout.contains("data was kept"), "{}", r.stdout);
        assert!(data.exists());

        kumikit(&dirs, &["add", file.to_str().unwrap(), "--yes"]).await;
        let r = kumikit(&dirs, &["remove", "test.probe", "--yes"]).await;
        assert!(r.stdout.contains("and its data"), "{}", r.stdout);
        assert!(!data.exists());
        let r = kumikit(&dirs, &["list"]).await;
        assert!(
            r.stdout.contains("No plugins are installed"),
            "{}",
            r.stdout
        );
        let r = kumikit(&dirs, &["remove", "test.probe"]).await;
        assert!(!r.ok, "removing a missing plugin fails");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pack_builds_an_installable_package() {
    within(async {
        let dirs = Dirs::new();
        let src = dirs.root().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("plugin.toml"), PROBE_MANIFEST).unwrap();
        std::fs::copy(kumikit_test_plugins::PROBE_WASM, src.join("plugin.wasm")).unwrap();
        let out = dirs.root().join("probe.kkp");
        let r = kumikit(
            &dirs,
            &["pack", src.to_str().unwrap(), "-o", out.to_str().unwrap()],
        )
        .await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);
        let r = kumikit(&dirs, &["add", out.to_str().unwrap(), "--yes"]).await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);

        std::fs::remove_file(src.join("plugin.wasm")).unwrap();
        let r = kumikit(&dirs, &["pack", src.to_str().unwrap()]).await;
        assert!(!r.ok);
        assert!(r.stderr.contains("not in the package"), "{}", r.stderr);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_package_says_who_signed_it_and_the_key_never_shows() {
    within(async {
        let dirs = Dirs::new();
        let key = dirs.root().join("keys").join("acme.key");
        let r = kumikit(&dirs, &["keygen", "--out", key.to_str().unwrap()]).await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);
        let public = r
            .stdout
            .lines()
            .find_map(|l| l.strip_prefix("Public key: "))
            .unwrap()
            .to_owned();
        let pem = std::fs::read_to_string(&key).unwrap();
        let secret = pem.lines().nth(1).unwrap().to_owned();

        let src = dirs.root().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("plugin.toml"), PROBE_MANIFEST).unwrap();
        std::fs::copy(kumikit_test_plugins::PROBE_WASM, src.join("plugin.wasm")).unwrap();
        let out = dirs.root().join("probe.kkp");

        // The key from the environment, with every log on: it never appears.
        let run = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
            .args([
                "plugin",
                "pack",
                src.to_str().unwrap(),
                "-o",
                out.to_str().unwrap(),
                "--sign",
            ])
            .env("KUMIKIT_SIGNING_KEY", &pem)
            .env("KUMIKIT_LOG", "trace")
            .stdin(Stdio::null())
            .output()
            .await
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(run.status.success(), "{text}");
        assert!(text.contains(&public), "{text}");
        assert!(!text.contains(&secret), "the private key was printed");

        let r = kumikit(&dirs, &["verify", out.to_str().unwrap(), "--key", &public]).await;
        assert!(
            r.ok && r.stdout.contains("Signed by"),
            "{}{}",
            r.stdout,
            r.stderr
        );

        let r = kumikit(&dirs, &["add", out.to_str().unwrap(), "--yes"]).await;
        assert!(r.ok, "{}{}", r.stdout, r.stderr);
        assert!(r.stdout.contains("Signed by key ed25519:"), "{}", r.stdout);
        assert!(
            r.stdout.contains("not checked against a registry"),
            "{}",
            r.stdout
        );
        assert!(!r.stdout.contains("not signed"), "{}", r.stdout);

        let core = Core::start(&dirs).await;
        let mut c = Client::hello(&dirs.address).await;
        let list = c.call("plugin.list", json!({})).await;
        assert_eq!(list["plugins"][0]["signed"], true, "{list}");
        core.stop().await;

        // A key file others can read is refused (Unix).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
            let r = kumikit(
                &dirs,
                &[
                    "sign",
                    out.to_str().unwrap(),
                    "--key",
                    key.to_str().unwrap(),
                ],
            )
            .await;
            assert!(!r.ok && r.stderr.contains("chmod 600"), "{}", r.stderr);
        }
    })
    .await;
}

async fn kumikit_raw(args: &[&str]) -> Run {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    Run {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_needs_a_running_core_with_remote_access() {
    within(async {
        let dirs = Dirs::new();
        let r = kumikit_raw(&["pair", "--socket", &dirs.address]).await;
        assert!(!r.ok);
        assert!(r.stderr.contains("no core is running"), "{}", r.stderr);

        let core = Core::start(&dirs).await;
        let r = kumikit_raw(&["pair", "--socket", &dirs.address]).await;
        assert!(!r.ok);
        assert!(
            r.stderr.contains("start the core with --listen"),
            "{}",
            r.stderr
        );
        core.stop().await;

        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let r = kumikit_raw(&["pair", "--socket", &dirs.address]).await;
        assert!(r.ok, "{}", r.stderr);
        assert!(
            r.stdout
                .contains(&format!("Fingerprint:  {}", info.fingerprint)),
            "{}",
            r.stdout
        );
        assert!(
            r.stdout.contains(&format!("Address:      {}", info.url)),
            "{}",
            r.stdout
        );
        let r = kumikit_raw(&["pair", "--socket", &dirs.address, "--json"]).await;
        let code: serde_json::Value = serde_json::from_str(r.stdout.trim()).unwrap();
        assert_eq!(code["fingerprint"], info.fingerprint.as_str());
        assert_eq!(code["code"].as_str().unwrap().len(), 9);
        core.stop().await;
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_are_listed_and_revoked_running_or_not() {
    within(async {
        let dirs = Dirs::new();
        let (core, info) = Core::start_remote(&dirs, &[], false).await;
        let data = dirs.data();
        let data = data.to_str().unwrap();
        let devices = |args: &[&str]| {
            let mut all = vec!["devices"];
            all.extend_from_slice(args);
            all.extend_from_slice(&["--socket", &dirs.address, "--data-dir", data]);
            all.into_iter().map(str::to_owned).collect::<Vec<_>>()
        };
        let run = |args: Vec<String>| async move {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            kumikit_raw(&refs).await
        };
        let r = run(devices(&["list"])).await;
        assert!(
            r.stdout.contains("No devices are paired."),
            "{}{}",
            r.stdout,
            r.stderr
        );
        let id = support_pair(&dirs, &info).await;
        let r = run(devices(&["list"])).await;
        assert!(
            r.stdout.contains(&id) && r.stdout.contains("cli-test"),
            "{}",
            r.stdout
        );
        core.stop().await;

        // Offline, on the data directory.
        let r = run(devices(&["list"])).await;
        assert!(r.stdout.contains(&id), "{}{}", r.stdout, r.stderr);
        let r = run(devices(&["revoke", &id])).await;
        assert!(r.ok, "{}", r.stderr);
        let r = run(devices(&["list"])).await;
        assert!(r.stdout.contains("No devices are paired."), "{}", r.stdout);
        let r = run(devices(&["revoke", &id])).await;
        assert!(!r.ok);
        assert!(r.stderr.contains("no device"), "{}", r.stderr);
    })
    .await;
}

/// Pairs a device named `cli-test`: `kumikit pair --json` for the code, then `auth.pair`
/// over the remote transport.
async fn support_pair(dirs: &Dirs, info: &support::RemoteInfo) -> String {
    let r = kumikit_raw(&["pair", "--socket", &dirs.address, "--json"]).await;
    let code: serde_json::Value = serde_json::from_str(r.stdout.trim()).unwrap();
    let res = support::remote_call_once(
        info,
        "auth.pair",
        json!({"code": code["code"], "deviceName": "cli-test"}),
    )
    .await;
    res["result"]["device"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_copies_lines_to_the_local_core() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    within(async {
        let dirs = Dirs::new();
        let r = kumikit_raw(&["bridge", "--socket", &dirs.address]).await;
        assert!(!r.ok);
        assert!(r.stderr.contains("no core is running on this machine"), "{}", r.stderr);

        let core = Core::start(&dirs).await;
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_kumikit"))
            .args(["bridge", "--socket", &dirs.address])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"core.hello\",\"params\":{\"protocol\":\"0.5\",\"client\":{\"name\":\"t\",\"version\":\"0\"}}}\n\n")
            .await
            .unwrap();
        let hello: serde_json::Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(hello["result"]["protocol"], "0.5");
        // A local connection: no devices capability without --listen, auth not needed.
        stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"auth.token\",\"params\":{\"token\":\"x\"}}\n")
            .await
            .unwrap();
        let r: serde_json::Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(r["error"]["data"]["code"], "unsupported");
        drop(stdin);
        assert!(child.wait().await.unwrap().success());
        core.stop().await;
    })
    .await;
}
