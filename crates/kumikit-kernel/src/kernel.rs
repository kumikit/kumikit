//! The kernel service: workspaces, threads, the event log and subscriptions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use kumikit_protocol::event::{ApprovalResolved, QueueHeld, QueueHold, ThreadChange, TurnEnded};
use kumikit_protocol::local::{Platform, default_home_dir};
use kumikit_protocol::methods::{CoreConfigResult, ThreadSnapshot};
use kumikit_protocol::model::{
    ApprovalId, PermissionMode, Resolution, Thread, ThreadId, ThreadStatus, Timestamp, TurnOutcome,
    Workspace, WorkspaceId,
};
use kumikit_protocol::{ThreadEvent, WorkspaceEvent};
use tokio::sync::{broadcast, watch};

use crate::clock::{Clock, IdGen, SystemClock, UlidGen};
use crate::error::{KernelError, KernelResult};
use crate::fold::Fold;
use crate::process::{Supervisor, SupervisorConfig};
use crate::provider::Provider;
use crate::repo::{self, StoredEvent};
use crate::secrets::SecretStore;
use crate::store::{MIGRATIONS, Store, StoreError};
use crate::turns::ThreadRun;

/// Error text of `turn.ended` for turns a previous run of the core left open.
pub const CORE_RESTARTED: &str = "core_restarted";

/// Events returned per read while replaying.
const REPLAY_BATCH: usize = 256;

/// Workspace changes buffered per listener before it lags.
const WORKSPACE_EVENTS: usize = 256;

/// How to open a kernel.
#[derive(Debug, Clone)]
pub struct KernelConfig {
    /// Path of the SQLite database file.
    pub database: PathBuf,
    /// Time source.
    pub clock: Arc<dyn Clock>,
    /// Id source.
    pub ids: Arc<dyn IdGen>,
    /// Secret store file.
    pub secrets: PathBuf,
    /// Process supervisor settings.
    pub supervisor: SupervisorConfig,
    /// Platform default Kumikit home directory (for example `~/.kumikit`).
    pub default_home_dir: PathBuf,
}

impl KernelConfig {
    /// System clock and ULIDs, database at `database`, secrets in `secrets.json` next to
    /// it, the supervisor using this process's environment.
    pub fn new(database: impl Into<PathBuf>) -> Self {
        let database = database.into();
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let default_home_dir = default_home_dir(Platform::current(), &env)
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                database
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."))
            });
        KernelConfig {
            secrets: database.with_file_name("secrets.json"),
            database,
            clock: Arc::new(SystemClock),
            ids: Arc::new(UlidGen::default()),
            supervisor: SupervisorConfig::from_process(),
            default_home_dir,
        }
    }
}

/// The kernel. Cheap to clone; all clones share state.
#[derive(Clone, Debug)]
pub struct Kernel {
    pub(crate) inner: Arc<Inner>,
}

#[derive(Debug)]
pub(crate) struct Inner {
    pub(crate) store: Store,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) ids: Arc<dyn IdGen>,
    /// Last committed seq per thread, bumped after each commit. Subscriptions wait on it.
    heads: Mutex<HashMap<ThreadId, watch::Sender<u64>>>,
    /// Registered providers.
    providers: Mutex<Vec<Arc<dyn Provider>>>,
    /// Live changes to threads, for `workspace:<id>` topics. Nothing is kept for late
    /// listeners.
    workspace_events: broadcast::Sender<WorkspaceEvent>,
    /// Ids of revoked devices, for closing their connections.
    pub(crate) revocations: broadcast::Sender<String>,
    /// Threads with a running turn.
    runs: Mutex<HashMap<ThreadId, ThreadRun>>,
    /// Serialises queue changes with the decision of what runs next, so a message queued
    /// while a turn ends is never left behind (`turns.rs`).
    pub(crate) dispatch: tokio::sync::Mutex<()>,
    /// Provider session per thread, started on first use in this process.
    sessions: Mutex<HashMap<ThreadId, Session>>,
    pub(crate) secrets: SecretStore,
    pub(crate) supervisor: Supervisor,
    pub(crate) schedules: crate::schedule::State,
    pub(crate) gateway: crate::gateway::State,
    pub(crate) default_home_dir: PathBuf,
}

/// A started provider session.
#[derive(Debug, Clone)]
pub(crate) struct Session {
    /// The provider's id.
    pub(crate) provider: String,
    /// The provider's session id.
    pub(crate) id: String,
}

/// Locks a mutex, recovering from poisoning. Every kernel mutex guards plain maps or
/// lists that stay consistent even if a holder panicked mid-update.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Kernel {
    /// Opens the database (running migrations) and ends turns a previous run left open.
    pub async fn open(config: KernelConfig) -> KernelResult<Kernel> {
        let path = config.database.clone();
        let store = tokio::task::spawn_blocking(move || Store::open(&path, MIGRATIONS))
            .await
            .map_err(|e| StoreError::Task(e.to_string()))??;
        let kernel = Kernel {
            inner: Arc::new(Inner {
                store,
                clock: config.clock,
                ids: config.ids,
                heads: Mutex::new(HashMap::new()),
                providers: Mutex::new(Vec::new()),
                workspace_events: broadcast::Sender::new(WORKSPACE_EVENTS),
                revocations: broadcast::Sender::new(crate::devices::REVOCATIONS),
                runs: Mutex::new(HashMap::new()),
                dispatch: tokio::sync::Mutex::new(()),
                sessions: Mutex::new(HashMap::new()),
                secrets: SecretStore::new(config.secrets),
                supervisor: Supervisor::new(config.supervisor),
                schedules: Default::default(),
                gateway: Default::default(),
                default_home_dir: config.default_home_dir,
            }),
        };
        kernel.recover().await?;
        if let Err(e) = kernel.inner.secrets.preload().await {
            tracing::warn!(error = %e, "cannot read the secret store");
        }
        Ok(kernel)
    }

    /// Abandons running turns (the next start ends them as `core_restarted`), ends
    /// provider sessions and subscriptions, kills supervised processes, and closes the
    /// database after queued writes.
    pub async fn close(&self) {
        self.stop_scheduler();
        self.stop_turns().await;
        self.inner.supervisor.shutdown().await;
        self.lock_heads().clear();
        self.inner.store.close().await;
    }

    fn lock_heads(&self) -> MutexGuard<'_, HashMap<ThreadId, watch::Sender<u64>>> {
        lock(&self.inner.heads)
    }

    pub(crate) fn lock_providers(&self) -> MutexGuard<'_, Vec<Arc<dyn Provider>>> {
        lock(&self.inner.providers)
    }

    pub(crate) fn lock_runs(&self) -> MutexGuard<'_, HashMap<ThreadId, ThreadRun>> {
        lock(&self.inner.runs)
    }

    pub(crate) fn lock_sessions(&self) -> MutexGuard<'_, HashMap<ThreadId, Session>> {
        lock(&self.inner.sessions)
    }

    /// The secret store.
    pub fn secrets(&self) -> &SecretStore {
        &self.inner.secrets
    }

    /// The process supervisor.
    pub fn supervisor(&self) -> &Supervisor {
        &self.inner.supervisor
    }

    /// A new id from the configured generator.
    pub(crate) fn new_id(&self) -> String {
        self.inner.ids.next()
    }

    /// The current time from the configured clock.
    pub fn now(&self) -> Timestamp {
        self.inner.clock.now()
    }

    // ---- core config ----

    /// Reads core-level configuration (active and default Kumikit home directory).
    pub async fn core_config(&self) -> KernelResult<CoreConfigResult> {
        let default_home = self.inner.default_home_dir.to_string_lossy().into_owned();
        let saved = self
            .inner
            .store
            .read(|c| repo::core_setting_get(c, "home_dir"))
            .await?;
        let home_dir = saved.unwrap_or_else(|| default_home.clone());
        Ok(CoreConfigResult {
            home_dir,
            default_home_dir: default_home,
        })
    }

    /// Updates the Kumikit home directory (`None` or empty resets to the platform default).
    pub async fn set_core_config(
        &self,
        home_dir: Option<String>,
    ) -> KernelResult<CoreConfigResult> {
        let cleaned = home_dir.as_deref().map(str::trim).filter(|s| !s.is_empty());
        match cleaned {
            None => {
                self.inner
                    .store
                    .write(|c| repo::core_setting_delete(c, "home_dir"))
                    .await?;
            }
            Some(raw) => {
                if raw.chars().any(|c| c.is_control()) {
                    return Err(KernelError::InvalidParams(
                        "home directory contains control characters".into(),
                    ));
                }
                let expanded = expand_tilde(raw);
                if !Path::new(&expanded).is_absolute() {
                    return Err(KernelError::InvalidParams(format!(
                        "home directory must be an absolute path: `{raw}`"
                    )));
                }
                let val = expanded;
                self.inner
                    .store
                    .write(move |c| repo::core_setting_set(c, "home_dir", &val))
                    .await?;
            }
        }
        self.core_config().await
    }

    /// Ends turns left open by a previous run (`docs/spec/03-kernel.md`, turn rules).
    async fn recover(&self) -> KernelResult<()> {
        let open = self.inner.store.read(repo::open_turns).await?;
        for (thread, turn) in open {
            let t = turn.clone();
            let pending = self
                .inner
                .store
                .read(move |c| repo::pending_approvals_of_turn(c, &t))
                .await?;
            for approval in pending {
                self.append(
                    &thread,
                    ThreadEvent::ApprovalResolved(ApprovalResolved {
                        approval,
                        decision: Resolution::Cancelled,
                        by: None,
                        plugin: None,
                    }),
                )
                .await?;
            }
            self.append(
                &thread,
                ThreadEvent::TurnEnded(TurnEnded {
                    turn,
                    outcome: TurnOutcome::Failed,
                    error: Some(CORE_RESTARTED.into()),
                }),
            )
            .await?;
        }
        // Messages that were waiting when the core stopped wait for the user again.
        let queued = self.inner.store.read(repo::threads_with_open_queue).await?;
        for thread in queued {
            self.append(
                &thread,
                ThreadEvent::QueueHeld(QueueHeld {
                    reason: QueueHold::Restarted,
                }),
            )
            .await?;
        }
        Ok(())
    }

    // ---- workspaces ----

    /// Opens a workspace for an absolute directory path. The same root returns the same
    /// workspace, reopening it if it was closed.
    pub async fn workspace_open(&self, path: &str) -> KernelResult<Workspace> {
        let root = canonical_dir(path).await?;
        let name = Path::new(&root)
            .file_name()
            .map_or_else(|| root.clone(), |n| n.to_string_lossy().into_owned());
        let id = WorkspaceId(self.new_id());
        let created = self.now();
        self.inner
            .store
            .write(move |c| {
                if let Some(existing) = repo::workspace_by_root(c, &root)? {
                    if existing.closed {
                        repo::workspace_set_closed(c, &existing.workspace.id, false)?;
                    }
                    return Ok(existing.workspace);
                }
                let ws = Workspace {
                    id,
                    name,
                    root,
                    kind: None,
                };
                repo::workspace_insert(c, &ws, &created)?;
                Ok(ws)
            })
            .await
    }

    /// Opens or creates the core-managed chat workspace under `<homeDir>/chats` so users
    /// can start standalone chats without selecting a project folder.
    pub async fn workspace_chat(&self) -> KernelResult<Workspace> {
        if let Some(existing) = self.inner.store.read(repo::workspace_chat).await? {
            let root = PathBuf::from(&existing.workspace.root);
            tokio::task::spawn_blocking(move || std::fs::create_dir_all(&root))
                .await
                .map_err(|e| StoreError::Task(e.to_string()))?
                .map_err(|e| KernelError::Io(format!("cannot prepare chats folder: {e}")))?;
            if existing.closed {
                let id = existing.workspace.id.clone();
                self.inner
                    .store
                    .write(move |c| repo::workspace_set_closed(c, &id, false))
                    .await?;
            }
            return Ok(existing.workspace);
        }
        let cfg = self.core_config().await?;
        let chats_dir = PathBuf::from(&cfg.home_dir).join("chats");
        let dir_to_create = chats_dir.clone();
        tokio::task::spawn_blocking(move || std::fs::create_dir_all(&dir_to_create))
            .await
            .map_err(|e| StoreError::Task(e.to_string()))?
            .map_err(|e| KernelError::Io(format!("cannot create chats folder: {e}")))?;
        let root = canonical_dir(&chats_dir.to_string_lossy()).await?;
        let id = WorkspaceId(self.new_id());
        let created = self.now();
        self.inner
            .store
            .write(move |c| {
                if let Some(existing) = repo::workspace_chat(c)? {
                    if existing.closed {
                        repo::workspace_set_closed(c, &existing.workspace.id, false)?;
                    }
                    return Ok(existing.workspace);
                }
                if let Some(existing) = repo::workspace_by_root(c, &root)? {
                    if existing.closed {
                        repo::workspace_set_closed(c, &existing.workspace.id, false)?;
                    }
                    return Ok(existing.workspace);
                }
                let ws = Workspace {
                    id,
                    name: "Chats".into(),
                    root,
                    kind: Some(kumikit_protocol::model::WorkspaceKind::Chat),
                };
                repo::workspace_insert(c, &ws, &created)?;
                Ok(ws)
            })
            .await
    }

    /// Open workspaces.
    pub async fn workspace_list(&self) -> KernelResult<Vec<Workspace>> {
        self.inner.store.read(repo::workspace_list).await
    }

    /// An open workspace: `not_found` if there is none with this id, `conflict` if it is
    /// closed.
    pub async fn workspace_get(&self, id: &WorkspaceId) -> KernelResult<Workspace> {
        let ws = id.clone();
        let row = self
            .inner
            .store
            .read(move |c| repo::workspace_get(c, &ws))
            .await?
            .ok_or_else(|| KernelError::NotFound(format!("no workspace {id}")))?;
        if row.closed {
            return Err(KernelError::Conflict(format!("workspace {id} is closed")));
        }
        Ok(row.workspace)
    }

    /// The workspace a thread belongs to.
    pub async fn thread_workspace(&self, thread: &ThreadId) -> KernelResult<WorkspaceId> {
        Ok(self.thread_row(thread).await?.thread.workspace)
    }

    /// The thread an approval belongs to.
    pub async fn approval_thread(&self, approval: &ApprovalId) -> KernelResult<ThreadId> {
        let id = approval.clone();
        self.inner
            .store
            .read(move |c| repo::approval_get(c, &id))
            .await?
            .map(|(thread, _, _)| thread)
            .ok_or_else(|| KernelError::NotFound(format!("no approval {approval}")))
    }

    /// Hides a workspace from the list; its threads and events are kept.
    pub async fn workspace_close(&self, id: &WorkspaceId) -> KernelResult<()> {
        let id = id.clone();
        self.inner
            .store
            .write(move |c| repo::workspace_set_closed(c, &id, true))
            .await
    }

    /// Permanently deletes a workspace and all its threads, turns, approvals, and events,
    /// removing any managed worktrees under `<homeDir>/worktrees/` exclusively referenced
    /// by those threads. Fails with `conflict` if any thread has a running or waiting turn.
    pub async fn workspace_delete(&self, id: &WorkspaceId) -> KernelResult<()> {
        let threads = self.thread_list(id, true).await?;
        for t in &threads {
            if self.has_run(&t.id) {
                return Err(KernelError::Conflict(format!(
                    "thread {} in workspace {id} has a running turn",
                    t.id
                )));
            }
        }
        let mut sessions = Vec::new();
        {
            let mut guard = self.lock_sessions();
            for t in &threads {
                if let Some(session) = guard.remove(&t.id) {
                    sessions.push(session);
                }
                self.gateway_revoke(&t.id);
            }
        }
        for session in sessions {
            let provider = self
                .lock_providers()
                .iter()
                .find(|p| p.id() == session.provider)
                .cloned();
            if let Some(provider) = provider {
                provider.end_session(session.id).await;
            }
        }
        let ws = id.clone();
        let deleted = self
            .inner
            .store
            .write(move |c| repo::workspace_delete(c, &ws))
            .await?;
        {
            let mut heads = lock(&self.inner.heads);
            for t in &deleted {
                heads.remove(&t.id);
            }
        }
        let mut worktrees: std::collections::HashSet<String> = std::collections::HashSet::new();
        for t in &deleted {
            if let Some(wt) = &t.worktree_path {
                worktrees.insert(wt.clone());
            }
        }
        for wt_path in worktrees {
            let wt_check = wt_path.clone();
            let remaining = self
                .inner
                .store
                .read(move |c| repo::threads_using_worktree(c, &wt_check))
                .await
                .unwrap_or(1);
            if remaining == 0 {
                self.remove_worktree_if_managed(&wt_path).await;
            }
        }
        for t in deleted {
            self.announce(WorkspaceEvent::ThreadDeleted(ThreadChange { thread: t }));
        }
        Ok(())
    }

    // ---- threads ----

    /// Creates a thread with a concrete provider and model (test helper).
    #[cfg(test)]
    pub(crate) async fn thread_insert(
        &self,
        workspace: &WorkspaceId,
        provider: String,
        model: String,
        title: Option<String>,
        permission_mode: Option<PermissionMode>,
    ) -> KernelResult<Thread> {
        self.thread_insert_with_worktree(workspace, provider, model, title, permission_mode, None)
            .await
    }

    /// Creates a thread, optionally pinned to a linked Git worktree path.
    pub(crate) async fn thread_insert_with_worktree(
        &self,
        workspace: &WorkspaceId,
        provider: String,
        model: String,
        title: Option<String>,
        permission_mode: Option<PermissionMode>,
        worktree_path: Option<String>,
    ) -> KernelResult<Thread> {
        let now = self.now();
        let permission_mode = permission_mode.filter(|m| *m != PermissionMode::Standard);
        let worktree_path = match worktree_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(p) => Some(canonical_dir(p).await?),
            None => None,
        };
        let thread = Thread {
            id: ThreadId(self.new_id()),
            workspace: workspace.clone(),
            title,
            provider,
            model,
            permission_mode,
            worktree_path,
            status: ThreadStatus::Idle,
            created: now.clone(),
            updated: now,
        };
        let t = thread.clone();
        self.inner
            .store
            .write(move |c| {
                match repo::workspace_get(c, &t.workspace)? {
                    None => {
                        return Err(KernelError::NotFound(format!(
                            "no workspace {}",
                            t.workspace
                        )));
                    }
                    Some(ws) if ws.closed => {
                        return Err(KernelError::Conflict(format!(
                            "workspace {} is closed",
                            t.workspace
                        )));
                    }
                    Some(_) => {}
                }
                repo::thread_insert(c, &t)
            })
            .await?;
        self.announce(WorkspaceEvent::ThreadCreated(ThreadChange {
            thread: thread.clone(),
        }));
        Ok(thread)
    }

    /// The thread's effective permission mode (`standard` when unset).
    pub async fn thread_permission_mode(&self, id: &ThreadId) -> KernelResult<PermissionMode> {
        Ok(self
            .thread_row(id)
            .await?
            .thread
            .permission_mode
            .unwrap_or_default())
    }

    /// Threads of a workspace, most recently updated first.
    pub async fn thread_list(
        &self,
        workspace: &WorkspaceId,
        include_archived: bool,
    ) -> KernelResult<Vec<Thread>> {
        let ws = workspace.clone();
        self.inner
            .store
            .read(move |c| {
                if repo::workspace_get(c, &ws)?.is_none() {
                    return Err(KernelError::NotFound(format!("no workspace {ws}")));
                }
                Ok(repo::thread_list(c, &ws, include_archived)?
                    .iter()
                    .map(repo::ThreadRow::public)
                    .collect())
            })
            .await
    }

    /// The stored thread row.
    pub(crate) async fn thread_row(&self, id: &ThreadId) -> KernelResult<repo::ThreadRow> {
        let id = id.clone();
        self.inner
            .store
            .read(move |c| repo::thread_require(c, &id))
            .await
    }

    /// A snapshot folded from the log, with the `seq` it reflects.
    pub async fn thread_get(&self, id: &ThreadId) -> KernelResult<ThreadSnapshot> {
        let id = id.clone();
        self.inner
            .store
            .read(move |c| {
                // One read transaction, so the row and the events are from the same commit.
                let tx = c.unchecked_transaction()?;
                let row = repo::thread_require(&tx, &id)?;
                let mut fold = Fold::new(id.clone());
                let mut after = 0;
                loop {
                    let batch = repo::read_events(&tx, &id, after, 4096)?;
                    let Some(last) = batch.last() else { break };
                    after = last.seq;
                    for e in &batch {
                        fold.apply(e.seq, &e.time, &e.event);
                    }
                }
                tx.finish()?;
                let mut thread = row.thread.clone();
                thread.status = if row.archived {
                    ThreadStatus::Archived
                } else {
                    fold.status
                };
                if let Some(updated) = &fold.updated
                    && updated > &thread.updated
                {
                    thread.updated = updated.clone();
                }
                Ok(ThreadSnapshot {
                    thread,
                    pending_approvals: fold.pending().cloned().collect(),
                    queue: fold.queue,
                    queue_hold: fold.queue_hold,
                    turns: fold.turns,
                    seq: fold.seq,
                })
            })
            .await
    }

    /// Archives a thread. Fails with `conflict` while a turn runs or waits.
    pub async fn thread_archive(&self, id: &ThreadId) -> KernelResult<()> {
        if self.has_run(id) {
            return Err(KernelError::Conflict(format!(
                "thread {id} has a running turn"
            )));
        }
        let thread = id.clone();
        self.inner
            .store
            .write(move |c| {
                let id = thread;
                let row = repo::thread_require(c, &id)?;
                if matches!(
                    row.thread.status,
                    ThreadStatus::Running | ThreadStatus::AwaitingApproval
                ) {
                    return Err(KernelError::Conflict(format!(
                        "thread {id} has a running turn"
                    )));
                }
                repo::thread_set_archived(c, &id)
            })
            .await?;
        self.announce_row(id, WorkspaceEvent::ThreadArchived).await;
        Ok(())
    }

    /// Restores an archived thread back into its workspace's active threads.
    pub async fn thread_unarchive(&self, id: &ThreadId) -> KernelResult<Thread> {
        let thread = id.clone();
        let updated = self.now();
        let row = self
            .inner
            .store
            .write(move |c| repo::thread_set_unarchived(c, &thread, &updated))
            .await?;
        let public = row.public();
        self.announce(WorkspaceEvent::ThreadUpdated(ThreadChange {
            thread: public.clone(),
        }));
        Ok(public)
    }

    /// Permanently deletes a thread and its history. Fails with `conflict` while a turn runs.
    pub async fn thread_delete(&self, id: &ThreadId) -> KernelResult<()> {
        if self.has_run(id) {
            return Err(KernelError::Conflict(format!(
                "thread {id} has a running turn"
            )));
        }
        let session = self.lock_sessions().remove(id);
        self.gateway_revoke(id);
        if let Some(session) = session {
            let provider = self
                .lock_providers()
                .iter()
                .find(|p| p.id() == session.provider)
                .cloned();
            if let Some(provider) = provider {
                provider.end_session(session.id).await;
            }
        }
        let thread = id.clone();
        let deleted = self
            .inner
            .store
            .write(move |c| repo::thread_delete(c, &thread))
            .await?;
        lock(&self.inner.heads).remove(id);
        if let Some(wt_path) = deleted.worktree_path.as_deref() {
            let wt_check = wt_path.to_owned();
            let remaining = self
                .inner
                .store
                .read(move |c| repo::threads_using_worktree(c, &wt_check))
                .await
                .unwrap_or(1);
            if remaining == 0 {
                self.remove_worktree_if_managed(wt_path).await;
            }
        }
        self.announce(WorkspaceEvent::ThreadDeleted(ThreadChange {
            thread: deleted,
        }));
        Ok(())
    }

    // ---- workspace events ----

    /// Live changes to threads on every workspace: created, status or title changed,
    /// archived. A listener that falls more than a buffer behind gets `Lagged` and should
    /// list the threads again.
    pub fn workspace_changes(&self) -> broadcast::Receiver<WorkspaceEvent> {
        self.inner.workspace_events.subscribe()
    }

    /// Fails with `not_found` if the workspace does not exist.
    pub async fn workspace_require(&self, id: &WorkspaceId) -> KernelResult<()> {
        let ws = id.clone();
        self.inner
            .store
            .read(move |c| match repo::workspace_get(c, &ws)? {
                Some(_) => Ok(()),
                None => Err(KernelError::NotFound(format!("no workspace {ws}"))),
            })
            .await
    }

    pub(crate) fn announce(&self, event: WorkspaceEvent) {
        // No listener is not an error.
        let _ = self.inner.workspace_events.send(event);
    }

    /// Announces a thread as it is stored now.
    async fn announce_row(&self, id: &ThreadId, make: fn(ThreadChange) -> WorkspaceEvent) {
        if self.inner.workspace_events.receiver_count() == 0 {
            return;
        }
        match self.thread_row(id).await {
            Ok(row) => self.announce(make(ThreadChange {
                thread: row.public(),
            })),
            Err(e) => tracing::warn!(thread = %id, error = %e, "cannot announce a thread change"),
        }
    }

    // ---- the log ----

    /// Appends an event, assigning `seq` and `time`, and wakes subscribers after the
    /// commit. Kernel-internal: providers emit through `EventSink`, which validates.
    pub(crate) async fn append(&self, thread: &ThreadId, event: ThreadEvent) -> KernelResult<u64> {
        let time = self.now();
        let id = thread.clone();
        // Only these kinds move a thread's derived status.
        let changes_status = matches!(
            event,
            ThreadEvent::TurnStarted(_)
                | ThreadEvent::ApprovalRequested(_)
                | ThreadEvent::ApprovalResolved(_)
                | ThreadEvent::TurnEnded(_)
        );
        let seq = self
            .inner
            .store
            .write(move |c| repo::append_event(c, &id, &time, &event))
            .await?;
        self.bump_head(thread, seq);
        if changes_status {
            self.announce_row(thread, WorkspaceEvent::ThreadUpdated)
                .await;
        }
        Ok(seq)
    }

    fn bump_head(&self, thread: &ThreadId, seq: u64) {
        let mut heads = self.lock_heads();
        match heads.get(thread) {
            Some(tx) => {
                tx.send_if_modified(|h| {
                    let newer = seq > *h;
                    if newer {
                        *h = seq;
                    }
                    newer
                });
            }
            None => {
                heads.insert(thread.clone(), watch::Sender::new(seq));
            }
        }
    }

    fn head_receiver(&self, thread: &ThreadId) -> watch::Receiver<u64> {
        self.lock_heads()
            .entry(thread.clone())
            .or_insert_with(|| watch::Sender::new(0))
            .subscribe()
    }

    /// Subscribes to a thread's log after `after`. Replay and live delivery are the same
    /// path: the subscription reads the log whenever the thread's head moves, so there is
    /// no gap or duplicate at the switchover.
    pub async fn subscribe(
        &self,
        thread: &ThreadId,
        after: u64,
    ) -> KernelResult<ThreadSubscription> {
        let head = self.thread_row(thread).await?.head;
        // The first `next_batch` reads the log after this receiver exists, so a commit
        // between the head read above and this line is still delivered.
        let rx = self.head_receiver(thread);
        Ok(ThreadSubscription {
            kernel: self.clone(),
            thread: thread.clone(),
            cursor: after,
            head,
            rx,
        })
    }
}

/// A cursor over one thread's log that waits for new events.
#[derive(Debug)]
pub struct ThreadSubscription {
    kernel: Kernel,
    thread: ThreadId,
    cursor: u64,
    head: u64,
    rx: watch::Receiver<u64>,
}

impl ThreadSubscription {
    /// The thread's last `seq` when the subscription started.
    pub fn head(&self) -> u64 {
        self.head
    }

    /// The thread.
    pub fn thread(&self) -> &ThreadId {
        &self.thread
    }

    /// The next events after the cursor, waiting until there is at least one. Returns
    /// `None` when the kernel closes.
    pub async fn next_batch(&mut self) -> KernelResult<Option<Vec<StoredEvent>>> {
        loop {
            // Mark the current head as seen *before* reading, so a commit that lands after
            // the read is guaranteed to wake `changed()` below.
            self.rx.borrow_and_update();
            let (thread, after) = (self.thread.clone(), self.cursor);
            let batch = self
                .kernel
                .inner
                .store
                .read(move |c| repo::read_events(c, &thread, after, REPLAY_BATCH))
                .await;
            let batch = match batch {
                Ok(b) => b,
                Err(KernelError::Store(StoreError::Closed)) => return Ok(None),
                Err(e) => return Err(e),
            };
            if let Some(last) = batch.last() {
                self.cursor = last.seq;
                return Ok(Some(batch));
            }
            if self.rx.changed().await.is_err() {
                return Ok(None);
            }
        }
    }
}

/// Canonicalizes an absolute directory path into the string stored as a workspace root.
async fn canonical_dir(path: &str) -> KernelResult<String> {
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(KernelError::InvalidParams(format!(
            "path must be absolute: {path}"
        )));
    }
    let shown = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let canonical = std::fs::canonicalize(&p)
            .map_err(|e| KernelError::NotFound(format!("cannot open {shown}: {e}")))?;
        if !canonical.is_dir() {
            return Err(KernelError::InvalidParams(format!(
                "not a directory: {shown}"
            )));
        }
        let s = canonical
            .to_str()
            .ok_or_else(|| KernelError::InvalidParams(format!("path is not UTF-8: {shown}")))?;
        Ok(strip_verbatim(s).to_owned())
    })
    .await
    .map_err(|e| StoreError::Task(e.to_string()))?
}

/// On Windows `canonicalize` returns `\\?\C:\…`; show the plain form for drive paths.
fn strip_verbatim(s: &str) -> &str {
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest,
        _ => s,
    }
}

fn expand_tilde(path: &str) -> String {
    let home = std::env::var("HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|v| !v.is_empty()));
    let Some(home) = home else {
        return path.to_owned();
    };
    if path == "~" {
        return home;
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix(r"~\")) {
        return Path::new(&home).join(rest).to_string_lossy().into_owned();
    }
    path.to_owned()
}

#[cfg(test)]
mod tests;
