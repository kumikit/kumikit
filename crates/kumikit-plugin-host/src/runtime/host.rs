//! The data each instance's store carries, and the host imports (`kumikit:host`).
//!
//! Every import that touches something outside the plugin checks the grants of the
//! plugin the instance acts for (for a delegated session, the contributing plugin) and
//! logs each refusal with the plugin id and the operation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use kumikit_kernel::grants::Grants;
use kumikit_kernel::process::{self, OwnerId, ProcessError, SpawnContext};
use kumikit_kernel::provider::{EventSink, ProviderEvent};
use kumikit_kernel::{Kernel, KernelError};
use kumikit_protocol::model::{ApprovalId, Decision};
use tokio::sync::Notify;
use wasmtime::component::{Resource, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use super::bindings::base::kumikit::host::types::{Error, ErrorCode};
use super::bindings::base::kumikit::host::{kv, log, types};
use super::bindings::base::kumikit::host::{process as wit_process, secrets, settings};
use super::bindings::provider::kumikit::host::events;
use crate::engine::{HasLimits, Limits};

pub(crate) fn err(code: ErrorCode, message: impl Into<String>) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

pub(crate) fn kernel_err(e: KernelError) -> Error {
    match e {
        KernelError::Limit(m) => err(ErrorCode::Limit, m),
        KernelError::NotFound(m) => err(ErrorCode::NotFound, m),
        KernelError::InvalidParams(m) => err(ErrorCode::InvalidArgument, m),
        KernelError::Conflict(m) => err(ErrorCode::Conflict, m),
        KernelError::PermissionDenied(m) => err(ErrorCode::PermissionDenied, m),
        KernelError::Io(m) => err(ErrorCode::Io, m),
        other => err(ErrorCode::Internal, other.to_string()),
    }
}

fn process_err(e: ProcessError) -> Error {
    match e {
        ProcessError::PermissionDenied(m) => err(ErrorCode::PermissionDenied, m),
        ProcessError::NotFound(m) => err(ErrorCode::NotFound, m),
        ProcessError::InvalidArgument(m) => err(ErrorCode::InvalidArgument, m),
        ProcessError::Io(m) => err(ErrorCode::Io, m),
    }
}

/// State shared between a session's instance and the host side of its provider: the
/// running turn's sink, approval answers and the cancel signal. Anything that must reach
/// a guest during `send` goes through here, because the instance itself is busy.
#[derive(Debug, Default)]
pub struct SessionShared {
    turn: Mutex<Option<EventSink>>,
    decisions: Mutex<HashMap<ApprovalId, Decision>>,
    decided: Notify,
    cancel_pending: AtomicBool,
    cancelled: Notify,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // The maps stay consistent if a holder panicked.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl SessionShared {
    /// A turn starts: events go to `sink`. Clears a cancel left from an earlier turn.
    pub fn begin_turn(&self, sink: EventSink) {
        self.cancel_pending.store(false, Ordering::Release);
        *lock(&self.turn) = Some(sink);
    }

    /// The turn is over; later emits fail with `conflict`.
    pub fn end_turn(&self) {
        lock(&self.turn).take();
        lock(&self.decisions).clear();
    }

    /// Delivers an approval answer, possibly before the guest waits for it.
    pub fn answer(&self, approval: ApprovalId, decision: Decision) {
        lock(&self.decisions).insert(approval, decision);
        self.decided.notify_waiters();
    }

    /// Cancels the running turn: the import the guest is blocked in (or its next
    /// blocking import) returns `cancelled`, once.
    pub fn cancel(&self) {
        self.cancel_pending.store(true, Ordering::Release);
        self.cancelled.notify_waiters();
    }

    fn take_cancel(&self) -> bool {
        self.cancel_pending.swap(false, Ordering::AcqRel)
    }

    fn sink(&self) -> Option<EventSink> {
        lock(&self.turn).clone()
    }

    /// Runs `fut` unless the turn is cancelled first.
    async fn cancellable<T>(&self, fut: impl Future<Output = T>) -> Result<T, Error> {
        let notified = self.cancelled.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.take_cancel() {
            return Err(cancelled());
        }
        tokio::select! {
            v = fut => Ok(v),
            () = notified => {
                self.take_cancel();
                Err(cancelled())
            }
        }
    }

    async fn decision(&self, id: &ApprovalId) -> Result<Decision, Error> {
        loop {
            let notified = self.decided.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(d) = lock(&self.decisions).remove(id) {
                return Ok(d);
            }
            self.cancellable(notified).await?;
        }
    }
}

fn cancelled() -> Error {
    err(ErrorCode::Cancelled, "the turn was cancelled")
}

/// Who an instance is and what it may do.
#[derive(Debug, Clone)]
pub struct Identity {
    /// The plugin whose code runs (kv namespace, data directory, logs).
    pub plugin: String,
    /// The plugin whose grants apply; differs from `plugin` for delegated sessions.
    pub grants_from: String,
    /// The grants.
    pub grants: Grants,
    /// Workspace roots the instance acts for (a session's workspace, or none).
    pub workspaces: Vec<PathBuf>,
}

/// The data in each instance's store.
pub struct HostState {
    pub(crate) wasi: WasiCtx,
    pub(crate) http: wasmtime_wasi_http::WasiHttpCtx,
    pub(crate) outbound: super::outbound::Outbound,
    pub(crate) table: ResourceTable,
    pub(crate) limits: Limits,
    pub(crate) kernel: Kernel,
    pub(crate) id: Identity,
    pub(crate) owner: OwnerId,
    pub(crate) session: Arc<SessionShared>,
    pub(crate) settings_schema: Option<PathBuf>,
    /// Workspace of the calling thread during a gateway `tool.call`, if any.
    pub(crate) call_workspace: Option<kumikit_protocol::model::WorkspaceId>,
    /// Model settings reported during the running `list-models`; `None` outside it.
    pub(crate) model_options: Option<ModelOptions>,
}

/// Model settings by model id, as reported through `kumikit:models/options`.
pub type ModelOptions =
    std::collections::HashMap<String, Vec<kumikit_protocol::methods::ModelOption>>;

impl std::fmt::Debug for HostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostState")
            .field("id", &self.id)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl wasmtime_wasi_http::WasiHttpView for HostState {
    fn http(&mut self) -> wasmtime_wasi_http::WasiHttpCtxView<'_> {
        wasmtime_wasi_http::WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: &mut self.outbound,
        }
    }
}

impl HasLimits for HostState {
    fn limits(&mut self) -> &mut Limits {
        &mut self.limits
    }
}

impl HostState {
    pub(crate) fn deny(&self, op: &str, reason: String) -> Error {
        tracing::warn!(
            plugin = %self.id.plugin,
            grants = %self.id.grants_from,
            op,
            %reason,
            "permission denied"
        );
        err(ErrorCode::PermissionDenied, reason)
    }
}

// ---- types, log, kv (both worlds) ----

impl types::Host for HostState {}

impl log::Host for HostState {
    async fn log(&mut self, level: log::Level, message: String) {
        let plugin = self.id.plugin.as_str();
        match level {
            log::Level::Trace => tracing::trace!(target: "plugin", plugin, "{message}"),
            log::Level::Debug => tracing::debug!(target: "plugin", plugin, "{message}"),
            log::Level::Info => tracing::info!(target: "plugin", plugin, "{message}"),
            log::Level::Warn => tracing::warn!(target: "plugin", plugin, "{message}"),
            log::Level::Error => tracing::error!(target: "plugin", plugin, "{message}"),
        }
    }
}

impl kv::Host for HostState {
    async fn get(&mut self, key: String) -> Result<Option<Vec<u8>>, Error> {
        self.kernel
            .kv_get(&self.id.plugin, &key)
            .await
            .map_err(kernel_err)
    }

    async fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), Error> {
        self.kernel
            .kv_set(&self.id.plugin, &key, value)
            .await
            .map_err(kernel_err)
    }

    async fn delete(&mut self, key: String) -> Result<(), Error> {
        self.kernel
            .kv_delete(&self.id.plugin, &key)
            .await
            .map_err(kernel_err)
    }

    async fn list(&mut self, prefix: String) -> Result<Vec<String>, Error> {
        self.kernel
            .kv_list(&self.id.plugin, &prefix)
            .await
            .map_err(kernel_err)
    }
}

// ---- secrets ----

impl secrets::Host for HostState {
    async fn get(&mut self, name: String) -> Result<Option<String>, Error> {
        if !self.id.grants.may_read_secret(&name) {
            return Err(self.deny(
                "secrets.get",
                format!("secret `{name}` is not in permissions.secrets"),
            ));
        }
        self.kernel.secrets().get(&name).await.map_err(kernel_err)
    }
}

impl super::bindings::base::kumikit::plugin::types::Host for HostState {}

// ---- settings ----

impl settings::Host for HostState {
    async fn get(&mut self) -> Result<String, Error> {
        let Some(path) = self.settings_schema.clone() else {
            return Ok("{}".into());
        };
        let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
            .await
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
            .map_err(|e| {
                err(
                    ErrorCode::Io,
                    format!("cannot read the settings schema: {e}"),
                )
            })?;
        let schema: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| err(ErrorCode::Internal, format!("settings schema: {e}")))?;
        let stored = self
            .kernel
            .settings_get(&self.id.plugin)
            .await
            .map_err(kernel_err)?;
        Ok(crate::settings::with_defaults(&schema, stored).to_string())
    }
}

// ---- process ----

/// The `child` resource: a supervised process.
#[derive(Debug, Clone)]
pub struct ChildRes(pub(crate) process::Child);

impl wit_process::Host for HostState {
    async fn spawn(&mut self, req: wit_process::SpawnRequest) -> Result<Resource<ChildRes>, Error> {
        // Fetch only the secrets the grants allow; the supervisor refuses the rest.
        let mut values = HashMap::new();
        for (_, name) in &req.secret_env {
            if self.id.grants.may_read_secret(name)
                && let Some(v) = self.kernel.secrets().get(name).await.map_err(kernel_err)?
            {
                values.insert(name.clone(), v);
            }
        }
        let lookup = move |name: &str| values.get(name).cloned();
        let ctx = SpawnContext {
            plugin: &self.id.plugin,
            owner: &self.owner,
            grants: &self.id.grants,
            workspaces: &self.id.workspaces,
            secret: &lookup,
        };
        let request = process::SpawnRequest {
            program: req.program,
            args: req.args,
            cwd: req.cwd,
            env: req.env,
            secret_env: req.secret_env,
        };
        let child = self
            .kernel
            .supervisor()
            .spawn(&ctx, request)
            .await
            .map_err(process_err)?;
        self.table
            .push(ChildRes(child))
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))
    }
}

impl HostState {
    fn child(&self, r: &Resource<ChildRes>) -> Result<process::Child, Error> {
        self.table
            .get(r)
            .map(|c| c.0.clone())
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))
    }
}

fn read_size(max: u32) -> usize {
    (max as usize).clamp(1, 1 << 20)
}

impl wit_process::HostChild for HostState {
    async fn write_stdin(&mut self, r: Resource<ChildRes>, data: Vec<u8>) -> Result<(), Error> {
        let child = self.child(&r)?;
        self.session
            .cancellable(child.write_stdin(&data))
            .await?
            .map_err(process_err)
    }

    async fn close_stdin(&mut self, r: Resource<ChildRes>) {
        if let Ok(child) = self.child(&r) {
            child.close_stdin().await;
        }
    }

    async fn read_stdout(
        &mut self,
        r: Resource<ChildRes>,
        max: u32,
    ) -> Result<Option<Vec<u8>>, Error> {
        let child = self.child(&r)?;
        self.session
            .cancellable(child.read_stdout(read_size(max)))
            .await
    }

    async fn read_stderr(
        &mut self,
        r: Resource<ChildRes>,
        max: u32,
    ) -> Result<Option<Vec<u8>>, Error> {
        let child = self.child(&r)?;
        self.session
            .cancellable(child.read_stderr(read_size(max)))
            .await
    }

    async fn wait(&mut self, r: Resource<ChildRes>) -> Result<wit_process::ExitStatus, Error> {
        let child = self.child(&r)?;
        let status = self.session.cancellable(child.wait()).await?;
        Ok(wit_process::ExitStatus { code: status.code })
    }

    async fn kill(&mut self, r: Resource<ChildRes>) {
        if let Ok(child) = self.child(&r) {
            child.kill().await;
        }
    }

    async fn drop(&mut self, r: Resource<ChildRes>) -> wasmtime::Result<()> {
        // Dropping the last handle kills the process tree.
        self.table.delete(r)?;
        Ok(())
    }
}

// ---- steering (kumikit:turn/input, ADR 0028) ----

use super::bindings::provider::kumikit::provider::types::UserInput as WitUserInput;
use super::bindings::turn::kumikit::turn::input as wit_input;

/// Steering carries text only: inputs with attachments are queued by the kernel instead.
fn wit_user_input(input: kumikit_protocol::model::UserInput) -> WitUserInput {
    WitUserInput { text: input.text }
}

impl wit_input::Host for HostState {
    async fn take(&mut self) -> Result<Option<WitUserInput>, Error> {
        let Some(sink) = self.session.sink() else {
            return Ok(None);
        };
        let taken = sink.take_input().await.map_err(kernel_err)?;
        Ok(taken.map(wit_user_input))
    }

    async fn read_stdout(
        &mut self,
        r: Resource<ChildRes>,
        max: u32,
    ) -> Result<wit_input::Read, Error> {
        let child = self.child(&r)?;
        let n = read_size(max);
        let Some(sink) = self.session.sink() else {
            // Outside a turn there is no input: a plain read.
            let out = self.session.cancellable(child.read_stdout(n)).await?;
            return Ok(out.map_or(wit_input::Read::Eof, wit_input::Read::Stdout));
        };
        loop {
            // Reading stdout is cancel-safe (a channel receive), so losing the race
            // drops no output.
            let next = self
                .session
                .cancellable(async {
                    // Input first: an agent that streams output nonstop is still steered.
                    tokio::select! {
                        biased;
                        () = sink.input_ready() => Err(()),
                        out = child.read_stdout(n) => Ok(out),
                    }
                })
                .await?;
            match next {
                Ok(Some(bytes)) => return Ok(wit_input::Read::Stdout(bytes)),
                Ok(None) => return Ok(wit_input::Read::Eof),
                Err(()) => {
                    if let Some(input) = sink.take_input().await.map_err(kernel_err)? {
                        return Ok(wit_input::Read::Input(wit_user_input(input)));
                    }
                    // Someone else took it first: keep reading.
                }
            }
        }
    }
}

// ---- events ----

fn json(what: &str, text: &str) -> Result<serde_json::Value, Error> {
    serde_json::from_str(text).map_err(|e| {
        err(
            ErrorCode::InvalidArgument,
            format!("{what} is not JSON: {e}"),
        )
    })
}

fn to_provider_event(e: events::Event) -> Result<ProviderEvent, Error> {
    use events::Event as E;
    Ok(match e {
        E::TextDelta(text) => ProviderEvent::TextDelta { text },
        E::TextDone(text) => ProviderEvent::TextDone { text },
        E::ReasoningDelta(text) => ProviderEvent::ReasoningDelta { text },
        E::ToolStarted(t) => ProviderEvent::ToolStarted {
            call: t.call,
            name: t.name,
            input: json("tool input", &t.input)?,
        },
        E::ToolOutput(t) => ProviderEvent::ToolOutput {
            call: t.call,
            chunk: t.chunk,
        },
        E::ToolFinished(t) => ProviderEvent::ToolFinished {
            call: t.call,
            ok: t.ok,
            result: t
                .result
                .as_deref()
                .map(|r| json("tool result", r))
                .transpose()?,
        },
        E::FileChanged(f) => ProviderEvent::FileChanged {
            path: f.path,
            diff: f.diff,
        },
        E::Usage(u) => ProviderEvent::Usage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cost: u.cost,
        },
        E::Plugin(p) => ProviderEvent::Plugin {
            kind: p.kind,
            data: json("plugin event data", &p.data)?,
        },
    })
}

impl HostState {
    fn turn_sink(&self) -> Result<EventSink, Error> {
        self.session
            .sink()
            .ok_or_else(|| err(ErrorCode::Conflict, "no turn is running"))
    }
}

impl events::Host for HostState {
    async fn emit(&mut self, event: events::Event) -> Result<(), Error> {
        let sink = self.turn_sink()?;
        if self.session.take_cancel() {
            return Err(cancelled());
        }
        let event = to_provider_event(event)?;
        sink.emit(event).await.map(|_| ()).map_err(kernel_err)
    }

    async fn request_approval(
        &mut self,
        kind: String,
        summary: String,
        detail: Option<String>,
    ) -> Result<events::Decision, Error> {
        let sink = self.turn_sink()?;
        let detail = detail
            .as_deref()
            .map(|d| json("approval detail", d))
            .transpose()?;
        let id = sink
            .request_approval(kind, summary, detail)
            .await
            .map_err(kernel_err)?;
        Ok(match self.session.decision(&id).await? {
            Decision::Allow => events::Decision::Allow,
            Decision::AllowAlways => events::Decision::AllowAlways,
            Decision::Deny => events::Decision::Deny,
        })
    }
}

impl super::bindings::provider::kumikit::provider::types::Host for HostState {}

impl super::bindings::session_events::kumikit::session::events::Host for HostState {
    async fn emit_commands(
        &mut self,
        commands: Vec<super::bindings::session_events::kumikit::session::events::CommandInfo>,
    ) -> Result<(), Error> {
        let sink = self.turn_sink()?;
        if self.session.take_cancel() {
            return Err(cancelled());
        }
        let commands = commands
            .into_iter()
            .map(|c| kumikit_protocol::model::SlashCommand {
                name: c.name,
                description: c.description,
                hint: c.hint,
            })
            .collect();
        sink.emit(ProviderEvent::CommandsAvailable { commands })
            .await
            .map(|_| ())
            .map_err(kernel_err)
    }

    async fn emit_limit(&mut self, reset_at: Option<String>, message: String) -> Result<(), Error> {
        let sink = self.turn_sink()?;
        if self.session.take_cancel() {
            return Err(cancelled());
        }
        sink.emit(ProviderEvent::LimitReached {
            reset_at: reset_at.map(kumikit_protocol::model::Timestamp),
            message,
        })
        .await
        .map(|_| ())
        .map_err(kernel_err)
    }
}

impl super::bindings::models::kumikit::models::options::Host for HostState {
    async fn set_options(
        &mut self,
        model: String,
        options: Vec<super::bindings::models::kumikit::models::options::ModelOption>,
    ) -> Result<(), Error> {
        let Some(map) = self.model_options.as_mut() else {
            return Err(err(
                ErrorCode::InvalidArgument,
                "set-options is only valid during list-models",
            ));
        };
        let options = options
            .into_iter()
            .map(|o| kumikit_protocol::methods::ModelOption {
                id: o.id,
                label: o.label,
                kind: o.kind,
                values: o
                    .values
                    .into_iter()
                    .map(|v| kumikit_protocol::methods::ModelOptionValue {
                        value: v.value,
                        label: v.label,
                        description: v.description,
                    })
                    .collect(),
                default: o.default,
            })
            .collect();
        map.insert(model, options);
        Ok(())
    }
}

// ---- WASI stdout/stderr to the log ----

/// Sends a guest's stdout or stderr to the core's log at debug level.
#[derive(Debug, Clone)]
pub(crate) struct LogStream {
    pub(crate) plugin: String,
    pub(crate) stream: &'static str,
}

impl wasmtime_wasi::cli::IsTerminal for LogStream {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl wasmtime_wasi::cli::StdoutStream for LogStream {
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

impl tokio::io::AsyncWrite for LogStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let text = String::from_utf8_lossy(buf);
        for line in text.lines().filter(|l| !l.is_empty()) {
            tracing::debug!(target: "plugin", plugin = %self.plugin, stream = self.stream, "{line}");
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

// ---- timers ----

impl super::bindings::base::kumikit::host::timers::Host for HostState {
    async fn set(
        &mut self,
        key: String,
        schedule: super::bindings::base::kumikit::host::timers::Schedule,
    ) -> Result<Option<String>, Error> {
        use super::bindings::base::kumikit::host::timers::Missed;
        use kumikit_kernel::schedule as ks;
        if !self.id.grants.schedules {
            return Err(self.deny("timers.set", "permissions.schedules is not granted".into()));
        }
        if key.starts_with(ks::MANIFEST_PREFIX) {
            return Err(err(
                ErrorCode::InvalidArgument,
                format!(
                    "keys starting with `{}` belong to the manifest",
                    ks::MANIFEST_PREFIX
                ),
            ));
        }
        let spec = ks::Spec {
            cron: schedule.cron,
            zone: schedule.time_zone,
            description: schedule.description,
            missed: match schedule.missed {
                Missed::Skip => ks::Missed::Skip,
                Missed::Once => ks::Missed::Once,
            },
        };
        let kernel = self.kernel.clone();
        let plugin = self.id.plugin.clone();
        kernel
            .schedule_set(
                &plugin,
                &key,
                spec,
                ks::Source::Plugin,
                jiff::Timestamp::now(),
            )
            .await
            .map_err(kernel_err)
    }

    async fn delete(&mut self, key: String) -> Result<(), Error> {
        if !self.id.grants.schedules {
            return Err(self.deny(
                "timers.delete",
                "permissions.schedules is not granted".into(),
            ));
        }
        if key.starts_with(kumikit_kernel::schedule::MANIFEST_PREFIX) {
            return Err(err(
                ErrorCode::InvalidArgument,
                "manifest schedules cannot be deleted",
            ));
        }
        let kernel = self.kernel.clone();
        let plugin = self.id.plugin.clone();
        kernel
            .schedule_delete(&plugin, &key)
            .await
            .map_err(kernel_err)
    }

    async fn list(
        &mut self,
    ) -> Result<Vec<super::bindings::base::kumikit::host::timers::Info>, Error> {
        use super::bindings::base::kumikit::host::timers::{Info, Missed, Schedule};
        if !self.id.grants.schedules {
            return Err(self.deny("timers.list", "permissions.schedules is not granted".into()));
        }
        let kernel = self.kernel.clone();
        let plugin = self.id.plugin.clone();
        let rows = kernel.schedule_list().await.map_err(kernel_err)?;
        Ok(rows
            .into_iter()
            .filter(|r| r.plugin == plugin)
            .map(|r| Info {
                key: r.key,
                schedule: Schedule {
                    cron: r.spec.cron,
                    time_zone: r.spec.zone,
                    description: r.spec.description,
                    missed: match r.spec.missed {
                        kumikit_kernel::schedule::Missed::Skip => Missed::Skip,
                        kumikit_kernel::schedule::Missed::Once => Missed::Once,
                    },
                },
                enabled: r.enabled,
                next_run: r.next_run,
            })
            .collect())
    }
}

impl super::bindings::usage_report::kumikit::usage::types::Host for HostState {}

/// A provider's plan limits snapshot in kernel form (ADR 0035).
pub(crate) fn limits_from_wit(
    l: super::bindings::usage_report::kumikit::usage::types::Limits,
) -> kumikit_protocol::model::UsageLimits {
    use kumikit_protocol::model::{LimitWindow, Timestamp, UsageLimits};
    UsageLimits {
        plan: l.plan,
        account: l.account,
        account_url: l.account_url,
        windows: l
            .windows
            .into_iter()
            .map(|w| LimitWindow {
                id: w.id,
                label: w.label,
                group: w.group,
                used_percent: f64::from(w.used_percent),
                resets_at: w.resets_at.map(Timestamp),
                model: w.model,
            })
            .collect(),
    }
}

impl super::bindings::usage_report::kumikit::usage::report::Host for HostState {
    async fn emit_limits(
        &mut self,
        limits: super::bindings::usage_report::kumikit::usage::types::Limits,
    ) -> Result<(), Error> {
        let sink = self.turn_sink()?;
        if self.session.take_cancel() {
            return Err(cancelled());
        }
        sink.report_limits(limits_from_wit(limits))
            .await
            .map_err(kernel_err)
    }

    async fn emit_turn_usage(
        &mut self,
        models: Vec<super::bindings::usage_report::kumikit::usage::types::ModelUsage>,
    ) -> Result<(), Error> {
        let sink = self.turn_sink()?;
        if self.session.take_cancel() {
            return Err(cancelled());
        }
        let models = models
            .into_iter()
            .map(|m| kumikit_protocol::model::ModelUsage {
                model: m.model,
                input_tokens: m.input_tokens,
                output_tokens: m.output_tokens,
                cached_input_tokens: Some(m.cached_input_tokens),
                cost: m.cost,
            })
            .collect();
        sink.emit(ProviderEvent::UsageByModel { models })
            .await
            .map(|_| ())
            .map_err(kernel_err)
    }
}
