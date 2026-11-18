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
| Plugin host | Installs, verifies, activates and unloads plugins; a separate crate above the kernel ([04](04-plugin-host.md)) |
| Tool gateway | Exposes plugin-contributed tools to agents over MCP |

## Domain model

```
Workspace { id, name, root: path }
Thread    { id, workspace, title, provider, model, status, created, updated }
Turn      { id, thread, started, ended?, outcome? }
Event     { thread, seq, time, kind, data }
Approval  { id, thread, turn, kind, summary, detail, status }
```

- IDs are ULIDs, assigned by the kernel (including approval ids).
- `status` of a thread: `idle`, `running`, `awaiting_approval`, `failed`, `archived`.
  It is derived from the log (`turn.started` → `running`, `approval.requested` →
  `awaiting_approval`, `approval.resolved` → `running`, `turn.ended` → `idle` or `failed`),
  except `archived`, which `thread.archive` sets.
- `status` of an approval: `pending`, `allowed`, `denied`, `cancelled`. A decision is
  `allow`, `allow_always` or `deny`.
- `Turn.outcome`: `done`, `cancelled`, `failed`.
- The domain model is the shared language of providers and clients. It is the reason the
  kernel, not a plugin, owns threads ([ADR 0006](../decisions/0006-kernel-owns-domain-model.md)).

### Thread events

| Kind | Data | Emitted by |
|---|---|---|
| `turn.started` | `turn`, `input` (`{ text }`; attachments are a later optional field) | kernel |
| `text.delta` | `turn`, `text` | provider |
| `text.done` | `turn`, `text` (full message) | provider |
| `reasoning.delta` | `turn`, `text` | provider |
| `tool.started` | `turn`, `call`, `name`, `input` | provider |
| `tool.output` | `turn`, `call`, `chunk` | provider |
| `tool.finished` | `turn`, `call`, `ok`, `result?` | provider |
| `file.changed` | `turn`, `path`, `diff?` | provider |
| `approval.requested` | `approval` | provider |
| `approval.resolved` | `approval`, `decision` (`allow`, `allow_always`, `deny`, `cancelled`), `by?` (the paired device that answered; absent for local and SSH connections and for cancellations) | kernel |
| `usage` | `turn`, `inputTokens`, `outputTokens`, `cost?` | provider |
| `turn.ended` | `turn`, `outcome` (`done`, `cancelled`, `failed`), `error?` | kernel |
| `plugin.event` | `plugin`, `kind`, `data` | any plugin, namespaced |

Providers emit through the host; the kernel validates every event (a provider cannot emit
`turn.started`, `turn.ended` or `approval.resolved`, events for another thread or turn,
or anything after its turn ended) and assigns `seq` and `time`. A provider asks for an
approval and gets its id from the kernel; the answer can reach the provider before that
call returns, so providers keep early answers.

Turn rules:
- One turn runs per thread. Messages sent during a turn wait in an in-memory queue (lost
  if the core stops) and start in order when the turn ends.
- Cancelling a turn first resolves its pending approvals with `decision: "cancelled"`,
  then emits `turn.ended { outcome: "cancelled" }`, and drops the queue.
- On startup the kernel ends every turn left open by a previous run: pending approvals
  are resolved as `cancelled`, then `turn.ended { outcome: "failed", error:
  "core_restarted" }` is appended.

`plugin.event` lets a plugin add its own event kinds without a protocol change. Clients
that do not understand one skip it; a UI plugin can register a renderer for it.

## Storage

- One SQLite database per core, WAL mode. Schema migrations are numbered, forward-only,
  and run at startup; the schema version lives in `PRAGMA user_version`. The core refuses
  to open a database with a newer version than it knows.
- One writer thread owns the only write connection. It assigns `seq`, and it commits
  queued appends together in one transaction (group commit). `synchronous=FULL`: an event
  is on disk before any client sees it, so a power loss can never take back a `seq` a
  client has already read. Reads use separate read-only connections.
- Derived columns (thread `status`, `updated`, turn `ended`/`outcome`, approval `status`)
  are written in the same transaction as the event that changes them, so they cannot
  disagree with the log.
- Core tables (workspaces, threads, events, approvals, devices, plugins) belong to the
  kernel. Plugins never get SQL access to them.
- The `devices` table records paired devices ([02](02-protocol.md#pairing)): id (ULID),
  name, the SHA-256 of the device token, created and last-seen times (last seen is
  written at most once a minute per device). The token itself is never stored. Revoking
  a device deletes its row and tells the core, which closes the device's connections.
- Each plugin gets a namespaced key-value store (`kv.get/set/delete/list` with prefix
  scans, keys up to 1 KB, values up to 1 MB), kept in the core database, and a private
  data directory `plugin-data/<id>/`. A plugin's data is deleted when it is uninstalled,
  after confirmation.
- The `plugins` table records each installed plugin: id, version, state (`installed`,
  `enabled`, `disabled`), the package's SHA-256, the granted permissions, the pinned
  publisher key (when the package was signed) and its source (a registry URL, `file` or
  `bundled`). Unpacked packages live in `plugins/<id>/<version>/` in the data directory.
  After every change the core rewrites `plugins.lock` from this table
  ([10](10-distribution.md#lockfiles)); the table stays the record.
- One core owns a data directory at a time: `kumikit serve`, and CLI commands that work
  on the data directory while no core runs, hold an exclusive lock on `core.lock`.
- Events are stored as JSON. Old threads can be compacted (text deltas folded into
  `text.done`) without changing what a replay shows.

## Secrets

- The secret store holds provider keys and bot tokens: values the core must read back.
  Device tokens are not secrets the core keeps; only their hashes are stored (Storage).
- Backend: the OS keychain where it works non-interactively, otherwise a 0600 file
  (`secrets.json`) in the core data directory. The choice is a setting, not a code path in
  plugins. Only the file backend exists so far; setting secrets over the protocol
  (`secret.*`) comes with the settings UI.
- A plugin reads only the secret names its manifest declares. Clients can set and delete
  secrets but never read them; the UI shows "set" or "not set".

## Processes

- Plugins spawn processes through the host (`process.spawn`), never directly.
- The program must be a bare name in the plugin's `spawn` permission list (a path such as
  `/tmp/x/claude` is refused even if `claude` is listed). The supervisor resolves it on
  the core's `PATH` (with `PATHEXT` on Windows), logs the resolved path and keeps it in its
  process table.
- The working directory must be inside a workspace the plugin is acting for (a session's
  thread workspace) and defaults to its root; paths are canonicalized before the check. A
  plugin acting for no workspace cannot spawn.
- The environment starts empty and gets, in order: an allowlist copied from the core
  (`PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `LANG`, `LC_*`, `TERM`, `TMPDIR`, `XDG_*`;
  on Windows `SystemRoot`, `windir`, `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`,
  `ProgramData`, `ProgramFiles*`, `PATHEXT`, `COMSPEC`, `TEMP`, `TMP`, `USERNAME`,
  `HOMEDRIVE`, `HOMEPATH`); variables the plugin sets explicitly (dynamic-loader variables
  `LD_*` and `DYLD_*` are refused); and secrets, only those the plugin names and is
  granted.
- The supervisor owns process lifetimes. On Linux and macOS each child starts in its own
  process group, which gets SIGTERM and, 2 s later, SIGKILL. On Windows each owner's
  children are in a job object that kills them when it is closed. Processes are
  terminated when a turn is cancelled (after the provider's grace period), a session ends,
  a plugin is unloaded or fails, or the core shuts down. A core killed with SIGKILL on
  Linux or macOS cannot clean up; its children keep running.
- The host reads children's stdout and stderr into bounded buffers, so a plugin that reads
  one stream cannot deadlock on the other.
- Every refused spawn is logged with the plugin id and the reason.

## Tool gateway

Plugins can contribute tools (`[contributes] tools`). The kernel serves them to agents
through a local MCP endpoint, so a provider can hand its agent CLI one MCP config that
reaches every enabled tool. Tool calls are checked against the calling thread's workspace
and logged as `tool.*` events.
