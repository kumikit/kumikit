//! Footprint budgets (`docs/spec/00-overview.md#footprint-budgets`) for the core:
//! cold start to accepting connections ≤ 300 ms, idle resident memory ≤ 30 MB, and
//! activating a typical WASM plugin ≤ 50 ms and ≤ 16 MB, and a core binary ≤ 32 MiB
//! (ADR 0012). Three phases, each the median of
//! five runs from a fresh data directory:
//!
//! 1. no plugins: cold start and idle memory;
//! 2. provider-acp and a manifest-only ACP plugin installed and enabled (offline, before
//!    `serve` starts): the same two budgets, so enabled but unused plugins cost nothing;
//! 3. activation: `thread.create` on the delegated provider loads provider-acp's
//!    control instance from the compile cache and calls `activate` and `list-models`
//!    (no agent is started); its round trip and the memory it adds;
//! 4. remote access on (`--listen 127.0.0.1:0`, M3): the first start, which generates the
//!    certificate, is reported; a later start and the idle memory with one paired client
//!    connected and past `core.hello` are held to the same budgets as phase 1;
//! 5. registries (M4): after a `registry.search` and an install by id from a fixture
//!    registry on loopback, the idle memory is held to the same budget. Enabling the
//!    plugin afterwards compiles its component inside the running core; the idle memory
//!    after that is reported, not gated (see the M4 plan's notes: it comes from compiling
//!    in-process, which predates M4).
//!
//! A measurement, not a correctness test: it is `#[ignore]`d and meant for a release
//! build. Run it with `sh scripts/footprint.sh`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

mod support;

use std::time::Duration;

use serde_json::json;
use support::{Client, Core, Dirs, kkp, plugin_cli, provider_acp_kkp};

const RUNS: usize = 5;
const START_BUDGET: Duration = Duration::from_millis(300);
const RSS_BUDGET_KB: u64 = 30 * 1024;
const ACTIVATE_BUDGET: Duration = Duration::from_millis(50);
const ACTIVATE_RSS_BUDGET_KB: u64 = 16 * 1024;
const BINARY_BUDGET_BYTES: u64 = 32 * 1024 * 1024;

/// A manifest-only plugin served by provider-acp. Its agent is never started here.
const DECLARATIVE: &str = r#"
id = "test.footprint"
name = "Footprint"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "agent", name = "Agent", acp = { command = "fake-acp-agent" } }]
"#;

async fn install_plugins(dirs: &Dirs) {
    for file in [
        provider_acp_kkp(dirs),
        kkp(dirs, "footprint", DECLARATIVE, &[]),
    ] {
        let (ok, out, err) = plugin_cli(dirs, &["add", file.to_str().unwrap(), "--yes"]).await;
        assert!(ok, "{out}{err}");
    }
}

/// The kernel's memory summary of a process (Linux only), for explaining the numbers:
/// which kind of memory grew. Returns `(field, KiB)` pairs.
fn smaps(pid: u32) -> Vec<(String, u64)> {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let kb = v.split_whitespace().next()?.parse().ok()?;
            Some((k.trim().to_owned(), kb))
        })
        .collect()
}

fn mb(kb: u64) -> f64 {
    kb as f64 / 1024.0
}

struct Sample {
    start: Duration,
    idle_kb: Option<u64>,
    activate: Option<Duration>,
    activate_kb: Option<u64>,
}

async fn sample(with_plugins: bool) -> Sample {
    let dirs = Dirs::new(); // fresh data directory: a cold start with migrations
    if with_plugins {
        install_plugins(&dirs).await;
    }
    let (core, start) = Core::start_timed(&dirs).await;
    let mut client = Client::hello(&dirs.address).await;
    // Let startup work settle before sampling idle memory.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let idle_kb = rss_kb(core.pid());
    let idle_smaps = smaps(core.pid());
    let (mut activate, mut activate_kb) = (None, None);
    if with_plugins {
        let ws = client
            .call("workspace.open", json!({"path": dirs.workspace()}))
            .await;
        let t = std::time::Instant::now();
        client
            .call(
                "thread.create",
                json!({"workspace": ws["workspace"]["id"], "provider": "test.footprint/agent"}),
            )
            .await;
        activate = Some(t.elapsed());
        tokio::time::sleep(Duration::from_secs(1)).await;
        activate_kb = rss_kb(core.pid());
        // Which kinds of memory the activation added (Linux).
        let after = smaps(core.pid());
        let grew: Vec<String> = after
            .iter()
            .filter_map(|(k, v)| {
                let before = idle_smaps.iter().find(|(bk, _)| bk == k)?.1;
                let d = v.saturating_sub(before);
                (d >= 64).then(|| format!("{k} +{:.1} MB", mb(d)))
            })
            .collect();
        if !grew.is_empty() {
            println!("  activation added: {}", grew.join(", "));
        }
    }
    drop(client);
    core.stop().await;
    Sample {
        start,
        idle_kb,
        activate,
        activate_kb,
    }
}

fn check_start_and_idle(label: &str, samples: &[Sample]) -> Option<u64> {
    let start = median(samples.iter().map(|s| s.start).collect());
    println!(
        "{label}: cold start to ready (median of {RUNS}): {} ms",
        start.as_millis()
    );
    assert!(
        start <= START_BUDGET,
        "{label}: cold start {start:?} exceeds {START_BUDGET:?}"
    );
    let idle: Vec<u64> = samples.iter().filter_map(|s| s.idle_kb).collect();
    if idle.is_empty() {
        println!("{label}: idle RSS not measured on this platform");
        return None;
    }
    let kb = median(idle);
    println!("{label}: idle RSS (median of {RUNS}): {:.1} MB", mb(kb));
    assert!(
        kb <= RSS_BUDGET_KB,
        "{label}: idle RSS {kb} KiB exceeds {RSS_BUDGET_KB} KiB"
    );
    Some(kb)
}

/// Resident memory of a process in KiB, where the platform makes it easy to read.
fn rss_kb(pid: u32) -> Option<u64> {
    if cfg!(target_os = "linux") {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
        line.split_whitespace().nth(1)?.parse().ok()
    } else if cfg!(target_os = "macos") {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8(out.stdout).ok()?.trim().parse().ok()
    } else {
        None
    }
}

fn median<T: Ord + Copy>(mut v: Vec<T>) -> T {
    v.sort_unstable();
    v[v.len() / 2]
}

/// Phase 4: returns the first start (with certificate generation) and a sample of a later
/// start with one paired, authenticated remote client.
async fn sample_remote() -> (Duration, Sample) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let dirs = Dirs::new();
    let t = std::time::Instant::now();
    let (core, _) = Core::start_remote(&dirs, &[], false).await;
    let first = t.elapsed();
    core.stop().await;
    let t = std::time::Instant::now();
    let (core, info) = Core::start_remote(&dirs, &[], false).await;
    let start = t.elapsed();
    let mut local = Client::hello(&dirs.address).await;
    let code = local.call("auth.pairing.create", json!({})).await;
    let paired = support::remote_call_once(
        &info,
        "auth.pair",
        json!({"code": code["code"], "deviceName": "footprint"}),
    )
    .await;
    let token = paired["result"]["token"].as_str().unwrap().to_owned();
    let mut ws = support::ws(&info).await;
    for (id, method, params) in [
        (1, "auth.token", json!({"token": token})),
        (
            2,
            "core.hello",
            json!({"protocol":"0.6", "client": {"name": "footprint", "version": "0"}}),
        ),
    ] {
        ws.send(Message::text(
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        ))
        .await
        .unwrap();
        loop {
            if let Some(Ok(Message::Text(t))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
                assert!(v.get("result").is_some(), "{method}: {v}");
                break;
            }
        }
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let idle_kb = rss_kb(core.pid());
    drop(ws);
    drop(local);
    core.stop().await;
    (
        first,
        Sample {
            start,
            idle_kb,
            activate: None,
            activate_kb: None,
        },
    )
}

/// Phase 5: idle memory after the core searched a registry and installed a plugin from
/// it, then after it enabled (compiled) that plugin.
async fn sample_registry() -> (Option<u64>, Option<u64>) {
    let dirs = Dirs::new();
    let root = dirs.root().join("reg");
    let www = root.join("www");
    std::fs::create_dir_all(&www).unwrap();
    let server = kumikit_test_registry_fixture::serve(www).await;
    let fixture = kumikit_test_registry_fixture::Fixture::new(&root, &server.url);
    let (key, _) = kumikit_plugin_host::package::SigningKey::generate().unwrap();
    fixture.publisher("acme", &key, &["acme"]);
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("plugin.toml"),
        "id = \"acme.probe\"\nname = \"Probe\"\nversion = \"0.1.0\"\nkind = [\"provider\"]\napi = \"^0.2\"\n\
         [runtime]\nwasm = \"plugin.wasm\"\nworlds = [\"kumikit:provider/plugin@0.2.0\"]\n\
         [contributes]\nproviders = [{ id = \"probe\", name = \"Probe\" }]\n",
    )
    .unwrap();
    std::fs::copy(kumikit_test_plugins::PROBE_WASM, src.join("plugin.wasm")).unwrap();
    fixture.publish(&src, Some(&key));
    fixture.build_index();
    let index = fixture.index_url();
    let (core, _) =
        Core::start_with_env(&dirs, None, &[], &[("KUMIKIT_DEFAULT_REGISTRY", &index)]).await;
    let mut c = Client::hello(&dirs.address).await;
    c.call("registry.search", json!({"query": "probe"})).await;
    let p = c
        .call("plugin.install", json!({"registry": {"id": "acme.probe"}}))
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let installed = rss_kb(core.pid());
    c.call(
        "plugin.enable",
        json!({"plugin": "acme.probe", "grant": p["plugin"]["requested"]}),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let enabled = rss_kb(core.pid());
    drop(c);
    core.stop().await;
    (installed, enabled)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "measurement; run with scripts/footprint.sh on a release build"]
async fn core_footprint_is_within_budget() {
    if cfg!(debug_assertions) {
        println!("warning: debug build; budgets apply to release builds");
    }
    let mut bare = Vec::new();
    for _ in 0..RUNS {
        bare.push(sample(false).await);
    }
    check_start_and_idle("no plugins", &bare);

    let mut plugged = Vec::new();
    for _ in 0..RUNS {
        plugged.push(sample(true).await);
    }
    check_start_and_idle("plugins enabled", &plugged);

    let activate = median(plugged.iter().filter_map(|s| s.activate).collect());
    println!(
        "activating provider-acp (thread.create, median of {RUNS}): {:.1} ms",
        activate.as_secs_f64() * 1000.0
    );
    assert!(
        activate <= ACTIVATE_BUDGET,
        "activation {activate:?} exceeds {ACTIVATE_BUDGET:?}"
    );
    let added: Vec<u64> = plugged
        .iter()
        .filter_map(|s| Some(s.activate_kb?.saturating_sub(s.idle_kb?)))
        .collect();
    if !added.is_empty() {
        let kb = median(added);
        println!(
            "memory added by activation (median of {RUNS}): {:.1} MB",
            mb(kb)
        );
        assert!(
            kb <= ACTIVATE_RSS_BUDGET_KB,
            "activation added {kb} KiB, over {ACTIVATE_RSS_BUDGET_KB} KiB"
        );
    }
    let binary = std::fs::metadata(env!("CARGO_BIN_EXE_kumikit")).map(|m| m.len());
    let bytes = binary.expect("the core binary exists");
    let mut firsts = Vec::new();
    let mut remote = Vec::new();
    for _ in 0..RUNS {
        let (first, s) = sample_remote().await;
        firsts.push(first);
        remote.push(s);
    }
    println!(
        "remote on: first start, generating the certificate (median of {RUNS}): {} ms",
        median(firsts).as_millis()
    );
    check_start_and_idle("remote on, one paired client", &remote);

    let mut registry = Vec::new();
    let mut compiled = Vec::new();
    for _ in 0..RUNS {
        let (installed, enabled) = sample_registry().await;
        registry.extend(installed);
        compiled.extend(enabled);
    }
    if !compiled.is_empty() {
        println!(
            "after enabling it in the running core (compiles the component; reported, not gated): idle RSS {:.1} MB",
            mb(median(compiled))
        );
    }
    if !registry.is_empty() {
        let kb = median(registry);
        println!(
            "after a registry search and install (median of {RUNS}): idle RSS {:.1} MB",
            mb(kb)
        );
        assert!(
            kb <= RSS_BUDGET_KB,
            "idle RSS after a registry install {kb} KiB exceeds {RSS_BUDGET_KB} KiB"
        );
    }

    println!("core binary: {:.1} MiB", bytes as f64 / (1024.0 * 1024.0));
    assert!(
        bytes <= BINARY_BUDGET_BYTES,
        "core binary {bytes} bytes exceeds {BINARY_BUDGET_BYTES} bytes"
    );
}
