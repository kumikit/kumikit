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
                json!({"protocol": "0.2", "client": {"name": "t", "version": "0"}}),
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
