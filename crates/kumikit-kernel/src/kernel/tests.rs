use std::sync::Arc;

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::event::{
    ApprovalRequested, ApprovalResolved, TurnEnded, TurnStarted, TurnText,
};
use kumikit_protocol::model::{
    Approval, ApprovalId, ApprovalStatus, Resolution, ThreadId, ThreadStatus, TurnId, TurnOutcome,
    UserInput, WorkspaceId,
};

use super::*;
use crate::clock::{SequentialIds, StepClock};

struct Env {
    dir: tempfile::TempDir,
    kernel: Kernel,
}

fn config(dir: &tempfile::TempDir) -> KernelConfig {
    KernelConfig {
        clock: Arc::new(StepClock::default()),
        ids: Arc::new(SequentialIds::new("ID")),
        ..KernelConfig::new(dir.path().join("core.sqlite"))
    }
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::open(config(&dir)).await.unwrap();
    Env { dir, kernel }
}

impl Env {
    async fn workspace(&self) -> WorkspaceId {
        let path = self.dir.path().to_str().unwrap();
        self.kernel.workspace_open(path).await.unwrap().id
    }

    async fn thread(&self) -> ThreadId {
        let ws = self.workspace().await;
        self.kernel
            .thread_insert(&ws, "p".into(), "m".into(), None, None)
            .await
            .unwrap()
            .id
    }

    async fn reopen(self) -> Env {
        self.kernel.close().await;
        let kernel = Kernel::open(config(&self.dir)).await.unwrap();
        Env {
            dir: self.dir,
            kernel,
        }
    }
}

fn started(turn: &str) -> ThreadEvent {
    ThreadEvent::TurnStarted(TurnStarted {
        turn: TurnId(turn.into()),
        input: UserInput::text("hi"),
        from: None,
        entry: None,
    })
}

fn delta(turn: &str, text: &str) -> ThreadEvent {
    ThreadEvent::TextDelta(TurnText {
        turn: TurnId(turn.into()),
        text: text.into(),
    })
}

fn requested(thread: &ThreadId, turn: &str, id: &str) -> ThreadEvent {
    ThreadEvent::ApprovalRequested(ApprovalRequested {
        approval: Approval {
            id: ApprovalId(id.into()),
            thread: thread.clone(),
            turn: TurnId(turn.into()),
            kind: "tool".into(),
            summary: "run it".into(),
            detail: None,
            status: ApprovalStatus::Pending,
        },
    })
}

fn ended(turn: &str, outcome: TurnOutcome) -> ThreadEvent {
    ThreadEvent::TurnEnded(TurnEnded {
        turn: TurnId(turn.into()),
        outcome,
        error: None,
    })
}

async fn all_events(k: &Kernel, t: &ThreadId) -> Vec<StoredEvent> {
    let t = t.clone();
    k.inner
        .store
        .read(move |c| repo::read_events(c, &t, 0, usize::MAX))
        .await
        .unwrap()
}

#[tokio::test]
async fn workspace_open_is_idempotent_and_reopens() {
    let e = env().await;
    let a = e.workspace().await;
    let b = e.workspace().await;
    assert_eq!(a, b);
    e.kernel.workspace_close(&a).await.unwrap();
    assert!(e.kernel.workspace_list().await.unwrap().is_empty());
    assert_eq!(e.workspace().await, a);
    assert_eq!(e.kernel.workspace_list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn workspace_open_rejects_bad_paths() {
    let e = env().await;
    let rel = e.kernel.workspace_open("relative/dir").await.unwrap_err();
    assert_eq!(rel.code(), kumikit_protocol::ErrorCode::InvalidParams);
    let missing = e.dir.path().join("missing");
    let err = e
        .kernel
        .workspace_open(missing.to_str().unwrap())
        .await
        .unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::NotFound);
    let file = e.dir.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let err = e
        .kernel
        .workspace_open(file.to_str().unwrap())
        .await
        .unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::InvalidParams);
}

#[tokio::test]
async fn closed_workspace_rejects_new_threads() {
    let e = env().await;
    let ws = e.workspace().await;
    e.kernel.workspace_close(&ws).await.unwrap();
    let err = e
        .kernel
        .thread_insert(&ws, "p".into(), "m".into(), None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::Conflict);
}

#[tokio::test]
async fn workspace_delete_removes_workspace_threads_and_rejects_running() {
    let e = env().await;
    let ws = e.workspace().await;
    let t1 = e
        .kernel
        .thread_insert(&ws, "p".into(), "m".into(), Some("active".into()), None)
        .await
        .unwrap()
        .id;
    let t2 = e
        .kernel
        .thread_insert(&ws, "p".into(), "m".into(), Some("archived".into()), None)
        .await
        .unwrap()
        .id;
    e.kernel.thread_archive(&t2).await.unwrap();

    // A running thread blocks workspace deletion with conflict.
    e.kernel.append(&t1, started("u1")).await.unwrap();
    let err = e.kernel.workspace_delete(&ws).await.unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::Conflict);

    // Once the turn finishes, deleting the workspace removes the workspace and all threads.
    e.kernel
        .append(&t1, ended("u1", TurnOutcome::Done))
        .await
        .unwrap();
    e.kernel.workspace_delete(&ws).await.unwrap();
    assert!(e.kernel.workspace_list().await.unwrap().is_empty());
    assert_eq!(
        e.kernel.thread_get(&t1).await.unwrap_err().code(),
        kumikit_protocol::ErrorCode::NotFound
    );
    assert_eq!(
        e.kernel.thread_get(&t2).await.unwrap_err().code(),
        kumikit_protocol::ErrorCode::NotFound
    );
    assert_eq!(
        e.kernel.workspace_delete(&ws).await.unwrap_err().code(),
        kumikit_protocol::ErrorCode::NotFound
    );
}

#[tokio::test]
async fn workspace_chat_creates_and_reuses_chat_workspace() {
    let e = env().await;
    let custom_home = e.dir.path().join("kk-home");
    e.kernel
        .set_core_config(Some(custom_home.to_str().unwrap().into()))
        .await
        .unwrap();

    let chat = e.kernel.workspace_chat().await.unwrap();
    assert_eq!(chat.name, "Chats");
    assert_eq!(
        chat.kind,
        Some(kumikit_protocol::model::WorkspaceKind::Chat)
    );
    assert!(std::path::Path::new(&chat.root).is_dir());

    // Calling again returns the same workspace idempotentently.
    let chat2 = e.kernel.workspace_chat().await.unwrap();
    assert_eq!(chat2.id, chat.id);

    // Threads can be created in the chat workspace without selecting a project folder.
    let t = e
        .kernel
        .thread_insert(
            &chat.id,
            "p".into(),
            "m".into(),
            Some("Quick question".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(t.workspace, chat.id);
}

#[tokio::test]
async fn seq_starts_at_one_and_is_contiguous_under_concurrency() {
    let e = env().await;
    let t = e.thread().await;
    e.kernel.append(&t, started("u")).await.unwrap();
    let appends = (0..100).map(|i| {
        let (k, t) = (e.kernel.clone(), t.clone());
        tokio::spawn(async move { k.append(&t, delta("u", &i.to_string())).await })
    });
    for a in appends.collect::<Vec<_>>() {
        a.await.unwrap().unwrap();
    }
    let seqs: Vec<u64> = all_events(&e.kernel, &t)
        .await
        .iter()
        .map(|e| e.seq)
        .collect();
    assert_eq!(seqs, (1..=101).collect::<Vec<_>>());
}

#[tokio::test]
async fn replay_from_any_cursor() {
    let e = env().await;
    let t = e.thread().await;
    e.kernel.append(&t, started("u")).await.unwrap();
    for i in 0..9 {
        e.kernel
            .append(&t, delta("u", &i.to_string()))
            .await
            .unwrap();
    }
    for after in 0..=10 {
        let mut sub = e.kernel.subscribe(&t, after).await.unwrap();
        assert_eq!(sub.head(), 10);
        if after == 10 {
            continue;
        }
        let batch = sub.next_batch().await.unwrap().unwrap();
        let seqs: Vec<u64> = batch.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, ((after + 1)..=10).collect::<Vec<_>>());
    }
}

#[tokio::test]
async fn subscription_follows_live_appends_without_gap_or_duplicate() {
    let e = env().await;
    let t = e.thread().await;
    e.kernel.append(&t, started("u")).await.unwrap();
    for i in 0..50 {
        e.kernel
            .append(&t, delta("u", &i.to_string()))
            .await
            .unwrap();
    }
    let mut sub = e.kernel.subscribe(&t, 20).await.unwrap();
    let (k, t2) = (e.kernel.clone(), t.clone());
    let writer = tokio::spawn(async move {
        for i in 0..500 {
            k.append(&t2, delta("u", &format!("live {i}")))
                .await
                .unwrap();
        }
    });
    let mut seen = Vec::new();
    while seen.last() != Some(&551) {
        for ev in sub.next_batch().await.unwrap().unwrap() {
            seen.push(ev.seq);
        }
    }
    writer.await.unwrap();
    assert_eq!(seen, (21..=551).collect::<Vec<_>>());
}

#[tokio::test]
async fn subscribe_to_missing_thread_is_not_found() {
    let e = env().await;
    let err = e
        .kernel
        .subscribe(&ThreadId("nope".into()), 0)
        .await
        .unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::NotFound);
}

#[tokio::test]
async fn snapshot_matches_derived_status() {
    let e = env().await;
    let t = e.thread().await;
    let ws = e.kernel.thread_row(&t).await.unwrap().thread.workspace;
    let check = |expected: ThreadStatus| {
        let (k, t, ws) = (e.kernel.clone(), t.clone(), ws.clone());
        async move {
            let snap = k.thread_get(&t).await.unwrap();
            let listed = k.thread_list(&ws, false).await.unwrap();
            assert_eq!(snap.thread.status, expected);
            assert_eq!(
                listed[0].status, expected,
                "derived column disagrees with the log"
            );
            snap
        }
    };
    check(ThreadStatus::Idle).await;
    e.kernel.append(&t, started("u")).await.unwrap();
    check(ThreadStatus::Running).await;
    e.kernel.append(&t, requested(&t, "u", "a1")).await.unwrap();
    let snap = check(ThreadStatus::AwaitingApproval).await;
    assert_eq!(snap.pending_approvals.len(), 1);
    assert_eq!(snap.seq, 2);
    e.kernel
        .append(
            &t,
            ThreadEvent::ApprovalResolved(ApprovalResolved {
                approval: ApprovalId("a1".into()),
                decision: Resolution::Allow,
                by: None,
                plugin: None,
            }),
        )
        .await
        .unwrap();
    check(ThreadStatus::Running).await;
    e.kernel
        .append(&t, ended("u", TurnOutcome::Failed))
        .await
        .unwrap();
    let snap = check(ThreadStatus::Failed).await;
    assert_eq!(snap.turns.len(), 1);
    assert_eq!(snap.turns[0].outcome, Some(TurnOutcome::Failed));
    assert!(snap.pending_approvals.is_empty());
    assert_eq!(snap.seq, 4);
}

#[tokio::test]
async fn log_invariants_are_enforced() {
    let e = env().await;
    let t = e.thread().await;
    let err = e
        .kernel
        .append(&t, ended("ghost", TurnOutcome::Done))
        .await
        .unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::Conflict);
    e.kernel.append(&t, started("u")).await.unwrap();
    e.kernel.append(&t, requested(&t, "u", "a1")).await.unwrap();
    let resolve = ThreadEvent::ApprovalResolved(ApprovalResolved {
        approval: ApprovalId("a1".into()),
        decision: Resolution::Deny,
        by: None,
        plugin: None,
    });
    e.kernel.append(&t, resolve.clone()).await.unwrap();
    let err = e.kernel.append(&t, resolve).await.unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::Conflict);
    // Failed appends leave no trace in the log.
    let seqs: Vec<u64> = all_events(&e.kernel, &t)
        .await
        .iter()
        .map(|e| e.seq)
        .collect();
    assert_eq!(seqs, vec![1, 2, 3]);
}

#[tokio::test]
async fn archive_rules() {
    let e = env().await;
    let t = e.thread().await;
    let ws = e.kernel.thread_row(&t).await.unwrap().thread.workspace;
    e.kernel.append(&t, started("u")).await.unwrap();
    let err = e.kernel.thread_archive(&t).await.unwrap_err();
    assert_eq!(err.code(), kumikit_protocol::ErrorCode::Conflict);
    e.kernel
        .append(&t, ended("u", TurnOutcome::Done))
        .await
        .unwrap();
    e.kernel.thread_archive(&t).await.unwrap();
    assert!(e.kernel.thread_list(&ws, false).await.unwrap().is_empty());
    let all = e.kernel.thread_list(&ws, true).await.unwrap();
    assert_eq!(all[0].status, ThreadStatus::Archived);
    assert_eq!(
        e.kernel.thread_get(&t).await.unwrap().thread.status,
        ThreadStatus::Archived
    );
}

#[tokio::test]
async fn restart_ends_open_turns() {
    let e = env().await;
    let t = e.thread().await;
    e.kernel.append(&t, started("u")).await.unwrap();
    e.kernel.append(&t, requested(&t, "u", "a1")).await.unwrap();
    let e = e.reopen().await;

    let events = all_events(&e.kernel, &t).await;
    let kinds: Vec<&str> = events.iter().map(|e| e.event.kind()).collect();
    assert_eq!(
        kinds,
        [
            "turn.started",
            "approval.requested",
            "approval.resolved",
            "turn.ended"
        ]
    );
    match &events[2].event {
        ThreadEvent::ApprovalResolved(r) => assert_eq!(r.decision, Resolution::Cancelled),
        other => panic!("unexpected {other:?}"),
    }
    match &events[3].event {
        ThreadEvent::TurnEnded(t) => {
            assert_eq!(t.outcome, TurnOutcome::Failed);
            assert_eq!(t.error.as_deref(), Some(CORE_RESTARTED));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        e.kernel.thread_get(&t).await.unwrap().thread.status,
        ThreadStatus::Failed
    );

    // A second restart has nothing to recover.
    let e = e.reopen().await;
    assert_eq!(all_events(&e.kernel, &t).await.len(), 4);
}

#[tokio::test]
async fn close_ends_subscriptions() {
    let e = env().await;
    let t = e.thread().await;
    let mut sub = e.kernel.subscribe(&t, 0).await.unwrap();
    let waiter = tokio::spawn(async move { sub.next_batch().await });
    e.kernel.close().await;
    assert!(waiter.await.unwrap().unwrap().is_none());
}
