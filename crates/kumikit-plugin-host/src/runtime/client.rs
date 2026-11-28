//! `kumikit:host/client`: the protocol surface for plugins that act like clients
//! (`docs/spec/03-kernel.md#client-interface`). Every call checks the operation against
//! `permissions.client` and the workspace against `permissions.workspaces`; messages
//! and answers are recorded as the plugin's.

use std::collections::VecDeque;

use kumikit_kernel::{Kernel, ThreadSubscription};
use kumikit_protocol::event::QueueTrigger;
use kumikit_protocol::methods::ModelInfo;
use kumikit_protocol::methods::SteerStatus;
use kumikit_protocol::model::{
    ApprovalId, Decision, PermissionMode, QueueEntryId, ThreadId, UserInput, WorkspaceId,
};
use kumikit_protocol::{Topic, WorkspaceEvent};
use tokio::sync::broadcast;
use wasmtime::component::Resource;
use wasmtime_wasi::p2::DynPollable;

use super::bindings::base::kumikit::host::client::{self, Event};
use super::bindings::base::kumikit::host::types::{Error, ErrorCode};
use super::bindings::threads_client::kumikit::threads::client as wit_threads_client;
use super::bindings::threads_client_v2::kumikit::threads::client as wit_threads_client_v2;
use super::bindings::turn_client::kumikit::turn::client as wit_turn_client;
use super::host::{HostState, err, kernel_err};

/// Most events a subscription buffers before the guest reads them.
const BUFFER: usize = 1024;

enum Source {
    Thread(ThreadSubscription),
    Workspace(WorkspaceId, broadcast::Receiver<WorkspaceEvent>),
}

/// The `subscription` resource.
pub struct SubscriptionRes {
    topic: String,
    /// The workspace the topic belongs to, re-checked on every read.
    workspace: WorkspaceId,
    source: Source,
    buffer: VecDeque<Event>,
    ended: bool,
    seq: u64,
}

impl std::fmt::Debug for SubscriptionRes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionRes")
            .field("topic", &self.topic)
            .field("buffered", &self.buffer.len())
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}

fn event(topic: &str, seq: u64, time: String, value: serde_json::Value) -> Event {
    Event {
        topic: topic.to_owned(),
        seq,
        time,
        kind: value
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or_default()
            .to_owned(),
        data: value
            .get("data")
            .map_or_else(|| "null".to_owned(), |d| d.to_string()),
    }
}

#[async_trait::async_trait]
impl wasmtime_wasi::p2::Pollable for SubscriptionRes {
    async fn ready(&mut self) {
        if !self.buffer.is_empty() || self.ended {
            return;
        }
        match &mut self.source {
            Source::Thread(sub) => match sub.next_batch().await {
                Ok(Some(batch)) => {
                    for e in batch {
                        let value = serde_json::to_value(&e.event).unwrap_or_default();
                        self.buffer
                            .push_back(event(&self.topic, e.seq, e.time.0, value));
                    }
                }
                _ => self.ended = true,
            },
            Source::Workspace(ws, rx) => loop {
                match rx.recv().await {
                    Ok(change) => {
                        let thread = match &change {
                            WorkspaceEvent::ThreadCreated(c)
                            | WorkspaceEvent::ThreadUpdated(c)
                            | WorkspaceEvent::ThreadArchived(c)
                            | WorkspaceEvent::ThreadDeleted(c) => &c.thread,
                        };
                        if &thread.workspace != ws {
                            continue;
                        }
                        let time = thread.updated.0.clone();
                        self.seq += 1;
                        let value = serde_json::to_value(&change).unwrap_or_default();
                        self.buffer
                            .push_back(event(&self.topic, self.seq, time, value));
                        break;
                    }
                    // A lagging reader misses changes; the guest re-reads the list.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        self.ended = true;
                        break;
                    }
                }
            },
        }
        if self.buffer.len() > BUFFER {
            self.buffer.drain(..self.buffer.len() - BUFFER);
        }
    }
}

/// What a client call is checked against, cloned out of the store so no borrow of it
/// is held across an await.
#[derive(Clone)]
struct Access {
    kernel: Kernel,
    /// The plugin whose code runs (for logs).
    plugin: String,
    /// The plugin whose grants apply, recorded as the one acting.
    actor: String,
    grants: kumikit_kernel::grants::Grants,
    /// Workspace of the calling thread during a gateway `tool.call`, if any.
    call_workspace: Option<WorkspaceId>,
    /// The calling thread during a gateway `tool.call`, if any.
    call_thread: Option<ThreadId>,
}

impl Access {
    fn deny(&self, op: &str, reason: String) -> Error {
        tracing::warn!(plugin = %self.plugin, grants = %self.actor, op, %reason, "permission denied");
        err(ErrorCode::PermissionDenied, reason)
    }

    fn need(&self, op: &str) -> Result<(), Error> {
        if self.grants.may_client(op) {
            Ok(())
        } else {
            Err(self.deny(
                &format!("client.{op}"),
                format!("`{op}` is not in permissions.client"),
            ))
        }
    }

    async fn may_act_in(&self, ws: &WorkspaceId) -> Result<bool, Error> {
        // Tool calls with `workspaces = "linked"` may act in their calling thread's
        // workspace automatically (ADR 0030) while staying isolated from other workspaces
        // unless linked or granted `all`.
        if self.grants.workspaces == kumikit_kernel::grants::WorkspacesGrant::Linked
            && self.call_workspace.as_ref() == Some(ws)
        {
            return Ok(true);
        }
        self.kernel
            .plugin_may_act_in(&self.actor, &self.grants, ws)
            .await
            .map_err(kernel_err)
    }

    async fn need_workspace(&self, op: &str, ws: &WorkspaceId) -> Result<(), Error> {
        if self.may_act_in(ws).await? {
            Ok(())
        } else {
            Err(self.deny(
                &format!("client.{op}"),
                format!("workspace {ws} is not one this plugin may act in"),
            ))
        }
    }

    async fn need_thread(&self, op: &str, thread: &ThreadId) -> Result<WorkspaceId, Error> {
        let ws = self
            .kernel
            .thread_workspace(thread)
            .await
            .map_err(kernel_err)?;
        self.need_workspace(op, &ws).await?;
        Ok(ws)
    }
}

impl HostState {
    fn access(&self) -> Access {
        Access {
            kernel: self.kernel.clone(),
            plugin: self.id.plugin.clone(),
            actor: self.id.grants_from.clone(),
            grants: self.id.grants.clone(),
            call_workspace: self.call_workspace.clone(),
            call_thread: self.call_thread.clone(),
        }
    }
}

fn thread_record(t: kumikit_protocol::model::Thread) -> client::Thread {
    let status = serde_json::to_value(t.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    client::Thread {
        id: t.id.0,
        workspace: t.workspace.0,
        title: t.title,
        provider: t.provider,
        model: t.model,
        status,
    }
}

impl client::Host for HostState {
    async fn list_workspaces(&mut self) -> Result<Vec<client::Workspace>, Error> {
        let a = self.access();
        a.need("workspaces.list")?;
        let mut out = Vec::new();
        for w in a.kernel.workspace_list().await.map_err(kernel_err)? {
            if a.may_act_in(&w.id).await? {
                out.push(client::Workspace {
                    id: w.id.0,
                    name: w.name,
                    root: w.root,
                });
            }
        }
        Ok(out)
    }

    async fn list_threads(&mut self, workspace: String) -> Result<Vec<client::Thread>, Error> {
        let a = self.access();
        a.need("threads.read")?;
        let ws = WorkspaceId(workspace);
        a.need_workspace("threads.read", &ws).await?;
        let threads = a.kernel.thread_list(&ws, false).await.map_err(kernel_err)?;
        Ok(threads.into_iter().map(thread_record).collect())
    }

    async fn get_thread(&mut self, thread: String) -> Result<String, Error> {
        let a = self.access();
        a.need("threads.read")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.read", &thread).await?;
        let snapshot = a.kernel.thread_get(&thread).await.map_err(kernel_err)?;
        serde_json::to_string(&snapshot).map_err(|e| err(ErrorCode::Internal, e.to_string()))
    }

    async fn list_providers(&mut self) -> Result<Vec<client::Provider>, Error> {
        let a = self.access();
        a.need("threads.create")?;
        Ok(a.kernel
            .provider_list()
            .into_iter()
            .map(|p| client::Provider {
                id: p.id,
                name: p.name,
            })
            .collect())
    }

    async fn create_thread(
        &mut self,
        workspace: String,
        provider: Option<String>,
        model: Option<String>,
        title: Option<String>,
    ) -> Result<client::Thread, Error> {
        let a = self.access();
        a.need("threads.create")?;
        let ws = WorkspaceId(workspace);
        a.need_workspace("threads.create", &ws).await?;
        let t = a
            .kernel
            .thread_create(&ws, provider, model, title, None)
            .await
            .map_err(kernel_err)?;
        tracing::info!(plugin = %a.actor.as_str(), thread = %t.id, "plugin created a thread");
        Ok(thread_record(t))
    }

    async fn send(&mut self, thread: String, text: String) -> Result<String, Error> {
        let a = self.access();
        a.need("threads.send")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.send", &thread).await?;
        let sent = a
            .kernel
            .thread_send_from(
                &thread,
                UserInput::text(text),
                Some(a.actor.as_str().to_owned()),
            )
            .await
            .map_err(kernel_err)?;
        Ok(sent.turn.0)
    }

    async fn cancel(&mut self, thread: String) -> Result<(), Error> {
        let a = self.access();
        a.need("threads.cancel")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.cancel", &thread).await?;
        a.kernel.thread_cancel(&thread).await.map_err(kernel_err)
    }

    async fn respond_approval(
        &mut self,
        approval: String,
        decision: client::Decision,
    ) -> Result<(), Error> {
        let a = self.access();
        a.need("approvals.respond")?;
        let approval = ApprovalId(approval);
        let thread = a
            .kernel
            .approval_thread(&approval)
            .await
            .map_err(kernel_err)?;
        a.need_thread("approvals.respond", &thread).await?;
        let decision = match decision {
            client::Decision::Allow => Decision::Allow,
            client::Decision::Deny => Decision::Deny,
        };
        let actor = a.actor.clone();
        a.kernel
            .respond_approval_by_plugin(&approval, decision, &actor)
            .await
            .map_err(kernel_err)
    }

    async fn subscribe(
        &mut self,
        topic: String,
        after: u64,
    ) -> Result<Resource<SubscriptionRes>, Error> {
        let a = self.access();
        a.need("threads.read")?;
        let parsed = Topic::parse(&topic)
            .ok_or_else(|| err(ErrorCode::InvalidArgument, format!("unknown topic {topic}")))?;
        let (workspace, source) = match parsed {
            Topic::Thread(t) => {
                let ws = a.need_thread("threads.read", &t).await?;
                let sub = a.kernel.subscribe(&t, after).await.map_err(kernel_err)?;
                (ws, Source::Thread(sub))
            }
            Topic::Workspace(w) => {
                // Listen first, so nothing after the check is missed.
                let rx = a.kernel.workspace_changes();
                a.need_workspace("threads.read", &w).await?;
                a.kernel.workspace_require(&w).await.map_err(kernel_err)?;
                (w.clone(), Source::Workspace(w, rx))
            }
            Topic::Reserved(name) => {
                return Err(err(
                    ErrorCode::InvalidArgument,
                    format!("topic {name} is reserved"),
                ));
            }
            // Usage data is for clients; no plugin permission covers it.
            Topic::Usage => {
                return Err(err(
                    ErrorCode::InvalidArgument,
                    "the usage topic is not available to plugins",
                ));
            }
        };
        let res = SubscriptionRes {
            topic,
            workspace,
            source,
            buffer: VecDeque::new(),
            ended: false,
            seq: 0,
        };
        self.table
            .push(res)
            .map_err(|e| err(ErrorCode::Limit, e.to_string()))
    }
}

impl wit_turn_client::Host for HostState {
    async fn steer(
        &mut self,
        thread: String,
        text: String,
    ) -> Result<wit_turn_client::SteerResult, Error> {
        let a = self.access();
        a.need("threads.steer")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.steer", &thread).await?;
        let r = a
            .kernel
            .thread_steer_from(
                &thread,
                UserInput::text(text),
                Some(a.actor.as_str().to_owned()),
            )
            .await
            .map_err(kernel_err)?;
        let status = match r.status {
            SteerStatus::Accepted => wit_turn_client::SteerStatus::Accepted,
            SteerStatus::Queued => wit_turn_client::SteerStatus::Queued,
        };
        Ok(wit_turn_client::SteerResult {
            status,
            turn: r.turn.0,
            entry: r.entry.map(|e| e.0),
        })
    }
}

impl wit_threads_client::Host for HostState {
    async fn defer(
        &mut self,
        thread: String,
        text: String,
        trigger_json: String,
    ) -> Result<String, Error> {
        let a = self.access();
        a.need("threads.defer")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.defer", &thread).await?;
        let trigger: QueueTrigger = serde_json::from_str(&trigger_json).map_err(|e| {
            err(
                ErrorCode::InvalidArgument,
                format!("invalid trigger JSON: {e}"),
            )
        })?;
        let entry = a
            .kernel
            .thread_defer_from(&thread, UserInput::text(text), trigger, a.actor, a.grants)
            .await
            .map_err(kernel_err)?;
        serde_json::to_string(&entry).map_err(|e| err(ErrorCode::Internal, e.to_string()))
    }

    async fn list_queue(&mut self, thread: String) -> Result<String, Error> {
        let a = self.access();
        a.need("threads.queue")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.queue", &thread).await?;
        let snap = a.kernel.thread_get(&thread).await.map_err(kernel_err)?;
        serde_json::to_string(&snap.queue).map_err(|e| err(ErrorCode::Internal, e.to_string()))
    }

    async fn remove_queued(&mut self, thread: String, entry: String) -> Result<(), Error> {
        let a = self.access();
        a.need("threads.queue")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.queue", &thread).await?;
        a.kernel
            .queue_remove(&thread, &QueueEntryId(entry))
            .await
            .map_err(kernel_err)
    }

    async fn read_events(
        &mut self,
        thread: String,
        after: u64,
        limit: u32,
    ) -> Result<String, Error> {
        let a = self.access();
        a.need("threads.read")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.read", &thread).await?;
        let events = a
            .kernel
            .thread_read_events(&thread, after, limit as usize)
            .await
            .map_err(kernel_err)?;
        let out: Vec<serde_json::Value> = events
            .into_iter()
            .map(|e| {
                let mut v = serde_json::to_value(&e.event).unwrap_or_default();
                let kind = v
                    .get("kind")
                    .and_then(|k| k.as_str())
                    .unwrap_or_default()
                    .to_owned();
                let data = v
                    .get_mut("data")
                    .map(serde_json::Value::take)
                    .unwrap_or(serde_json::Value::Null);
                serde_json::json!({
                    "seq": e.seq,
                    "time": e.time.0,
                    "kind": kind,
                    "data": data,
                })
            })
            .collect();
        serde_json::to_string(&out).map_err(|e| err(ErrorCode::Internal, e.to_string()))
    }

    async fn suggest_title(&mut self, thread: String, title: String) -> Result<bool, Error> {
        let a = self.access();
        a.need("threads.title.suggest")?;
        let thread = ThreadId(thread);
        a.need_thread("threads.title.suggest", &thread).await?;
        a.kernel
            .thread_suggest_title(&thread, &title)
            .await
            .map_err(kernel_err)
    }
}

/// How strict a permission mode is, lower being stricter (ADR 0034).
fn strictness(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Plan => 0,
        PermissionMode::Standard => 1,
        PermissionMode::AcceptEdits => 2,
        PermissionMode::FullAccess => 3,
    }
}

/// The origin of a thread a plugin creates (ADR 0036). During a tool call the origin is
/// always the calling thread, so a plugin can't pick another thread's mode to be bounded
/// by, or leave the bound out by naming no origin (ADR 0034). Outside a tool call the
/// origin is what the request names.
fn resolve_origin(
    call_thread: Option<&ThreadId>,
    requested: Option<ThreadId>,
) -> Result<Option<ThreadId>, String> {
    match (call_thread, requested) {
        (Some(calling), None) => Ok(Some(calling.clone())),
        (Some(calling), Some(named)) if &named != calling => Err(format!(
            "origin {named} is not the thread that is calling ({calling})"
        )),
        (_, requested) => Ok(requested),
    }
}

/// The permission mode a plugin-created thread gets (ADR 0034): never `full_access`,
/// never looser than `origin`'s mode; with none requested, `standard` or the origin's
/// mode if that is stricter. `Err` carries the reason for `permission-denied`.
fn bounded_mode(
    requested: Option<PermissionMode>,
    origin: Option<PermissionMode>,
) -> Result<PermissionMode, String> {
    if requested == Some(PermissionMode::FullAccess) {
        return Err("a plugin cannot create a full_access thread".into());
    }
    match (requested, origin) {
        (Some(r), Some(o)) if strictness(r) > strictness(o) => Err(format!(
            "permission mode {} is looser than the origin thread's {}",
            r.as_str(),
            o.as_str()
        )),
        (Some(r), _) => Ok(r),
        (None, Some(o)) if strictness(o) < strictness(PermissionMode::Standard) => Ok(o),
        (None, _) => Ok(PermissionMode::Standard),
    }
}

impl wit_threads_client_v2::Host for HostState {
    async fn defer(
        &mut self,
        thread: String,
        text: String,
        trigger_json: String,
    ) -> Result<String, Error> {
        wit_threads_client::Host::defer(self, thread, text, trigger_json).await
    }

    async fn list_queue(&mut self, thread: String) -> Result<String, Error> {
        wit_threads_client::Host::list_queue(self, thread).await
    }

    async fn remove_queued(&mut self, thread: String, entry: String) -> Result<(), Error> {
        wit_threads_client::Host::remove_queued(self, thread, entry).await
    }

    async fn read_events(
        &mut self,
        thread: String,
        after: u64,
        limit: u32,
    ) -> Result<String, Error> {
        wit_threads_client::Host::read_events(self, thread, after, limit).await
    }

    async fn suggest_title(&mut self, thread: String, title: String) -> Result<bool, Error> {
        wit_threads_client::Host::suggest_title(self, thread, title).await
    }

    async fn list_models(&mut self, provider: String) -> Result<String, Error> {
        let a = self.access();
        a.need("threads.create")?;
        let models: Vec<ModelInfo> = a
            .kernel
            .provider_models(&provider)
            .await
            .map_err(kernel_err)?
            .into_iter()
            .map(|m| ModelInfo {
                id: m.id,
                label: m.label,
                capabilities: m.capabilities,
                options: m.options,
            })
            .collect();
        serde_json::to_string(&models).map_err(|e| err(ErrorCode::Internal, e.to_string()))
    }

    async fn create_thread(
        &mut self,
        request: wit_threads_client_v2::CreateRequest,
    ) -> Result<client::Thread, Error> {
        let a = self.access();
        a.need("threads.create")?;
        let ws = WorkspaceId(request.workspace);
        a.need_workspace("threads.create", &ws).await?;
        let requested = request
            .permission_mode
            .as_deref()
            .map(|m| {
                PermissionMode::parse(m).ok_or_else(|| {
                    err(
                        ErrorCode::InvalidArgument,
                        format!("unknown permission mode {m}"),
                    )
                })
            })
            .transpose()?;
        let origin_id = resolve_origin(a.call_thread.as_ref(), request.origin.map(ThreadId))
            .map_err(|reason| a.deny("client.threads.create", reason))?;
        let origin = match &origin_id {
            Some(id) => {
                let origin_ws = a.need_thread("threads.create", id).await?;
                if origin_ws != ws {
                    return Err(a.deny(
                        "client.threads.create",
                        format!("origin thread {id} is not in workspace {ws}"),
                    ));
                }
                let snap = a.kernel.thread_get(id).await.map_err(kernel_err)?;
                Some(snap.thread.permission_mode.unwrap_or_default())
            }
            None => None,
        };
        let mode = bounded_mode(requested, origin)
            .map_err(|reason| a.deny("client.threads.create", reason))?;
        let options = (!request.options.is_empty()).then(|| {
            request
                .options
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>()
        });
        // A thread started for an origin is a side thread of it (ADR 0036). The kernel
        // then inherits what the request leaves out from the origin, so the mode is always
        // passed: inheriting it could make the thread looser than the mode checked above.
        let (parent, side) = match &origin_id {
            Some(id) => (Some(id.clone()), true),
            None => (None, false),
        };
        let permission_mode =
            (origin_id.is_some() || mode != PermissionMode::Standard).then_some(mode);
        let t = a
            .kernel
            .thread_create_opts(
                &ws,
                request.provider,
                request.model,
                request.title,
                permission_mode,
                options,
                None,
                parent,
                side,
            )
            .await
            .map_err(kernel_err)?;
        tracing::info!(plugin = %a.actor.as_str(), thread = %t.id, mode = mode.as_str(), "plugin created a thread");
        Ok(thread_record(t))
    }
}

impl client::HostSubscription for HostState {
    async fn subscribe(
        &mut self,
        this: Resource<SubscriptionRes>,
    ) -> wasmtime::Result<Resource<DynPollable>> {
        wasmtime_wasi::p2::subscribe(&mut self.table, this)
    }

    async fn next(
        &mut self,
        this: Resource<SubscriptionRes>,
        max: u32,
    ) -> Result<Vec<Event>, Error> {
        let ws = {
            let s = self
                .table
                .get(&this)
                .map_err(|e| err(ErrorCode::NotFound, e.to_string()))?;
            s.workspace.clone()
        };
        if !self.access().may_act_in(&ws).await? {
            let s = self
                .table
                .get_mut(&this)
                .map_err(|e| err(ErrorCode::NotFound, e.to_string()))?;
            s.ended = true;
            s.buffer.clear();
        }
        let s = self
            .table
            .get_mut(&this)
            .map_err(|e| err(ErrorCode::NotFound, e.to_string()))?;
        if s.buffer.is_empty() && s.ended {
            return Err(err(
                ErrorCode::Conflict,
                format!("the subscription to {} ended", s.topic),
            ));
        }
        let n = (max as usize).min(s.buffer.len());
        Ok(s.buffer.drain(..n).collect())
    }

    async fn drop(&mut self, this: Resource<SubscriptionRes>) -> wasmtime::Result<()> {
        self.table.delete(this)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PermissionMode::{AcceptEdits, FullAccess, Plan, Standard};

    #[test]
    fn full_access_is_never_granted() {
        assert!(bounded_mode(Some(FullAccess), None).is_err());
        assert!(bounded_mode(Some(FullAccess), Some(FullAccess)).is_err());
    }

    #[test]
    fn during_a_tool_call_the_origin_is_the_calling_thread() {
        let calling = ThreadId("calling".into());
        let other = ThreadId("other".into());
        // No origin named: the calling thread bounds the new one (ADR 0034). Without this a
        // tool could leave the bound out and get a looser mode.
        assert_eq!(
            resolve_origin(Some(&calling), None),
            Ok(Some(calling.clone()))
        );
        assert_eq!(
            resolve_origin(Some(&calling), Some(calling.clone())),
            Ok(Some(calling.clone()))
        );
        // Another thread named as origin is refused, so the bound can't come from a
        // thread the tool picks.
        assert!(resolve_origin(Some(&calling), Some(other)).is_err());
    }

    #[test]
    fn outside_a_tool_call_the_origin_is_what_the_request_names() {
        let named = ThreadId("named".into());
        assert_eq!(resolve_origin(None, None), Ok(None));
        assert_eq!(resolve_origin(None, Some(named.clone())), Ok(Some(named)));
    }

    #[test]
    fn a_mode_looser_than_the_origin_is_refused() {
        assert!(bounded_mode(Some(Standard), Some(Plan)).is_err());
        assert!(bounded_mode(Some(AcceptEdits), Some(Standard)).is_err());
        assert_eq!(bounded_mode(Some(Plan), Some(Standard)), Ok(Plan));
        assert_eq!(
            bounded_mode(Some(AcceptEdits), Some(FullAccess)),
            Ok(AcceptEdits)
        );
    }

    #[test]
    fn the_default_is_standard_or_a_stricter_origin() {
        assert_eq!(bounded_mode(None, None), Ok(Standard));
        assert_eq!(bounded_mode(None, Some(Plan)), Ok(Plan));
        assert_eq!(bounded_mode(None, Some(AcceptEdits)), Ok(Standard));
        assert_eq!(bounded_mode(None, Some(FullAccess)), Ok(Standard));
        assert_eq!(bounded_mode(Some(AcceptEdits), None), Ok(AcceptEdits));
    }
}
