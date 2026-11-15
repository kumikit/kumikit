//! Footprint budgets (`docs/spec/00-overview.md#footprint-budgets`) for the core:
//! cold start to accepting connections ≤ 300 ms, idle resident memory ≤ 30 MB.
//!
//! A measurement, not a correctness test: it is `#[ignore]`d and meant for a release
//! build. Run it with `sh scripts/footprint.sh`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

mod support;

use std::time::Duration;

use support::{Client, Core, Dirs};

const RUNS: usize = 5;
const START_BUDGET: Duration = Duration::from_millis(300);
const RSS_BUDGET_KB: u64 = 30 * 1024;

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
    let mut starts = Vec::new();
    let mut rss = Vec::new();
    for _ in 0..RUNS {
        let dirs = Dirs::new(); // fresh data directory: a cold start with migrations
        let (core, start) = Core::start_timed(&dirs).await;
        starts.push(start);
        let client = Client::hello(&dirs.address).await;
        // Let startup work settle before sampling idle memory.
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Some(kb) = rss_kb(core.pid()) {
            rss.push(kb);
        }
        drop(client);
        core.stop().await;
    }
    let start = median(starts);
    println!(
        "cold start to ready (median of {RUNS}): {} ms",
        start.as_millis()
    );
    assert!(
        start <= START_BUDGET,
        "cold start {start:?} exceeds {START_BUDGET:?}"
    );
    if rss.is_empty() {
        println!("idle RSS: not measured on this platform");
    } else {
        let kb = median(rss);
        println!("idle RSS (median of {RUNS}): {:.1} MB", kb as f64 / 1024.0);
        assert!(
            kb <= RSS_BUDGET_KB,
            "idle RSS {kb} KiB exceeds {RSS_BUDGET_KB} KiB"
        );
    }
}
