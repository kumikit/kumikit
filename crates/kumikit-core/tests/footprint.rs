//! Footprint budgets (`docs/spec/00-overview.md#footprint-budgets`) for the core:
//! cold start to accepting connections ≤ 300 ms, idle resident memory ≤ 30 MB, and
//! activating a typical WASM plugin ≤ 50 ms and ≤ 12 MB, and a core binary ≤ 32 MiB
//! (ADR 0012). Three phases, each the median of
//! five runs from a fresh data directory:
//!
//! 1. no plugins: cold start and idle memory;
//! 2. provider-acp and a manifest-only ACP plugin installed and enabled (offline, before
//!    `serve` starts): the same two budgets, so enabled but unused plugins cost nothing;
//! 3. activation: `thread.create` on the delegated provider loads provider-acp's
//!    control instance from the compile cache and calls `activate` and `list-models`
//!    (no agent is started); its round trip and the memory it adds.
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
const ACTIVATE_RSS_BUDGET_KB: u64 = 12 * 1024;
const BINARY_BUDGET_BYTES: u64 = 32 * 1024 * 1024;

/// A manifest-only plugin served by provider-acp. Its agent is never started here.
const DECLARATIVE: &str = r#"
id = "test.footprint"
name = "Footprint"
version = "0.1.0"
kind = ["provider"]
api = "^0.1"
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
    println!("core binary: {:.1} MiB", bytes as f64 / (1024.0 * 1024.0));
    assert!(
        bytes <= BINARY_BUDGET_BYTES,
        "core binary {bytes} bytes exceeds {BINARY_BUDGET_BYTES} bytes"
    );
}
