//! Turns: one running turn per thread, a FIFO queue for further messages, cancel, and
//! approvals (`docs/spec/03-kernel.md`, turn rules; `docs/spec/05-providers.md`).

use std::collections::VecDeque;
use std::sync::Arc;

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::event::{ApprovalResolved, TurnEnded, TurnStarted};
use kumikit_protocol::methods::ThreadSendResult;
use kumikit_protocol::model::{
    ApprovalId, Decision, Resolution, Thread, ThreadId, TurnId, TurnOutcome, UserInput, WorkspaceId,
};
use tokio::sync::oneshot;

use crate::error::{KernelError, KernelResult};
use crate::kernel::Kernel;
use crate::provider::{EventSink, Provider, SessionConfig};
use crate::repo;

/// In-memory state of a thread with a running turn.
#[derive(Debug)]
pub(crate) struct ThreadRun {
    /// The running turn.
    pub(crate) turn: TurnId,
    /// Fires to cancel the running turn. Dropping it (kernel close) abandons the turn.
    cancel: Option<oneshot::Sender<()>>,
    /// Messages waiting for the running turn to end.
    queue: VecDeque<(TurnId, UserInput)>,
}

/// How a turn's provider call ended.
enum Ending {
    Done,
    Failed(String),
    Cancelled,
    /// The kernel is closing: leave the turn open for recovery on the next start.
    Abandoned,
}

impl Kernel {
    /// Registers a provider. Fails with `conflict` if its id is taken.
    pub fn register_provider(&self, provider: Arc<dyn Provider>) -> KernelResult<()> {
        let mut providers = self.lock_providers();
        if providers.iter().any(|p| p.id() == provider.id()) {
            return Err(KernelError::Conflict(format!(
                "provider {} is already registered",
                provider.id()
            )));
        }
        providers.push(provider);
        Ok(())
    }

    fn provider(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.lock_providers().iter().find(|p| p.id() == id).cloned()
    }

    /// Creates a thread. `provider` defaults to the only registered provider, `model` to
    /// the provider's first model.
    pub async fn thread_create(
        &self,
        workspace: &WorkspaceId,
        provider: Option<String>,
        model: Option<String>,
        title: Option<String>,
    ) -> KernelResult<Thread> {
        let provider = match provider {
            Some(id) => self
                .provider(&id)
                .ok_or_else(|| KernelError::InvalidParams(format!("no provider {id}")))?,
            None => {
                let providers = self.lock_providers();
                match providers.as_slice() {
                    [only] => Arc::clone(only),
                    [] => return Err(KernelError::Unsupported("no provider is available".into())),
                    _ => {
                        return Err(KernelError::InvalidParams(
                            "several providers are available; name one".into(),
                        ));
                    }
                }
            }
        };
        let models = provider
            .list_models()
            .await
            .map_err(|e| KernelError::PluginFailed(e.message))?;
        let model = match model {
            Some(m) if models.iter().any(|x| x.id == m) => m,
            Some(m) => {
                return Err(KernelError::InvalidParams(format!(
                    "provider {} has no model {m}",
                    provider.id()
                )));
            }
            None => models.first().map(|m| m.id.clone()).ok_or_else(|| {
                KernelError::PluginFailed(format!("provider {} has no models", provider.id()))
            })?,
        };
        self.thread_insert(workspace, provider.id().to_owned(), model, title)
            .await
    }

    /// Sends a user message: starts a turn, or queues it behind the running one.
    pub async fn thread_send(
        &self,
        thread: &ThreadId,
        input: UserInput,
    ) -> KernelResult<ThreadSendResult> {
        let row = self.thread_row(thread).await?;
        if row.archived {
            return Err(KernelError::Conflict(format!(
                "thread {thread} is archived"
            )));
        }
        let provider = self.provider(&row.thread.provider).ok_or_else(|| {
            KernelError::PluginFailed(format!("provider {} is not available", row.thread.provider))
        })?;
        let turn = TurnId(self.new_id());
        let cancel_rx = {
            let mut runs = self.lock_runs();
            if let Some(run) = runs.get_mut(thread) {
                run.queue.push_back((turn.clone(), input));
                return Ok(ThreadSendResult { turn, queued: true });
            }
            let (tx, rx) = oneshot::channel();
            runs.insert(
                thread.clone(),
                ThreadRun {
                    turn: turn.clone(),
                    cancel: Some(tx),
                    queue: VecDeque::new(),
                },
            );
            rx
        };
        let kernel = self.clone();
        let (t, first) = (thread.clone(), turn.clone());
        tokio::spawn(async move {
            kernel
                .run_turns(t, row.thread, provider, first, input, cancel_rx)
                .await
        });
        Ok(ThreadSendResult {
            turn,
            queued: false,
        })
    }

    /// Cancels the running turn and drops queued messages. A thread with no running turn
    /// is left as it is.
    pub async fn thread_cancel(&self, thread: &ThreadId) -> KernelResult<()> {
        self.thread_row(thread).await?;
        if let Some(run) = self.lock_runs().get_mut(thread) {
            run.queue.clear();
            if let Some(tx) = run.cancel.take() {
                let _ = tx.send(());
            }
        }
        Ok(())
    }

    /// Answers a pending approval. The first answer wins; later ones fail with
    /// `conflict`. `by` is the answering device, if the connection has one.
    pub async fn respond_approval(
        &self,
        approval: &ApprovalId,
        decision: Decision,
        by: Option<String>,
    ) -> KernelResult<()> {
        let id = approval.clone();
        let (thread, _turn, _status) = self
            .inner
            .store
            .read(move |c| repo::approval_get(c, &id))
            .await?
            .ok_or_else(|| KernelError::NotFound(format!("no approval {approval}")))?;
        // The log arbitrates: the append fails with `conflict` unless still pending.
        self.append(
            &thread,
            ThreadEvent::ApprovalResolved(ApprovalResolved {
                approval: approval.clone(),
                decision: decision.into(),
                by,
            }),
        )
        .await?;
        let row = self.thread_row(&thread).await?;
        let session = self.lock_sessions().get(&thread).cloned();
        if let (Some(provider), Some(session)) = (self.provider(&row.thread.provider), session) {
            provider
                .respond_approval(session, approval.clone(), decision)
                .await
                .map_err(|e| KernelError::PluginFailed(e.message))?;
        }
        Ok(())
    }

    async fn run_turns(
        self,
        thread: ThreadId,
        meta: Thread,
        provider: Arc<dyn Provider>,
        mut turn: TurnId,
        mut input: UserInput,
        mut cancel: oneshot::Receiver<()>,
    ) {
        loop {
            let finished = self
                .run_one(&thread, &meta, &provider, &turn, input, &mut cancel)
                .await;
            let next = {
                let mut runs = self.lock_runs();
                let next = match (&finished, runs.get_mut(&thread)) {
                    (Ok(true), Some(run)) => run.queue.pop_front().map(|(t, i)| {
                        let (tx, rx) = oneshot::channel();
                        run.turn = t.clone();
                        run.cancel = Some(tx);
                        (t, i, rx)
                    }),
                    _ => None,
                };
                if next.is_none() {
                    runs.remove(&thread);
                }
                next
            };
            match next {
                Some((t, i, rx)) => (turn, input, cancel) = (t, i, rx),
                None => break,
            }
        }
    }

    /// Runs one turn. `Ok(true)` if it ended normally (the queue may continue),
    /// `Ok(false)` if it was abandoned, `Err` if the log could not be written.
    async fn run_one(
        &self,
        thread: &ThreadId,
        meta: &Thread,
        provider: &Arc<dyn Provider>,
        turn: &TurnId,
        input: UserInput,
        cancel: &mut oneshot::Receiver<()>,
    ) -> KernelResult<bool> {
        self.append(
            thread,
            ThreadEvent::TurnStarted(TurnStarted {
                turn: turn.clone(),
                input: input.clone(),
            }),
        )
        .await?;
        let sink = EventSink::new(
            self.clone(),
            thread.clone(),
            turn.clone(),
            provider.id().to_owned(),
        );
        let ending = match self.session_for(meta, provider).await {
            Err(message) => Ending::Failed(message),
            Ok(session) => {
                tokio::select! {
                    r = provider.send(session.clone(), input, sink.clone()) => match r {
                        Ok(()) => Ending::Done,
                        Err(e) => Ending::Failed(e.message),
                    },
                    c = &mut *cancel => {
                        provider.cancel(session).await;
                        if c.is_ok() { Ending::Cancelled } else { Ending::Abandoned }
                    }
                }
            }
        };
        sink.close();
        let (outcome, error) = match ending {
            Ending::Done => (TurnOutcome::Done, None),
            Ending::Failed(m) => (TurnOutcome::Failed, Some(m)),
            Ending::Cancelled => (TurnOutcome::Cancelled, None),
            Ending::Abandoned => return Ok(false),
        };
        // Approvals nobody answered end with the turn.
        let t = turn.clone();
        let pending = self
            .inner
            .store
            .read(move |c| repo::pending_approvals_of_turn(c, &t))
            .await?;
        for approval in pending {
            let resolved = self
                .append(
                    thread,
                    ThreadEvent::ApprovalResolved(ApprovalResolved {
                        approval,
                        decision: Resolution::Cancelled,
                        by: None,
                    }),
                )
                .await;
            match resolved {
                // A client answered between the read and this append; that answer stands.
                Ok(_) | Err(KernelError::Conflict(_)) => {}
                Err(e) => return Err(e),
            }
        }
        self.append(
            thread,
            ThreadEvent::TurnEnded(TurnEnded {
                turn: turn.clone(),
                outcome,
                error,
            }),
        )
        .await?;
        Ok(true)
    }

    /// The provider session for a thread, started on first use in this process.
    async fn session_for(
        &self,
        meta: &Thread,
        provider: &Arc<dyn Provider>,
    ) -> Result<String, String> {
        if let Some(s) = self.lock_sessions().get(&meta.id) {
            return Ok(s.clone());
        }
        let (id, ws) = (meta.id.clone(), meta.workspace.clone());
        let (row, workspace) = self
            .inner
            .store
            .read(move |c| {
                Ok::<_, KernelError>((repo::thread_require(c, &id)?, repo::workspace_get(c, &ws)?))
            })
            .await
            .map_err(|e| e.to_string())?;
        let workspace = workspace.ok_or_else(|| format!("no workspace {}", meta.workspace))?;
        let session = provider
            .start_session(SessionConfig {
                thread: meta.id.clone(),
                workspace_root: workspace.workspace.root,
                model: meta.model.clone(),
                resume: row.provider_session,
                mcp_endpoint: None,
                options: Vec::new(),
            })
            .await
            .map_err(|e| e.message)?;
        let (id, s) = (meta.id.clone(), session.clone());
        self.inner
            .store
            .write(move |c| repo::thread_set_session(c, &id, &s))
            .await
            .map_err(|e: KernelError| e.to_string())?;
        self.lock_sessions()
            .insert(meta.id.clone(), session.clone());
        Ok(session)
    }

    /// Whether a turn runs or waits for this thread.
    pub(crate) fn has_run(&self, thread: &ThreadId) -> bool {
        self.lock_runs().contains_key(thread)
    }

    /// Drops all runs (abandoning their turns) and ends all provider sessions.
    pub(crate) async fn stop_turns(&self) {
        self.lock_runs().clear();
        let sessions: Vec<(ThreadId, String)> = self.lock_sessions().drain().collect();
        for (thread, session) in sessions {
            let id = thread.clone();
            let provider = self
                .inner
                .store
                .read(move |c| repo::thread_require(c, &id))
                .await
                .ok()
                .and_then(|row| self.provider(&row.thread.provider));
            if let Some(p) = provider {
                p.end_session(session).await;
            }
        }
    }
}

#[cfg(test)]
mod tests;
