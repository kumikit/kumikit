use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use tokio::sync::Semaphore;

use super::*;
use crate::KernelConfig;
use crate::clock::{SequentialIds, StepClock};

async fn kernel(dir: &tempfile::TempDir) -> Kernel {
    Kernel::open(KernelConfig {
        clock: Arc::new(StepClock::default()),
        ids: Arc::new(SequentialIds::new("ID")),
        ..KernelConfig::new(dir.path().join("core.sqlite"))
    })
    .await
    .unwrap()
}

/// Records fires; each fire waits for a permit, so a test can keep a run going.
#[derive(Default)]
struct FakeRunner {
    fired: StdMutex<Vec<(String, String, String)>>,
    gate: Option<Arc<Semaphore>>,
    off: StdMutex<Vec<String>>,
}

impl Runner for FakeRunner {
    fn runnable(&self, plugin: &str) -> bool {
        !self.off.lock().unwrap().iter().any(|p| p == plugin)
    }
    fn fire(
        &self,
        plugin: String,
        key: String,
        due: String,
    ) -> BoxFuture<'static, Result<(), String>> {
        self.fired.lock().unwrap().push((plugin, key, due));
        let gate = self.gate.clone();
        Box::pin(async move {
            if let Some(g) = gate {
                let _ = g.acquire().await;
            }
            Ok(())
        })
    }
}

fn spec(cron: &str, missed: Missed) -> Spec {
    Spec {
        cron: cron.into(),
        zone: Some("UTC".into()),
        description: "d".into(),
        missed,
    }
}

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

#[tokio::test]
async fn no_schedules_arm_no_timer() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    k.set_schedule_runner(Arc::new(FakeRunner::default()));
    assert_eq!(k.schedule_wake().await.unwrap(), None);
    k.schedule_set(
        "a.p",
        "x",
        spec("0 9 * * *", Missed::Skip),
        Source::Plugin,
        t("2026-11-20T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(
        k.schedule_wake().await.unwrap(),
        Some(t("2026-11-21T09:00:00Z"))
    );
    k.schedule_set_enabled("a.p", "x", false, t("2026-11-20T10:00:00Z"))
        .await
        .unwrap();
    assert_eq!(
        k.schedule_wake().await.unwrap(),
        None,
        "a paused schedule arms nothing"
    );
}

#[tokio::test]
async fn a_due_schedule_fires_once_and_moves_on() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    let runner = Arc::new(FakeRunner::default());
    k.set_schedule_runner(runner.clone());
    let next = k
        .schedule_set(
            "a.p",
            "x",
            spec("*/10 * * * *", Missed::Skip),
            Source::Plugin,
            t("2026-11-20T10:01:00Z"),
        )
        .await
        .unwrap();
    assert_eq!(next.as_deref(), Some("2026-11-20T10:10:00Z"));
    assert!(
        k.schedule_tick(t("2026-11-20T10:09:59Z"))
            .await
            .unwrap()
            .is_empty()
    );
    let out = k.schedule_tick(t("2026-11-20T10:10:01Z")).await.unwrap();
    assert_eq!(
        out,
        [Outcome::Fired {
            plugin: "a.p".into(),
            key: "x".into(),
            due: "2026-11-20T10:10:00Z".into()
        }]
    );
    assert_eq!(runner.fired.lock().unwrap().len(), 1);
    let row = &k.schedule_list().await.unwrap()[0];
    assert_eq!(row.next_run.as_deref(), Some("2026-11-20T10:20:00Z"));
}

#[tokio::test]
async fn missed_runs_follow_their_policy_and_never_catch_up_twice() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    let runner = Arc::new(FakeRunner::default());
    k.set_schedule_runner(runner.clone());
    let at = t("2026-11-20T08:00:00Z");
    k.schedule_set(
        "a.p",
        "skip",
        spec("0 * * * *", Missed::Skip),
        Source::Plugin,
        at,
    )
    .await
    .unwrap();
    k.schedule_set(
        "a.p",
        "once",
        spec("0 * * * *", Missed::Once),
        Source::Plugin,
        at,
    )
    .await
    .unwrap();
    // The core was down from 08:00 to 12:30: four runs of each were missed.
    let out = k.schedule_tick(t("2026-11-20T12:30:00Z")).await.unwrap();
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(out.contains(&Outcome::Missed {
        plugin: "a.p".into(),
        key: "skip".into(),
        due: "2026-11-20T09:00:00Z".into()
    }));
    assert!(out.contains(&Outcome::Fired {
        plugin: "a.p".into(),
        key: "once".into(),
        due: "2026-11-20T09:00:00Z".into()
    }));
    for r in k.schedule_list().await.unwrap() {
        assert_eq!(
            r.next_run.as_deref(),
            Some("2026-11-20T13:00:00Z"),
            "{}",
            r.key
        );
    }
    assert_eq!(runner.fired.lock().unwrap().len(), 1);
    let skip = k
        .schedule_list()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.key == "skip")
        .unwrap();
    assert_eq!(skip.last_outcome.as_deref(), Some("missed"));
}

#[tokio::test]
async fn a_run_still_going_is_skipped_not_doubled() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    let gate = Arc::new(Semaphore::new(0));
    let runner = Arc::new(FakeRunner {
        gate: Some(gate.clone()),
        ..Default::default()
    });
    k.set_schedule_runner(runner.clone());
    k.schedule_set(
        "a.p",
        "x",
        spec("* * * * *", Missed::Skip),
        Source::Plugin,
        t("2026-11-20T10:00:00Z"),
    )
    .await
    .unwrap();
    assert!(matches!(
        k.schedule_tick(t("2026-11-20T10:01:00Z")).await.unwrap()[0],
        Outcome::Fired { .. }
    ));
    assert!(matches!(
        k.schedule_tick(t("2026-11-20T10:02:00Z")).await.unwrap()[0],
        Outcome::Skipped { .. }
    ));
    assert!(matches!(
        k.schedule_run_now("a.p", "x", t("2026-11-20T10:02:10Z"))
            .await,
        Err(KernelError::Conflict(_))
    ));
    assert_eq!(runner.fired.lock().unwrap().len(), 1);
    gate.add_permits(1);
}

#[tokio::test]
async fn a_plugin_that_may_not_run_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    let runner = Arc::new(FakeRunner::default());
    runner.off.lock().unwrap().push("a.p".into());
    k.set_schedule_runner(runner.clone());
    k.schedule_set(
        "a.p",
        "x",
        spec("* * * * *", Missed::Skip),
        Source::Plugin,
        t("2026-11-20T10:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(k.schedule_wake().await.unwrap(), None);
    assert!(
        k.schedule_tick(t("2026-11-20T10:05:00Z"))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        k.schedule_run_now("a.p", "x", t("2026-11-20T10:05:00Z"))
            .await,
        Err(KernelError::Conflict(_))
    ));
}

#[tokio::test]
async fn manifest_triggers_sync_and_limits_hold() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    let now = t("2026-11-20T10:00:00Z");
    k.schedule_sync_manifest("a.p", &["0 9 * * *".into(), "@hourly".into()], now)
        .await
        .unwrap();
    let keys: Vec<String> = k
        .schedule_list()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(keys, ["manifest:0", "manifest:1"]);
    k.schedule_sync_manifest("a.p", &["0 9 * * *".into()], now)
        .await
        .unwrap();
    assert_eq!(k.schedule_list().await.unwrap().len(), 1);
    assert_eq!(k.schedule_list().await.unwrap()[0].source, Source::Manifest);
    assert!(matches!(
        k.schedule_set(
            "a.p",
            "bad",
            spec("61 * * * *", Missed::Skip),
            Source::Plugin,
            now
        )
        .await,
        Err(KernelError::InvalidParams(_))
    ));
    for i in 1..MAX_PER_PLUGIN {
        k.schedule_set(
            "a.p",
            &format!("k{i}"),
            spec("@daily", Missed::Skip),
            Source::Plugin,
            now,
        )
        .await
        .unwrap();
    }
    assert!(matches!(
        k.schedule_set(
            "a.p",
            "one-more",
            spec("@daily", Missed::Skip),
            Source::Plugin,
            now
        )
        .await,
        Err(KernelError::Limit(_))
    ));
    k.schedules_delete_plugin("a.p").await.unwrap();
    assert!(k.schedule_list().await.unwrap().is_empty());
}
