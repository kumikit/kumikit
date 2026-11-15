# 03 — Kernel

The kernel is the part of the core that is not a plugin. It stays small: anything that can
be a plugin is one.

## Kernel services

| Service | Responsibility |
|---|---|
| Protocol server | Transports, authentication, JSON-RPC routing, subscriptions |
| Domain model | Workspaces, threads, turns, events, approvals |
| Event log | Append-only per-thread log; replay from a cursor |
| Storage | SQLite database; per-plugin key-value store |
| Secret store | Secrets for the core and its plugins; never returned to clients |
| Process supervisor | Spawns, tracks and kills child processes on behalf of plugins |
| Workspace files | Scoped file access for plugins and clients |
| Permission broker | Checks every host call against the plugin's granted permissions |
| Plugin host | Installs, verifies, activates and unloads plugins ([04](04-plugin-host.md)) |
| Tool gateway | Exposes plugin-contributed tools to agents over MCP |

## Domain model

```
Workspace { id, name, root: path }
Thread    { id, workspace, title, provider, model, status, created, updated }
Turn      { id, thread, started, ended?, outcome? }
Event     { thread, seq, time, kind, data }
Approval  { id, thread, turn, kind, summary, detail, status }
```

- IDs are ULIDs.
- `status` of a thread: `idle`, `running`, `awaiting_approval`, `failed`, `archived`.
- The domain model is the shared language of providers and clients. It is the reason the
  kernel, not a plugin, owns threads ([ADR 0006](../decisions/0006-kernel-owns-domain-model.md)).

### Thread events

| Kind | Data | Emitted by |
|---|---|---|
| `turn.started` | `turn`, `input` (user message, attachments) | kernel |
| `text.delta` | `turn`, `text` | provider |
| `text.done` | `turn`, `text` (full message) | provider |
| `reasoning.delta` | `turn`, `text` | provider |
| `tool.started` | `turn`, `call`, `name`, `input` | provider |
| `tool.output` | `turn`, `call`, `chunk` | provider |
| `tool.finished` | `turn`, `call`, `ok`, `result?` | provider |
| `file.changed` | `turn`, `path`, `diff?` | provider |
| `approval.requested` | `approval` | provider |
| `approval.resolved` | `approval`, `decision`, `by` (device) | kernel |
| `usage` | `turn`, `input_tokens`, `output_tokens`, `cost?` | provider |
| `turn.ended` | `turn`, `outcome` (`done`, `cancelled`, `failed`), `error?` | kernel |
| `plugin.event` | `plugin`, `kind`, `data` | any plugin, namespaced |

Providers emit through the host; the kernel validates every event (a provider cannot emit
`turn.ended` or events for another thread) and assigns `seq` and `time`.

`plugin.event` lets a plugin add its own event kinds without a protocol change. Clients
that do not understand one skip it; a UI plugin can register a renderer for it.

## Storage

- One SQLite database per core, WAL mode. Schema migrations are numbered, forward-only,
  and run at startup; the schema version lives in `PRAGMA user_version`.
- Core tables (workspaces, threads, events, approvals, devices, plugins) belong to the
  kernel. Plugins never get SQL access to them.
- Each plugin gets a namespaced key-value store (`kv.get/set/delete/list` with prefix
  scans, values up to 1 MB) and a private data directory. A plugin's data is deleted when
  it is uninstalled, after confirmation.
- Events are stored as JSON. Old threads can be compacted (text deltas folded into
  `text.done`) without changing what a replay shows.

## Secrets

- The secret store holds provider keys, bot tokens and device tokens of paired clients.
- Backend: the OS keychain where it works non-interactively, otherwise a 0600 file in the
  core data directory. The choice is a setting, not a code path in plugins.
- A plugin reads only the secret names its manifest declares. Clients can set and delete
  secrets but never read them; the UI shows "set" or "not set".

## Processes

- Plugins spawn processes through the host (`process.spawn`), never directly.
- The command must be in the plugin's `spawn` permission list; the host resolves it on
  `PATH` and records the resolved path.
- The working directory must be inside a workspace the plugin is acting for.
- The environment is built from an allowlist plus variables the plugin sets explicitly;
  secrets are passed only when the plugin asks for them by name.
- The supervisor owns process lifetimes: when a turn is cancelled, a plugin is unloaded or
  the core shuts down, its processes are terminated (process group / job object).

## Tool gateway

Plugins can contribute tools (`[contributes] tools`). The kernel serves them to agents
through a local MCP endpoint, so a provider can hand its agent CLI one MCP config that
reaches every enabled tool. Tool calls are checked against the calling thread's workspace
and logged as `tool.*` events.
