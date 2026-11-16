//! The kernel service: workspaces, threads, the event log and subscriptions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use kumikit_protocol::event::{ApprovalResolved, ThreadChange, TurnEnded};
use kumikit_protocol::methods::ThreadSnapshot;
use kumikit_protocol::model::{
    Resolution, Thread, ThreadId, ThreadStatus, Timestamp, TurnOutcome, Workspace, WorkspaceId,
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
}

impl KernelConfig {
    /// System clock and ULIDs, database at `database`, secrets in `secrets.json` next to
    /// it, the supervisor using this process's environment.
    pub fn new(database: impl Into<PathBuf>) -> Self {
        let database = database.into();
        KernelConfig {
            secrets: database.with_file_name("secrets.json"),
            database,
            clock: Arc::new(SystemClock),
            ids: Arc::new(UlidGen::default()),
            supervisor: SupervisorConfig::from_process(),
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
    /// Threads with a running turn.
    runs: Mutex<HashMap<ThreadId, ThreadRun>>,
    /// Provider session per thread, started on first use in this process.
    sessions: Mutex<HashMap<ThreadId, Session>>,
    pub(crate) secrets: SecretStore,
    pub(crate) supervisor: Supervisor,
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
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
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
                runs: Mutex::new(HashMap::new()),
                sessions: Mutex::new(HashMap::new()),
                secrets: SecretStore::new(config.secrets),
                supervisor: Supervisor::new(config.supervisor),
            }),
        };
        kernel.recover().await?;
        Ok(kernel)
    }

    /// Abandons running turns (the next start ends them as `core_restarted`), ends
    /// provider sessions and subscriptions, kills supervised processes, and closes the
    /// database after queued writes.
    pub async fn close(&self) {
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
                let ws = Workspace { id, name, root };
                repo::workspace_insert(c, &ws, &created)?;
                Ok(ws)
            })
            .await
    }

    /// Open workspaces.
    pub async fn workspace_list(&self) -> KernelResult<Vec<Workspace>> {
        self.inner.store.read(repo::workspace_list).await
    }

    /// Hides a workspace from the list; its threads and events are kept.
    pub async fn workspace_close(&self, id: &WorkspaceId) -> KernelResult<()> {
        let id = id.clone();
        self.inner
            .store
            .write(move |c| repo::workspace_set_closed(c, &id, true))
            .await
    }

    // ---- threads ----

    /// Creates a thread with a concrete provider and model (see `thread_create`).
    pub(crate) async fn thread_insert(
        &self,
        workspace: &WorkspaceId,
        provider: String,
        model: String,
        title: Option<String>,
    ) -> KernelResult<Thread> {
        let now = self.now();
        let thread = Thread {
            id: ThreadId(self.new_id()),
            workspace: workspace.clone(),
            title,
            provider,
            model,
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
                if let Some(updated) = &fold.updated {
                    thread.updated = updated.clone();
                }
                Ok(ThreadSnapshot {
                    thread,
                    pending_approvals: fold.pending().cloned().collect(),
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

    fn announce(&self, event: WorkspaceEvent) {
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

#[cfg(test)]
mod tests;
