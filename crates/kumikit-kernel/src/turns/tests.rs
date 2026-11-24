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
        .thread_update(&e.thread, Some("two".into()), None, None)
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
            Some("two".into()),
            Some(PermissionMode::FullAccess),
            Some("Updated".into()),
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
        .thread_update(&e.thread, None, Some(PermissionMode::Standard), None)
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
