//! Turns: one running turn per thread, a FIFO queue for further messages, cancel, and
//! approvals (`docs/spec/03-kernel.md`, turn rules; `docs/spec/05-providers.md`).

use std::collections::VecDeque;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use kumikit_protocol::event::{ApprovalResolved, ThreadChange, TurnEnded, TurnStarted};
use kumikit_protocol::methods::{ProviderInfo, ThreadSendResult};
use kumikit_protocol::model::{
    ApprovalId, Attachment, Decision, PermissionMode, Resolution, Thread, ThreadId, TurnId,
    TurnOutcome, UserInput, WorkspaceId,
};
use kumikit_protocol::{ThreadEvent, WorkspaceEvent};
use tokio::sync::oneshot;

use crate::error::{KernelError, KernelResult};
use crate::kernel::{Kernel, Session};
use crate::provider::{EventSink, Model, Provider, SessionConfig};
use crate::repo;

/// Maximum attachments accepted on a single user turn.
const MAX_ATTACHMENTS: usize = 16;
/// Maximum total decoded bytes across attachments on a single user turn (4 MiB).
const MAX_ATTACHMENT_BYTES: usize = 4 * 1024 * 1024;

fn validate_attachments(attachments: &[Attachment]) -> KernelResult<()> {
    if attachments.len() > MAX_ATTACHMENTS {
        return Err(KernelError::InvalidParams(format!(
            "at most {MAX_ATTACHMENTS} attachments per message"
        )));
    }
    let mut total = 0usize;
    for a in attachments {
        if a.name.trim().is_empty() || a.name.len() > 255 {
            return Err(KernelError::InvalidParams(
                "attachment name must be 1..=255 bytes".into(),
            ));
        }
        if a.mime_type.trim().is_empty() || a.mime_type.len() > 128 {
            return Err(KernelError::InvalidParams(
                "attachment mimeType must be 1..=128 bytes".into(),
            ));
        }
        let decoded = BASE64.decode(&a.data).map_err(|_| {
            KernelError::InvalidParams(format!("attachment {} is not valid base64", a.name))
        })?;
        total = total.saturating_add(decoded.len());
        if total > MAX_ATTACHMENT_BYTES {
            return Err(KernelError::InvalidParams(
                "attachments exceed the 4 MiB per-message limit".into(),
            ));
        }
    }
    Ok(())
}

/// A message waiting to start a turn: the turn it gets, the input, and the plugin that
/// sent it, if one did.
#[derive(Debug, Clone)]
pub(crate) struct Queued {
    turn: TurnId,
    input: UserInput,
    from: Option<String>,
}

/// In-memory state of a thread with a running turn.
#[derive(Debug)]
pub(crate) struct ThreadRun {
    /// The running turn.
    pub(crate) turn: TurnId,
    /// The thread's provider.
    provider: String,
    /// Fires to cancel the running turn. Dropping it (kernel close) abandons the turn.
    cancel: Option<oneshot::Sender<()>>,
    /// Messages waiting for the running turn to end.
    queue: VecDeque<Queued>,
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

    /// Removes a provider. Its running turns are cancelled, queued messages dropped and
    /// its sessions forgotten (the caller ends them). Returns false if it was not
    /// registered.
    pub fn unregister_provider(&self, id: &str) -> bool {
        let removed = {
            let mut providers = self.lock_providers();
            let before = providers.len();
            providers.retain(|p| p.id() != id);
            providers.len() != before
        };
        for run in self.lock_runs().values_mut() {
            if run.provider == id {
                run.queue.clear();
                if let Some(tx) = run.cancel.take() {
                    let _ = tx.send(());
                }
            }
        }
        let gone: Vec<ThreadId> = self
            .lock_sessions()
            .iter()
            .filter(|(_, s)| s.provider == id)
            .map(|(t, _)| t.clone())
            .collect();
        for t in &gone {
            self.gateway_revoke(t);
        }
        self.lock_sessions().retain(|_, s| s.provider != id);
        removed
    }

    /// Whether a turn runs, or waits on an approval, on any of these providers.
    pub fn turns_running_on(&self, providers: &[String]) -> bool {
        self.lock_runs()
            .values()
            .any(|r| providers.contains(&r.provider))
    }

    /// Forgets a thread's provider session, so its next turn starts one again with
    /// `resume`. Providers call this when they lose a session (for example, after
    /// unloading the instance that ran it).
    pub fn forget_session(&self, thread: &ThreadId) {
        self.lock_sessions().remove(thread);
        self.gateway_revoke(thread);
    }

    fn provider(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.lock_providers().iter().find(|p| p.id() == id).cloned()
    }

    /// Registered providers, sorted by id.
    pub fn provider_list(&self) -> Vec<ProviderInfo> {
        let mut list: Vec<ProviderInfo> = self
            .lock_providers()
            .iter()
            .map(|p| ProviderInfo {
                id: p.id().to_owned(),
                name: p.name().to_owned(),
                plugin: p.contributor().to_owned(),
            })
            .collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// A provider's models; the first is the default. Fails with `not_found` for an
    /// unknown provider and `plugin_failed` if the provider fails.
    pub async fn provider_models(&self, id: &str) -> KernelResult<Vec<Model>> {
        let provider = self
            .provider(id)
            .ok_or_else(|| KernelError::NotFound(format!("no provider {id}")))?;
        provider
            .list_models()
            .await
            .map_err(|e| KernelError::PluginFailed(e.message))
    }

    /// Creates a thread. `provider` defaults to the only registered provider, `model` to
    /// the provider's first model, and `permission_mode` to `standard`.
    pub async fn thread_create(
        &self,
        workspace: &WorkspaceId,
        provider: Option<String>,
        model: Option<String>,
        title: Option<String>,
        permission_mode: Option<PermissionMode>,
    ) -> KernelResult<Thread> {
        self.thread_create_with_worktree(workspace, provider, model, title, permission_mode, None)
            .await
    }

    /// Creates a thread, optionally running inside a linked Git worktree path while
    /// keeping the thread attached to `workspace`.
    pub async fn thread_create_with_worktree(
        &self,
        workspace: &WorkspaceId,
        provider: Option<String>,
        model: Option<String>,
        title: Option<String>,
        permission_mode: Option<PermissionMode>,
        worktree_path: Option<String>,
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
        self.thread_insert_with_worktree(
            workspace,
            provider.id().to_owned(),
            model,
            title,
            permission_mode,
            worktree_path,
        )
        .await
    }

    /// Updates a thread's model, permission mode, and/or title. Fails with `conflict`
    /// while a turn runs or if the thread is archived. Changing the model or permission
    /// mode restarts the provider session (with `resume`) on the next turn.
    pub async fn thread_update(
        &self,
        id: &ThreadId,
        model: Option<String>,
        permission_mode: Option<PermissionMode>,
        title: Option<String>,
    ) -> KernelResult<Thread> {
        if self.has_run(id) {
            return Err(KernelError::Conflict(format!(
                "thread {id} has a running turn"
            )));
        }
        let row = self.thread_row(id).await?;
        if row.archived {
            return Err(KernelError::Conflict(format!("thread {id} is archived")));
        }
        let next_model = match model {
            Some(m) if m != row.thread.model => {
                let provider = self.provider(&row.thread.provider).ok_or_else(|| {
                    KernelError::PluginFailed(format!(
                        "provider {} is not available",
                        row.thread.provider
                    ))
                })?;
                let models = provider
                    .list_models()
                    .await
                    .map_err(|e| KernelError::PluginFailed(e.message))?;
                if !models.iter().any(|x| x.id == m) {
                    return Err(KernelError::InvalidParams(format!(
                        "provider {} has no model {m}",
                        provider.id()
                    )));
                }
                m
            }
            Some(m) => m,
            None => row.thread.model.clone(),
        };
        let next_mode = match permission_mode {
            Some(PermissionMode::Standard) => None,
            Some(m) => Some(m),
            None => row.thread.permission_mode,
        };
        let next_title = match title {
            Some(t) => Some(t),
            None => row.thread.title.clone(),
        };
        let restart_session =
            next_model != row.thread.model || next_mode != row.thread.permission_mode;
        let updated = self.now();
        let tid = id.clone();
        let m_for_db = next_model.clone();
        let t_for_db = next_title.clone();
        let saved = self
            .inner
            .store
            .write(move |c| {
                repo::thread_update(c, &tid, &m_for_db, next_mode, t_for_db.as_deref(), &updated)
            })
            .await?;
        if restart_session {
            let old_session = self.lock_sessions().remove(id);
            self.gateway_revoke(id);
            if let Some(session) = old_session
                && let Some(provider) = self.provider(&session.provider)
            {
                provider.end_session(session.id).await;
            }
        }
        let public = saved.public();
        self.announce(WorkspaceEvent::ThreadUpdated(ThreadChange {
            thread: public.clone(),
        }));
        Ok(public)
    }

    /// Sends a user message: starts a turn, or queues it behind the running one.
    pub async fn thread_send(
        &self,
        thread: &ThreadId,
        input: UserInput,
    ) -> KernelResult<ThreadSendResult> {
        self.thread_send_from(thread, input, None).await
    }

    /// Like [`Kernel::thread_send`], for a message a plugin sends: `turn.started` records
    /// it in `from`.
    pub async fn thread_send_from(
        &self,
        thread: &ThreadId,
        input: UserInput,
        from: Option<String>,
    ) -> KernelResult<ThreadSendResult> {
        validate_attachments(&input.attachments)?;
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
                run.queue.push_back(Queued {
                    turn: turn.clone(),
                    input,
                    from,
                });
                return Ok(ThreadSendResult { turn, queued: true });
            }
            let (tx, rx) = oneshot::channel();
            runs.insert(
                thread.clone(),
                ThreadRun {
                    turn: turn.clone(),
                    provider: row.thread.provider.clone(),
                    cancel: Some(tx),
                    queue: VecDeque::new(),
                },
            );
            rx
        };
        let kernel = self.clone();
        let t = thread.clone();
        let first = Queued {
            turn: turn.clone(),
            input,
            from,
        };
        tokio::spawn(async move {
            kernel
                .run_turns(t, row.thread, provider, first, cancel_rx)
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
        self.respond_approval_as(approval, decision, by, None).await
    }

    /// Like [`Kernel::respond_approval`], recording the answering plugin. A plugin cannot
    /// answer `allow_always` (`invalid_params`).
    pub async fn respond_approval_by_plugin(
        &self,
        approval: &ApprovalId,
        decision: Decision,
        plugin: &str,
    ) -> KernelResult<()> {
        if decision == Decision::AllowAlways {
            return Err(KernelError::InvalidParams(
                "a plugin cannot answer allow_always".into(),
            ));
        }
        self.respond_approval_as(approval, decision, None, Some(plugin.to_owned()))
            .await
    }

    async fn respond_approval_as(
        &self,
        approval: &ApprovalId,
        decision: Decision,
        by: Option<String>,
        plugin: Option<String>,
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
                plugin,
            }),
        )
        .await?;
        // A gateway call's approval is the kernel's own: hand the answer to it.
        if self.gateway_resolved(approval, decision.into()) {
            return Ok(());
        }
        let row = self.thread_row(&thread).await?;
        let session = self.lock_sessions().get(&thread).cloned();
        if let (Some(provider), Some(session)) = (self.provider(&row.thread.provider), session) {
            provider
                .respond_approval(session.id, approval.clone(), decision)
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
        mut next_up: Queued,
        mut cancel: oneshot::Receiver<()>,
    ) {
        loop {
            let turn = next_up.turn.clone();
            let finished = self
                .run_one(&thread, &meta, &provider, next_up, &mut cancel)
                .await;
            if let Err(e) = &finished {
                tracing::warn!(%thread, %turn, error = %e, "turn could not be recorded");
            }
            let next = {
                let mut runs = self.lock_runs();
                let next = match (&finished, runs.get_mut(&thread)) {
                    (Ok(true), Some(run)) => run.queue.pop_front().map(|q| {
                        let (tx, rx) = oneshot::channel();
                        run.turn = q.turn.clone();
                        run.cancel = Some(tx);
                        (q, rx)
                    }),
                    _ => None,
                };
                if next.is_none() {
                    runs.remove(&thread);
                }
                next
            };
            match next {
                Some((q, rx)) => (next_up, cancel) = (q, rx),
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
        queued: Queued,
        cancel: &mut oneshot::Receiver<()>,
    ) -> KernelResult<bool> {
        let Queued { turn, input, from } = queued;
        let turn = &turn;
        self.append(
            thread,
            ThreadEvent::TurnStarted(TurnStarted {
                turn: turn.clone(),
                input: input.clone(),
                from: from.map(|plugin| kumikit_protocol::event::MessageFrom { plugin }),
            }),
        )
        .await?;
        let sink = EventSink::new(
            self.clone(),
            thread.clone(),
            turn.clone(),
            provider.plugin_id().to_owned(),
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
            // A gateway call waiting on it learns that the turn ended.
            self.gateway_resolved(&approval, Resolution::Cancelled);
            let resolved = self
                .append(
                    thread,
                    ThreadEvent::ApprovalResolved(ApprovalResolved {
                        approval,
                        decision: Resolution::Cancelled,
                        by: None,
                        plugin: None,
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
            return Ok(s.id.clone());
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
        let effective_root = row
            .thread
            .worktree_path
            .clone()
            .unwrap_or_else(|| workspace.workspace.root.clone());
        let mcp_server = self
            .gateway_for_session(
                &meta.id,
                &meta.workspace,
                &effective_root,
                provider.contributor(),
            )
            .await
            .map_err(|e| e.to_string())?;
        let permission_mode = row.thread.permission_mode.unwrap_or_default();
        let session = provider
            .start_session(SessionConfig {
                thread: meta.id.clone(),
                workspace_root: effective_root,
                model: row.thread.model,
                resume: row.provider_session,
                mcp_server,
                options: vec![(
                    "permission-mode".to_owned(),
                    permission_mode.as_str().to_owned(),
                )],
            })
            .await
            .map_err(|e| e.message)?;
        let (id, s) = (meta.id.clone(), session.clone());
        self.inner
            .store
            .write(move |c| repo::thread_set_session(c, &id, &s))
            .await
            .map_err(|e: KernelError| e.to_string())?;
        self.lock_sessions().insert(
            meta.id.clone(),
            Session {
                provider: provider.id().to_owned(),
                id: session.clone(),
            },
        );
        Ok(session)
    }

    /// Whether a turn runs or waits for this thread.
    pub(crate) fn has_run(&self, thread: &ThreadId) -> bool {
        self.lock_runs().contains_key(thread)
    }

    /// Drops all runs (abandoning their turns) and ends all provider sessions.
    pub(crate) async fn stop_turns(&self) {
        self.lock_runs().clear();
        let sessions: Vec<Session> = self.lock_sessions().drain().map(|(_, s)| s).collect();
        for session in sessions {
            if let Some(p) = self.provider(&session.provider) {
                p.end_session(session.id).await;
            }
        }
    }
}

#[cfg(test)]
mod tests;
