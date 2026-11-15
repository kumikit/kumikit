//! SQL for the domain tables. Every function is synchronous and runs inside a store job
//! (writes on the writer thread, reads on the read pool).

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::model::{
    ApprovalId, ApprovalStatus, Thread, ThreadId, ThreadStatus, Timestamp, TurnId, Workspace,
    WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use crate::error::{KernelError, KernelResult};
use crate::fold::next_status;
use crate::store::StoreError;

/// A workspace row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRow {
    /// The workspace.
    pub workspace: Workspace,
    /// Hidden from `workspace.list`.
    pub closed: bool,
}

/// A thread row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRow {
    /// The thread, with `status` as derived from the log.
    pub thread: Thread,
    /// Archived by `thread.archive`.
    pub archived: bool,
    /// Last assigned seq.
    pub head: u64,
    /// The provider's own session id, if it has one.
    pub provider_session: Option<String>,
}

impl ThreadRow {
    /// The thread as clients see it: `archived` overrides the log status.
    pub fn public(&self) -> Thread {
        let mut t = self.thread.clone();
        if self.archived {
            t.status = ThreadStatus::Archived;
        }
        t
    }
}

/// An event as stored in the log.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    /// Sequence number within the thread.
    pub seq: u64,
    /// When it was appended.
    pub time: Timestamp,
    /// Kind and data.
    pub event: ThreadEvent,
}

fn to_u64(v: i64) -> KernelResult<u64> {
    u64::try_from(v).map_err(|_| StoreError::Corrupt(format!("negative integer {v}")).into())
}

fn to_i64(v: u64) -> KernelResult<i64> {
    i64::try_from(v).map_err(|_| KernelError::InvalidParams(format!("integer {v} out of range")))
}

fn workspace_from_row(row: &Row<'_>) -> rusqlite::Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        workspace: Workspace {
            id: WorkspaceId(row.get("id")?),
            name: row.get("name")?,
            root: row.get("root")?,
        },
        closed: row.get::<_, i64>("closed")? != 0,
    })
}

/// The workspace with this root, if any.
pub fn workspace_by_root(conn: &Connection, root: &str) -> KernelResult<Option<WorkspaceRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, root, closed FROM workspaces WHERE root = ?1",
            [root],
            workspace_from_row,
        )
        .optional()?)
}

/// The workspace with this id, if any.
pub fn workspace_get(conn: &Connection, id: &WorkspaceId) -> KernelResult<Option<WorkspaceRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, root, closed FROM workspaces WHERE id = ?1",
            [id.as_str()],
            workspace_from_row,
        )
        .optional()?)
}

/// Inserts a new open workspace.
pub fn workspace_insert(
    conn: &Connection,
    ws: &Workspace,
    created: &Timestamp,
) -> KernelResult<()> {
    conn.execute(
        "INSERT INTO workspaces (id, name, root, created) VALUES (?1, ?2, ?3, ?4)",
        params![ws.id.as_str(), ws.name, ws.root, created.0],
    )?;
    Ok(())
}

/// Sets a workspace's `closed` flag. Fails with `not_found` if it does not exist.
pub fn workspace_set_closed(conn: &Connection, id: &WorkspaceId, closed: bool) -> KernelResult<()> {
    let n = conn.execute(
        "UPDATE workspaces SET closed = ?2 WHERE id = ?1",
        params![id.as_str(), i64::from(closed)],
    )?;
    if n == 0 {
        return Err(KernelError::NotFound(format!("no workspace {id}")));
    }
    Ok(())
}

/// Open workspaces, by name.
pub fn workspace_list(conn: &Connection) -> KernelResult<Vec<Workspace>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, root, closed FROM workspaces WHERE closed = 0 ORDER BY name, id",
    )?;
    let rows = stmt.query_map([], workspace_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?.workspace);
    }
    Ok(out)
}

const THREAD_COLUMNS: &str = "id, workspace, title, provider, model, status, archived, created, updated, head, provider_session";

fn thread_from_row(row: &Row<'_>) -> rusqlite::Result<(ThreadRow, String, i64)> {
    let status: String = row.get("status")?;
    let head: i64 = row.get("head")?;
    Ok((
        ThreadRow {
            thread: Thread {
                id: ThreadId(row.get("id")?),
                workspace: WorkspaceId(row.get("workspace")?),
                title: row.get("title")?,
                provider: row.get("provider")?,
                model: row.get("model")?,
                status: ThreadStatus::Idle,
                created: Timestamp(row.get("created")?),
                updated: Timestamp(row.get("updated")?),
            },
            archived: row.get::<_, i64>("archived")? != 0,
            head: 0,
            provider_session: row.get("provider_session")?,
        },
        status,
        head,
    ))
}

fn finish_thread((mut row, status, head): (ThreadRow, String, i64)) -> KernelResult<ThreadRow> {
    row.thread.status = ThreadStatus::parse(&status)
        .ok_or_else(|| StoreError::Corrupt(format!("unknown thread status {status:?}")))?;
    row.head = to_u64(head)?;
    Ok(row)
}

/// Inserts a new thread (status `idle`, head 0).
pub fn thread_insert(conn: &Connection, t: &Thread) -> KernelResult<()> {
    conn.execute(
        "INSERT INTO threads (id, workspace, title, provider, model, status, created, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            t.id.as_str(),
            t.workspace.as_str(),
            t.title,
            t.provider,
            t.model,
            t.status.as_str(),
            t.created.0,
            t.updated.0
        ],
    )?;
    Ok(())
}

/// The thread with this id, if any.
pub fn thread_get(conn: &Connection, id: &ThreadId) -> KernelResult<Option<ThreadRow>> {
    conn.query_row(
        &format!("SELECT {THREAD_COLUMNS} FROM threads WHERE id = ?1"),
        [id.as_str()],
        thread_from_row,
    )
    .optional()?
    .map(finish_thread)
    .transpose()
}

/// The thread with this id, or `not_found`.
pub fn thread_require(conn: &Connection, id: &ThreadId) -> KernelResult<ThreadRow> {
    thread_get(conn, id)?.ok_or_else(|| KernelError::NotFound(format!("no thread {id}")))
}

/// Threads of a workspace, most recently updated first.
pub fn thread_list(
    conn: &Connection,
    workspace: &WorkspaceId,
    include_archived: bool,
) -> KernelResult<Vec<ThreadRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {THREAD_COLUMNS} FROM threads
         WHERE workspace = ?1 AND (?2 OR archived = 0)
         ORDER BY updated DESC, id DESC"
    ))?;
    let rows = stmt.query_map(
        params![workspace.as_str(), include_archived],
        thread_from_row,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(finish_thread(row?)?);
    }
    Ok(out)
}

/// Marks a thread archived.
pub fn thread_set_archived(conn: &Connection, id: &ThreadId) -> KernelResult<()> {
    conn.execute(
        "UPDATE threads SET archived = 1 WHERE id = ?1",
        [id.as_str()],
    )?;
    Ok(())
}

/// Stores the provider's session id for a thread.
pub fn thread_set_session(conn: &Connection, id: &ThreadId, session: &str) -> KernelResult<()> {
    conn.execute(
        "UPDATE threads SET provider_session = ?2 WHERE id = ?1",
        params![id.as_str(), session],
    )?;
    Ok(())
}

/// Appends an event to a thread's log and updates the derived columns in the same
/// transaction. Returns the new `seq`.
pub fn append_event(
    conn: &Connection,
    thread: &ThreadId,
    time: &Timestamp,
    event: &ThreadEvent,
) -> KernelResult<u64> {
    let row = thread_require(conn, thread)?;
    if row.archived && matches!(event, ThreadEvent::TurnStarted(_)) {
        return Err(KernelError::Conflict(format!(
            "thread {thread} is archived"
        )));
    }
    let seq = row.head + 1;
    let seq_i = to_i64(seq)?;

    let mut value = serde_json::to_value(event)?;
    let data = value
        .get_mut("data")
        .map(Value::take)
        .unwrap_or(Value::Null);
    conn.execute(
        "INSERT INTO events (thread, seq, time, kind, data) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            thread.as_str(),
            seq_i,
            time.0,
            event.kind(),
            data.to_string()
        ],
    )?;

    match event {
        ThreadEvent::TurnStarted(e) => {
            conn.execute(
                "INSERT INTO turns (id, thread, started_seq, started) VALUES (?1, ?2, ?3, ?4)",
                params![e.turn.as_str(), thread.as_str(), seq_i, time.0],
            )?;
        }
        ThreadEvent::TurnEnded(e) => {
            let n = conn.execute(
                "UPDATE turns SET ended = ?3, outcome = ?4
                 WHERE id = ?1 AND thread = ?2 AND ended IS NULL",
                params![e.turn.as_str(), thread.as_str(), time.0, e.outcome.as_str()],
            )?;
            if n == 0 {
                return Err(KernelError::Conflict(format!(
                    "turn {} is not running",
                    e.turn
                )));
            }
        }
        ThreadEvent::ApprovalRequested(e) => {
            let a = &e.approval;
            if &a.thread != thread || a.status != ApprovalStatus::Pending {
                return Err(KernelError::InvalidParams(
                    "approval must be pending and belong to this thread".into(),
                ));
            }
            conn.execute(
                "INSERT INTO approvals (id, thread, turn, kind, summary, detail, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
                params![
                    a.id.as_str(),
                    thread.as_str(),
                    a.turn.as_str(),
                    a.kind,
                    a.summary,
                    a.detail.as_ref().map(Value::to_string)
                ],
            )?;
        }
        ThreadEvent::ApprovalResolved(e) => {
            let status = serde_json::to_value(e.decision.status())?;
            let decision = serde_json::to_value(e.decision)?;
            let n = conn.execute(
                "UPDATE approvals SET status = ?3, decision = ?4, decided_by = ?5
                 WHERE id = ?1 AND thread = ?2 AND status = 'pending'",
                params![
                    e.approval.as_str(),
                    thread.as_str(),
                    status.as_str(),
                    decision.as_str(),
                    e.by
                ],
            )?;
            if n == 0 {
                return Err(KernelError::Conflict(format!(
                    "approval {} is not pending",
                    e.approval
                )));
            }
        }
        _ => {}
    }

    let pending: i64 = conn.query_row(
        "SELECT count(*) FROM approvals WHERE thread = ?1 AND status = 'pending'",
        [thread.as_str()],
        |r| r.get(0),
    )?;
    let pending = usize::try_from(pending).unwrap_or(usize::MAX);
    let status = next_status(row.thread.status, event, pending);
    conn.execute(
        "UPDATE threads SET head = ?2, status = ?3, updated = ?4 WHERE id = ?1",
        params![thread.as_str(), seq_i, status.as_str(), time.0],
    )?;
    Ok(seq)
}

/// Up to `limit` events with `seq > after`, in order.
pub fn read_events(
    conn: &Connection,
    thread: &ThreadId,
    after: u64,
    limit: usize,
) -> KernelResult<Vec<StoredEvent>> {
    let mut stmt = conn.prepare_cached(
        "SELECT seq, time, kind, data FROM events
         WHERE thread = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map(params![thread.as_str(), to_i64(after)?, limit], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, time, kind, data) = row?;
        let data: Value = serde_json::from_str(&data)?;
        let event: ThreadEvent =
            serde_json::from_value(serde_json::json!({ "kind": kind, "data": data }))?;
        out.push(StoredEvent {
            seq: to_u64(seq)?,
            time: Timestamp(time),
            event,
        });
    }
    Ok(out)
}

/// Turns without `turn.ended`, as `(thread, turn)`.
pub fn open_turns(conn: &Connection) -> KernelResult<Vec<(ThreadId, TurnId)>> {
    let mut stmt = conn
        .prepare("SELECT thread, id FROM turns WHERE ended IS NULL ORDER BY thread, started_seq")?;
    let rows = stmt.query_map([], |r| Ok((ThreadId(r.get(0)?), TurnId(r.get(1)?))))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Pending approvals of a turn.
pub fn pending_approvals_of_turn(
    conn: &Connection,
    turn: &TurnId,
) -> KernelResult<Vec<ApprovalId>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM approvals WHERE turn = ?1 AND status = 'pending' ORDER BY rowid",
    )?;
    let rows = stmt.query_map([turn.as_str()], |r| Ok(ApprovalId(r.get(0)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The thread an approval belongs to and its status, if it exists.
pub fn approval_get(
    conn: &Connection,
    id: &ApprovalId,
) -> KernelResult<Option<(ThreadId, TurnId, String)>> {
    Ok(conn
        .query_row(
            "SELECT thread, turn, status FROM approvals WHERE id = ?1",
            [id.as_str()],
            |r| Ok((ThreadId(r.get(0)?), TurnId(r.get(1)?), r.get(2)?)),
        )
        .optional()?)
}
