//! The process supervisor (`docs/spec/03-kernel.md#processes`).
//!
//! Plugins never start processes themselves; the plugin host calls
//! [`Supervisor::spawn`] for them. The supervisor checks the request against the
//! plugin's [`Grants`] and the workspaces it acts for, builds the environment, starts the
//! program as the root of its own process tree (a process group on Unix, a job object on
//! Windows), and kills trees when their owner ends.
//!
//! Each child's stdout is read into a bounded channel (the child waits when the plugin
//! does not read), stderr into a ring buffer that drops the oldest bytes, so a plugin
//! that never reads stderr cannot stall the child.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{Notify, mpsc, watch};

use crate::grants::Grants;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix::Tree;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Tree;

#[cfg(test)]
mod tests;

/// Environment variables copied from the core into every child, when set. A trailing
/// `*` matches any suffix.
pub const ENV_ALLOWLIST: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_*", "TERM", "TMPDIR", "XDG_*",
];

/// Windows variables copied as well (matched without regard to case).
pub const ENV_ALLOWLIST_WINDOWS: &[&str] = &[
    "SystemRoot",
    "windir",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles*",
    "PATHEXT",
    "COMSPEC",
    "TEMP",
    "TMP",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
];

/// Variable prefixes a plugin may not set: they make the dynamic loader load code.
pub const ENV_REFUSED_PREFIXES: &[&str] = &["LD_", "DYLD_"];

/// Bytes of stderr kept per child.
pub const STDERR_KEEP: usize = 1 << 20;

const STDOUT_CHUNK: usize = 64 * 1024;
const STDOUT_CHUNKS: usize = 16;

/// Why a spawn or process call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProcessError {
    /// The grants or the workspace rules do not allow it.
    #[error("{0}")]
    PermissionDenied(String),
    /// The program or a secret was not found.
    #[error("{0}")]
    NotFound(String),
    /// The request is malformed.
    #[error("{0}")]
    InvalidArgument(String),
    /// The OS refused.
    #[error("{0}")]
    Io(String),
}

/// Who owns a process: the plugin host uses one owner per plugin instance. Killing an
/// owner kills every process it started.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerId(pub String);

impl fmt::Display for OwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a plugin asks to start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnRequest {
    /// A bare program name, resolved on the core's `PATH`.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Working directory; defaults to the first workspace.
    pub cwd: Option<String>,
    /// Variables the plugin sets.
    pub env: Vec<(String, String)>,
    /// Variables set from secrets: (variable, secret name).
    pub secret_env: Vec<(String, String)>,
}

/// Everything a spawn is checked against.
pub struct SpawnContext<'a> {
    /// The plugin, for logs.
    pub plugin: &'a str,
    /// Whose process this is.
    pub owner: &'a OwnerId,
    /// The grants that apply (for a delegated session, the contributing plugin's).
    pub grants: &'a Grants,
    /// Workspace roots the owner acts for; the first is the default `cwd`.
    pub workspaces: &'a [PathBuf],
    /// Looks up a secret's value by name; called only for granted names.
    pub secret: &'a (dyn Fn(&str) -> Option<String> + Sync),
}

impl fmt::Debug for SpawnContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpawnContext")
            .field("plugin", &self.plugin)
            .field("owner", &self.owner)
            .field("workspaces", &self.workspaces)
            .finish_non_exhaustive()
    }
}

/// A started process, as the supervisor records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Owner.
    pub owner: OwnerId,
    /// The plugin that asked.
    pub plugin: String,
    /// The requested program name.
    pub program: String,
    /// Where `PATH` resolution found it.
    pub resolved: PathBuf,
    /// OS process id.
    pub pid: u32,
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    /// The exit code, if it exited normally.
    pub code: Option<i32>,
}

/// Supervisor settings.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// The core's environment, from which the allowlist is copied and `PATH` is read.
    pub env: Vec<(OsString, OsString)>,
    /// Time between asking a tree to stop (SIGTERM) and killing it.
    pub kill_grace: Duration,
}

impl SupervisorConfig {
    /// Settings using this process's environment.
    pub fn from_process() -> Self {
        SupervisorConfig {
            env: std::env::vars_os().collect(),
            kill_grace: Duration::from_secs(2),
        }
    }
}

/// The process supervisor. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Supervisor {
    inner: Arc<SupervisorInner>,
}

#[derive(Debug)]
struct SupervisorInner {
    config: SupervisorConfig,
    table: Mutex<HashMap<OwnerId, Vec<Entry>>>,
}

#[derive(Debug)]
struct Entry {
    info: ProcessInfo,
    child: Weak<ChildInner>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // The tables stay consistent if a holder panicked.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Supervisor {
    /// A supervisor with these settings.
    pub fn new(config: SupervisorConfig) -> Self {
        Supervisor {
            inner: Arc::new(SupervisorInner {
                config,
                table: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn core_env(&self, name: &str) -> Option<&OsString> {
        self.inner
            .config
            .env
            .iter()
            .find(|(k, _)| env_name_eq(&k.to_string_lossy(), name))
            .map(|(_, v)| v)
    }

    /// Starts a process after checking it against `ctx`. Every refusal is logged with
    /// the plugin id.
    pub async fn spawn(
        &self,
        ctx: &SpawnContext<'_>,
        req: SpawnRequest,
    ) -> Result<Child, ProcessError> {
        let checked = self.check(ctx, &req).inspect_err(|e| {
            if let ProcessError::PermissionDenied(reason) = e {
                tracing::warn!(plugin = ctx.plugin, op = "process.spawn", %reason, "permission denied");
            }
        })?;
        let mut cmd = Command::new(&checked.resolved);
        cmd.args(&req.args)
            .current_dir(&checked.cwd)
            .env_clear()
            .envs(checked.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Tree::prepare(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| ProcessError::Io(format!("cannot start {}: {e}", req.program)))?;
        let tree = match Tree::attach(&child) {
            Ok(t) => t,
            Err(e) => {
                let _ = child.start_kill();
                return Err(ProcessError::Io(format!(
                    "cannot supervise {}: {e}",
                    req.program
                )));
            }
        };
        let pid = child.id().unwrap_or(0);
        let info = ProcessInfo {
            owner: ctx.owner.clone(),
            plugin: ctx.plugin.to_owned(),
            program: req.program.clone(),
            resolved: checked.resolved,
            pid,
        };
        tracing::info!(
            plugin = ctx.plugin,
            program = %info.program,
            resolved = %info.resolved.display(),
            pid,
            "process started"
        );
        let (status_tx, status_rx) = watch::channel(None);
        let inner = Arc::new(ChildInner::start(
            &mut child,
            tree,
            self.inner.config.kill_grace,
            status_rx,
        ));
        tokio::spawn(async move {
            let code = child.wait().await.ok().and_then(|s| s.code());
            let _ = status_tx.send(Some(ExitStatus { code }));
        });
        let mut table = lock(&self.inner.table);
        let entries = table.entry(ctx.owner.clone()).or_default();
        entries.retain(|e| e.child.strong_count() > 0);
        entries.push(Entry {
            info,
            child: Arc::downgrade(&inner),
        });
        Ok(Child { inner })
    }

    fn check(&self, ctx: &SpawnContext<'_>, req: &SpawnRequest) -> Result<Checked, ProcessError> {
        let program = req.program.as_str();
        if !is_bare_name(program) {
            return Err(ProcessError::PermissionDenied(format!(
                "`{program}` is not a bare program name; only names listed in permissions.spawn can be started"
            )));
        }
        if !ctx.grants.may_spawn(program) {
            return Err(ProcessError::PermissionDenied(format!(
                "`{program}` is not in permissions.spawn"
            )));
        }
        let roots: Vec<PathBuf> = ctx
            .workspaces
            .iter()
            .filter_map(|r| std::fs::canonicalize(r).ok())
            .collect();
        let Some(first) = roots.first() else {
            return Err(ProcessError::PermissionDenied(
                "a process can only be started for a workspace".into(),
            ));
        };
        let cwd = match &req.cwd {
            None => first.clone(),
            Some(c) => {
                let p = Path::new(c);
                let joined = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    first.join(p)
                };
                let canon = std::fs::canonicalize(&joined).map_err(|e| {
                    ProcessError::InvalidArgument(format!("working directory {c}: {e}"))
                })?;
                if !roots.iter().any(|r| canon.starts_with(r)) {
                    return Err(ProcessError::PermissionDenied(format!(
                        "working directory {c} is outside the workspace"
                    )));
                }
                canon
            }
        };

        let mut env: Vec<(OsString, OsString)> = self
            .inner
            .config
            .env
            .iter()
            .filter(|(k, _)| env_allowed(&k.to_string_lossy()))
            .cloned()
            .collect();
        for (k, v) in &req.env {
            check_env_name(k)?;
            if ENV_REFUSED_PREFIXES
                .iter()
                .any(|p| k.to_ascii_uppercase().starts_with(p))
            {
                return Err(ProcessError::PermissionDenied(format!(
                    "plugins may not set {k}"
                )));
            }
            set_env(&mut env, k, v.into());
        }
        for (k, secret) in &req.secret_env {
            check_env_name(k)?;
            if !ctx.grants.may_read_secret(secret) {
                return Err(ProcessError::PermissionDenied(format!(
                    "secret `{secret}` is not in permissions.secrets"
                )));
            }
            let value = (ctx.secret)(secret)
                .ok_or_else(|| ProcessError::NotFound(format!("secret `{secret}` is not set")))?;
            set_env(&mut env, k, value.into());
        }

        let path = self.core_env("PATH").cloned().unwrap_or_default();
        let resolved = which::which_in(program, Some(path), &cwd)
            .map_err(|_| ProcessError::NotFound(format!("`{program}` was not found on PATH")))?;
        Ok(Checked { resolved, cwd, env })
    }

    /// Processes started and not yet dropped, for diagnostics.
    pub fn processes(&self) -> Vec<ProcessInfo> {
        let table = lock(&self.inner.table);
        table
            .values()
            .flatten()
            .filter(|e| e.child.strong_count() > 0)
            .map(|e| e.info.clone())
            .collect()
    }

    /// Kills every process `owner` started (stop, grace period, kill), and forgets them.
    pub async fn kill_owner(&self, owner: &OwnerId) {
        let entries = lock(&self.inner.table).remove(owner).unwrap_or_default();
        kill_all(entries).await;
    }

    /// Kills every supervised process; used when the core stops.
    pub async fn shutdown(&self) {
        let entries: Vec<Entry> = lock(&self.inner.table)
            .drain()
            .flat_map(|(_, v)| v)
            .collect();
        kill_all(entries).await;
    }
}

/// Kills the processes concurrently, so the grace periods overlap.
async fn kill_all(entries: Vec<Entry>) {
    let mut set = tokio::task::JoinSet::new();
    for child in entries.iter().filter_map(|e| e.child.upgrade()) {
        set.spawn(async move { child.kill().await });
    }
    while set.join_next().await.is_some() {}
}

struct Checked {
    resolved: PathBuf,
    cwd: PathBuf,
    env: Vec<(OsString, OsString)>,
}

/// A name without any path: no separators, not `.` or `..`, not empty.
fn is_bare_name(program: &str) -> bool {
    !program.is_empty()
        && program != "."
        && program != ".."
        && !program.contains(['/', '\\', ':', '\0'])
}

fn check_env_name(name: &str) -> Result<(), ProcessError> {
    if name.is_empty() || name.contains(['=', '\0']) {
        Err(ProcessError::InvalidArgument(format!(
            "`{name}` is not a valid environment variable name"
        )))
    } else {
        Ok(())
    }
}

/// Environment names compare without case on Windows.
fn env_name_eq(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name
            .get(..prefix.len())
            .is_some_and(|p| env_name_eq(p, prefix)),
        None => env_name_eq(pattern, name),
    }
}

fn env_allowed(name: &str) -> bool {
    ENV_ALLOWLIST.iter().any(|p| pattern_matches(p, name))
        || (cfg!(windows)
            && ENV_ALLOWLIST_WINDOWS
                .iter()
                .any(|p| pattern_matches(p, name)))
}

fn set_env(env: &mut Vec<(OsString, OsString)>, name: &str, value: OsString) {
    env.retain(|(k, _)| !env_name_eq(&k.to_string_lossy(), name));
    env.push((name.into(), value));
}

/// A process started by the supervisor. Dropping the last handle kills its tree.
#[derive(Debug, Clone)]
pub struct Child {
    inner: Arc<ChildInner>,
}

struct ChildInner {
    tree: Tree,
    kill_grace: Duration,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    stdout: tokio::sync::Mutex<StdoutReader>,
    stderr: Arc<StderrBuffer>,
    status: watch::Receiver<Option<ExitStatus>>,
}

impl fmt::Debug for ChildInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChildInner").finish_non_exhaustive()
    }
}

struct StdoutReader {
    rx: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
}

#[derive(Default)]
struct StderrBuffer {
    state: Mutex<StderrState>,
    changed: Notify,
}

#[derive(Default)]
struct StderrState {
    bytes: VecDeque<u8>,
    eof: bool,
}

impl ChildInner {
    /// Takes the pipes and starts the reader tasks.
    fn start(
        child: &mut tokio::process::Child,
        tree: Tree,
        kill_grace: Duration,
        status: watch::Receiver<Option<ExitStatus>>,
    ) -> Self {
        let stdin = child.stdin.take();
        let (tx, rx) = mpsc::channel(STDOUT_CHUNKS);
        if let Some(mut out) = child.stdout.take() {
            tokio::spawn(async move {
                let mut buf = vec![0u8; STDOUT_CHUNK];
                loop {
                    match out.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
        let stderr = Arc::new(StderrBuffer::default());
        match child.stderr.take() {
            Some(mut err) => {
                let buffer = Arc::clone(&stderr);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; STDOUT_CHUNK];
                    loop {
                        let n = match err.read(&mut buf).await {
                            Ok(0) | Err(_) => 0,
                            Ok(n) => n,
                        };
                        let mut state = lock(&buffer.state);
                        if n == 0 {
                            state.eof = true;
                            drop(state);
                            buffer.changed.notify_waiters();
                            break;
                        }
                        state.bytes.extend(&buf[..n]);
                        let excess = state.bytes.len().saturating_sub(STDERR_KEEP);
                        state.bytes.drain(..excess);
                        drop(state);
                        buffer.changed.notify_waiters();
                    }
                });
            }
            None => lock(&stderr.state).eof = true,
        }
        ChildInner {
            tree,
            kill_grace,
            stdin: tokio::sync::Mutex::new(stdin),
            stdout: tokio::sync::Mutex::new(StdoutReader {
                rx,
                pending: Vec::new(),
            }),
            stderr,
            status,
        }
    }

    async fn kill(&self) {
        if self.status.borrow().is_none() {
            self.tree.stop();
            let mut status = self.status.clone();
            let _ = tokio::time::timeout(self.kill_grace, status.wait_for(Option::is_some)).await;
        }
        // Also reaches descendants that outlived the root.
        self.tree.kill();
    }
}

impl Drop for ChildInner {
    fn drop(&mut self) {
        self.tree.kill();
    }
}

impl Child {
    /// Writes to stdin.
    pub async fn write_stdin(&self, data: &[u8]) -> Result<(), ProcessError> {
        let mut stdin = self.inner.stdin.lock().await;
        let pipe = stdin
            .as_mut()
            .ok_or_else(|| ProcessError::InvalidArgument("stdin is closed".into()))?;
        pipe.write_all(data)
            .await
            .and(pipe.flush().await)
            .map_err(|e| ProcessError::Io(format!("writing to stdin: {e}")))
    }

    /// Closes stdin.
    pub async fn close_stdin(&self) {
        self.inner.stdin.lock().await.take();
    }

    /// Up to `max` bytes of stdout, waiting for at least one; `None` at EOF.
    pub async fn read_stdout(&self, max: usize) -> Option<Vec<u8>> {
        let max = max.max(1);
        let mut out = self.inner.stdout.lock().await;
        if out.pending.is_empty() {
            out.pending = out.rx.recv().await?;
        }
        let n = max.min(out.pending.len());
        Some(out.pending.drain(..n).collect())
    }

    /// Up to `max` bytes of stderr, waiting for at least one; `None` at EOF. Only the
    /// last [`STDERR_KEEP`] bytes are kept.
    pub async fn read_stderr(&self, max: usize) -> Option<Vec<u8>> {
        let max = max.max(1);
        let buffer = &self.inner.stderr;
        loop {
            let notified = buffer.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = lock(&buffer.state);
                if !state.bytes.is_empty() {
                    let n = max.min(state.bytes.len());
                    return Some(state.bytes.drain(..n).collect());
                }
                if state.eof {
                    return None;
                }
            }
            notified.await;
        }
    }

    /// Waits for the process to exit.
    pub async fn wait(&self) -> ExitStatus {
        let mut status = self.inner.status.clone();
        match status.wait_for(Option::is_some).await {
            Ok(s) => s.unwrap_or(ExitStatus { code: None }),
            Err(_) => ExitStatus { code: None },
        }
    }

    /// Stops the process and its descendants: asks first, kills after the grace period.
    pub async fn kill(&self) {
        self.inner.kill().await;
    }
}
