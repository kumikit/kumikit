use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use kumikit_protocol::ErrorCode;
use kumikit_protocol::model::ThreadStatus;
use tokio::sync::Notify;

use super::*;
use crate::KernelConfig;
use crate::clock::{SequentialIds, StepClock};
use crate::provider::{BoxFuture, Model, ProviderError, ProviderEvent, ProviderResult};
use crate::repo::StoredEvent;

/// A fake provider; the message text picks the behavior:
/// `echo <x>`, `approve`, `fail`, `plugin`, `block`, `late`.
#[derive(Debug, Default)]
struct Fake {
    decisions: Mutex<HashMap<ApprovalId, Decision>>,
    decided: Notify,
    cancels: AtomicUsize,
    resumes: Mutex<Vec<Option<String>>>,
    late_sink: Mutex<Option<EventSink>>,
}

impl Fake {
    async fn wait_decision(&self, id: &ApprovalId) -> Decision {
        loop {
            let notified = self.decided.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(d) = self.decisions.lock().unwrap().remove(id) {
                return d;
            }
            notified.await;
        }
    }
}

impl Provider for Fake {
    fn id(&self) -> &str {
        "fake"
    }
    fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>> {
        Box::pin(async {
            Ok(["one", "two"]
                .map(|id| Model {
                    id: id.into(),
                    label: id.into(),
                    capabilities: vec![],
                })
                .to_vec())
        })
    }
    fn start_session(&self, config: SessionConfig) -> BoxFuture<'_, ProviderResult<String>> {
        Box::pin(async move {
            self.resumes.lock().unwrap().push(config.resume.clone());
            Ok(format!("session-{}", config.thread))
        })
    }
    fn send(
        &self,
        _session: String,
        input: UserInput,
        sink: EventSink,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async move {
            let emit = |e| {
                let sink = sink.clone();
                async move {
                    sink.emit(e)
                        .await
                        .map_err(|e| ProviderError::new(e.to_string()))
                }
            };
            match input.text.as_str() {
                t if t.starts_with("echo ") => {
                    let text = t["echo ".len()..].to_owned();
                    emit(ProviderEvent::TextDelta { text: text.clone() }).await?;
                    emit(ProviderEvent::TextDone { text }).await?;
                    Ok(())
                }
                "approve" => {
                    let id = sink
                        .request_approval("tool", "run it", None)
                        .await
                        .map_err(|e| ProviderError::new(e.to_string()))?;
                    let d = self.wait_decision(&id).await;
                    emit(ProviderEvent::TextDelta {
                        text: format!("{d:?}"),
                    })
                    .await?;
                    Ok(())
                }
                "fail" => Err(ProviderError::new("boom")),
                "plugin" => {
                    emit(ProviderEvent::Plugin {
                        kind: "k".into(),
                        data: serde_json::json!(1),
                    })
                    .await?;
                    Ok(())
                }
                "block" => std::future::pending().await,
                "late" => {
                    *self.late_sink.lock().unwrap() = Some(sink.clone());
                    Ok(())
                }
                other => Err(ProviderError::new(format!("unknown script {other}"))),
            }
        })
    }
    fn respond_approval(
        &self,
        _s: String,
        approval: ApprovalId,
        decision: Decision,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async move {
            self.decisions.lock().unwrap().insert(approval, decision);
            self.decided.notify_waiters();
            Ok(())
        })
    }
    fn cancel(&self, _s: String) -> BoxFuture<'_, ()> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }
    fn end_session(&self, _s: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

struct Env {
    dir: tempfile::TempDir,
    kernel: Kernel,
    fake: Arc<Fake>,
    thread: ThreadId,
}

fn config(dir: &tempfile::TempDir) -> KernelConfig {
    KernelConfig {
        database: dir.path().join("core.sqlite"),
        clock: Arc::new(StepClock::default()),
        ids: Arc::new(SequentialIds::new("ID")),
    }
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::open(config(&dir)).await.unwrap();
    let fake = Arc::new(Fake::default());
    kernel.register_provider(fake.clone()).unwrap();
    let ws = kernel
        .workspace_open(dir.path().to_str().unwrap())
        .await
        .unwrap();
    let thread = kernel
        .thread_create(&ws.id, None, None, None)
        .await
        .unwrap()
        .id;
    Env {
        dir,
        kernel,
        fake,
        thread,
    }
}

impl Env {
    async fn send(&self, text: &str) -> ThreadSendResult {
        self.kernel
            .thread_send(&self.thread, UserInput { text: text.into() })
            .await
            .unwrap()
    }

    /// Reads the log from `after` until an event of `kind` arrives (inclusive).
    async fn until(&self, after: u64, kind: &str) -> Vec<StoredEvent> {
        let mut sub = self.kernel.subscribe(&self.thread, after).await.unwrap();
        let mut out = Vec::new();
        loop {
            for e in sub.next_batch().await.unwrap().unwrap() {
                let done = e.event.kind() == kind;
                out.push(e);
                if done {
                    return out;
                }
            }
        }
    }
}

fn kinds(events: &[StoredEvent]) -> Vec<&'static str> {
    events.iter().map(|e| e.event.kind()).collect()
}

fn turn_ended(events: &[StoredEvent]) -> &kumikit_protocol::event::TurnEnded {
    match &events.last().unwrap().event {
        ThreadEvent::TurnEnded(e) => e,
        other => panic!("last event is {other:?}"),
    }
}

#[tokio::test]
async fn echo_turn() {
    let e = env().await;
    let sent = e.send("echo hi").await;
    assert!(!sent.queued);
    let events = e.until(0, "turn.ended").await;
    assert_eq!(
        kinds(&events),
        ["turn.started", "text.delta", "text.done", "turn.ended"]
    );
    assert_eq!(turn_ended(&events).outcome, TurnOutcome::Done);
    assert_eq!(events[0].event.turn(), Some(&sent.turn));
    assert_eq!(
        e.kernel.thread_get(&e.thread).await.unwrap().thread.status,
        ThreadStatus::Idle
    );
}

#[tokio::test]
async fn provider_error_fails_the_turn() {
    let e = env().await;
    e.send("fail").await;
    let events = e.until(0, "turn.ended").await;
    let end = turn_ended(&events);
    assert_eq!(end.outcome, TurnOutcome::Failed);
    assert_eq!(end.error.as_deref(), Some("boom"));
    assert_eq!(
        e.kernel.thread_get(&e.thread).await.unwrap().thread.status,
        ThreadStatus::Failed
    );
    // A failed thread accepts the next message.
    e.send("echo again").await;
    let events = e.until(events.last().unwrap().seq, "turn.ended").await;
    assert_eq!(turn_ended(&events).outcome, TurnOutcome::Done);
}

#[tokio::test]
async fn approval_first_answer_wins() {
    let e = env().await;
    e.send("approve").await;
    let events = e.until(0, "approval.requested").await;
    let ThreadEvent::ApprovalRequested(req) = &events.last().unwrap().event else {
        panic!()
    };
    let id = req.approval.id.clone();
    assert_eq!(
        e.kernel.thread_get(&e.thread).await.unwrap().thread.status,
        ThreadStatus::AwaitingApproval
    );
    e.kernel
        .respond_approval(&id, Decision::AllowAlways, Some("dev1".into()))
        .await
        .unwrap();
    let err = e
        .kernel
        .respond_approval(&id, Decision::Deny, None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
    let rest = e.until(events.last().unwrap().seq, "turn.ended").await;
    assert_eq!(
        kinds(&rest),
        ["approval.resolved", "text.delta", "turn.ended"]
    );
    let ThreadEvent::ApprovalResolved(res) = &rest[0].event else {
        panic!()
    };
    assert_eq!(res.decision, Resolution::AllowAlways);
    assert_eq!(res.by.as_deref(), Some("dev1"));
    let ThreadEvent::TextDelta(d) = &rest[1].event else {
        panic!()
    };
    assert_eq!(d.text, "AllowAlways");
}

#[tokio::test]
async fn unknown_approval_is_not_found() {
    let e = env().await;
    let err = e
        .kernel
        .respond_approval(&ApprovalId("nope".into()), Decision::Allow, None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn cancel_during_approval() {
    let e = env().await;
    e.send("approve").await;
    let events = e.until(0, "approval.requested").await;
    e.kernel.thread_cancel(&e.thread).await.unwrap();
    let rest = e.until(events.last().unwrap().seq, "turn.ended").await;
    assert_eq!(kinds(&rest), ["approval.resolved", "turn.ended"]);
    let ThreadEvent::ApprovalResolved(res) = &rest[0].event else {
        panic!()
    };
    assert_eq!(res.decision, Resolution::Cancelled);
    assert_eq!(turn_ended(&rest).outcome, TurnOutcome::Cancelled);
    assert_eq!(e.fake.cancels.load(Ordering::SeqCst), 1);
    assert_eq!(
        e.kernel.thread_get(&e.thread).await.unwrap().thread.status,
        ThreadStatus::Idle
    );
}

#[tokio::test]
async fn cancel_drops_the_queue() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    assert!(e.send("echo a").await.queued);
    assert!(e.send("echo b").await.queued);
    e.kernel.thread_cancel(&e.thread).await.unwrap();
    let events = e.until(0, "turn.ended").await;
    assert_eq!(turn_ended(&events).outcome, TurnOutcome::Cancelled);
    // The run is gone once the turn ended and nothing was queued.
    for _ in 0..1000 {
        if !e.kernel.has_run(&e.thread) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(!e.kernel.has_run(&e.thread));
    assert_eq!(e.kernel.thread_get(&e.thread).await.unwrap().turns.len(), 1);
}

#[tokio::test]
async fn queued_messages_run_in_order() {
    let e = env().await;
    let first = e.send("approve").await;
    let events = e.until(0, "approval.requested").await;
    let q1 = e.send("echo one").await;
    let q2 = e.send("echo two").await;
    assert!(q1.queued && q2.queued);
    let ThreadEvent::ApprovalRequested(req) = &events.last().unwrap().event else {
        panic!()
    };
    e.kernel
        .respond_approval(&req.approval.id, Decision::Allow, None)
        .await
        .unwrap();

    let mut ends = Vec::new();
    let mut after = 0;
    while ends.len() < 3 {
        let batch = e.until(after, "turn.ended").await;
        after = batch.last().unwrap().seq;
        ends.push(turn_ended(&batch).turn.clone());
    }
    assert_eq!(ends, [first.turn, q1.turn, q2.turn]);
}

#[tokio::test]
async fn sink_is_closed_after_the_turn() {
    let e = env().await;
    e.send("late").await;
    e.until(0, "turn.ended").await;
    let sink = e.fake.late_sink.lock().unwrap().take().unwrap();
    let err = sink
        .emit(ProviderEvent::TextDelta {
            text: "too late".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
    let err = sink
        .request_approval("tool", "late", None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
}

#[tokio::test]
async fn plugin_events_are_stamped_with_the_provider() {
    let e = env().await;
    e.send("plugin").await;
    let events = e.until(0, "turn.ended").await;
    assert_eq!(
        kinds(&events),
        ["turn.started", "plugin.event", "turn.ended"]
    );
    let ThreadEvent::PluginEvent(p) = &events[1].event else {
        panic!()
    };
    assert_eq!((p.plugin.as_str(), p.kind.as_str()), ("fake", "k"));
}

#[tokio::test]
async fn archive_is_refused_while_a_turn_runs() {
    let e = env().await;
    e.send("block").await;
    let err = e.kernel.thread_archive(&e.thread).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
}

#[tokio::test]
async fn thread_create_defaults_and_validation() {
    let e = env().await;
    let ws = e.kernel.workspace_list().await.unwrap()[0].id.clone();
    let t = e.kernel.thread_create(&ws, None, None, None).await.unwrap();
    assert_eq!((t.provider.as_str(), t.model.as_str()), ("fake", "one"));
    let t = e
        .kernel
        .thread_create(
            &ws,
            Some("fake".into()),
            Some("two".into()),
            Some("T".into()),
        )
        .await
        .unwrap();
    assert_eq!(t.model, "two");
    let err = e
        .kernel
        .thread_create(&ws, None, Some("three".into()), None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidParams);
    let err = e
        .kernel
        .thread_create(&ws, Some("other".into()), None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidParams);
    let err = e
        .kernel
        .register_provider(Arc::new(Fake::default()))
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
}

#[tokio::test]
async fn no_provider_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::open(config(&dir)).await.unwrap();
    let ws = kernel
        .workspace_open(dir.path().to_str().unwrap())
        .await
        .unwrap();
    let err = kernel
        .thread_create(&ws.id, None, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Unsupported);
}

#[tokio::test]
async fn session_starts_once_and_resumes_after_restart() {
    let e = env().await;
    e.send("echo a").await;
    let first = e.until(0, "turn.ended").await;
    e.send("echo b").await;
    e.until(first.last().unwrap().seq, "turn.ended").await;
    assert_eq!(*e.fake.resumes.lock().unwrap(), vec![None]);

    e.kernel.close().await;
    let kernel = Kernel::open(config(&e.dir)).await.unwrap();
    let fake = Arc::new(Fake::default());
    kernel.register_provider(fake.clone()).unwrap();
    kernel
        .thread_send(
            &e.thread,
            UserInput {
                text: "echo c".into(),
            },
        )
        .await
        .unwrap();
    let mut sub = kernel.subscribe(&e.thread, 0).await.unwrap();
    let mut ended = 0;
    while ended < 3 {
        ended += sub
            .next_batch()
            .await
            .unwrap()
            .unwrap()
            .iter()
            .filter(|e| e.event.kind() == "turn.ended")
            .count();
    }
    let expected = format!("session-{}", e.thread);
    assert_eq!(*fake.resumes.lock().unwrap(), vec![Some(expected)]);
}

#[tokio::test]
async fn close_abandons_running_turns_for_recovery() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    e.kernel.close().await;
    let kernel = Kernel::open(config(&e.dir)).await.unwrap();
    let snap = kernel.thread_get(&e.thread).await.unwrap();
    assert_eq!(snap.thread.status, ThreadStatus::Failed);
    let mut sub = kernel.subscribe(&e.thread, 0).await.unwrap();
    let events = sub.next_batch().await.unwrap().unwrap();
    assert_eq!(kinds(&events), ["turn.started", "turn.ended"]);
    assert_eq!(
        turn_ended(&events).error.as_deref(),
        Some(crate::CORE_RESTARTED)
    );
}
