//! `kumikit:host/client`: the protocol surface for plugins that act like clients
//! (`docs/spec/03-kernel.md#client-interface`). Every call checks the operation against
//! `permissions.client` and the workspace against `permissions.workspaces`; messages
//! and answers are recorded as the plugin's.

use std::collections::VecDeque;

use kumikit_kernel::{Kernel, ThreadSubscription};
use kumikit_protocol::model::{ApprovalId, Decision, ThreadId, UserInput, WorkspaceId};
use kumikit_protocol::{Topic, WorkspaceEvent};
use tokio::sync::broadcast;
use wasmtime::component::Resource;
use wasmtime_wasi::p2::DynPollable;

use super::bindings::base::kumikit::host::client::{self, Event};
use super::bindings::base::kumikit::host::types::{Error, ErrorCode};
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
