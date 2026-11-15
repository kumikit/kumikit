//! SQLite storage (`docs/spec/03-kernel.md#storage`).
//!
//! One writer thread owns the only write connection. Callers hand it closures; it runs
//! whatever is queued in one transaction (group commit), each closure inside its own
//! savepoint so one failure does not undo the others, and replies only after the commit
//! is on disk (`synchronous=FULL`). Reads run on a small pool of read-only connections
//! under `spawn_blocking`; WAL lets them proceed while the writer commits.

mod migrate;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use rusqlite::{Connection, OpenFlags};
use tokio::sync::oneshot;

pub use migrate::{MIGRATIONS, migrate};

/// Errors from the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite failed.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Stored JSON could not be read or written.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// The database was written by a newer core.
    #[error("database schema version {found} is newer than this core supports ({known})")]
    NewerSchema {
        /// `user_version` found in the file.
        found: u32,
        /// The newest version this binary knows.
        known: u32,
    },
    /// The store was closed.
    #[error("store is closed")]
    Closed,
    /// The blocking read task failed (panicked or was cancelled).
    #[error("read task failed: {0}")]
    Task(String),
    /// Stored data violates an invariant (for example an unknown status string).
    #[error("corrupt data: {0}")]
    Corrupt(String),
}

/// Result alias for storage calls.
pub type StoreResult<T> = Result<T, StoreError>;

/// A write job: `run` executes on the writer connection inside a savepoint; `finish`
/// delivers the result once the commit outcome is known (also when `run` never ran).
trait WriteJob: Send {
    /// Runs the job; returns whether it succeeded.
    fn run(&mut self, conn: &Connection) -> bool;
    /// Delivers the result. `outcome` is `Err` if the job could not be committed.
    fn finish(self: Box<Self>, outcome: StoreResult<()>);
}

type Job = Box<dyn WriteJob>;

struct TypedJob<R, E, F> {
    work: Option<F>,
    result: Option<Result<R, E>>,
    reply: oneshot::Sender<Result<R, E>>,
}

impl<R, E, F> WriteJob for TypedJob<R, E, F>
where
    R: Send,
    E: From<StoreError> + Send,
    F: FnOnce(&Connection) -> Result<R, E> + Send,
{
    fn run(&mut self, conn: &Connection) -> bool {
        let Some(work) = self.work.take() else {
            return false;
        };
        let result = work(conn);
        let ok = result.is_ok();
        self.result = Some(result);
        ok
    }

    fn finish(self: Box<Self>, outcome: StoreResult<()>) {
        let result = match (outcome, self.result) {
            (Err(e), _) => Err(E::from(e)),
            (Ok(()), Some(result)) => result,
            (Ok(()), None) => Err(E::from(StoreError::Task("write job did not run".into()))),
        };
        let _ = self.reply.send(result);
    }
}

enum Command {
    Job(Job),
    Close(oneshot::Sender<()>),
}

/// Handle to the database. Cheap to clone.
#[derive(Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    writer: mpsc::Sender<Command>,
    readers: Mutex<Vec<Connection>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.inner.path)
            .finish_non_exhaustive()
    }
}

/// Maximum idle read connections kept in the pool.
const MAX_IDLE_READERS: usize = 4;

impl Store {
    /// Opens (creating if needed) the database at `path`, applies `migrations`, and
    /// starts the writer thread. Blocking: call it from `spawn_blocking` or at startup.
    pub fn open(path: &Path, migrations: &[&str]) -> StoreResult<Store> {
        let mut conn = Connection::open(path)?;
        configure_writer(&conn)?;
        migrate(&mut conn, migrations)?;

        let (tx, rx) = mpsc::channel::<Command>();
        thread::Builder::new()
            .name("kumikit-store-writer".into())
            .spawn(move || writer_loop(conn, rx))
            .map_err(|e| StoreError::Task(format!("spawn writer: {e}")))?;

        Ok(Store {
            inner: Arc::new(Inner {
                path: path.to_path_buf(),
                writer: tx,
                readers: Mutex::new(Vec::new()),
            }),
        })
    }

    /// Runs `f` on the writer connection, inside a transaction shared with other queued
    /// writes. Returns after the transaction has committed. If `f` fails, only its own
    /// changes are rolled back.
    pub async fn write<R, E, F>(&self, f: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Connection) -> Result<R, E> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel::<Result<R, E>>();
        let job: Job = Box::new(TypedJob {
            work: Some(f),
            result: None,
            reply: tx,
        });
        self.inner
            .writer
            .send(Command::Job(job))
            .map_err(|_| E::from(StoreError::Closed))?;
        rx.await.map_err(|_| E::from(StoreError::Closed))?
    }

    /// Runs `f` on a read-only connection on the blocking pool.
    pub async fn read<R, E, F>(&self, f: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Connection) -> Result<R, E> + Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let conn = match inner.lock_readers().pop() {
                Some(c) => c,
                None => open_reader(&inner.path).map_err(E::from)?,
            };
            let result = f(&conn);
            let mut pool = inner.lock_readers();
            if pool.len() < MAX_IDLE_READERS {
                pool.push(conn);
            }
            result
        })
        .await
        .map_err(|e| E::from(StoreError::Task(e.to_string())))?
    }

    /// Stops the writer after the jobs already queued, and waits for it. Later calls to
    /// [`Store::write`] fail with [`StoreError::Closed`].
    pub async fn close(&self) {
        let (tx, rx) = oneshot::channel();
        if self.inner.writer.send(Command::Close(tx)).is_ok() {
            let _ = rx.await;
        }
        self.inner.lock_readers().clear();
    }
}

impl Inner {
    fn lock_readers(&self) -> std::sync::MutexGuard<'_, Vec<Connection>> {
        // A panic while holding this lock leaves only a Vec of idle connections behind,
        // which is still valid, so recover from poisoning.
        self.readers.lock().unwrap_or_else(|p| p.into_inner())
    }
}

fn configure_writer(conn: &Connection) -> StoreResult<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

fn open_reader(path: &Path) -> StoreResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

fn writer_loop(conn: Connection, rx: mpsc::Receiver<Command>) {
    let mut close: Option<oneshot::Sender<()>> = None;
    while close.is_none() {
        let Ok(first) = rx.recv() else { break };
        let mut batch = Vec::new();
        match first {
            Command::Job(job) => batch.push(job),
            Command::Close(done) => close = Some(done),
        }
        // Group commit: take everything already queued.
        while close.is_none() {
            match rx.try_recv() {
                Ok(Command::Job(job)) => batch.push(job),
                Ok(Command::Close(done)) => close = Some(done),
                Err(_) => break,
            }
        }
        if !batch.is_empty() {
            run_batch(&conn, batch);
        }
    }
    drop(conn);
    if let Some(done) = close {
        let _ = done.send(());
    }
}

fn run_batch(conn: &Connection, batch: Vec<Job>) {
    if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
        let msg = e.to_string();
        for job in batch {
            job.finish(Err(StoreError::Task(format!("begin failed: {msg}"))));
        }
        return;
    }
    let mut done: Vec<(Job, Option<StoreError>)> = Vec::with_capacity(batch.len());
    for mut job in batch {
        if let Err(e) = conn.execute_batch("SAVEPOINT job") {
            done.push((job, Some(StoreError::Sqlite(e))));
            continue;
        }
        let ok = job.run(conn);
        let end = if ok {
            conn.execute_batch("RELEASE job")
        } else {
            // The job's own error is delivered by `finish`; this only undoes its writes.
            conn.execute_batch("ROLLBACK TO job; RELEASE job")
        };
        done.push((job, end.err().map(StoreError::Sqlite)));
    }
    let commit = conn.execute_batch("COMMIT");
    let commit_msg = commit.err().map(|e| e.to_string());
    if commit_msg.is_some() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    for (job, own_error) in done {
        let outcome = match (&commit_msg, own_error) {
            (Some(msg), _) => Err(StoreError::Task(format!("commit failed: {msg}"))),
            (None, Some(e)) => Err(e),
            (None, None) => Ok(()),
        };
        job.finish(outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SCHEMA: &[&str] =
        &["CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL) STRICT;"];

    fn open(dir: &tempfile::TempDir) -> Store {
        Store::open(&dir.path().join("db.sqlite"), TEST_SCHEMA).unwrap()
    }

    #[tokio::test]
    async fn write_then_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let _: usize = store
            .write(|c| Ok::<_, StoreError>(c.execute("INSERT INTO t (v) VALUES ('a')", [])?))
            .await
            .unwrap();
        let n: i64 = store
            .read(|c| Ok::<_, StoreError>(c.query_row("SELECT count(*) FROM t", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(n, 1);
        store.close().await;
    }

    #[tokio::test]
    async fn failed_job_rolls_back_only_itself() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let bad = store.write(|c| {
            c.execute("INSERT INTO t (v) VALUES ('bad')", [])?;
            Err::<(), _>(StoreError::Corrupt("boom".into()))
        });
        let good = store
            .write(|c| Ok::<_, StoreError>(c.execute("INSERT INTO t (v) VALUES ('good')", [])?));
        let (bad, good) = tokio::join!(bad, good);
        assert!(matches!(bad, Err(StoreError::Corrupt(_))));
        good.unwrap();
        let rows: Vec<String> = store
            .read(|c| {
                let mut s = c.prepare("SELECT v FROM t ORDER BY id")?;
                let rows = s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
                Ok::<_, StoreError>(rows)
            })
            .await
            .unwrap();
        assert_eq!(rows, vec!["good".to_owned()]);
        store.close().await;
    }

    #[tokio::test]
    async fn many_concurrent_writes_all_commit() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let writes = (0..200).map(|i| {
            let s = store.clone();
            tokio::spawn(async move {
                s.write(move |c| {
                    Ok::<_, StoreError>(
                        c.execute("INSERT INTO t (v) VALUES (?1)", [i.to_string()])?,
                    )
                })
                .await
            })
        });
        for w in writes.collect::<Vec<_>>() {
            w.await.unwrap().unwrap();
        }
        let n: i64 = store
            .read(|c| Ok::<_, StoreError>(c.query_row("SELECT count(*) FROM t", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(n, 200);
        store.close().await;
    }

    #[tokio::test]
    async fn pragmas_are_set() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let (mode, sync, fk): (String, i64, i64) = store
            .write(|c| {
                Ok::<_, StoreError>((
                    c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
                    c.query_row("PRAGMA synchronous", [], |r| r.get(0))?,
                    c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(sync, 2, "synchronous=FULL");
        assert_eq!(fk, 1);
        store.close().await;
    }

    #[tokio::test]
    async fn readers_cannot_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let r = store
            .read(|c| Ok::<_, StoreError>(c.execute("INSERT INTO t (v) VALUES ('x')", [])?))
            .await;
        assert!(r.is_err());
        store.close().await;
    }

    #[tokio::test]
    async fn closed_store_rejects_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        store.close().await;
        let r: StoreResult<()> = store.write(|_| Ok(())).await;
        assert!(matches!(r, Err(StoreError::Closed)));
    }
}
