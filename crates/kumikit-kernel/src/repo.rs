//! SQL for the domain tables. Every function is synchronous and runs inside a store job
//! (writes on the writer thread, reads on the read pool).

use std::collections::BTreeMap;

use kumikit_protocol::ThreadEvent;
use kumikit_protocol::event::{MessageFrom, QueueEntry, QueueHold, QueueTrigger};
use kumikit_protocol::model::{
    ApprovalId, ApprovalStatus, PermissionMode, QueueEntryId, Thread, ThreadForkInfo, ThreadId,
    ThreadStatus, Timestamp, TitleSource, TurnId, TurnPurpose, UserInput, Workspace, WorkspaceId,
    WorkspaceKind,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use crate::error::{KernelError, KernelResult};
use crate::fold::next_status;
use crate::handoff::PendingHandoff;
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
    /// Pending handoff waiting for the next completed turn, if any.
    pub pending_handoff: Option<PendingHandoff>,
    /// Last parent-thread event seq already shared with this side thread.
    pub parent_seen_seq: u64,
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
    let kind_str: String = row.get("kind")?;
    let kind = match WorkspaceKind::parse(&kind_str) {
        Some(WorkspaceKind::Chat) => Some(WorkspaceKind::Chat),
        _ => None,
    };
    Ok(WorkspaceRow {
        workspace: Workspace {
            id: WorkspaceId(row.get("id")?),
            name: row.get("name")?,
            root: row.get("root")?,
            kind,
        },
        closed: row.get::<_, i64>("closed")? != 0,
    })
}

/// The workspace with this root, if any.
pub fn workspace_by_root(conn: &Connection, root: &str) -> KernelResult<Option<WorkspaceRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, root, kind, closed FROM workspaces WHERE root = ?1",
            [root],
            workspace_from_row,
        )
        .optional()?)
}

/// The core-managed chat workspace, if one has been created.
pub fn workspace_chat(conn: &Connection) -> KernelResult<Option<WorkspaceRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, root, kind, closed FROM workspaces WHERE kind = 'chat' LIMIT 1",
            [],
            workspace_from_row,
        )
        .optional()?)
}

/// The workspace with this id, if any.
pub fn workspace_get(conn: &Connection, id: &WorkspaceId) -> KernelResult<Option<WorkspaceRow>> {
    Ok(conn
        .query_row(
            "SELECT id, name, root, kind, closed FROM workspaces WHERE id = ?1",
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
    let kind = ws.kind.unwrap_or_default().as_str();
    conn.execute(
        "INSERT INTO workspaces (id, name, root, kind, created) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![ws.id.as_str(), ws.name, ws.root, kind, created.0],
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

/// Permanently deletes a workspace and all its threads, turns, approvals, events, and plugin links.
/// Fails with `not_found` if absent, or `conflict` if any thread has a running or waiting turn.
pub fn workspace_delete(conn: &Connection, id: &WorkspaceId) -> KernelResult<Vec<Thread>> {
    if workspace_get(conn, id)?.is_none() {
        return Err(KernelError::NotFound(format!("no workspace {id}")));
    }
    let rows = thread_list(conn, id, true, true)?;
    for row in &rows {
        if matches!(
            row.thread.status,
            ThreadStatus::Running | ThreadStatus::AwaitingApproval
        ) {
            return Err(KernelError::Conflict(format!(
                "thread {} in workspace {id} has a running turn",
                row.thread.id
            )));
        }
    }
    let wid = id.as_str();
    conn.execute(
        "DELETE FROM queued_messages WHERE thread IN (SELECT id FROM threads WHERE workspace = ?1)",
        [wid],
    )?;
    conn.execute(
        "DELETE FROM approvals WHERE thread IN (SELECT id FROM threads WHERE workspace = ?1)",
        [wid],
    )?;
    conn.execute(
        "DELETE FROM turns WHERE thread IN (SELECT id FROM threads WHERE workspace = ?1)",
        [wid],
    )?;
    conn.execute(
        "DELETE FROM events WHERE thread IN (SELECT id FROM threads WHERE workspace = ?1)",
        [wid],
    )?;
    conn.execute("DELETE FROM threads WHERE workspace = ?1", [wid])?;
    conn.execute("DELETE FROM plugin_workspaces WHERE workspace = ?1", [wid])?;
    conn.execute("DELETE FROM workspaces WHERE id = ?1", [wid])?;
    Ok(rows.into_iter().map(|r| r.public()).collect())
}

/// Open workspaces, by name.
pub fn workspace_list(conn: &Connection) -> KernelResult<Vec<Workspace>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, root, kind, closed FROM workspaces WHERE closed = 0 ORDER BY name, id",
    )?;
    let rows = stmt.query_map([], workspace_from_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?.workspace);
    }
    Ok(out)
}

const THREAD_COLUMNS: &str = "id, workspace, title, title_source, provider, model, permission_mode, worktree_path, forked_from_json, parent_thread, side, parent_seen_seq, status, archived, created, updated, head, provider_session, pending_handoff, options_json";

#[allow(clippy::type_complexity)]
fn thread_from_row(
    row: &Row<'_>,
) -> rusqlite::Result<(
    ThreadRow,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<String>,
)> {
    let status: String = row.get("status")?;
    let permission_mode: Option<String> = row.get("permission_mode")?;
    let title_source: Option<String> = row.get("title_source")?;
    let forked_from_json: Option<String> = row.get("forked_from_json")?;
    let parent_thread: Option<String> = row.get("parent_thread")?;
    let side_flag: i64 = row.get("side")?;
    let parent_seen_seq: i64 = row.get("parent_seen_seq")?;
    let head: i64 = row.get("head")?;
    let pending_handoff: Option<String> = row.get("pending_handoff")?;
    let options_json: Option<String> = row.get("options_json")?;
    let options = match options_json {
        None => BTreeMap::new(),
        Some(raw) => serde_json::from_str(&raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?,
    };
    Ok((
        ThreadRow {
            thread: Thread {
                id: ThreadId(row.get("id")?),
                workspace: WorkspaceId(row.get("workspace")?),
                title: row.get("title")?,
                title_source: None,
                provider: row.get("provider")?,
                model: row.get("model")?,
                permission_mode: None,
                options,
                worktree_path: row.get("worktree_path")?,
                forked_from: None,
                parent: parent_thread.map(ThreadId),
                side: (side_flag != 0).then_some(true),
                status: ThreadStatus::Idle,
                created: Timestamp(row.get("created")?),
                updated: Timestamp(row.get("updated")?),
            },
            archived: row.get::<_, i64>("archived")? != 0,
            head: 0,
            provider_session: row.get("provider_session")?,
            pending_handoff: None,
            parent_seen_seq: 0,
        },
        status,
        permission_mode,
        title_source,
        forked_from_json,
        parent_seen_seq,
        head,
        pending_handoff,
    ))
}

#[allow(clippy::type_complexity)]
fn finish_thread(
    (
        mut row,
        status,
        permission_mode,
        title_source,
        forked_from_json,
        parent_seen_seq,
        head,
        pending_handoff,
    ): (
        ThreadRow,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        i64,
        Option<String>,
    ),
) -> KernelResult<ThreadRow> {
    row.thread.status = ThreadStatus::parse(&status)
        .ok_or_else(|| StoreError::Corrupt(format!("unknown thread status {status:?}")))?;
    row.thread.permission_mode = match permission_mode {
        None => None,
        Some(m) => Some(
            PermissionMode::parse(&m)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown permission mode {m:?}")))?,
        ),
    };
    row.thread.title_source = match title_source {
        None => None,
        Some(s) => Some(
            TitleSource::parse(&s)
                .ok_or_else(|| StoreError::Corrupt(format!("unknown title source {s:?}")))?,
        ),
    };
    row.thread.forked_from = match forked_from_json {
        None => None,
        Some(raw) => Some(serde_json::from_str::<ThreadForkInfo>(&raw)?),
    };
    row.pending_handoff = match pending_handoff {
        None => None,
        Some(raw) => Some(serde_json::from_str(&raw)?),
    };
    row.parent_seen_seq = to_u64(parent_seen_seq)?;
    row.head = to_u64(head)?;
    Ok(row)
}

/// Inserts a new thread (status `idle`, head 0).
pub fn thread_insert(conn: &Connection, t: &Thread) -> KernelResult<()> {
    let forked_json = t
        .forked_from
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        "INSERT INTO threads (id, workspace, title, provider, model, permission_mode, worktree_path, forked_from_json, parent_thread, side, status, created, updated, options_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            t.id.as_str(),
            t.workspace.as_str(),
            t.title,
            t.provider,
            t.model,
            t.permission_mode.map(PermissionMode::as_str),
            t.worktree_path,
            forked_json,
            t.parent.as_ref().map(ThreadId::as_str),
            i64::from(t.side.unwrap_or(false)),
            t.status.as_str(),
            t.created.0,
            t.updated.0,
            options_json(&t.options)?
        ],
    )?;
    Ok(())
}

fn options_json(options: &BTreeMap<String, String>) -> KernelResult<Option<String>> {
    Ok(if options.is_empty() {
        None
    } else {
        Some(serde_json::to_string(options)?)
    })
}

/// Replaces a thread's model settings (ADR 0033).
pub fn thread_set_options(
    conn: &Connection,
    id: &ThreadId,
    options: &BTreeMap<String, String>,
) -> KernelResult<ThreadRow> {
    conn.execute(
        "UPDATE threads SET options_json = ?2 WHERE id = ?1",
        params![id.as_str(), options_json(options)?],
    )?;
    thread_require(conn, id)
}

/// Updates the parent-thread event sequence cursor already shared with a side thread.
pub fn thread_set_parent_seen_seq(conn: &Connection, id: &ThreadId, seq: u64) -> KernelResult<()> {
    conn.execute(
        "UPDATE threads SET parent_seen_seq = ?2 WHERE id = ?1",
        params![id.as_str(), to_i64(seq)?],
    )?;
    Ok(())
}

/// Inserts a forked thread with copied events (re-sequenced 1..=N) and optional pending
/// handoff so its first turn receives bounded conversation history. Copied turns are
/// already finished, so they live in the forked event log without colliding with the
/// source thread's `turns` primary keys.
pub fn thread_fork_copy(
    conn: &Connection,
    thread: &Thread,
    copied: &[StoredEvent],
    pending_handoff: Option<&PendingHandoff>,
) -> KernelResult<()> {
    thread_insert(conn, thread)?;
    let mut next_seq: u64 = 0;
    for ev in copied {
        next_seq += 1;
        let mut value = serde_json::to_value(&ev.event)?;
        let data = value
            .get_mut("data")
            .map(Value::take)
            .unwrap_or(Value::Null);
        conn.execute(
            "INSERT INTO events (thread, seq, time, kind, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                thread.id.as_str(),
                to_i64(next_seq)?,
                ev.time.0,
                ev.event.kind(),
                data.to_string(),
            ],
        )?;
    }
    conn.execute(
        "UPDATE threads SET head = ?2 WHERE id = ?1",
        params![thread.id.as_str(), to_i64(next_seq)?],
    )?;
    if let Some(h) = pending_handoff {
        thread_set_pending_handoff(conn, &thread.id, Some(h))?;
    }
    Ok(())
}

/// Updates a thread's provider, model, permission mode, and/or title, bumping `updated`.
/// When `clear_session` is true, `provider_session` is cleared. When `pending_handoff` is
/// `Some(h)`, `pending_handoff` is replaced with `h`.
#[allow(clippy::too_many_arguments)]
pub fn thread_update(
    conn: &Connection,
    id: &ThreadId,
    provider: &str,
    model: &str,
    permission_mode: Option<PermissionMode>,
    title: Option<&str>,
    title_source: Option<TitleSource>,
    updated: &Timestamp,
    clear_session: bool,
    pending_handoff: Option<Option<&PendingHandoff>>,
) -> KernelResult<ThreadRow> {
    conn.execute(
        "UPDATE threads
         SET provider = ?2, model = ?3, permission_mode = ?4, title = ?5,
             title_source = ?6, updated = ?7
         WHERE id = ?1",
        params![
            id.as_str(),
            provider,
            model,
            permission_mode.map(PermissionMode::as_str),
            title,
            title_source.map(TitleSource::as_str),
            updated.0,
        ],
    )?;
    if clear_session {
        thread_clear_session(conn, id)?;
    }
    if let Some(handoff) = pending_handoff {
        thread_set_pending_handoff(conn, id, handoff)?;
    }
    thread_require(conn, id)
}

/// Sets a thread's title as `agent`-suggested unless `title_source == 'user'`. Returns
/// `(updated, row)`.
pub fn thread_suggest_title(
    conn: &Connection,
    id: &ThreadId,
    title: &str,
    updated: &Timestamp,
) -> KernelResult<(bool, ThreadRow)> {
    let row = thread_require(conn, id)?;
    if row.thread.title_source == Some(TitleSource::User) {
        return Ok((false, row));
    }
    conn.execute(
        "UPDATE threads SET title = ?2, title_source = 'agent', updated = ?3 WHERE id = ?1",
        params![id.as_str(), title, updated.0],
    )?;
    Ok((true, thread_require(conn, id)?))
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

/// Threads of a workspace, most recently updated first. Side threads are excluded unless
/// `include_side` is true.
pub fn thread_list(
    conn: &Connection,
    workspace: &WorkspaceId,
    include_archived: bool,
    include_side: bool,
) -> KernelResult<Vec<ThreadRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {THREAD_COLUMNS} FROM threads
         WHERE workspace = ?1 AND (?2 OR archived = 0) AND (?3 OR side = 0)
         ORDER BY updated DESC, id DESC"
    ))?;
    let rows = stmt.query_map(
        params![workspace.as_str(), include_archived, include_side],
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

/// Restores an archived thread back to active status.
pub fn thread_set_unarchived(
    conn: &Connection,
    id: &ThreadId,
    updated: &Timestamp,
) -> KernelResult<ThreadRow> {
    conn.execute(
        "UPDATE threads SET archived = 0, updated = ?2 WHERE id = ?1",
        params![id.as_str(), updated.0],
    )?;
    thread_require(conn, id)
}

/// Permanently deletes a thread and all its approvals, turns, and events.
pub fn thread_delete(conn: &Connection, id: &ThreadId) -> KernelResult<Thread> {
    let row = thread_require(conn, id)?;
    if matches!(
        row.thread.status,
        ThreadStatus::Running | ThreadStatus::AwaitingApproval
    ) {
        return Err(KernelError::Conflict(format!(
            "thread {id} has a running turn"
        )));
    }
    let tid = id.as_str();
    conn.execute("DELETE FROM queued_messages WHERE thread = ?1", [tid])?;
    conn.execute("DELETE FROM approvals WHERE thread = ?1", [tid])?;
    conn.execute("DELETE FROM turns WHERE thread = ?1", [tid])?;
    conn.execute("DELETE FROM events WHERE thread = ?1", [tid])?;
    conn.execute("DELETE FROM threads WHERE id = ?1", [tid])?;
    Ok(row.public())
}

/// Returns every thread id linked to a worktree path, grouped by `worktree_path`.
pub fn threads_by_worktree_path(
    conn: &Connection,
) -> KernelResult<std::collections::HashMap<String, Vec<ThreadId>>> {
    let mut stmt =
        conn.prepare("SELECT worktree_path, id FROM threads WHERE worktree_path IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, ThreadId(r.get(1)?))))?;
    let mut map: std::collections::HashMap<String, Vec<ThreadId>> =
        std::collections::HashMap::new();
    for row in rows {
        let (path, id) = row?;
        map.entry(path).or_default().push(id);
    }
    Ok(map)
}

/// Counts threads currently linked to `worktree_path`.
pub fn threads_using_worktree(conn: &Connection, path: &str) -> KernelResult<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM threads WHERE worktree_path = ?1",
        [path],
        |r| r.get(0),
    )?)
}

/// Detaches threads from a worktree path that has been removed from disk.
pub fn threads_clear_worktree_path(conn: &Connection, path: &str) -> KernelResult<()> {
    conn.execute(
        "UPDATE threads SET worktree_path = NULL WHERE worktree_path = ?1",
        [path],
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

/// Clears the stored provider session id for a thread.
pub fn thread_clear_session(conn: &Connection, id: &ThreadId) -> KernelResult<()> {
    conn.execute(
        "UPDATE threads SET provider_session = NULL WHERE id = ?1",
        [id.as_str()],
    )?;
    Ok(())
}

/// Sets or clears a thread's pending handoff metadata.
pub fn thread_set_pending_handoff(
    conn: &Connection,
    id: &ThreadId,
    pending: Option<&PendingHandoff>,
) -> KernelResult<()> {
    let encoded = pending.map(serde_json::to_string).transpose()?;
    conn.execute(
        "UPDATE threads SET pending_handoff = ?2 WHERE id = ?1",
        params![id.as_str(), encoded],
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
            // A turn started from the queue takes its entry out in the same transaction.
            if let Some(entry) = &e.entry {
                let n = conn.execute(
                    "DELETE FROM queued_messages WHERE id = ?1 AND thread = ?2",
                    params![entry.as_str(), thread.as_str()],
                )?;
                if n == 0 {
                    return Err(KernelError::Conflict(format!(
                        "queued message {entry} is not in the queue"
                    )));
                }
            }
            conn.execute(
                "INSERT INTO turns (id, thread, started_seq, started, provider, model, purpose)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    e.turn.as_str(),
                    thread.as_str(),
                    seq_i,
                    time.0,
                    e.provider,
                    e.model,
                    e.purpose.map(TurnPurpose::as_str),
                ],
            )?;
        }
        ThreadEvent::QueueAdded(e) => {
            let position: i64 = if e.front {
                conn.query_row(
                    "SELECT coalesce(min(position), 1) - 1 FROM queued_messages WHERE thread = ?1",
                    [thread.as_str()],
                    |r| r.get(0),
                )?
            } else {
                conn.query_row(
                    "SELECT coalesce(max(position), -1) + 1 FROM queued_messages WHERE thread = ?1",
                    [thread.as_str()],
                    |r| r.get(0),
                )?
            };
            let entry = &e.entry;
            let trigger_json = entry
                .trigger
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let next_at: Option<String> = match &entry.trigger {
                Some(QueueTrigger::At { time }) => Some(time.0.clone()),
                _ => None,
            };
            let wait_thread: Option<&str> = match &entry.trigger {
                Some(QueueTrigger::AfterTurn { thread: target, .. }) => Some(target.as_str()),
                _ => None,
            };
            let sender = entry.from.as_ref().and_then(|f| f.plugin.as_deref());
            let deferred_entry = entry
                .from
                .as_ref()
                .and_then(|f| f.deferred.as_ref())
                .map(|d| d.as_str());
            conn.execute(
                "INSERT INTO queued_messages
                     (id, thread, turn, position, input, sender, created, updated,
                      trigger_json, next_at, wait_thread, deferred_entry)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9, ?10, ?11)",
                params![
                    entry.id.as_str(),
                    thread.as_str(),
                    entry.turn.as_str(),
                    position,
                    serde_json::to_string(&entry.input)?,
                    sender,
                    time.0,
                    trigger_json,
                    next_at,
                    wait_thread,
                    deferred_entry,
                ],
            )?;
        }
        ThreadEvent::QueueUpdated(e) => {
            let entry = &e.entry;
            let trigger_json = entry
                .trigger
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let next_at: Option<String> = match &entry.trigger {
                Some(QueueTrigger::At { time }) => Some(time.0.clone()),
                _ => None,
            };
            let wait_thread: Option<&str> = match &entry.trigger {
                Some(QueueTrigger::AfterTurn { thread: target, .. }) => Some(target.as_str()),
                _ => None,
            };
            let sender = entry.from.as_ref().and_then(|f| f.plugin.as_deref());
            let deferred_entry = entry
                .from
                .as_ref()
                .and_then(|f| f.deferred.as_ref())
                .map(|d| d.as_str());
            let n = conn.execute(
                "UPDATE queued_messages
                 SET input = ?3, updated = ?4, trigger_json = ?5,
                     next_at = CASE WHEN ?5 IS NULL THEN NULL ELSE coalesce(?6, next_at) END,
                     wait_thread = ?7, sender = ?8, deferred_entry = ?9
                 WHERE id = ?1 AND thread = ?2",
                params![
                    entry.id.as_str(),
                    thread.as_str(),
                    serde_json::to_string(&entry.input)?,
                    time.0,
                    trigger_json,
                    next_at,
                    wait_thread,
                    sender,
                    deferred_entry,
                ],
            )?;
            if n == 0 {
                return Err(KernelError::NotFound(format!(
                    "no queued message {}",
                    e.entry.id
                )));
            }
        }
        ThreadEvent::QueueRemoved(e) => {
            let n = conn.execute(
                "DELETE FROM queued_messages WHERE id = ?1 AND thread = ?2",
                params![e.entry.as_str(), thread.as_str()],
            )?;
            if n == 0 {
                return Err(KernelError::NotFound(format!(
                    "no queued message {}",
                    e.entry
                )));
            }
        }
        ThreadEvent::QueueMoved(e) => {
            let mut current: Vec<String> = queue_list(conn, thread)?
                .into_iter()
                .map(|q| q.id.0)
                .collect();
            let mut next: Vec<String> = e.order.iter().map(|q| q.0.clone()).collect();
            current.sort_unstable();
            next.sort_unstable();
            if current != next {
                return Err(KernelError::InvalidParams(
                    "order must list every queued message once".into(),
                ));
            }
            for (i, id) in e.order.iter().enumerate() {
                conn.execute(
                    "UPDATE queued_messages SET position = ?3 WHERE id = ?1 AND thread = ?2",
                    params![id.as_str(), thread.as_str(), to_i64(i as u64)?],
                )?;
            }
        }
        ThreadEvent::QueueHeld(e) => {
            conn.execute(
                "UPDATE threads SET queue_hold = ?2 WHERE id = ?1",
                params![thread.as_str(), e.reason.as_str()],
            )?;
        }
        ThreadEvent::QueueResumed(_) => {
            conn.execute(
                "UPDATE threads SET queue_hold = NULL WHERE id = ?1",
                [thread.as_str()],
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

/// A deferred queue row with its scheduling metadata.
#[derive(Debug, Clone)]
pub struct DeferredRow {
    /// The thread it belongs to.
    pub thread: ThreadId,
    /// The queue entry.
    pub entry: QueueEntry,
    /// Next timer run, RFC 3339 UTC, if armed.
    pub next_at: Option<String>,
    /// Attempts completed so far (for `poll_command`).
    pub attempts: u32,
}

fn parse_queue_entry(
    id: String,
    turn: String,
    input: String,
    sender: Option<String>,
    deferred_entry: Option<String>,
    trigger_json: Option<String>,
) -> KernelResult<QueueEntry> {
    let input: UserInput = serde_json::from_str(&input)?;
    let trigger: Option<QueueTrigger> = trigger_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?;
    let from = if sender.is_some() || deferred_entry.is_some() {
        Some(MessageFrom {
            plugin: sender,
            deferred: deferred_entry.map(QueueEntryId),
        })
    } else {
        None
    };
    Ok(QueueEntry {
        id: QueueEntryId(id),
        turn: TurnId(turn),
        input,
        from,
        trigger,
    })
}

/// A thread's queued messages (both ready and waiting on a trigger), in order.
pub fn queue_list(conn: &Connection, thread: &ThreadId) -> KernelResult<Vec<QueueEntry>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, turn, input, sender, deferred_entry, trigger_json FROM queued_messages
         WHERE thread = ?1 ORDER BY position, created, id",
    )?;
    let rows = stmt.query_map([thread.as_str()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, turn, input, sender, deferred_entry, trigger_json) = row?;
        out.push(parse_queue_entry(
            id,
            turn,
            input,
            sender,
            deferred_entry,
            trigger_json,
        )?);
    }
    Ok(out)
}

/// A thread's ready-to-run queued messages (those without a pending trigger), in order.
pub fn queue_ready_list(conn: &Connection, thread: &ThreadId) -> KernelResult<Vec<QueueEntry>> {
    Ok(queue_list(conn, thread)?
        .into_iter()
        .filter(|q| q.trigger.is_none())
        .collect())
}

/// Why a thread's queue is held, if it is.
pub fn queue_hold(conn: &Connection, thread: &ThreadId) -> KernelResult<Option<QueueHold>> {
    let hold: Option<Option<String>> = conn
        .query_row(
            "SELECT queue_hold FROM threads WHERE id = ?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    match hold {
        None => Err(KernelError::NotFound(format!("no thread {thread}"))),
        Some(None) => Ok(None),
        Some(Some(h)) => QueueHold::parse(&h)
            .map(Some)
            .ok_or_else(|| StoreError::Corrupt(format!("unknown queue hold {h:?}")).into()),
    }
}

/// Number of ready (non-deferred) queued messages on unheld threads across all workspaces.
pub fn count_ready_unheld_queue(conn: &Connection) -> KernelResult<u32> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM queued_messages q
         JOIN threads t ON t.id = q.thread
         WHERE t.queue_hold IS NULL AND q.trigger_json IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

/// Threads whose queue has ready messages or command triggers and is not held.
pub fn threads_with_open_queue(conn: &Connection) -> KernelResult<Vec<ThreadId>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT q.thread FROM queued_messages q JOIN threads t ON t.id = q.thread
         WHERE t.queue_hold IS NULL
           AND (q.trigger_json IS NULL
                OR q.trigger_json LIKE '%\"after_command\"%'
                OR q.trigger_json LIKE '%\"poll_command\"%')
         ORDER BY q.thread",
    )?;
    let rows = stmt.query_map([], |r| Ok(ThreadId(r.get(0)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Clears `next_at` on `poll_command` entries at startup so interrupted commands wait for
/// resume (ADR 0027).
pub fn pause_command_triggers(conn: &Connection) -> KernelResult<()> {
    conn.execute(
        "UPDATE queued_messages SET next_at = NULL
         WHERE trigger_json LIKE '%\"after_command\"%'
            OR trigger_json LIKE '%\"poll_command\"%'",
        [],
    )?;
    Ok(())
}

/// Earliest `next_at` among active (unheld) deferred entries.
pub fn defer_earliest_at(conn: &Connection) -> KernelResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT min(q.next_at) FROM queued_messages q
             JOIN threads t ON t.id = q.thread
             WHERE q.next_at IS NOT NULL AND t.queue_hold IS NULL",
            [],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

fn read_deferred_rows(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> KernelResult<Vec<DeferredRow>> {
    let rows = stmt.query_map(params, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, i64>(8)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (thread, id, turn, input, sender, deferred_entry, trigger_json, next_at, attempts) =
            row?;
        out.push(DeferredRow {
            thread: ThreadId(thread),
            entry: parse_queue_entry(id, turn, input, sender, deferred_entry, trigger_json)?,
            next_at,
            attempts: u32::try_from(attempts).unwrap_or(0),
        });
    }
    Ok(out)
}

/// Deferred entries whose `next_at <= now` on unheld threads.
pub fn defer_due(conn: &Connection, now: &str) -> KernelResult<Vec<DeferredRow>> {
    let mut stmt = conn.prepare(
        "SELECT q.thread, q.id, q.turn, q.input, q.sender, q.deferred_entry,
                q.trigger_json, q.next_at, q.attempts
         FROM queued_messages q
         JOIN threads t ON t.id = q.thread
         WHERE q.next_at IS NOT NULL AND q.next_at <= ?1 AND t.queue_hold IS NULL
         ORDER BY q.next_at, q.id",
    )?;
    read_deferred_rows(&mut stmt, [now])
}

/// Deferred entries waiting for `target`'s turn to end.
pub fn defer_waiting_on_turn(
    conn: &Connection,
    target: &ThreadId,
) -> KernelResult<Vec<DeferredRow>> {
    let mut stmt = conn.prepare(
        "SELECT q.thread, q.id, q.turn, q.input, q.sender, q.deferred_entry,
                q.trigger_json, q.next_at, q.attempts
         FROM queued_messages q
         WHERE q.wait_thread = ?1
         ORDER BY q.created, q.id",
    )?;
    read_deferred_rows(&mut stmt, [target.as_str()])
}

/// All `(owner_thread, wait_thread)` edges for `after_turn` loop detection.
pub fn defer_after_turn_edges(conn: &Connection) -> KernelResult<Vec<(ThreadId, ThreadId)>> {
    let mut stmt = conn
        .prepare("SELECT thread, wait_thread FROM queued_messages WHERE wait_thread IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((ThreadId(r.get(0)?), ThreadId(r.get(1)?))))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Number of pending plugin-set deferred entries on `thread`.
pub fn defer_count_by_plugin(conn: &Connection, thread: &ThreadId) -> KernelResult<usize> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM queued_messages
         WHERE thread = ?1 AND trigger_json IS NOT NULL AND sender IS NOT NULL",
        [thread.as_str()],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// Updates `next_at` and `attempts` for a deferred entry.
pub fn defer_set_next_at(
    conn: &Connection,
    entry: &QueueEntryId,
    next_at: Option<&str>,
    attempts: u32,
) -> KernelResult<()> {
    conn.execute(
        "UPDATE queued_messages SET next_at = ?2, attempts = ?3 WHERE id = ?1",
        params![entry.as_str(), next_at, i64::from(attempts)],
    )?;
    Ok(())
}

/// Command-triggered deferred entries for `thread` (re-armed on `queue_resume`).
pub fn defer_command_entries_for_thread(
    conn: &Connection,
    thread: &ThreadId,
) -> KernelResult<Vec<DeferredRow>> {
    let mut stmt = conn.prepare(
        "SELECT q.thread, q.id, q.turn, q.input, q.sender, q.deferred_entry,
                q.trigger_json, q.next_at, q.attempts
         FROM queued_messages q
         WHERE q.thread = ?1
           AND (q.trigger_json LIKE '%\"after_command\"%'
                OR q.trigger_json LIKE '%\"poll_command\"%')
         ORDER BY q.position, q.created, q.id",
    )?;
    read_deferred_rows(&mut stmt, [thread.as_str()])
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

/// Reads every event of `thread`, in sequence order.
pub fn read_all_events(conn: &Connection, thread: &ThreadId) -> KernelResult<Vec<StoredEvent>> {
    let mut out = Vec::new();
    let mut after = 0;
    loop {
        let batch = read_events(conn, thread, after, 4096)?;
        let Some(last) = batch.last() else { break };
        after = last.seq;
        out.extend(batch);
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

/// Reads a core-level configuration value by key.
pub fn core_setting_get(conn: &Connection, key: &str) -> KernelResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM core_settings WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()?)
}

/// Writes or replaces a core-level configuration value.
pub fn core_setting_set(conn: &Connection, key: &str, value: &str) -> KernelResult<()> {
    conn.execute(
        "INSERT INTO core_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Removes a core-level configuration value so the platform default applies.
pub fn core_setting_delete(conn: &Connection, key: &str) -> KernelResult<()> {
    conn.execute("DELETE FROM core_settings WHERE key = ?1", [key])?;
    Ok(())
}
