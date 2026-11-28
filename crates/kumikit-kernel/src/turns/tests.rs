use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use kumikit_protocol::ErrorCode;
use kumikit_protocol::event::{QueueAddReason, QueueHold};
use kumikit_protocol::model::ThreadStatus;
use tokio::sync::Notify;

use super::*;
use crate::KernelConfig;
use crate::clock::{SequentialIds, StepClock};
use crate::provider::{BoxFuture, Model, ProviderError, ProviderEvent, ProviderResult};
use crate::repo::StoredEvent;

/// The user's message as a provider sees it: the first-turn title hint (`TITLE_HINT`) and
/// the blank line after it are dropped, so scripts match on what the user typed.
fn user_text(text: &str) -> &str {
    if text.starts_with(crate::turns::TITLE_HINT) {
        text.split_once("\n\n").map_or(text, |(_, rest)| rest)
    } else {
        text
    }
}

/// A fake provider; the message text picks the behavior:
/// `echo <x>`, `approve`, `approve-fail`, `fail`, `plugin`, `block`, `late`, `steer`.
/// Model `one` reports `steer`; `two` does not.
#[derive(Debug, Default)]
struct Fake {
    decisions: Mutex<HashMap<ApprovalId, Decision>>,
    decided: Notify,
    cancels: AtomicUsize,
    resumes: Mutex<Vec<Option<String>>>,
    late_sink: Mutex<Option<EventSink>>,
    limit_reads: AtomicUsize,
    fail_limit_reads: AtomicUsize,
    limit_read_enter: Mutex<Option<Arc<Notify>>>,
    limit_read_release: Mutex<Option<Arc<Notify>>>,
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
                    options: vec![],
                    capabilities: if id == "one" {
                        vec!["steer".into()]
                    } else {
                        vec![]
                    },
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
            match user_text(&input.text) {
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
                "approve-fail" => {
                    let id = sink
                        .request_approval("tool", "run it", None)
                        .await
                        .map_err(|e| ProviderError::new(e.to_string()))?;
                    self.wait_decision(&id).await;
                    Err(ProviderError::new("boom"))
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
                "steer" => {
                    sink.input_ready().await;
                    let input = sink
                        .take_input()
                        .await
                        .map_err(|e| ProviderError::new(e.to_string()))?
                        .ok_or_else(|| ProviderError::new("ready but empty"))?;
                    emit(ProviderEvent::TextDone {
                        text: format!("steered: {}", input.text),
                    })
                    .await?;
                    Ok(())
                }
                "late" => {
                    *self.late_sink.lock().unwrap() = Some(sink.clone());
                    Ok(())
                }
                "usage" => {
                    // A running total: the second report replaces the first.
                    for (input, output) in [(5, 1), (10, 2)] {
                        emit(ProviderEvent::Usage {
                            input_tokens: input,
                            output_tokens: output,
                            cost: Some(0.25),
                        })
                        .await?;
                    }
                    Ok(())
                }
                "models" => {
                    let m =
                        |model: &str, input, cached, cost| kumikit_protocol::model::ModelUsage {
                            model: model.into(),
                            input_tokens: input,
                            output_tokens: 1,
                            cached_input_tokens: Some(cached),
                            cost,
                        };
                    emit(ProviderEvent::UsageByModel {
                        models: vec![m("one", 100, 80, Some(1.0)), m("small", 20, 5, None)],
                    })
                    .await?;
                    Ok(())
                }
                "limits" => {
                    sink.report_limits(test_limits(42.0))
                        .await
                        .map_err(|e| ProviderError::new(e.to_string()))?;
                    Ok(())
                }
                "usage-then-block" => {
                    emit(ProviderEvent::Usage {
                        input_tokens: 40,
                        output_tokens: 7,
                        cost: Some(0.15),
                    })
                    .await?;
                    std::future::pending().await
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
    fn reads_limits(&self) -> bool {
        true
    }
    fn read_limits(
        &self,
    ) -> BoxFuture<'_, ProviderResult<Option<kumikit_protocol::model::UsageLimits>>> {
        let enter = self.limit_read_enter.lock().unwrap().clone();
        let release = self.limit_read_release.lock().unwrap().clone();
        Box::pin(async move {
            let n = self.limit_reads.fetch_add(1, Ordering::SeqCst);
            if let Some(enter) = enter {
                enter.notify_waiters();
            }
            if let Some(release) = release {
                release.notified().await;
            }
            if self
                .fail_limit_reads
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
                .is_ok()
            {
                return Err(ProviderError::new("reader failed"));
            }
            Ok(Some(test_limits(10.0 + n as f64)))
        })
    }
}

fn test_limits(pct: f64) -> kumikit_protocol::model::UsageLimits {
    kumikit_protocol::model::UsageLimits {
        plan: Some("Pro".into()),
        windows: vec![kumikit_protocol::model::LimitWindow {
            id: "five-hour".into(),
            label: "5 hours".into(),
            group: None,
            used_percent: pct,
            resets_at: Some(kumikit_protocol::model::Timestamp(
                "2026-11-15T15:00:00.000Z".into(),
            )),
            model: None,
        }],
        ..Default::default()
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
        clock: Arc::new(StepClock::default()),
        ids: Arc::new(SequentialIds::new("ID")),
        ..KernelConfig::new(dir.path().join("core.sqlite"))
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
        .thread_create(&ws.id, None, None, None, None)
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
            .thread_send(&self.thread, UserInput::text(text))
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

impl Env {
    /// Waits until the thread has no run (its last turn ended and nothing is next).
    async fn idle(&self) {
        for _ in 0..10_000 {
            if !self.kernel.has_run(&self.thread) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("thread {} still runs", self.thread);
    }

    /// The queue's texts, in order, and its hold, from a snapshot.
    async fn queue(&self) -> (Vec<String>, Option<QueueHold>) {
        let snap = self.kernel.thread_get(&self.thread).await.unwrap();
        let texts = snap.queue.into_iter().map(|q| q.input.text).collect();
        (texts, snap.queue_hold)
    }

    /// Waits for `n` more `turn.ended` events after `after`; returns every event read
    /// and the turns that ended, in order.
    async fn ends(&self, after: u64, n: usize) -> (Vec<StoredEvent>, Vec<TurnId>) {
        let mut all = Vec::new();
        let mut ended = Vec::new();
        let mut after = after;
        while ended.len() < n {
            let batch = self.until(after, "turn.ended").await;
            after = batch.last().unwrap().seq;
            ended.push(turn_ended(&batch).turn.clone());
            all.extend(batch);
        }
        (all, ended)
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
async fn cancel_holds_the_queue_until_resume() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    let a = e.send("echo a").await;
    let b = e.send("echo b").await;
    assert!(a.queued && a.entry.is_some());
    e.kernel.thread_cancel(&e.thread).await.unwrap();
    let events = e.until(0, "queue.held").await;
    assert_eq!(
        kinds(&events),
        [
            "turn.started",
            "queue.added",
            "queue.added",
            "turn.ended",
            "queue.held"
        ]
    );
    e.idle().await;
    assert_eq!(
        e.queue().await,
        (
            vec!["echo a".into(), "echo b".into()],
            Some(QueueHold::Stopped)
        )
    );
    assert_eq!(e.kernel.thread_get(&e.thread).await.unwrap().turns.len(), 1);

    e.kernel.queue_resume(&e.thread).await.unwrap();
    let head = events.last().unwrap().seq;
    let (events, ended) = e.ends(head, 2).await;
    assert_eq!(ended, [a.turn, b.turn]);
    assert_eq!(kinds(&events)[0], "queue.resumed");
    let ThreadEvent::TurnStarted(started) = &events[1].event else {
        panic!("{:?}", events[1])
    };
    assert_eq!(started.entry, a.entry, "turn.started names its queue entry");
    e.idle().await;
    assert_eq!(e.queue().await, (vec![], None));
}

#[tokio::test]
async fn a_failed_turn_holds_the_queue() {
    let e = env().await;
    e.send("approve-fail").await;
    let events = e.until(0, "approval.requested").await;
    e.send("echo a").await;
    let ThreadEvent::ApprovalRequested(req) = &events.last().unwrap().event else {
        panic!()
    };
    e.kernel
        .respond_approval(&req.approval.id, Decision::Allow, None)
        .await
        .unwrap();
    let events = e.until(0, "queue.held").await;
    assert_eq!(
        turn_ended(&events[..events.len() - 1]).outcome,
        TurnOutcome::Failed
    );
    e.idle().await;
    assert_eq!(
        e.queue().await,
        (vec!["echo a".into()], Some(QueueHold::Failed))
    );
}

#[tokio::test]
async fn a_restart_holds_the_queue_and_keeps_it() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    let a = e.send("echo a").await;
    e.kernel.close().await;

    let kernel = Kernel::open(config(&e.dir)).await.unwrap();
    let fake = Arc::new(Fake::default());
    kernel.register_provider(fake).unwrap();
    let e = Env { kernel, ..e };
    let events = e.until(0, "queue.held").await;
    assert_eq!(
        kinds(&events),
        ["turn.started", "queue.added", "turn.ended", "queue.held"]
    );
    assert_eq!(
        e.queue().await,
        (vec!["echo a".into()], Some(QueueHold::Restarted))
    );

    // The entry came back from the database, not from memory.
    e.kernel.queue_resume(&e.thread).await.unwrap();
    let (_, ended) = e.ends(events.last().unwrap().seq, 1).await;
    assert_eq!(ended, [a.turn]);
}

#[tokio::test]
async fn queued_messages_can_be_edited_moved_and_removed() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    let a = e.send("echo a").await.entry.unwrap();
    let b = e.send("echo b").await.entry.unwrap();
    let c = e.send("echo c").await.entry.unwrap();

    let edited = e
        .kernel
        .queue_update(&e.thread, &b, UserInput::text("echo b2"))
        .await
        .unwrap();
    assert_eq!(edited.input.text, "echo b2");
    e.kernel.queue_move(&e.thread, &c, Some(&a)).await.unwrap();
    e.kernel.queue_move(&e.thread, &c, Some(&c)).await.unwrap();
    e.kernel.queue_remove(&e.thread, &a).await.unwrap();
    assert_eq!(
        e.queue().await,
        (vec!["echo c".into(), "echo b2".into()], None)
    );
    e.kernel.queue_move(&e.thread, &c, None).await.unwrap();
    assert_eq!(
        e.queue().await,
        (vec!["echo b2".into(), "echo c".into()], None)
    );

    let gone = QueueEntryId("nope".into());
    for err in [
        e.kernel.queue_remove(&e.thread, &a).await.unwrap_err(),
        e.kernel.queue_remove(&e.thread, &gone).await.unwrap_err(),
        e.kernel
            .queue_move(&e.thread, &b, Some(&gone))
            .await
            .unwrap_err(),
        e.kernel
            .queue_update(&e.thread, &gone, UserInput::text("x"))
            .await
            .unwrap_err(),
    ] {
        assert_eq!(err.code(), ErrorCode::NotFound, "{err}");
    }

    // The table agrees with the fold.
    let t = e.thread.clone();
    let stored = e
        .kernel
        .inner
        .store
        .read(move |c| repo::queue_list(c, &t))
        .await
        .unwrap();
    let ids: Vec<_> = stored.into_iter().map(|q| q.id).collect();
    assert_eq!(ids, [b, c]);
}

#[tokio::test]
async fn send_now_stops_the_running_turn_and_runs_that_entry() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    let a = e.send("echo a").await;
    let b = e.send("echo b").await;
    let now = e
        .kernel
        .queue_send_now(&e.thread, b.entry.as_ref().unwrap())
        .await
        .unwrap();
    assert!(now.queued);
    assert_eq!(now.turn, b.turn);
    let (events, ended) = e.ends(0, 2).await;
    assert_eq!(ended[1], b.turn);
    assert_eq!(
        kinds(&events)[3..],
        [
            "turn.ended",
            "queue.held",
            "turn.started",
            "text.delta",
            "text.done",
            "turn.ended"
        ]
    );
    e.idle().await;
    assert_eq!(
        e.queue().await,
        (vec!["echo a".into()], Some(QueueHold::Stopped))
    );

    // Idle: send now starts it at once; the queue stays held.
    let idle = e
        .kernel
        .queue_send_now(&e.thread, a.entry.as_ref().unwrap())
        .await
        .unwrap();
    assert!(!idle.queued);
    let (_, ended) = e.ends(events.last().unwrap().seq, 1).await;
    assert_eq!(ended, [a.turn]);
    e.idle().await;
    assert_eq!(e.queue().await, (vec![], Some(QueueHold::Stopped)));
}

#[tokio::test]
async fn sending_resumes_a_held_queue_after_the_new_message() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    let a = e.send("echo a").await;
    e.kernel.thread_cancel(&e.thread).await.unwrap();
    let held = e.until(0, "queue.held").await;
    e.idle().await;

    let fresh = e.send("echo fresh").await;
    assert!(
        !fresh.queued,
        "an idle thread starts the new message at once"
    );
    let (events, ended) = e.ends(held.last().unwrap().seq, 2).await;
    assert_eq!(ended, [fresh.turn, a.turn]);
    assert_eq!(kinds(&events)[..2], ["queue.resumed", "turn.started"]);
}

async fn steer(e: &Env, text: &str) -> ThreadSteerResult {
    e.kernel
        .thread_steer(&e.thread, UserInput::text(text))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_steer_reaches_the_running_turn_where_the_provider_takes_it() {
    let e = env().await;
    let sent = e.send("steer").await;
    e.until(0, "turn.started").await;
    let r = steer(&e, "go left").await;
    assert_eq!(r.status, SteerStatus::Accepted);
    assert_eq!(r.turn, sent.turn);
    let events = e.until(0, "turn.ended").await;
    assert_eq!(
        kinds(&events),
        ["turn.started", "input.steered", "text.done", "turn.ended"]
    );
    let ThreadEvent::InputSteered(s) = &events[1].event else {
        panic!()
    };
    assert_eq!(
        (s.turn.clone(), s.input.text.as_str()),
        (sent.turn, "go left")
    );
    assert_eq!(turn_ended(&events).outcome, TurnOutcome::Done);
}

#[tokio::test]
async fn an_untaken_steer_goes_to_the_front_of_the_queue() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    e.send("echo later").await;
    assert_eq!(steer(&e, "echo now").await.status, SteerStatus::Accepted);
    e.kernel.thread_cancel(&e.thread).await.unwrap();
    let events = e.until(0, "queue.held").await;
    assert_eq!(
        kinds(&events),
        [
            "turn.started",
            "queue.added",
            "turn.ended",
            "queue.added",
            "queue.held"
        ]
    );
    let ThreadEvent::QueueAdded(back) = &events[3].event else {
        panic!()
    };
    assert!(back.front);
    assert_eq!(back.reason, Some(QueueAddReason::NotSteered));
    e.idle().await;
    assert_eq!(
        e.queue().await,
        (
            vec!["echo now".into(), "echo later".into()],
            Some(QueueHold::Stopped)
        )
    );
}

#[tokio::test]
async fn an_untaken_steer_runs_next_after_a_clean_turn() {
    let e = env().await;
    e.send("approve").await;
    let events = e.until(0, "approval.requested").await;
    assert_eq!(steer(&e, "echo after").await.status, SteerStatus::Accepted);
    let ThreadEvent::ApprovalRequested(req) = &events.last().unwrap().event else {
        panic!()
    };
    e.kernel
        .respond_approval(&req.approval.id, Decision::Allow, None)
        .await
        .unwrap();
    let (events, _) = e.ends(0, 2).await;
    let started: Vec<_> = events
        .iter()
        .filter_map(|ev| match &ev.event {
            ThreadEvent::TurnStarted(t) => Some(t.input.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(started, ["approve", "echo after"]);
}

#[tokio::test]
async fn a_model_without_steer_or_a_message_with_attachments_is_queued_first() {
    use kumikit_protocol::model::Attachment;
    let e = env().await;
    e.kernel
        .thread_update(&e.thread, None, Some("two".into()), None, None, None, None)
        .await
        .unwrap();
    e.send("block").await;
    e.until(0, "turn.started").await;
    e.send("echo b").await;
    let r = steer(&e, "echo a").await;
    assert_eq!(r.status, SteerStatus::Queued);
    assert!(r.entry.is_some());
    assert_eq!(
        e.queue().await,
        (vec!["echo a".into(), "echo b".into()], None)
    );

    // Model `one` can be steered, but not with attachments.
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    let input = UserInput {
        text: "look".into(),
        attachments: vec![Attachment {
            name: "a.png".into(),
            mime_type: "image/png".into(),
            data: "AA==".into(),
            width: None,
            height: None,
        }],
    };
    let r = e.kernel.thread_steer(&e.thread, input).await.unwrap();
    assert_eq!(r.status, SteerStatus::Queued);
}

#[tokio::test]
async fn steering_an_idle_thread_is_a_conflict() {
    let e = env().await;
    let err = e
        .kernel
        .thread_steer(&e.thread, UserInput::text("hi"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
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
    let t = e
        .kernel
        .thread_create(&ws, None, None, None, None)
        .await
        .unwrap();
    assert_eq!((t.provider.as_str(), t.model.as_str()), ("fake", "one"));
    assert_eq!(t.permission_mode, None);
    let t = e
        .kernel
        .thread_create(
            &ws,
            Some("fake".into()),
            Some("two".into()),
            Some("T".into()),
            Some(kumikit_protocol::model::PermissionMode::AcceptEdits),
        )
        .await
        .unwrap();
    assert_eq!(t.model, "two");
    assert_eq!(
        t.permission_mode,
        Some(kumikit_protocol::model::PermissionMode::AcceptEdits)
    );
    let err = e
        .kernel
        .thread_create(&ws, None, Some("three".into()), None, None)
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidParams);
    let err = e
        .kernel
        .thread_create(&ws, Some("other".into()), None, None, None)
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
        .thread_create(&ws.id, None, None, None, None)
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
        .thread_send(&e.thread, UserInput::text("echo c"))
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

#[tokio::test]
async fn unregistering_a_provider_cancels_its_running_turn() {
    let e = env().await;
    e.send("block").await;
    e.until(0, "turn.started").await;
    assert!(e.kernel.unregister_provider("fake"));
    let events = e.until(0, "turn.ended").await;
    assert_eq!(turn_ended(&events).outcome, TurnOutcome::Cancelled);
    assert!(!e.kernel.unregister_provider("fake"), "already gone");
    let err = e
        .kernel
        .thread_send(&e.thread, UserInput::text("echo x"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::PluginFailed);
}

#[tokio::test]
async fn a_forgotten_session_is_started_again_with_resume() {
    let e = env().await;
    e.send("echo a").await;
    let first = e.until(0, "turn.ended").await;
    e.kernel.forget_session(&e.thread);
    e.send("echo b").await;
    e.until(first.last().unwrap().seq, "turn.ended").await;
    let resumes = e.fake.resumes.lock().unwrap().clone();
    let session = format!("session-{}", e.thread);
    assert_eq!(resumes, [None, Some(session)]);
}

#[tokio::test]
async fn providers_are_listed_with_their_models() {
    let e = env().await;
    let list = e.kernel.provider_list();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (
            list[0].id.as_str(),
            list[0].name.as_str(),
            list[0].plugin.as_str()
        ),
        ("fake", "fake", "fake")
    );
    let models = e.kernel.provider_models("fake").await.unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["one", "two"]);
    let err = e.kernel.provider_models("nope").await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn workspace_changes_follow_thread_status() {
    use kumikit_protocol::WorkspaceEvent;
    let e = env().await;
    let mut rx = e.kernel.workspace_changes();
    e.send("echo hi").await;
    e.until(0, "turn.ended").await;
    // A turn moves the thread to running and back to idle; text events change nothing.
    let mut seen = Vec::new();
    for _ in 0..2 {
        let ev = rx.recv().await.unwrap();
        seen.push((ev.kind(), ev.thread().status));
    }
    assert_eq!(
        seen,
        [
            ("thread.updated", ThreadStatus::Running),
            ("thread.updated", ThreadStatus::Idle)
        ]
    );
    e.kernel.thread_archive(&e.thread).await.unwrap();
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, WorkspaceEvent::ThreadArchived(_)));
    assert_eq!(ev.thread().status, ThreadStatus::Archived);
    let ws = ev.thread().workspace.clone();
    let created = e
        .kernel
        .thread_create(&ws, None, None, Some("second".into()), None)
        .await
        .unwrap();
    let ev = rx.recv().await.unwrap();
    assert!(matches!(ev, WorkspaceEvent::ThreadCreated(_)));
    assert_eq!(ev.thread(), &created);
    assert!(rx.try_recv().is_err(), "no other change");
}

#[tokio::test]
async fn thread_update_changes_mode_and_restarts_idle_session() {
    use kumikit_protocol::model::PermissionMode;

    let e = env().await;
    e.send("echo a").await;
    let first = e.until(0, "turn.ended").await;
    assert_eq!(*e.fake.resumes.lock().unwrap(), vec![None]);

    let updated = e
        .kernel
        .thread_update(
            &e.thread,
            None,
            Some("two".into()),
            Some(PermissionMode::FullAccess),
            Some("Updated".into()),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(updated.model, "two");
    assert_eq!(updated.permission_mode, Some(PermissionMode::FullAccess));
    assert_eq!(updated.title.as_deref(), Some("Updated"));

    // Next turn restarts the session with resume and the updated config.
    e.send("echo b").await;
    e.until(first.last().unwrap().seq, "turn.ended").await;
    let expected = format!("session-{}", e.thread);
    assert_eq!(*e.fake.resumes.lock().unwrap(), vec![None, Some(expected)]);

    // Resetting back to Standard clears the stored override.
    let reset = e
        .kernel
        .thread_update(
            &e.thread,
            None,
            None,
            Some(PermissionMode::Standard),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(reset.permission_mode, None);
}

#[tokio::test]
async fn thread_send_validates_attachments() {
    use kumikit_protocol::model::Attachment;

    let e = env().await;
    let bad = e
        .kernel
        .thread_send(
            &e.thread,
            UserInput {
                text: "hi".into(),
                attachments: vec![Attachment {
                    name: "img.png".into(),
                    mime_type: "image/png".into(),
                    data: "not-valid-base64!!!".into(),
                    width: None,
                    height: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(bad.code(), ErrorCode::InvalidParams);

    e.kernel
        .thread_send(
            &e.thread,
            UserInput {
                text: "echo ok".into(),
                attachments: vec![Attachment {
                    name: "shot.png".into(),
                    mime_type: "image/png".into(),
                    data: "iVBORw0KGgo=".into(),
                    width: Some(10),
                    height: Some(10),
                }],
            },
        )
        .await
        .unwrap();
    e.until(0, "turn.ended").await;
}

/// Second fake provider that records every prompt text and resume id it receives.
#[derive(Debug, Default)]
struct RecordingProvider {
    id: String,
    prompts: Mutex<Vec<String>>,
    resumes: Mutex<Vec<Option<String>>>,
    fail_summary: std::sync::atomic::AtomicBool,
    sent: Notify,
}

impl RecordingProvider {
    fn new(id: &str) -> Arc<Self> {
        Arc::new(RecordingProvider {
            id: id.to_owned(),
            ..Default::default()
        })
    }
}

impl Provider for RecordingProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>> {
        Box::pin(async {
            Ok(vec![Model {
                id: "alpha".into(),
                label: "Alpha".into(),
                options: vec![],
                capabilities: vec![],
            }])
        })
    }
    fn start_session(&self, config: SessionConfig) -> BoxFuture<'_, ProviderResult<String>> {
        Box::pin(async move {
            self.resumes.lock().unwrap().push(config.resume);
            Ok(format!("{}-session-{}", self.id, config.thread))
        })
    }
    fn send(
        &self,
        _session: String,
        input: UserInput,
        sink: EventSink,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async move {
            self.prompts.lock().unwrap().push(input.text.clone());
            self.sent.notify_waiters();
            if input.text.ends_with("block") {
                return std::future::pending().await;
            }
            if input.text == crate::handoff::SUMMARY_PROMPT {
                if self.fail_summary.load(Ordering::SeqCst) {
                    return Err(ProviderError::new("summary unavailable"));
                }
                sink.emit(ProviderEvent::TextDone {
                    text: "Summary: user wants feature X.".into(),
                })
                .await
                .map_err(|e| ProviderError::new(e.to_string()))?;
                return Ok(());
            }
            sink.emit(ProviderEvent::TextDone {
                text: format!("reply from {}", self.id),
            })
            .await
            .map_err(|e| ProviderError::new(e.to_string()))?;
            Ok(())
        })
    }
    fn respond_approval(
        &self,
        _s: String,
        _a: ApprovalId,
        _d: Decision,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async { Ok(()) })
    }
    fn cancel(&self, _s: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn end_session(&self, _s: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn switch_provider_hands_off_transcript_and_records_attribution() {
    let e = env().await;
    let other = RecordingProvider::new("other");
    e.kernel.register_provider(other.clone()).unwrap();

    e.send("echo first answer").await;
    let first = e.until(0, "turn.ended").await;
    let first_start = first
        .iter()
        .find_map(|ev| match &ev.event {
            ThreadEvent::TurnStarted(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(first_start.provider.as_deref(), Some("fake"));
    assert_eq!(first_start.model.as_deref(), Some("one"));

    let switched = e
        .kernel
        .thread_update(
            &e.thread,
            Some("other".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(switched.provider, "other");
    assert_eq!(switched.model, "alpha");

    let after_switch = first.last().unwrap().seq;
    e.send("continue the work").await;
    let second = e.until(after_switch, "turn.ended").await;

    // `other` started a fresh session and received the earlier exchange in its prompt.
    assert_eq!(*other.resumes.lock().unwrap(), vec![None]);
    let prompts = other.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("[Conversation handoff from fake (one)]"));
    assert!(prompts[0].contains("echo first answer"));
    assert!(prompts[0].contains("first answer"));
    assert!(prompts[0].ends_with("continue the work"));

    assert!(second.iter().any(|ev| matches!(
        &ev.event,
        ThreadEvent::ThreadSwitched(s)
            if s.from.provider == "fake"
                && s.to.provider == "other"
                && s.handoff == HandoffMode::Transcript
    )));
    assert!(second.iter().any(|ev| matches!(
        &ev.event,
        ThreadEvent::HandoffDelivered(d)
            if d.from.provider == "fake"
                && d.to.provider == "other"
                && d.messages == 2
                && !d.truncated
                && d.rendered.contains("first answer")
    )));
    let second_start = second
        .iter()
        .find_map(|ev| match &ev.event {
            ThreadEvent::TurnStarted(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(second_start.provider.as_deref(), Some("other"));
    assert_eq!(second_start.model.as_deref(), Some("alpha"));

    // Next turn on `other` does NOT resend the handoff.
    let seq = second.last().unwrap().seq;
    e.send("plain follow up").await;
    let third = e.until(seq, "turn.ended").await;
    assert_eq!(other.prompts.lock().unwrap()[1], "plain follow up");
    assert!(
        !third
            .iter()
            .any(|ev| matches!(&ev.event, ThreadEvent::HandoffDelivered(_)))
    );
}

#[tokio::test]
async fn cancelled_first_turn_after_switch_resends_handoff() {
    let e = env().await;
    let other = RecordingProvider::new("other");
    e.kernel.register_provider(other.clone()).unwrap();

    e.send("echo prior context").await;
    let first = e.until(0, "turn.ended").await;

    e.kernel
        .thread_update(
            &e.thread,
            Some("other".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    // First turn after switch blocks and is cancelled once the provider receives it.
    let seq = first.last().unwrap().seq;
    let entered = other.sent.notified();
    tokio::pin!(entered);
    entered.as_mut().enable();
    e.send("block").await;
    entered.await;
    e.kernel.thread_cancel(&e.thread).await.unwrap();
    let cancelled = e.until(seq, "turn.ended").await;
    assert!(
        !cancelled
            .iter()
            .any(|ev| matches!(&ev.event, ThreadEvent::HandoffDelivered(_)))
    );

    // Second turn must send the handoff again and mark it delivered on completion.
    let seq = cancelled.last().unwrap().seq;
    e.send("try again").await;
    let done = e.until(seq, "turn.ended").await;
    let prompts = other.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("prior context"));
    assert!(prompts[1].ends_with("try again"));
    assert_eq!(*other.resumes.lock().unwrap(), vec![None, None]);
    assert!(
        done.iter()
            .any(|ev| matches!(&ev.event, ThreadEvent::HandoffDelivered(_)))
    );
}

#[tokio::test]
async fn summary_handoff_runs_summary_turn_and_falls_back_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::open(config(&dir)).await.unwrap();
    let p1 = RecordingProvider::new("p1");
    let p2 = RecordingProvider::new("p2");
    kernel.register_provider(p1.clone()).unwrap();
    kernel.register_provider(p2.clone()).unwrap();
    let ws = kernel
        .workspace_open(dir.path().to_str().unwrap())
        .await
        .unwrap();

    // 1) Successful summary handoff.
    let t1 = kernel
        .thread_create(&ws.id, Some("p1".into()), None, None, None)
        .await
        .unwrap()
        .id;
    let e1 = Env {
        dir,
        kernel: kernel.clone(),
        fake: Arc::new(Fake::default()),
        thread: t1.clone(),
    };
    e1.send("build feature X").await;
    let first = e1.until(0, "turn.ended").await;
    let seq = first.last().unwrap().seq;

    kernel
        .thread_update(
            &t1,
            Some("p2".into()),
            None,
            None,
            None,
            Some(HandoffMode::Summary),
            None,
        )
        .await
        .unwrap();
    e1.send("continue").await;
    let after = e1.until(seq, "handoff.delivered").await;
    assert!(after.iter().any(|ev| matches!(
        &ev.event,
        ThreadEvent::TurnStarted(ts) if ts.purpose == Some(TurnPurpose::HandoffSummary)
    )));
    assert!(after.iter().any(|ev| matches!(
        &ev.event,
        ThreadEvent::ThreadSwitched(s) if s.handoff == HandoffMode::Summary
    )));
    let p2_prompt = p2.prompts.lock().unwrap()[0].clone();
    assert!(p2_prompt.contains("## Handoff summary"));
    assert!(p2_prompt.contains("Summary: user wants feature X."));
    assert!(p2_prompt.contains("build feature X"));
    assert!(!p2_prompt.contains(crate::handoff::SUMMARY_PROMPT));

    // 2) Failing summary falls back to transcript mode.
    p1.fail_summary.store(true, Ordering::SeqCst);
    let t2 = kernel
        .thread_create(&ws.id, Some("p1".into()), None, None, None)
        .await
        .unwrap()
        .id;
    let e2 = Env {
        dir: tempfile::tempdir().unwrap(),
        kernel: kernel.clone(),
        fake: Arc::new(Fake::default()),
        thread: t2.clone(),
    };
    e2.send("build feature Y").await;
    let first2 = e2.until(0, "turn.ended").await;
    let seq2 = first2.last().unwrap().seq;

    kernel
        .thread_update(
            &t2,
            Some("p2".into()),
            None,
            None,
            None,
            Some(HandoffMode::Summary),
            None,
        )
        .await
        .unwrap();
    e2.send("continue Y").await;
    let after2 = e2.until(seq2, "handoff.delivered").await;
    assert!(after2.iter().any(|ev| matches!(
        &ev.event,
        ThreadEvent::ThreadSwitched(s) if s.handoff == HandoffMode::Transcript
    )));
    let p2_prompt2 = p2.prompts.lock().unwrap()[1].clone();
    assert!(p2_prompt2.contains("automatic handoff summary could not be generated"));
    assert!(p2_prompt2.contains("build feature Y"));
}

#[tokio::test]
async fn switch_provider_refused_while_running() {
    let e = env().await;
    let other = RecordingProvider::new("other");
    e.kernel.register_provider(other).unwrap();
    e.send("block").await;
    e.until(0, "turn.started").await;
    let err = e
        .kernel
        .thread_update(
            &e.thread,
            Some("other".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), ErrorCode::Conflict);
    e.kernel.thread_cancel(&e.thread).await.unwrap();
    e.until(0, "turn.ended").await;
}

/// ADR 0030 title hint: only the first turn of an untitled thread gets it, and the log
/// keeps the user's own text.
#[tokio::test]
async fn first_turn_of_an_untitled_thread_gets_the_title_hint() {
    let e = env().await;
    let rec = RecordingProvider::new("other");
    e.kernel.register_provider(rec.clone()).unwrap();
    e.kernel
        .thread_update(
            &e.thread,
            Some("other".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    e.send("hello there").await;
    let first = e.until(0, "turn.ended").await;
    e.send("and again").await;
    let second = e.until(first.last().unwrap().seq, "turn.ended").await;

    let prompts = rec.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].starts_with(crate::turns::TITLE_HINT));
    assert!(prompts[0].ends_with("hello there"));
    assert_eq!(prompts[1], "and again");
    let logged: Vec<String> = first
        .iter()
        .chain(second.iter())
        .filter_map(|ev| match &ev.event {
            ThreadEvent::TurnStarted(t) => Some(t.input.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        logged,
        vec!["hello there".to_string(), "and again".to_string()]
    );
}

#[tokio::test]
async fn a_titled_thread_gets_no_title_hint() {
    let e = env().await;
    let rec = RecordingProvider::new("other");
    e.kernel.register_provider(rec.clone()).unwrap();
    e.kernel
        .thread_update(
            &e.thread,
            Some("other".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        e.kernel
            .thread_suggest_title(&e.thread, "Planning the release")
            .await
            .unwrap()
    );

    e.send("plan it").await;
    e.until(0, "turn.ended").await;

    assert_eq!(*rec.prompts.lock().unwrap(), vec!["plan it".to_string()]);
}

/// A title given at creation (a client's `thread.create`, or `start_thread`) is kept: an
/// agent's suggestion doesn't replace it, and the title hint is not sent for it.
#[tokio::test]
async fn a_title_given_at_creation_is_kept_and_gets_no_hint() {
    let e = env().await;
    let rec = RecordingProvider::new("other");
    e.kernel.register_provider(rec.clone()).unwrap();
    let ws = e
        .kernel
        .thread_get(&e.thread)
        .await
        .unwrap()
        .thread
        .workspace;
    let t = e
        .kernel
        .thread_create(
            &ws,
            Some("other".into()),
            None,
            Some("Release review".into()),
            None,
        )
        .await
        .unwrap();

    assert!(
        !e.kernel
            .thread_suggest_title(&t.id, "Planning")
            .await
            .unwrap()
    );
    let snap = e.kernel.thread_get(&t.id).await.unwrap();
    assert_eq!(snap.thread.title.as_deref(), Some("Release review"));
    assert!(
        !e.kernel
            .wants_title_hint(&t.id, &TurnId("turn-1".into()))
            .await
            .unwrap()
    );
}

// ---- usage statistics and plan limits (ADR 0035) ----

fn summary_params(days: u32) -> kumikit_protocol::methods::UsageSummaryParams {
    kumikit_protocol::methods::UsageSummaryParams {
        provider: None,
        days,
        tz_offset_minutes: None,
    }
}

#[tokio::test]
async fn a_turns_last_usage_report_counts_once_and_outlives_its_thread() {
    let e = env().await;
    let mut usage = e.kernel.usage_changes();
    e.send("usage").await;
    e.until(0, "turn.ended").await;
    let s = e.kernel.usage_summary(summary_params(1)).await.unwrap();
    assert_eq!(s.totals.turns, 1);
    assert_eq!((s.totals.input_tokens, s.totals.output_tokens), (10, 2));
    assert_eq!(s.totals.cost, Some(0.25));
    assert_eq!(s.threads, 1);
    assert_eq!(s.models[0].provider, "fake");
    assert_eq!(s.daily.len(), 1);
    let ev = usage.recv().await.unwrap();
    assert_eq!(ev.kind(), "usage.recorded");

    e.kernel.thread_delete(&e.thread).await.unwrap();
    let s = e.kernel.usage_summary(summary_params(1)).await.unwrap();
    assert_eq!(s.totals.turns, 1, "deleting a thread keeps its usage");
}

#[tokio::test]
async fn usage_by_model_adds_up_and_breaks_down() {
    let e = env().await;
    e.send("models").await;
    let events = e.until(0, "turn.ended").await;
    let usage = events
        .iter()
        .find_map(|ev| match &ev.event {
            ThreadEvent::Usage(u) => Some(u.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!((usage.input_tokens, usage.output_tokens), (120, 2));
    assert_eq!(usage.cached_input_tokens, Some(85));
    assert_eq!(usage.cost, Some(1.0));
    assert_eq!(usage.models.len(), 2);
    let s = e.kernel.usage_summary(summary_params(30)).await.unwrap();
    let rows: Vec<(&str, u64)> = s
        .models
        .iter()
        .map(|r| (r.model.as_str(), r.totals.input_tokens))
        .collect();
    assert_eq!(rows, vec![("one", 100), ("small", 20)]);
    assert_eq!(s.totals.cached_input_tokens, 85);
}

#[tokio::test]
async fn summary_days_are_bounded() {
    let e = env().await;
    for days in [0, crate::usage::MAX_SUMMARY_DAYS + 1] {
        let err = e
            .kernel
            .usage_summary(summary_params(days))
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidParams);
    }
}

#[tokio::test]
async fn limits_reported_in_a_turn_are_stored_for_its_provider() {
    let e = env().await;
    let before = e.kernel.usage_limits(None).await.unwrap();
    assert_eq!(before.len(), 1);
    assert!(before[0].limits.is_none() && before[0].observed.is_none());
    let mut usage = e.kernel.usage_changes();
    e.send("limits").await;
    e.until(0, "turn.ended").await;
    let ev = usage.recv().await.unwrap();
    let kumikit_protocol::UsageEvent::Limits(snap) = ev else {
        panic!("{ev:?}")
    };
    assert_eq!(snap.provider, "fake");
    let stored = e.kernel.usage_limits(Some("fake")).await.unwrap();
    assert_eq!(stored, vec![snap]);
    assert_eq!(
        stored[0].limits.as_ref().unwrap().windows[0].used_percent,
        42.0
    );
    let err = e.kernel.usage_limits(Some("nope")).await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn a_refresh_reads_once_per_interval() {
    let e = env().await;
    let first = e.kernel.usage_refresh("fake").await.unwrap();
    assert_eq!(first.limits.unwrap().windows[0].used_percent, 10.0);
    // Within the interval the stored snapshot comes back without asking again.
    let again = e.kernel.usage_refresh("fake").await.unwrap();
    assert_eq!(again.limits.unwrap().windows[0].used_percent, 10.0);
    assert_eq!(e.fake.limit_reads.load(Ordering::SeqCst), 1);
    let err = e.kernel.usage_refresh("nope").await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::NotFound);
}

#[derive(Debug)]
struct NoLimits;

impl Provider for NoLimits {
    fn id(&self) -> &str {
        "plain"
    }
    fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn start_session(&self, _c: SessionConfig) -> BoxFuture<'_, ProviderResult<String>> {
        Box::pin(async { Ok("s".into()) })
    }
    fn send(&self, _s: String, _i: UserInput, _k: EventSink) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async { Ok(()) })
    }
    fn respond_approval(
        &self,
        _s: String,
        _a: ApprovalId,
        _d: Decision,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async { Ok(()) })
    }
    fn cancel(&self, _s: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn end_session(&self, _s: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn a_provider_without_a_reader_cannot_refresh() {
    let e = env().await;
    e.kernel.register_provider(Arc::new(NoLimits)).unwrap();
    let err = e.kernel.usage_refresh("plain").await.unwrap_err();
    assert_eq!(err.code(), ErrorCode::Unsupported);
    let list = e.kernel.provider_list();
    let flags: Vec<(&str, Option<bool>)> = list
        .iter()
        .map(|p| (p.id.as_str(), p.reads_limits))
        .collect();
    assert_eq!(flags, vec![("fake", Some(true)), ("plain", None)]);
}

#[derive(Debug, Default)]
struct ManualClock {
    secs: std::sync::atomic::AtomicI64,
}

impl ManualClock {
    fn advance(&self, by: i64) {
        self.secs.fetch_add(by, Ordering::SeqCst);
    }
}

impl crate::clock::Clock for ManualClock {
    fn now(&self) -> kumikit_protocol::model::Timestamp {
        let secs = self.secs.load(Ordering::SeqCst);
        let base = time::macros::datetime!(2026-11-15 10:00:00 UTC);
        crate::clock::format_time(base.saturating_add(time::Duration::seconds(secs)))
    }
}

async fn env_with_clock(clock: Arc<ManualClock>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let cfg = KernelConfig {
        clock,
        ids: Arc::new(SequentialIds::new("ID")),
        ..KernelConfig::new(dir.path().join("core.sqlite"))
    };
    let kernel = Kernel::open(cfg).await.unwrap();
    let fake = Arc::new(Fake::default());
    kernel.register_provider(fake.clone()).unwrap();
    let ws = kernel
        .workspace_open(dir.path().to_str().unwrap())
        .await
        .unwrap();
    let thread = kernel
        .thread_create(&ws.id, None, None, None, None)
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

#[tokio::test]
async fn a_refresh_runs_again_after_the_cooldown_expires() {
    let clock = Arc::new(ManualClock::default());
    let e = env_with_clock(clock.clone()).await;

    let first = e.kernel.usage_refresh("fake").await.unwrap();
    assert_eq!(first.limits.unwrap().windows[0].used_percent, 10.0);
    assert_eq!(e.fake.limit_reads.load(Ordering::SeqCst), 1);

    clock.advance(59);
    let cached = e.kernel.usage_refresh("fake").await.unwrap();
    assert_eq!(cached.limits.unwrap().windows[0].used_percent, 10.0);
    assert_eq!(e.fake.limit_reads.load(Ordering::SeqCst), 1);

    clock.advance(1);
    let refreshed = e.kernel.usage_refresh("fake").await.unwrap();
    assert_eq!(refreshed.limits.unwrap().windows[0].used_percent, 11.0);
    assert_eq!(e.fake.limit_reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_failed_refresh_does_not_start_the_cooldown() {
    let clock = Arc::new(ManualClock::default());
    let e = env_with_clock(clock).await;
    e.fake.fail_limit_reads.store(2, Ordering::SeqCst);

    let err1 = e.kernel.usage_refresh("fake").await.unwrap_err();
    assert_eq!(err1.code(), ErrorCode::PluginFailed);
    // An immediate retry still runs the reader and surfaces the error instead of
    // returning an empty/stale snapshot as success.
    let err2 = e.kernel.usage_refresh("fake").await.unwrap_err();
    assert_eq!(err2.code(), ErrorCode::PluginFailed);
    assert_eq!(e.fake.limit_reads.load(Ordering::SeqCst), 2);

    // Once the reader recovers, an immediate retry succeeds and starts the cooldown.
    let ok = e.kernel.usage_refresh("fake").await.unwrap();
    assert_eq!(ok.limits.unwrap().windows[0].used_percent, 12.0);
    assert_eq!(e.fake.limit_reads.load(Ordering::SeqCst), 3);

    let cached = e.kernel.usage_refresh("fake").await.unwrap();
    assert_eq!(cached.limits.unwrap().windows[0].used_percent, 12.0);
    assert_eq!(e.fake.limit_reads.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn concurrent_refreshes_run_the_reader_once() {
    let clock = Arc::new(ManualClock::default());
    let e = env_with_clock(clock).await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    *e.fake.limit_read_enter.lock().unwrap() = Some(entered.clone());
    *e.fake.limit_read_release.lock().unwrap() = Some(release.clone());

    let k1 = e.kernel.clone();
    let k2 = e.kernel.clone();
    let entered_wait = entered.notified();
    tokio::pin!(entered_wait);
    entered_wait.as_mut().enable();

    let first = tokio::spawn(async move { k1.usage_refresh("fake").await });
    entered_wait.await;
    let second = tokio::spawn(async move { k2.usage_refresh("fake").await });
    // Let the second task reach the per-provider refresh gate before releasing the first.
    tokio::task::yield_now().await;
    release.notify_waiters();

    let (r1, r2) = tokio::join!(first, second);
    let s1 = r1.unwrap().unwrap();
    let s2 = r2.unwrap().unwrap();
    assert_eq!(s1.limits.unwrap().windows[0].used_percent, 10.0);
    assert_eq!(s2.limits.unwrap().windows[0].used_percent, 10.0);
    assert_eq!(e.fake.limit_reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelled_turn_keeps_partial_usage() {
    let e = env().await;
    e.send("usage-then-block").await;
    let evs = e.until(0, "usage").await;
    e.kernel.thread_cancel(&e.thread).await.unwrap();
    let ended = e.until(evs.last().unwrap().seq, "turn.ended").await;
    assert_eq!(turn_ended(&ended).outcome, TurnOutcome::Cancelled);

    let s = e.kernel.usage_summary(summary_params(1)).await.unwrap();
    assert_eq!(s.totals.turns, 1);
    assert_eq!((s.totals.input_tokens, s.totals.output_tokens), (40, 7));
    assert_eq!(s.totals.cost, Some(0.15));
}
