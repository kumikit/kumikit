//! Turns: one running turn per thread, the durable queue for further messages, cancel,
//! and approvals (`docs/spec/03-kernel.md`, turn rules and the queue;
//! `docs/spec/05-providers.md`; ADR 0027).
//!
//! The queue lives in the log (`queue.*` events) and its derived table. Every decision
//! about what runs next, and every queue change, happens under `Inner::dispatch`, so a
//! message queued while a turn ends is either seen by the dispatcher or starts its own
//! turn: never both, never neither.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use kumikit_protocol::event::{
    ApprovalResolved, MessageFrom, QueueAddReason, QueueAdded, QueueEntry, QueueHeld, QueueHold,
    QueueMoved, QueueRemoval, QueueRemoved, QueueResumed, QueueUpdated, ThreadChange, TurnEnded,
    TurnStarted,
};
use kumikit_protocol::methods::{ProviderInfo, SteerStatus, ThreadSendResult, ThreadSteerResult};
use kumikit_protocol::model::{
    ApprovalId, Attachment, Decision, PermissionMode, QueueEntryId, Resolution, Thread, ThreadId,
    TurnId, TurnOutcome, UserInput, WorkspaceId,
};
use kumikit_protocol::{ThreadEvent, WorkspaceEvent};
use tokio::sync::oneshot;

use crate::error::{KernelError, KernelResult};
use crate::kernel::{Kernel, Session};
use crate::provider::{EventSink, Inbox, Model, Provider, SessionConfig};
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

/// A message about to start a turn: the turn it gets, the input, the plugin that sent
/// it (if one did) and the queue entry it comes from (if it waited).
#[derive(Debug, Clone)]
pub(crate) struct Queued {
    turn: TurnId,
    input: UserInput,
    from: Option<String>,
    entry: Option<QueueEntryId>,
}

impl From<QueueEntry> for Queued {
    fn from(e: QueueEntry) -> Self {
        Queued {
            turn: e.turn,
            input: e.input,
            from: e.from.map(|f| f.plugin),
            entry: Some(e.id),
        }
    }
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
    /// The entry `thread.queue.sendNow` asked to run once the running turn stops.
    next: Option<QueueEntryId>,
    /// Steering messages for the running turn (ADR 0028); a new one per turn.
    inbox: Arc<Inbox>,
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

    /// Removes a provider. Its running turns are cancelled (their queues are held) and
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
                run.next = None;
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
    /// it in `from`. Sending resumes a held queue (ADR 0027).
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
        let provider = self.provider_of(&row.thread)?;
        let _dispatch = self.inner.dispatch.lock().await;
        if self.queue_hold_of(thread).await?.is_some() {
            self.append(thread, ThreadEvent::QueueResumed(QueueResumed {}))
                .await?;
        }
        let turn = TurnId(self.new_id());
        if self.has_run(thread) {
            let entry = QueueEntry {
                id: QueueEntryId(self.new_id()),
                turn: turn.clone(),
                input,
                from: from.map(|plugin| MessageFrom { plugin }),
            };
            let id = entry.id.clone();
            self.append(
                thread,
                ThreadEvent::QueueAdded(QueueAdded {
                    entry,
                    front: false,
                    reason: None,
                }),
            )
            .await?;
            return Ok(ThreadSendResult {
                turn,
                queued: true,
                entry: Some(id),
            });
        }
        let first = Queued {
            turn: turn.clone(),
            input,
            from,
            entry: None,
        };
        self.start_run(thread, row.thread, provider, first).await?;
        Ok(ThreadSendResult {
            turn,
            queued: false,
            entry: None,
        })
    }

    /// Cancels the running turn. Queued messages stay, held with `stopped`. A thread
    /// with no running turn is left as it is.
    pub async fn thread_cancel(&self, thread: &ThreadId) -> KernelResult<()> {
        self.thread_row(thread).await?;
        if let Some(run) = self.lock_runs().get_mut(thread) {
            run.next = None;
            if let Some(tx) = run.cancel.take() {
                let _ = tx.send(());
            }
        }
        Ok(())
    }

    /// Sends a message to the running turn's agent (ADR 0028). If the thread's model can
    /// be steered and the message has no attachments, it waits in the turn's inbox until
    /// the provider takes it (`input.steered`); otherwise it goes to the front of the
    /// queue. `conflict` when no turn runs: send it with [`Kernel::thread_send`].
    pub async fn thread_steer(
        &self,
        thread: &ThreadId,
        input: UserInput,
    ) -> KernelResult<ThreadSteerResult> {
        self.thread_steer_from(thread, input, None).await
    }

    /// Like [`Kernel::thread_steer`], recording `from` (a plugin id) if the message falls
    /// back to the queue.
    pub async fn thread_steer_from(
        &self,
        thread: &ThreadId,
        input: UserInput,
        from: Option<String>,
    ) -> KernelResult<ThreadSteerResult> {
        validate_attachments(&input.attachments)?;
        let row = self.thread_row(thread).await?;
        if row.archived {
            return Err(KernelError::Conflict(format!(
                "thread {thread} is archived"
            )));
        }
        let steerable = input.attachments.is_empty() && self.can_steer(&row.thread).await;
        let _dispatch = self.inner.dispatch.lock().await;
        let (turn, inbox) = match self.lock_runs().get(thread) {
            Some(run) => (run.turn.clone(), Arc::clone(&run.inbox)),
            None => {
                return Err(KernelError::Conflict(format!(
                    "thread {thread} has no running turn; send the message instead"
                )));
            }
        };
        if steerable && inbox.push(input.clone()) {
            return Ok(ThreadSteerResult {
                status: SteerStatus::Accepted,
                turn,
                entry: None,
            });
        }
        let entry = QueueEntry {
            id: QueueEntryId(self.new_id()),
            turn: TurnId(self.new_id()),
            input,
            from: from.map(|plugin| MessageFrom { plugin }),
        };
        let id = entry.id.clone();
        self.append(
            thread,
            ThreadEvent::QueueAdded(QueueAdded {
                entry,
                front: true,
                reason: Some(QueueAddReason::NotSteered),
            }),
        )
        .await?;
        Ok(ThreadSteerResult {
            status: SteerStatus::Queued,
            turn,
            entry: Some(id),
        })
    }

    /// Whether the thread's model reports the `steer` capability.
    async fn can_steer(&self, thread: &Thread) -> bool {
        let Some(provider) = self.provider(&thread.provider) else {
            return false;
        };
        provider.list_models().await.is_ok_and(|models| {
            models
                .iter()
                .any(|m| m.id == thread.model && m.capabilities.iter().any(|c| c == "steer"))
        })
    }

    // ---- the queue (ADR 0027) ----

    /// Replaces a queued message's input.
    pub async fn queue_update(
        &self,
        thread: &ThreadId,
        entry: &QueueEntryId,
        input: UserInput,
    ) -> KernelResult<QueueEntry> {
        validate_attachments(&input.attachments)?;
        self.thread_row(thread).await?;
        let _dispatch = self.inner.dispatch.lock().await;
        let mut queued = self.queue_entry(thread, entry).await?;
        queued.input = input;
        self.append(
            thread,
            ThreadEvent::QueueUpdated(QueueUpdated {
                entry: queued.clone(),
            }),
        )
        .await?;
        Ok(queued)
    }

    /// Removes a queued message.
    pub async fn queue_remove(&self, thread: &ThreadId, entry: &QueueEntryId) -> KernelResult<()> {
        self.thread_row(thread).await?;
        let _dispatch = self.inner.dispatch.lock().await;
        self.queue_entry(thread, entry).await?;
        self.append(
            thread,
            ThreadEvent::QueueRemoved(QueueRemoved {
                entry: entry.clone(),
                reason: QueueRemoval::Removed,
            }),
        )
        .await?;
        Ok(())
    }

    /// Moves a queued message before `before`, or to the end when `before` is `None`.
    pub async fn queue_move(
        &self,
        thread: &ThreadId,
        entry: &QueueEntryId,
        before: Option<&QueueEntryId>,
    ) -> KernelResult<()> {
        self.thread_row(thread).await?;
        let _dispatch = self.inner.dispatch.lock().await;
        let current: Vec<QueueEntryId> = self
            .queue_of(thread)
            .await?
            .into_iter()
            .map(|q| q.id)
            .collect();
        if !current.contains(entry) {
            return Err(KernelError::NotFound(format!("no queued message {entry}")));
        }
        if before == Some(entry) {
            return Ok(());
        }
        let mut order: Vec<QueueEntryId> =
            current.iter().filter(|q| *q != entry).cloned().collect();
        match before {
            Some(b) => {
                let at = order
                    .iter()
                    .position(|q| q == b)
                    .ok_or_else(|| KernelError::NotFound(format!("no queued message {b}")))?;
                order.insert(at, entry.clone());
            }
            None => order.push(entry.clone()),
        }
        if order != current {
            self.append(thread, ThreadEvent::QueueMoved(QueueMoved { order }))
                .await?;
        }
        Ok(())
    }

    /// Starts a queued message now. While a turn runs, that turn is cancelled and this
    /// message runs next; the rest of the queue is held.
    pub async fn queue_send_now(
        &self,
        thread: &ThreadId,
        entry: &QueueEntryId,
    ) -> KernelResult<ThreadSendResult> {
        let row = self.thread_row(thread).await?;
        if row.archived {
            return Err(KernelError::Conflict(format!(
                "thread {thread} is archived"
            )));
        }
        let _dispatch = self.inner.dispatch.lock().await;
        let queued = self.queue_entry(thread, entry).await?;
        if let Some(run) = self.lock_runs().get_mut(thread) {
            run.next = Some(queued.id.clone());
            if let Some(tx) = run.cancel.take() {
                let _ = tx.send(());
            }
            return Ok(ThreadSendResult {
                turn: queued.turn,
                queued: true,
                entry: Some(queued.id),
            });
        }
        let provider = self.provider_of(&row.thread)?;
        let turn = queued.turn.clone();
        self.start_run(thread, row.thread, provider, queued.into())
            .await?;
        Ok(ThreadSendResult {
            turn,
            queued: false,
            entry: None,
        })
    }

    /// Lets a held queue start its entries again; an idle thread starts the first one.
    pub async fn queue_resume(&self, thread: &ThreadId) -> KernelResult<()> {
        let row = self.thread_row(thread).await?;
        if row.archived {
            return Err(KernelError::Conflict(format!(
                "thread {thread} is archived"
            )));
        }
        let _dispatch = self.inner.dispatch.lock().await;
        if self.queue_hold_of(thread).await?.is_some() {
            self.append(thread, ThreadEvent::QueueResumed(QueueResumed {}))
                .await?;
        }
        if self.has_run(thread) {
            return Ok(());
        }
        let Some(first) = self.queue_of(thread).await?.into_iter().next() else {
            return Ok(());
        };
        let provider = self.provider_of(&row.thread)?;
        self.start_run(thread, row.thread, provider, first.into())
            .await
    }

    async fn queue_of(&self, thread: &ThreadId) -> KernelResult<Vec<QueueEntry>> {
        let t = thread.clone();
        self.inner
            .store
            .read(move |c| repo::queue_list(c, &t))
            .await
    }

    async fn queue_hold_of(&self, thread: &ThreadId) -> KernelResult<Option<QueueHold>> {
        let t = thread.clone();
        self.inner
            .store
            .read(move |c| repo::queue_hold(c, &t))
            .await
    }

    async fn queue_entry(
        &self,
        thread: &ThreadId,
        entry: &QueueEntryId,
    ) -> KernelResult<QueueEntry> {
        self.queue_of(thread)
            .await?
            .into_iter()
            .find(|q| &q.id == entry)
            .ok_or_else(|| KernelError::NotFound(format!("no queued message {entry}")))
    }

    fn provider_of(&self, thread: &Thread) -> KernelResult<Arc<dyn Provider>> {
        self.provider(&thread.provider).ok_or_else(|| {
            KernelError::PluginFailed(format!("provider {} is not available", thread.provider))
        })
    }

    /// Registers a run for an idle thread, appends its `turn.started` and spawns the turn
    /// loop. The caller holds `dispatch`.
    async fn start_run(
        &self,
        thread: &ThreadId,
        meta: Thread,
        provider: Arc<dyn Provider>,
        first: Queued,
    ) -> KernelResult<()> {
        let (tx, rx) = oneshot::channel();
        self.lock_runs().insert(
            thread.clone(),
            ThreadRun {
                turn: first.turn.clone(),
                provider: meta.provider.clone(),
                cancel: Some(tx),
                next: None,
                inbox: Inbox::new(),
            },
        );
        if let Err(e) = self.begin_turn(thread, &first).await {
            self.lock_runs().remove(thread);
            return Err(e);
        }
        let kernel = self.clone();
        let t = thread.clone();
        tokio::spawn(async move { kernel.run_turns(t, meta, provider, first, rx).await });
        Ok(())
    }

    /// Appends `turn.started`; for a queued message this also takes it out of the queue.
    async fn begin_turn(&self, thread: &ThreadId, q: &Queued) -> KernelResult<()> {
        self.append(
            thread,
            ThreadEvent::TurnStarted(TurnStarted {
                turn: q.turn.clone(),
                input: q.input.clone(),
                from: q.from.clone().map(|plugin| MessageFrom { plugin }),
                entry: q.entry.clone(),
            }),
        )
        .await?;
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
            let (outcome, error) = match self
                .run_one(&thread, &meta, &provider, next_up, &mut cancel)
                .await
            {
                Ok(Some(ended)) => ended,
                // Abandoned: the kernel is closing; the next start ends the turn.
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!(%thread, %turn, error = %e, "turn could not be recorded");
                    self.lock_runs().remove(&thread);
                    break;
                }
            };
            match self.end_turn(&thread, &turn, outcome, error).await {
                Ok(Some((q, rx))) => (next_up, cancel) = (q, rx),
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!(%thread, error = %e, "the next queued message could not start");
                    self.lock_runs().remove(&thread);
                    break;
                }
            }
        }
    }

    /// Ends a turn and starts what runs next: the entry `sendNow` asked for, else the
    /// queue's first entry unless the queue is held. A stopped or failed turn holds the
    /// rest of the queue. All under `dispatch`: when nothing is next the run is dropped
    /// before `turn.ended` is appended, so a client reacting to `turn.ended` finds the
    /// thread idle, and a message sent meanwhile waits for the lock and starts after it.
    async fn end_turn(
        &self,
        thread: &ThreadId,
        turn: &TurnId,
        outcome: TurnOutcome,
        error: Option<String>,
    ) -> KernelResult<Option<(Queued, oneshot::Receiver<()>)>> {
        let _dispatch = self.inner.dispatch.lock().await;
        let (requested, unsteered) = match self.lock_runs().get_mut(thread) {
            Some(run) => (run.next.take(), run.inbox.close()),
            None => (None, Vec::new()),
        };
        // Steering messages the turn did not take go to the front of the queue, in order.
        let unsteered: Vec<QueueEntry> = unsteered
            .into_iter()
            .map(|input| QueueEntry {
                id: QueueEntryId(self.new_id()),
                turn: TurnId(self.new_id()),
                input,
                from: None,
            })
            .collect();
        let mut queue = unsteered.clone();
        queue.extend(self.queue_of(thread).await?);
        let mut held = self.queue_hold_of(thread).await?.is_some();
        let requested = requested.and_then(|id| queue.iter().find(|q| q.id == id).cloned());
        let others = queue
            .iter()
            .any(|q| requested.as_ref().is_none_or(|r| r.id != q.id));
        let hold = match outcome {
            TurnOutcome::Done => None,
            TurnOutcome::Cancelled => Some(QueueHold::Stopped),
            TurnOutcome::Failed => Some(QueueHold::Failed),
        };
        let hold = hold.filter(|_| others && !held);
        held |= hold.is_some();
        let next = requested.or_else(|| if held { None } else { queue.into_iter().next() });
        if next.is_none() {
            self.lock_runs().remove(thread);
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
        for entry in unsteered.into_iter().rev() {
            self.append(
                thread,
                ThreadEvent::QueueAdded(QueueAdded {
                    entry,
                    front: true,
                    reason: Some(QueueAddReason::NotSteered),
                }),
            )
            .await?;
        }
        if let Some(reason) = hold {
            self.append(thread, ThreadEvent::QueueHeld(QueueHeld { reason }))
                .await?;
        }
        let Some(entry) = next else {
            return Ok(None);
        };
        let q = Queued::from(entry);
        let (tx, rx) = oneshot::channel();
        match self.lock_runs().get_mut(thread) {
            Some(run) => {
                run.turn = q.turn.clone();
                run.cancel = Some(tx);
                run.inbox = Inbox::new();
            }
            // The kernel is closing and dropped the run.
            None => return Ok(None),
        }
        if let Err(e) = self.begin_turn(thread, &q).await {
            self.lock_runs().remove(thread);
            return Err(e);
        }
        Ok(Some((q, rx)))
    }

    /// Runs one turn whose `turn.started` is already in the log, up to (not including)
    /// its `turn.ended`. Returns the outcome and error, `None` if it was abandoned, `Err`
    /// if the log could not be written.
    async fn run_one(
        &self,
        thread: &ThreadId,
        meta: &Thread,
        provider: &Arc<dyn Provider>,
        queued: Queued,
        cancel: &mut oneshot::Receiver<()>,
    ) -> KernelResult<Option<(TurnOutcome, Option<String>)>> {
        let Queued { turn, input, .. } = queued;
        let turn = &turn;
        let inbox = self
            .lock_runs()
            .get(thread)
            .map_or_else(Inbox::new, |r| Arc::clone(&r.inbox));
        let sink = EventSink::new(
            self.clone(),
            thread.clone(),
            turn.clone(),
            provider.plugin_id().to_owned(),
            inbox,
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
            Ending::Abandoned => return Ok(None),
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
        Ok(Some((outcome, error)))
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
