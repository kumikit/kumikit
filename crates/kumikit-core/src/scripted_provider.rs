//! Temporary: deleted in M1, see ADR 0009 (`docs/decisions/0009-temporary-scripted-provider.md`).
//!
//! A deterministic provider so M0 runs and tests without a real agent. The thread's model
//! picks the scenario:
//!
//! - `echo`: streams the input back word by word, then `text.done` and `usage`.
//! - `tool-approval`: announces a tool call, asks for approval and waits; on allow it
//!   reports (canned) tool output, on deny a failed call; then `text.done` and `usage`.
//! - `fail`: one delta, then the turn fails.
//!
//! No timers, no randomness, no processes: the tool output is canned text.

use std::collections::HashMap;
use std::sync::Mutex;

use kumikit_kernel::provider::{
    BoxFuture, EventSink, Model, Provider, ProviderError, ProviderEvent, ProviderResult,
    SessionConfig,
};
use kumikit_protocol::model::{ApprovalId, Decision, UserInput};
use serde_json::json;
use tokio::sync::Notify;

/// Provider id.
pub const ID: &str = "scripted";

/// The scripted provider.
#[derive(Debug, Default)]
pub struct ScriptedProvider {
    /// Model per session, recorded at `start_session`.
    models: Mutex<HashMap<String, String>>,
    /// Answers that arrived, possibly before the turn started waiting for them.
    decisions: Mutex<HashMap<ApprovalId, Decision>>,
    decided: Notify,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // The maps stay valid if a holder panicked.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn sink_err(e: kumikit_kernel::KernelError) -> ProviderError {
    ProviderError::new(e.to_string())
}

impl ScriptedProvider {
    async fn decision(&self, id: &ApprovalId) -> Decision {
        loop {
            // Register for the wakeup before checking, so an answer that lands between
            // the check and the await is not missed.
            let notified = self.decided.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(d) = lock(&self.decisions).remove(id) {
                return d;
            }
            notified.await;
        }
    }

    async fn echo(&self, input: &UserInput, sink: &EventSink) -> ProviderResult<()> {
        for word in input.text.split_inclusive(' ') {
            sink.emit(ProviderEvent::TextDelta {
                text: word.to_owned(),
            })
            .await
            .map_err(sink_err)?;
        }
        sink.emit(ProviderEvent::TextDone {
            text: input.text.clone(),
        })
        .await
        .map_err(sink_err)?;
        let words = u64::try_from(input.text.split_whitespace().count()).unwrap_or(u64::MAX);
        sink.emit(ProviderEvent::Usage {
            input_tokens: words,
            output_tokens: words,
            cost: None,
        })
        .await
        .map_err(sink_err)?;
        Ok(())
    }

    async fn tool_approval(&self, sink: &EventSink) -> ProviderResult<()> {
        let emit = |e: ProviderEvent| async move { sink.emit(e).await.map_err(sink_err) };
        emit(ProviderEvent::TextDelta {
            text: "I'll list the files.".into(),
        })
        .await?;
        let input = json!({ "command": "ls" });
        emit(ProviderEvent::ToolStarted {
            call: "call-1".into(),
            name: "shell".into(),
            input: input.clone(),
        })
        .await?;
        let approval = sink
            .request_approval("tool", "Run `ls` in the workspace", Some(input))
            .await
            .map_err(sink_err)?;
        let text = match self.decision(&approval).await {
            Decision::Allow | Decision::AllowAlways => {
                emit(ProviderEvent::ToolOutput {
                    call: "call-1".into(),
                    chunk: "README.md\nsrc\n".into(),
                })
                .await?;
                emit(ProviderEvent::ToolFinished {
                    call: "call-1".into(),
                    ok: true,
                    result: Some(json!({ "exitCode": 0 })),
                })
                .await?;
                "The workspace has README.md and src."
            }
            Decision::Deny => {
                emit(ProviderEvent::ToolFinished {
                    call: "call-1".into(),
                    ok: false,
                    result: Some(json!({ "denied": true })),
                })
                .await?;
                "The tool call was denied."
            }
        };
        emit(ProviderEvent::TextDelta {
            text: format!(" {text}"),
        })
        .await?;
        emit(ProviderEvent::TextDone {
            text: format!("I'll list the files. {text}"),
        })
        .await?;
        emit(ProviderEvent::Usage {
            input_tokens: 12,
            output_tokens: 9,
            cost: None,
        })
        .await?;
        Ok(())
    }
}

impl Provider for ScriptedProvider {
    fn id(&self) -> &str {
        ID
    }

    fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>> {
        Box::pin(async {
            Ok([
                ("echo", "Echo", vec![]),
                (
                    "tool-approval",
                    "Tool call with approval",
                    vec!["tools", "approvals"],
                ),
                ("fail", "Failing turn", vec![]),
            ]
            .into_iter()
            .map(|(id, label, caps)| Model {
                id: id.into(),
                label: label.into(),
                capabilities: caps.into_iter().map(str::to_owned).collect(),
            })
            .collect())
        })
    }

    fn start_session(&self, config: SessionConfig) -> BoxFuture<'_, ProviderResult<String>> {
        Box::pin(async move {
            let session = config
                .resume
                .unwrap_or_else(|| format!("scripted-{}", config.thread));
            lock(&self.models).insert(session.clone(), config.model);
            Ok(session)
        })
    }

    fn send(
        &self,
        session: String,
        input: UserInput,
        sink: EventSink,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async move {
            let model = lock(&self.models).get(&session).cloned();
            match model.as_deref() {
                Some("echo") => self.echo(&input, &sink).await,
                Some("tool-approval") => self.tool_approval(&sink).await,
                Some("fail") => {
                    sink.emit(ProviderEvent::TextDelta {
                        text: "Starting…".into(),
                    })
                    .await
                    .map_err(sink_err)?;
                    Err(ProviderError::new("scripted failure"))
                }
                Some(other) => Err(ProviderError::new(format!("unknown model {other}"))),
                None => Err(ProviderError::new(format!("unknown session {session}"))),
            }
        })
    }

    fn respond_approval(
        &self,
        _session: String,
        approval: ApprovalId,
        decision: Decision,
    ) -> BoxFuture<'_, ProviderResult<()>> {
        Box::pin(async move {
            lock(&self.decisions).insert(approval, decision);
            self.decided.notify_waiters();
            Ok(())
        })
    }

    fn cancel(&self, _session: String) -> BoxFuture<'_, ()> {
        // Nothing to stop: the kernel drops the `send` future, which ends the turn.
        Box::pin(async {})
    }

    fn end_session(&self, session: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            lock(&self.models).remove(&session);
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kumikit_kernel::{Kernel, KernelConfig, ThreadSubscription};
    use kumikit_protocol::ThreadEvent;
    use kumikit_protocol::model::{ThreadId, TurnOutcome};

    use super::*;

    async fn setup(model: &str) -> (tempfile::TempDir, Kernel, ThreadId) {
        let dir = tempfile::tempdir().unwrap();
        let kernel = Kernel::open(KernelConfig::new(dir.path().join("db")))
            .await
            .unwrap();
        kernel
            .register_provider(Arc::new(ScriptedProvider::default()))
            .unwrap();
        let ws = kernel
            .workspace_open(dir.path().to_str().unwrap())
            .await
            .unwrap();
        let t = kernel
            .thread_create(&ws.id, None, Some(model.into()), None)
            .await
            .unwrap();
        (dir, kernel, t.id)
    }

    async fn until(sub: &mut ThreadSubscription, kind: &str) -> Vec<ThreadEvent> {
        let mut out = Vec::new();
        loop {
            for e in sub.next_batch().await.unwrap().unwrap() {
                let done = e.event.kind() == kind;
                out.push(e.event);
                if done {
                    return out;
                }
            }
        }
    }

    fn kinds(events: &[ThreadEvent]) -> Vec<&'static str> {
        events.iter().map(ThreadEvent::kind).collect()
    }

    fn send(text: &str) -> UserInput {
        UserInput { text: text.into() }
    }

    #[tokio::test]
    async fn echo() {
        let (_d, k, t) = setup("echo").await;
        let mut sub = k.subscribe(&t, 0).await.unwrap();
        k.thread_send(&t, send("hello there")).await.unwrap();
        let events = until(&mut sub, "turn.ended").await;
        assert_eq!(
            kinds(&events),
            [
                "turn.started",
                "text.delta",
                "text.delta",
                "text.done",
                "usage",
                "turn.ended"
            ]
        );
        let ThreadEvent::TextDone(done) = &events[3] else {
            panic!()
        };
        assert_eq!(done.text, "hello there");
    }

    #[tokio::test]
    async fn tool_approval_allow_and_deny() {
        for (decision, ok) in [(Decision::Allow, true), (Decision::Deny, false)] {
            let (_d, k, t) = setup("tool-approval").await;
            let mut sub = k.subscribe(&t, 0).await.unwrap();
            k.thread_send(&t, send("list files")).await.unwrap();
            let first = until(&mut sub, "approval.requested").await;
            assert_eq!(
                kinds(&first),
                [
                    "turn.started",
                    "text.delta",
                    "tool.started",
                    "approval.requested"
                ]
            );
            let ThreadEvent::ApprovalRequested(req) = &first[3] else {
                panic!()
            };
            k.respond_approval(&req.approval.id, decision, None)
                .await
                .unwrap();
            let rest = until(&mut sub, "turn.ended").await;
            let expected: &[&str] = if ok {
                &[
                    "approval.resolved",
                    "tool.output",
                    "tool.finished",
                    "text.delta",
                    "text.done",
                    "usage",
                    "turn.ended",
                ]
            } else {
                &[
                    "approval.resolved",
                    "tool.finished",
                    "text.delta",
                    "text.done",
                    "usage",
                    "turn.ended",
                ]
            };
            assert_eq!(kinds(&rest), expected);
            let finished = rest.iter().find_map(|e| match e {
                ThreadEvent::ToolFinished(f) => Some(f.ok),
                _ => None,
            });
            assert_eq!(finished, Some(ok));
            k.close().await;
        }
    }

    #[tokio::test]
    async fn fail() {
        let (_d, k, t) = setup("fail").await;
        let mut sub = k.subscribe(&t, 0).await.unwrap();
        k.thread_send(&t, send("x")).await.unwrap();
        let events = until(&mut sub, "turn.ended").await;
        assert_eq!(kinds(&events), ["turn.started", "text.delta", "turn.ended"]);
        let ThreadEvent::TurnEnded(end) = &events[2] else {
            panic!()
        };
        assert_eq!(end.outcome, TurnOutcome::Failed);
        assert_eq!(end.error.as_deref(), Some("scripted failure"));
    }

    #[tokio::test]
    async fn answer_before_wait_is_kept() {
        let p = ScriptedProvider::default();
        let id = ApprovalId("a".into());
        p.respond_approval("s".into(), id.clone(), Decision::Deny)
            .await
            .unwrap();
        assert_eq!(p.decision(&id).await, Decision::Deny);
    }
}
