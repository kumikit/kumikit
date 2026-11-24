//! Deferred messages / wake-ups (`docs/spec/03-kernel.md#the-queue`, ADR 0027).
//!
//! A deferred message is a [`QueueEntry`] carrying a [`QueueTrigger`]. While its trigger
//! is pending it stays visible in the thread's queue but is skipped by normal queue
//! dispatch. When the trigger fires, the kernel appends a one-line wake-up trailer (plus
//! exit code and the last 8 KiB of output for command triggers), clears the trigger, and
//! starts the message if the thread is idle (or leaves it in the ready queue if busy — a
//! deferred message never steers a running turn).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp as JiffTs};
use kumikit_protocol::ThreadEvent;
use kumikit_protocol::event::{
    MessageFrom, QueueAdded, QueueEntry, QueueRemoval, QueueRemoved, QueueTrigger, QueueUpdated,
};
use kumikit_protocol::model::{
    PermissionMode, QueueEntryId, Resolution, ThreadId, TurnId, TurnOutcome, UserInput,
};
use serde_json::json;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::error::{KernelError, KernelResult};
use crate::grants::Grants;
use crate::kernel::{Kernel, lock};
use crate::process::{OwnerId, ProcessError, SpawnContext, SpawnRequest};
use crate::repo;
use crate::turns::{Queued, validate_attachments};

/// Most pending plugin-set deferred entries allowed on one thread (ADR 0027).
pub const MAX_PLUGIN_DEFERRED_PER_THREAD: usize = 10;

/// Maximum tail bytes of command output appended when a command trigger fires (8 KiB).
pub const MAX_COMMAND_OUTPUT_BYTES: usize = 8 * 1024;

/// Maximum `timeoutS` accepted for `after_command` (24 hours).
pub const MAX_COMMAND_TIMEOUT_S: u32 = 86_400;

/// Minimum `everyS` accepted for `poll_command` (5 seconds).
pub const MIN_POLL_EVERY_S: u32 = 5;

/// Maximum `maxTries` accepted for `poll_command`.
pub const MAX_POLL_TRIES: u32 = 1000;

/// Runtime state of the deferred-message engine inside [`Kernel`].
#[derive(Default)]
pub(crate) struct State {
    changed: Arc<Notify>,
    timer: Mutex<Option<JoinHandle<()>>>,
    /// Running `after_command` tasks keyed by queue entry id, aborted on remove/sendNow/close.
    commands: Mutex<HashMap<QueueEntryId, JoinHandle<()>>>,
    /// Cached grants of plugin-initiated command triggers (for `poll_command` retries / resume).
    plugin_grants: Mutex<HashMap<QueueEntryId, (String, Grants)>>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("defer::State")
            .field("commands", &lock(&self.commands).len())
            .finish_non_exhaustive()
    }
}

fn parse_jiff(s: &str) -> KernelResult<JiffTs> {
    s.parse::<JiffTs>()
        .map_err(|e| KernelError::InvalidParams(format!("invalid RFC 3339 timestamp `{s}`: {e}")))
}

fn format_cmd(argv: &[String]) -> String {
    argv.join(" ")
}

fn validate_trigger(trigger: &QueueTrigger) -> KernelResult<()> {
    match trigger {
        QueueTrigger::At { time } => {
            parse_jiff(&time.0)?;
            Ok(())
        }
        QueueTrigger::AfterTurn { .. } => Ok(()),
        QueueTrigger::AfterCommand {
            argv, timeout_s, ..
        } => {
            if argv.is_empty() || argv[0].trim().is_empty() {
                return Err(KernelError::InvalidParams(
                    "after_command argv must name a program".into(),
                ));
            }
            if *timeout_s == 0 || *timeout_s > MAX_COMMAND_TIMEOUT_S {
                return Err(KernelError::InvalidParams(format!(
                    "timeoutS must be in 1..={MAX_COMMAND_TIMEOUT_S}"
                )));
            }
            Ok(())
        }
        QueueTrigger::PollCommand {
            argv,
            every_s,
            max_tries,
            ..
        } => {
            if argv.is_empty() || argv[0].trim().is_empty() {
                return Err(KernelError::InvalidParams(
                    "poll_command argv must name a program".into(),
                ));
            }
            if *every_s < MIN_POLL_EVERY_S {
                return Err(KernelError::InvalidParams(format!(
                    "everyS must be at least {MIN_POLL_EVERY_S}"
                )));
            }
            if *max_tries == 0 || *max_tries > MAX_POLL_TRIES {
                return Err(KernelError::InvalidParams(format!(
                    "maxTries must be in 1..={MAX_POLL_TRIES}"
                )));
            }
            Ok(())
        }
    }
}

/// Returns true if adding `owner -> target` creates a cycle in the `after_turn` graph.
fn would_loop(edges: &[(ThreadId, ThreadId)], owner: &ThreadId, target: &ThreadId) -> bool {
    if owner == target {
        return true;
    }
    let mut adj: HashMap<&ThreadId, Vec<&ThreadId>> = HashMap::new();
    for (u, v) in edges {
        adj.entry(u).or_default().push(v);
    }
    let mut stack = vec![target];
    let mut seen = HashSet::new();
    while let Some(cur) = stack.pop() {
        if cur == owner {
            return true;
        }
        if seen.insert(cur)
            && let Some(nexts) = adj.get(cur)
        {
            stack.extend(nexts.iter().copied());
        }
    }
    false
}

/// Captures the last [`MAX_COMMAND_OUTPUT_BYTES`] bytes written to it.
#[derive(Default)]
struct TailBuf {
    bytes: VecDeque<u8>,
    truncated: bool,
}

impl TailBuf {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend(chunk);
        if self.bytes.len() > MAX_COMMAND_OUTPUT_BYTES {
            let excess = self.bytes.len() - MAX_COMMAND_OUTPUT_BYTES;
            self.bytes.drain(..excess);
            self.truncated = true;
        }
    }

    fn into_string(self) -> String {
        let slice: Vec<u8> = self.bytes.into_iter().collect();
        let text = String::from_utf8_lossy(&slice).into_owned();
        if self.truncated {
            format!("...(truncated to last 8 KiB)\n{text}")
        } else {
            text
        }
    }
}

fn process_err(e: ProcessError) -> KernelError {
    match e {
        ProcessError::PermissionDenied(m) => KernelError::PermissionDenied(m),
        ProcessError::NotFound(m) => KernelError::NotFound(m),
        ProcessError::InvalidArgument(m) => KernelError::InvalidParams(m),
        ProcessError::Io(m) => KernelError::Io(m),
    }
}

impl Kernel {
    fn defer_state(&self) -> &State {
        &self.inner.defer
    }

    /// Queues a deferred message set by a user/client (ADR 0027).
    pub async fn thread_defer(
        &self,
        thread: &ThreadId,
        input: UserInput,
        trigger: QueueTrigger,
    ) -> KernelResult<QueueEntry> {
        self.thread_defer_internal(thread, input, trigger, None)
            .await
    }

    /// Queues a deferred message set by a plugin (through `threads.defer`). Enforces
    /// the per-thread agent limit, `permissions.spawn` for command triggers, and an
    /// approval of kind `command` unless the thread is in `full_access`.
    pub async fn thread_defer_from(
        &self,
        thread: &ThreadId,
        input: UserInput,
        trigger: QueueTrigger,
        plugin: String,
        grants: Grants,
    ) -> KernelResult<QueueEntry> {
        self.thread_defer_internal(thread, input, trigger, Some((plugin, grants)))
            .await
    }

    async fn thread_defer_internal(
        &self,
        thread: &ThreadId,
        input: UserInput,
        trigger: QueueTrigger,
        actor: Option<(String, Grants)>,
    ) -> KernelResult<QueueEntry> {
        validate_attachments(&input.attachments)?;
        validate_trigger(&trigger)?;
        let row = self.thread_row(thread).await?;
        if row.archived {
            return Err(KernelError::Conflict(format!(
                "thread {thread} is archived"
            )));
        }

        if let QueueTrigger::AfterTurn { thread: target, .. } = &trigger {
            self.thread_row(target).await?;
            let edges = self.inner.store.read(repo::defer_after_turn_edges).await?;
            if would_loop(&edges, thread, target) {
                return Err(KernelError::InvalidParams(
                    "deferred after_turn trigger would loop".into(),
                ));
            }
        }

        if let Some((plugin, grants)) = &actor {
            let tid = thread.clone();
            let count = self
                .inner
                .store
                .read(move |c| repo::defer_count_by_plugin(c, &tid))
                .await?;
            if count >= MAX_PLUGIN_DEFERRED_PER_THREAD {
                return Err(KernelError::InvalidParams(format!(
                    "at most {MAX_PLUGIN_DEFERRED_PER_THREAD} pending plugin-deferred messages per thread"
                )));
            }
            if let QueueTrigger::AfterCommand { argv, cwd, .. }
            | QueueTrigger::PollCommand { argv, cwd, .. } = &trigger
            {
                let program = &argv[0];
                if !grants.may_spawn(program) {
                    return Err(KernelError::PermissionDenied(format!(
                        "plugin `{plugin}` is not granted spawn permission for `{program}`"
                    )));
                }
                let mode = row.thread.permission_mode.unwrap_or_default();
                if mode != PermissionMode::FullAccess {
                    let turn = self.running_turn(thread).ok_or_else(|| {
                        KernelError::PermissionDenied(
                            "a plugin command trigger requires an active turn for user approval unless permissionMode is full_access".into(),
                        )
                    })?;
                    let summary = format!("Run deferred command `{}`", format_cmd(argv));
                    let detail = json!({
                        "plugin": plugin,
                        "argv": argv,
                        "cwd": cwd,
                    });
                    let decision = self
                        .request_kernel_approval(thread, &turn, "command", summary, detail)
                        .await?;
                    if !matches!(decision, Resolution::Allow | Resolution::AllowAlways) {
                        return Err(KernelError::PermissionDenied(
                            "the user did not allow this deferred command".into(),
                        ));
                    }
                }
            }
        }

        let entry = QueueEntry {
            id: QueueEntryId(self.new_id()),
            turn: TurnId(self.new_id()),
            input,
            from: actor.as_ref().map(|(p, _)| MessageFrom::plugin(p.clone())),
            trigger: Some(trigger.clone()),
        };

        {
            let _dispatch = self.inner.dispatch.lock().await;
            self.append(
                thread,
                ThreadEvent::QueueAdded(QueueAdded {
                    entry: entry.clone(),
                    front: false,
                    reason: None,
                }),
            )
            .await?;
            if matches!(trigger, QueueTrigger::PollCommand { .. }) {
                let id = entry.id.clone();
                let now = self.now().0;
                self.inner
                    .store
                    .write(move |c| repo::defer_set_next_at(c, &id, Some(&now), 0))
                    .await?;
            }
        }

        if let Some(pair) = actor {
            lock(&self.defer_state().plugin_grants).insert(entry.id.clone(), pair);
        }

        match &trigger {
            QueueTrigger::At { .. } | QueueTrigger::PollCommand { .. } => {
                self.defer_state().changed.notify_one();
            }
            QueueTrigger::AfterCommand {
                argv,
                cwd,
                timeout_s,
            } => {
                self.spawn_after_command(
                    thread.clone(),
                    entry.id.clone(),
                    argv.clone(),
                    cwd.clone(),
                    *timeout_s,
                );
            }
            QueueTrigger::AfterTurn { .. } => {}
        }

        Ok(entry)
    }

    /// Earliest `next_at` timestamp waiting on the deferred timer, if any.
    pub async fn defer_wake(&self) -> KernelResult<Option<JiffTs>> {
        let next = self.inner.store.read(repo::defer_earliest_at).await?;
        Ok(next.as_deref().and_then(|s| s.parse::<JiffTs>().ok()))
    }

    /// Evaluates all `at` and `poll_command` entries due at or before `now`. Deterministic:
    /// tests call this with explicit timestamps instead of sleeping.
    pub async fn defer_tick(&self, now: JiffTs) -> KernelResult<usize> {
        let now_str = now.to_string();
        let due = self
            .inner
            .store
            .read(move |c| repo::defer_due(c, &now_str))
            .await?;
        let mut fired = 0usize;
        for row in due {
            match row.entry.trigger.clone() {
                Some(QueueTrigger::At { time }) => {
                    let who = source_label(&row.entry.from);
                    let trailer = format!("Deferred message: scheduled for {} · {who}", time.0);
                    if self
                        .fire_deferred(&row.thread, &row.entry.id, trailer, None)
                        .await?
                    {
                        fired += 1;
                    }
                }
                Some(QueueTrigger::PollCommand {
                    argv,
                    cwd,
                    every_s,
                    max_tries,
                }) => {
                    let attempt = row.attempts.saturating_add(1);
                    let res = self
                        .run_deferred_command(&row.thread, &row.entry.id, &argv, cwd.as_deref(), 60)
                        .await;
                    let cmd = format_cmd(&argv);
                    let who = source_label(&row.entry.from);
                    match res {
                        Ok((Some(0), false, out)) => {
                            let trailer = format!(
                                "Deferred message: after \"{cmd}\" succeeded (attempt {attempt}/{max_tries}) · {who}"
                            );
                            if self
                                .fire_deferred(&row.thread, &row.entry.id, trailer, Some(out))
                                .await?
                            {
                                fired += 1;
                            }
                        }
                        Ok((code, timed_out, out)) if attempt >= max_tries => {
                            let status = if timed_out {
                                "timed out".to_owned()
                            } else {
                                format!(
                                    "exited {}",
                                    code.map_or_else(|| "by signal".into(), |c| c.to_string())
                                )
                            };
                            let trailer = format!(
                                "Deferred message: after \"{cmd}\" failed after {max_tries} attempts (last {status}) · {who}"
                            );
                            if self
                                .fire_deferred(&row.thread, &row.entry.id, trailer, Some(out))
                                .await?
                            {
                                fired += 1;
                            }
                        }
                        Err(e) if attempt >= max_tries => {
                            let trailer = format!(
                                "Deferred message: after \"{cmd}\" failed after {max_tries} attempts ({e}) · {who}"
                            );
                            if self
                                .fire_deferred(&row.thread, &row.entry.id, trailer, None)
                                .await?
                            {
                                fired += 1;
                            }
                        }
                        _ => {
                            let next = now
                                .checked_add(SignedDuration::from_secs(i64::from(every_s)))
                                .unwrap_or(now);
                            let id = row.entry.id.clone();
                            let next_s = next.to_string();
                            self.inner
                                .store
                                .write(move |c| {
                                    repo::defer_set_next_at(c, &id, Some(&next_s), attempt)
                                })
                                .await?;
                            self.defer_state().changed.notify_one();
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(fired)
    }

    /// Starts the single background timer task for `at` and `poll_command` entries.
    /// Called once by `kumikit serve`; unit tests drive [`Kernel::defer_tick`] directly.
    pub fn start_defer_timer(&self) {
        let mut slot = lock(&self.defer_state().timer);
        if slot.is_some() {
            return;
        }
        let kernel = self.clone();
        let changed = Arc::clone(&self.defer_state().changed);
        *slot = Some(tokio::spawn(async move {
            // Fire overdue `at` entries once at startup.
            let _ = kernel.defer_tick(JiffTs::now()).await;
            loop {
                let wake = match kernel.defer_wake().await {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::debug!(error = %e, "defer timer stops");
                        return;
                    }
                };
                match wake {
                    None => changed.notified().await,
                    Some(at) => {
                        let wait = JiffTs::now().duration_until(at);
                        let wait = Duration::try_from(wait).unwrap_or_default();
                        tokio::select! {
                            () = tokio::time::sleep(wait) => {}
                            () = changed.notified() => continue,
                        }
                    }
                }
                if let Err(e) = kernel.defer_tick(JiffTs::now()).await {
                    tracing::debug!(error = %e, "defer timer stops");
                    return;
                }
            }
        }));
    }

    /// Stops the deferred timer and all running command-trigger tasks (kernel close).
    pub(crate) fn stop_defer_timer(&self) {
        if let Some(t) = lock(&self.defer_state().timer).take() {
            t.abort();
        }
        for (_, h) in lock(&self.defer_state().commands).drain() {
            h.abort();
        }
    }

    /// Cancels any running command task associated with `entry`.
    pub(crate) fn cancel_defer_task(&self, entry: &QueueEntryId) {
        if let Some(h) = lock(&self.defer_state().commands).remove(entry) {
            h.abort();
        }
        lock(&self.defer_state().plugin_grants).remove(entry);
    }

    /// Re-arms command triggers on `thread` after `queue.resume` (ADR 0027).
    pub(crate) async fn rearm_command_triggers(&self, thread: &ThreadId) -> KernelResult<()> {
        let tid = thread.clone();
        let rows = self
            .inner
            .store
            .read(move |c| repo::defer_command_entries_for_thread(c, &tid))
            .await?;
        let mut notify = false;
        for row in rows {
            match row.entry.trigger {
                Some(QueueTrigger::AfterCommand {
                    argv,
                    cwd,
                    timeout_s,
                }) => {
                    if !lock(&self.defer_state().commands).contains_key(&row.entry.id) {
                        self.spawn_after_command(row.thread, row.entry.id, argv, cwd, timeout_s);
                    }
                }
                Some(QueueTrigger::PollCommand { .. }) if row.next_at.is_none() => {
                    let id = row.entry.id.clone();
                    let now = self.now().0;
                    let attempts = row.attempts;
                    self.inner
                        .store
                        .write(move |c| repo::defer_set_next_at(c, &id, Some(&now), attempts))
                        .await?;
                    notify = true;
                }
                _ => {}
            }
        }
        if notify {
            self.defer_state().changed.notify_one();
        }
        Ok(())
    }

    /// Hook called right after a turn ends on `finished_thread` to fire waiting
    /// `after_turn` triggers.
    pub(crate) fn on_turn_ended_triggers(
        &self,
        finished_thread: ThreadId,
        outcome: TurnOutcome,
    ) -> crate::provider::BoxFuture<'static, ()> {
        let kernel = self.clone();
        Box::pin(async move {
            let tid = finished_thread.clone();
            let rows = match kernel
                .inner
                .store
                .read(move |c| repo::defer_waiting_on_turn(c, &tid))
                .await
            {
                Ok(r) => r,
                Err(_) => return,
            };
            for row in rows {
                let Some(QueueTrigger::AfterTurn {
                    thread: target,
                    only_on_success,
                }) = row.entry.trigger
                else {
                    continue;
                };
                if only_on_success && outcome != TurnOutcome::Done {
                    let _dispatch = kernel.inner.dispatch.lock().await;
                    let _ = kernel
                        .append(
                            &row.thread,
                            ThreadEvent::QueueRemoved(QueueRemoved {
                                entry: row.entry.id.clone(),
                                reason: QueueRemoval::ConditionNotMet,
                            }),
                        )
                        .await;
                    continue;
                }
                let who = source_label(&row.entry.from);
                let trailer = format!(
                    "Deferred message: after thread {target} finished ({}) · {who}",
                    outcome.as_str()
                );
                let _ = kernel
                    .fire_deferred(&row.thread, &row.entry.id, trailer, None)
                    .await;
            }
        })
    }

    fn spawn_after_command(
        &self,
        thread: ThreadId,
        entry_id: QueueEntryId,
        argv: Vec<String>,
        cwd: Option<String>,
        timeout_s: u32,
    ) {
        let kernel = self.clone();
        let id = entry_id.clone();
        let handle = tokio::spawn(async move {
            let cmd = format_cmd(&argv);
            let res = kernel
                .run_deferred_command(&thread, &id, &argv, cwd.as_deref(), timeout_s)
                .await;
            lock(&kernel.defer_state().commands).remove(&id);
            let entry_from = kernel
                .queue_of(&thread)
                .await
                .ok()
                .and_then(|list| list.into_iter().find(|q| q.id == id))
                .and_then(|q| q.from);
            let who = source_label(&entry_from);
            match res {
                Ok((code, timed_out, out)) => {
                    let status = if timed_out {
                        format!("timed out after {timeout_s}s")
                    } else {
                        format!(
                            "exited {}",
                            code.map_or_else(|| "by signal".into(), |c| c.to_string())
                        )
                    };
                    let trailer = format!("Deferred message: after \"{cmd}\" {status} · {who}");
                    let _ = kernel.fire_deferred(&thread, &id, trailer, Some(out)).await;
                }
                Err(e) => {
                    let trailer =
                        format!("Deferred message: after \"{cmd}\" could not run ({e}) · {who}");
                    let _ = kernel.fire_deferred(&thread, &id, trailer, None).await;
                }
            }
        });
        lock(&self.defer_state().commands).insert(entry_id, handle);
    }

    /// Runs `argv` under the process supervisor in `thread`'s workspace/worktree,
    /// returning `(exit_code, timed_out, last_8kib_output)`.
    async fn run_deferred_command(
        &self,
        thread: &ThreadId,
        entry_id: &QueueEntryId,
        argv: &[String],
        cwd: Option<&str>,
        timeout_s: u32,
    ) -> KernelResult<(Option<i32>, bool, String)> {
        let row = self.thread_row(thread).await?;
        let ws = self.workspace_get(&row.thread.workspace).await?;
        let root = row.thread.worktree_path.unwrap_or_else(|| ws.root.clone());
        let workspaces = vec![PathBuf::from(root)];

        let (plugin_label, grants) = match lock(&self.defer_state().plugin_grants)
            .get(entry_id)
            .cloned()
        {
            Some(pair) => pair,
            None => {
                // User-initiated command: grant the single requested bare program name.
                let mut spawn = BTreeSet::new();
                spawn.insert(argv[0].clone());
                (
                    "user".to_owned(),
                    Grants {
                        spawn,
                        ..Default::default()
                    },
                )
            }
        };

        let owner = OwnerId(format!("defer:{entry_id}"));
        let req = SpawnRequest {
            program: argv[0].clone(),
            args: argv[1..].to_vec(),
            cwd: cwd.map(str::to_owned),
            env: Vec::new(),
            secret_env: Vec::new(),
        };
        let no_secret = |_: &str| None;
        let ctx = SpawnContext {
            plugin: &plugin_label,
            owner: &owner,
            grants: &grants,
            workspaces: &workspaces,
            secret: &no_secret,
        };
        let child = self
            .supervisor()
            .spawn(&ctx, req)
            .await
            .map_err(process_err)?;
        child.close_stdin().await;

        let out_child = child.clone();
        let out_task = tokio::spawn(async move {
            let mut buf = TailBuf::default();
            while let Some(chunk) = out_child.read_stdout(8192).await {
                buf.push(&chunk);
            }
            buf.into_string()
        });
        let err_child = child.clone();
        let err_task = tokio::spawn(async move {
            let mut buf = TailBuf::default();
            while let Some(chunk) = err_child.read_stderr(8192).await {
                buf.push(&chunk);
            }
            buf.into_string()
        });

        let timeout = Duration::from_secs(u64::from(timeout_s));
        let (code, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(status) => (status.code, false),
            Err(_) => {
                child.kill().await;
                let _ = child.wait().await;
                (None, true)
            }
        };
        self.supervisor().kill_owner(&owner).await;

        let stdout = out_task.await.unwrap_or_default();
        let stderr = err_task.await.unwrap_or_default();
        let combined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
            (true, true) => String::new(),
            (false, true) => stdout,
            (true, false) => stderr,
            (false, false) => format!("{stdout}\n--- stderr ---\n{stderr}"),
        };
        let mut final_buf = TailBuf::default();
        final_buf.push(combined.as_bytes());
        Ok((code, timed_out, final_buf.into_string()))
    }

    /// Fires a deferred entry: clears its trigger, appends its trailer (and optional
    /// command output), and starts it if the thread is idle and unheld (or leaves it in
    /// the ready queue if busy — never steers). Returns `true` if the entry was fired.
    async fn fire_deferred(
        &self,
        thread: &ThreadId,
        entry_id: &QueueEntryId,
        trailer: String,
        output: Option<String>,
    ) -> KernelResult<bool> {
        lock(&self.defer_state().plugin_grants).remove(entry_id);
        let _dispatch = self.inner.dispatch.lock().await;
        let Some(mut entry) = self
            .queue_of(thread)
            .await?
            .into_iter()
            .find(|q| &q.id == entry_id)
        else {
            return Ok(false);
        };
        if entry.trigger.is_none() {
            return Ok(false);
        }

        let mut text = entry.input.text.clone();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&trailer);
        if let Some(out) = output.filter(|s| !s.trim().is_empty()) {
            text.push_str("\n```\n");
            text.push_str(out.trim_end());
            text.push_str("\n```");
        }
        entry.input.text = text;
        entry.trigger = None;
        let plugin = entry.from.as_ref().and_then(|f| f.plugin.clone());
        entry.from = Some(MessageFrom {
            plugin,
            deferred: Some(entry.id.clone()),
        });

        self.append(
            thread,
            ThreadEvent::QueueUpdated(QueueUpdated {
                entry: entry.clone(),
            }),
        )
        .await?;

        // Never steer a running turn; if idle and queue is not held, start it now.
        if self.has_run(thread) || self.queue_hold_of(thread).await?.is_some() {
            return Ok(true);
        }
        let row = self.thread_row(thread).await?;
        if row.archived {
            return Ok(true);
        }
        let Ok(provider) = self.provider_of(&row.thread) else {
            return Ok(true);
        };
        self.start_run(thread, row.thread, provider, Queued::from(entry))
            .await?;
        Ok(true)
    }
}

fn source_label(from: &Option<MessageFrom>) -> String {
    match from.as_ref().and_then(|f| f.plugin.as_deref()) {
        Some(plugin) => format!("set by {plugin}"),
        None => "set by user".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KernelConfig;
    use crate::clock::{SequentialIds, StepClock};
    use crate::provider::{
        BoxFuture, EventSink, Model, Provider, ProviderError, ProviderEvent, ProviderResult,
        SessionConfig,
    };
    use kumikit_protocol::ErrorCode;
    use kumikit_protocol::model::{ApprovalId, Decision, Timestamp};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct EchoProvider {
        prompts: Mutex<Vec<String>>,
        fail_next: std::sync::atomic::AtomicBool,
        block_next: std::sync::atomic::AtomicBool,
        entered: Notify,
        runs: AtomicUsize,
    }

    impl Provider for EchoProvider {
        fn id(&self) -> &str {
            "echo"
        }
        fn list_models(&self) -> BoxFuture<'_, ProviderResult<Vec<Model>>> {
            Box::pin(async {
                Ok(vec![Model {
                    id: "m1".into(),
                    label: "M1".into(),
                    capabilities: vec![],
                }])
            })
        }
        fn start_session(&self, c: SessionConfig) -> BoxFuture<'_, ProviderResult<String>> {
            Box::pin(async move { Ok(format!("s-{}", c.thread)) })
        }
        fn send(
            &self,
            _s: String,
            input: UserInput,
            sink: EventSink,
        ) -> BoxFuture<'_, ProviderResult<()>> {
            Box::pin(async move {
                self.runs.fetch_add(1, Ordering::SeqCst);
                self.prompts.lock().unwrap().push(input.text.clone());
                self.entered.notify_waiters();
                if self.block_next.swap(false, Ordering::SeqCst) {
                    return std::future::pending().await;
                }
                if self.fail_next.swap(false, Ordering::SeqCst) {
                    return Err(ProviderError::new("boom"));
                }
                sink.emit(ProviderEvent::TextDone { text: "ok".into() })
                    .await
                    .map_err(|e| ProviderError::new(e.to_string()))?;
                Ok(())
            })
        }
        fn respond_approval(
            &self,
            _s: String,
            _a: ApprovalId,
            _d: Decision,
        ) -> BoxFuture<'_, ProviderResult<()>> {
            Box::pin(async { Ok(()) })
        }
        fn cancel(&self, _s: String) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn end_session(&self, _s: String) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
    }

    async fn setup() -> (
        tempfile::TempDir,
        Kernel,
        Arc<EchoProvider>,
        ThreadId,
        ThreadId,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = KernelConfig {
            clock: Arc::new(StepClock::default()),
            ids: Arc::new(SequentialIds::new("D")),
            ..KernelConfig::new(dir.path().join("core.sqlite"))
        };
        let kernel = Kernel::open(cfg).await.unwrap();
        let provider = Arc::new(EchoProvider::default());
        kernel.register_provider(provider.clone()).unwrap();
        let ws = kernel
            .workspace_open(dir.path().to_str().unwrap())
            .await
            .unwrap();
        let t1 = kernel
            .thread_create(&ws.id, None, None, None, None)
            .await
            .unwrap()
            .id;
        let t2 = kernel
            .thread_create(&ws.id, None, None, None, None)
            .await
            .unwrap()
            .id;
        (dir, kernel, provider, t1, t2)
    }

    async fn wait_events(
        kernel: &Kernel,
        thread: &ThreadId,
        after: u64,
        kind: &str,
    ) -> Vec<repo::StoredEvent> {
        let mut sub = kernel.subscribe(thread, after).await.unwrap();
        let mut out = Vec::new();
        while let Some(batch) = sub.next_batch().await.unwrap() {
            for e in batch {
                let hit = e.event.kind() == kind;
                out.push(e);
                if hit {
                    return out;
                }
            }
        }
        panic!("closed before {kind}");
    }

    #[tokio::test]
    async fn at_trigger_fires_once_and_queues_when_busy() {
        let (dir, kernel, provider, t1, _) = setup().await;
        let due = "2026-11-15T10:10:00.000Z";
        let entry = kernel
            .thread_defer(
                &t1,
                UserInput::text("check deploy"),
                QueueTrigger::At {
                    time: Timestamp(due.into()),
                },
            )
            .await
            .unwrap();
        assert!(entry.trigger.is_some());

        // Before `due`, tick does nothing.
        assert_eq!(
            kernel
                .defer_tick("2026-11-15T10:09:59Z".parse().unwrap())
                .await
                .unwrap(),
            0
        );

        // Restart keeps the `at` trigger unheld so it fires once after restart.
        kernel.close().await;
        let cfg = KernelConfig {
            clock: Arc::new(StepClock::default()),
            ids: Arc::new(SequentialIds::new("R")),
            ..KernelConfig::new(dir.path().join("core.sqlite"))
        };
        let kernel = Kernel::open(cfg).await.unwrap();
        kernel.register_provider(provider.clone()).unwrap();

        assert_eq!(
            kernel
                .defer_tick("2026-11-15T10:10:00Z".parse().unwrap())
                .await
                .unwrap(),
            1
        );
        let evs = wait_events(&kernel, &t1, 0, "turn.ended").await;
        let ts = evs
            .iter()
            .find_map(|e| match &e.event {
                ThreadEvent::TurnStarted(t) => Some(t),
                _ => None,
            })
            .unwrap();
        assert!(ts.input.text.contains("check deploy"));
        assert!(ts.input.text.contains("Deferred message: scheduled for"));
        assert_eq!(
            ts.from.as_ref().and_then(|f| f.deferred.as_ref()),
            Some(&entry.id)
        );
        // Second tick does not repeat it.
        assert_eq!(
            kernel
                .defer_tick("2026-11-15T10:15:00Z".parse().unwrap())
                .await
                .unwrap(),
            0
        );

        // When busy, a fired `at` message joins the queue instead of steering.
        provider.block_next.store(true, Ordering::SeqCst);
        let entered = provider.entered.notified();
        tokio::pin!(entered);
        entered.as_mut().enable();
        kernel
            .thread_send(&t1, UserInput::text("busy"))
            .await
            .unwrap();
        entered.await;
        kernel
            .thread_defer(
                &t1,
                UserInput::text("after busy"),
                QueueTrigger::At {
                    time: Timestamp("2026-11-15T10:20:00.000Z".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            kernel
                .defer_tick("2026-11-15T10:20:00Z".parse().unwrap())
                .await
                .unwrap(),
            1
        );
        let snap = kernel.thread_get(&t1).await.unwrap();
        assert_eq!(snap.queue.len(), 1);
        assert!(snap.queue[0].trigger.is_none());
        assert!(snap.queue[0].input.text.contains("after busy"));
        kernel.thread_cancel(&t1).await.unwrap();
        kernel.close().await;
    }

    #[tokio::test]
    async fn after_turn_fires_respects_only_on_success_and_refuses_loops() {
        let (_dir, kernel, provider, t1, t2) = setup().await;
        // Direct self-loop is refused.
        let err = kernel
            .thread_defer(
                &t1,
                UserInput::text("self"),
                QueueTrigger::AfterTurn {
                    thread: t1.clone(),
                    only_on_success: false,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidParams);

        // t1 waits on t2; then t2 waiting on t1 would loop -> refused.
        kernel
            .thread_defer(
                &t1,
                UserInput::text("review t2"),
                QueueTrigger::AfterTurn {
                    thread: t2.clone(),
                    only_on_success: true,
                },
            )
            .await
            .unwrap();
        let loop_err = kernel
            .thread_defer(
                &t2,
                UserInput::text("back to t1"),
                QueueTrigger::AfterTurn {
                    thread: t1.clone(),
                    only_on_success: false,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(loop_err.code(), ErrorCode::InvalidParams);

        // If t2 fails, `only_on_success: true` does NOT fire t1 and removes the entry.
        provider.fail_next.store(true, Ordering::SeqCst);
        kernel
            .thread_send(&t2, UserInput::text("fail me"))
            .await
            .unwrap();
        wait_events(&kernel, &t2, 0, "turn.ended").await;
        wait_events(&kernel, &t1, 0, "queue.removed").await;
        assert_eq!(provider.runs.load(Ordering::SeqCst), 1);

        // Re-arm and let t2 succeed -> t1 fires.
        kernel
            .thread_defer(
                &t1,
                UserInput::text("review t2 ok"),
                QueueTrigger::AfterTurn {
                    thread: t2.clone(),
                    only_on_success: true,
                },
            )
            .await
            .unwrap();
        kernel
            .thread_send(&t2, UserInput::text("succeed"))
            .await
            .unwrap();
        wait_events(&kernel, &t1, 0, "turn.ended").await;
        let prompts = provider.prompts.lock().unwrap().clone();
        assert!(
            prompts
                .iter()
                .any(|p| p.contains("review t2 ok") && p.contains("finished (done)"))
        );
        kernel.close().await;
    }

    #[tokio::test]
    async fn after_command_and_poll_command_run_under_supervisor() {
        let (_dir, kernel, provider, t1, t2) = setup().await;
        // after_command using `sh -c "echo hello-defer; exit 0"` (Unix) / `cmd` (Windows).
        let (prog, args): (&str, Vec<String>) = if cfg!(windows) {
            ("cmd", vec!["/C".into(), "echo hello-defer".into()])
        } else {
            ("sh", vec!["-c".into(), "echo hello-defer".into()])
        };
        let mut argv = vec![prog.to_owned()];
        argv.extend(args);
        kernel
            .thread_defer(
                &t1,
                UserInput::text("build finished"),
                QueueTrigger::AfterCommand {
                    argv,
                    cwd: None,
                    timeout_s: 30,
                },
            )
            .await
            .unwrap();
        wait_events(&kernel, &t1, 0, "turn.ended").await;
        let prompts = provider.prompts.lock().unwrap().clone();
        assert!(
            prompts.iter().any(|p| p.contains("build finished")
                && p.contains("exited 0")
                && p.contains("hello-defer")),
            "prompts={prompts:?}"
        );

        // poll_command failing twice hits max_tries=2 and fires with failure note.
        let fail_argv: Vec<String> = if cfg!(windows) {
            vec!["cmd".into(), "/C".into(), "exit 7".into()]
        } else {
            vec!["sh".into(), "-c".into(), "echo still-red; exit 7".into()]
        };
        kernel
            .thread_defer(
                &t2,
                UserInput::text("check ci"),
                QueueTrigger::PollCommand {
                    argv: fail_argv,
                    cwd: None,
                    every_s: 10,
                    max_tries: 2,
                },
            )
            .await
            .unwrap();
        // Attempt 1 (due immediately at StepClock time): fails and schedules +10s.
        assert_eq!(
            kernel
                .defer_tick("2026-11-15T10:05:00Z".parse().unwrap())
                .await
                .unwrap(),
            0
        );
        // Attempt 2 (+10s): reaches max_tries=2 and fires!
        assert_eq!(
            kernel
                .defer_tick("2026-11-15T10:05:11Z".parse().unwrap())
                .await
                .unwrap(),
            1
        );
        wait_events(&kernel, &t2, 0, "turn.ended").await;
        let prompts = provider.prompts.lock().unwrap().clone();
        assert!(
            prompts.iter().any(|p| p.contains("check ci")
                && p.contains("failed after 2 attempts")
                && p.contains("exited 7")),
            "prompts={prompts:?}"
        );
        kernel.close().await;
    }

    #[tokio::test]
    async fn plugin_command_trigger_without_spawn_is_denied() {
        let (_dir, kernel, _provider, t1, _) = setup().await;
        let err = kernel
            .thread_defer_from(
                &t1,
                UserInput::text("watch build"),
                QueueTrigger::AfterCommand {
                    argv: vec!["cargo".into(), "test".into()],
                    cwd: None,
                    timeout_s: 60,
                },
                "acme.helper".into(),
                Grants::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::PermissionDenied);
        kernel.close().await;
    }
}
