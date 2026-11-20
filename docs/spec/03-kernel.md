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
| `turn.started` | `turn`, `input` (`{ text }`; attachments are a later optional field), `from?` (`{ plugin }` when a plugin sent the message through the client interface) | kernel |
| `text.delta` | `turn`, `text` | provider |
| `text.done` | `turn`, `text` (full message) | provider |
| `reasoning.delta` | `turn`, `text` | provider |
| `tool.started` | `turn`, `call`, `name`, `input`, `source?` (`{ plugin, tool }` for a call through the tool gateway) | provider, or the kernel for gateway calls |
| `tool.output` | `turn`, `call`, `chunk` | provider, or the kernel |
| `tool.finished` | `turn`, `call`, `ok`, `result?` | provider, or the kernel |
| `file.changed` | `turn`, `path`, `diff?` | provider |
| `approval.requested` | `approval` | provider, or the kernel for gateway calls (kind `tool`) |
| `approval.resolved` | `approval`, `decision` (`allow`, `allow_always`, `deny`, `cancelled`), `by?` (the paired device that answered; absent for local and SSH connections and for cancellations), `plugin?` (the plugin that answered through the client interface) | kernel |
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

## Plugin settings

- A plugin with `[contributes] settings` has a JSON Schema; clients read it with
  `settings.schema` and change values with `settings.set`, which the kernel validates. The
  values are stored by the kernel per plugin and read by the plugin with
  `kumikit:host/settings.get` (its own only).
- A schema property with `"x-secret": "<name>"` is a secret field: it is set with
  `secret.set` under that name, never stored with the settings, and the name must be in
  the plugin's own `permissions.secrets` (checked at install).

## Secrets

- The secret store holds provider keys and bot tokens: values the core must read back.
  Device tokens are not secrets the core keeps; only their hashes are stored (Storage).
- Backend: the OS keychain where it works non-interactively, otherwise a 0600 file
  (`secrets.json`) in the core data directory. The choice is a setting, not a code path in
  plugins. Only the file backend exists so far. Clients set and delete secrets with
  `secret.*`.
- The core's log redacts the value of every stored secret (at least 8 characters long)
  wherever it appears in a message.
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

## Client interface

A plugin granted `permissions.client` operations can act like a client through
`kumikit:host/client` (bots, scheduled prompts): list workspaces, read, create and send to
threads, cancel turns and answer approvals. The kernel checks each call against the
operations granted and against `permissions.workspaces`: `none`, `linked` (only the
workspaces the user linked to the plugin with `plugin.setWorkspaces`) or `all`. Messages a
plugin sends carry `from { plugin }` in `turn.started`; approvals it answers carry
`plugin` in `approval.resolved`; it cannot answer `allow_always`. Subscriptions are
resources with a `wasi:io` pollable, so a plugin can wait on several things at once.

## Schedules

Decided in [ADR 0024](../decisions/0024-schedules.md).

- A schedule belongs to a plugin and has a key, a cron expression (five fields: minute,
  hour, day of month, month, day of week, with lists, ranges and steps; or `@hourly`,
  `@daily`, `@weekly`, `@monthly`), a time zone (the core's local zone by default, or an
  IANA name), a description, a missed-run policy (`skip` or `once`) and an enabled flag.
  Manifest triggers `schedule:<cron>` are schedules owned by the manifest (key
  `manifest:<n>`, the trigger's position); a plugin adds and removes its own with
  `kumikit:host/timers`. Both need `permissions.schedules`; a plugin has at most 64.
- Schedules are rows in the core database with their next run, so a restart needs no
  replay. One scheduler task sleeps until the earliest enabled next run and wakes on
  changes; with no enabled schedule it arms no timer.
- When one is due the kernel activates the plugin and calls `kumikit:plugin/timers.fire`.
  It does not fire while its previous call is still running (`skipped`). A time skipped
  by a DST change runs as if the clocks had not moved (02:30 becomes 03:30); a repeated
  hour runs once, at its first occurrence.
- Runs missed while the core was down, or by a wake more than 5 minutes late, follow the
  policy: `skip` records `missed`; `once` runs once, never more.
- `schedule.list`, `schedule.setEnabled`, `schedule.delete` and `schedule.run` show,
  pause, remove and run them now; `ui-settings` has a Schedules page for the active core.
  Disabling a plugin stops its schedules (its runs are not counted as missed until it is
  enabled again); uninstalling deletes them. Only `kumikit serve` runs the scheduler, not
  CLI commands that open the data directory.

## Tool gateway

Decided in [ADR 0025](../decisions/0025-tool-gateway.md).

- Plugins contribute tools in their manifest (`[[contributes.tools]]`: `name`, `title`,
  `description`, `input` schema file, `effect = "read" | "write"`, default `write`) and
  serve them with the `kumikit:tool/tool.call` export. Listing tools activates nothing.
  A tool's MCP name is `<plugin id with "." and "-" as "_">__<name>`, at most 64
  characters.
- A provider session gets an MCP server only if its plugin (the contributing plugin, for a
  delegated provider) is granted `tools`. The record names the core binary, the arguments
  `mcp --socket <socket>` and the variable `KUMIKIT_MCP_TOKEN`; the provider passes it to
  its agent. `kumikit mcp` connects to the local socket or pipe, sends `gateway.attach {
  token }` and then copies MCP messages both ways.
- The token is 32 random bytes, kept only in memory, bound to the session, and void when
  the session ends, the plugin is disabled or the core stops.
- The server speaks MCP `initialize` (protocol versions 2025-03-26, 2025-06-18 and
  2025-11-25), `ping`, `tools/list`, `tools/call`, `notifications/cancelled` and sends
  `notifications/tools/list_changed`. It offers every tool of every enabled plugin.
- A call is refused unless the session's thread has a running turn. The kernel validates
  the input against the schema, appends `tool.started` with `source`, runs the tool in
  the tool plugin's control instance acting for the thread's workspace (one call of a
  plugin at a time), and appends `tool.output` and `tool.finished`. A `write` tool first needs an approval (kind `tool`)
  in the running turn; `allow_always` covers that tool in that thread.
- Limits: input and result at most 1 MiB each (a longer result is cut and says so), 4
  calls in flight per session, the tool plugin's call limit and 5 minutes of wall time per
  call. Cancelling the turn cancels its calls.
