# 02 — Protocol

The protocol is the only contract between a client and a core. It is defined in Rust in
`kumikit-protocol`; TypeScript types in `@kumikit/protocol` are generated from it and never
edited by hand.

## Transports

| Transport | Used for | Authentication |
|---|---|---|
| Unix domain socket (`$XDG_RUNTIME_DIR/kumikit/core.sock`, mode 0600) | Local client on Linux/macOS | File permissions plus a peer-credential check (same user) |
| Named pipe (`\\.\pipe\kumikit-<user>`) | Local client on Windows | Pipe ACL restricted to the user |
| WebSocket over TLS (`wss://host:port/rpc`) | Remote clients | Device token from pairing (below) |
| WebSocket over an SSH tunnel | Remote clients without a public port | SSH, then a device token |

Every transport carries the same framing: one JSON-RPC 2.0 message per WebSocket text
frame, or newline-delimited JSON on sockets and pipes. A message is at most 8 MiB; a
longer line gets an `invalid_request` error and the connection is closed. JSON-RPC batches (arrays) are not supported and get a
single `-32600` error. A core listens only on the local socket unless the user enables
remote access.

### Local transport details

- **Socket path.** `$XDG_RUNTIME_DIR/kumikit/core.sock` when `XDG_RUNTIME_DIR` is set.
  Otherwise `$TMPDIR/kumikit-<uid>/core.sock` on macOS and `/tmp/kumikit-<uid>/core.sock`
  on Linux. The directory is created with mode 0700; the core refuses to start if it
  exists with another owner or wider permissions. `kumikit serve --socket <path>`
  overrides the path.
- **Stale sockets.** If the socket file exists and accepts a connection, another core is
  running and `serve` exits with an error. If it refuses connections, it is removed.
- **Peer check.** Each accepted connection's peer user id must equal the core's user id;
  other connections are closed before any message is read.
- **Named pipe.** The core creates `\\.\pipe\kumikit-<user>` as the first instance (if
  another process already owns the name, `serve` fails), rejects remote clients, and sets
  a DACL that grants access only to the core's user SID.
- **Readiness.** Once it accepts connections, `kumikit serve` prints one line
  `ready <address>` on stdout, so scripts can wait for it without polling.

### Pairing

1. On the core: `kumikit pair` (or Settings → Remote access) shows a one-time code and the
   core's certificate fingerprint. The code is valid for 5 minutes.
2. The client connects, checks the fingerprint, and sends `auth.pair { code, deviceName }`.
3. The core returns a long-lived device token. The client stores it in its secret store.
4. Later connections send `auth.token { token }` as the first message. The user can list
   and revoke devices on the core.

The core uses a self-signed certificate pinned at pairing unless the user supplies one.

## Handshake

The first call after authentication is `core.hello`:

```jsonc
// → client
{ "jsonrpc": "2.0", "id": 1, "method": "core.hello",
  "params": { "protocol": "0.2", "client": { "name": "kumikit-desktop", "version": "0.1.0" } } }

// ← core
{ "jsonrpc": "2.0", "id": 1, "result": {
    "protocol": "0.2",
    "core": { "version": "0.1.0", "name": "devbox", "os": "linux" },
    "plugins": [
      { "id": "kumikit.provider-acp", "version": "0.1.0",
        "contributes": { "providers": [], "commands": [], "settings": false } },
      { "id": "community.provider-gemini", "version": "0.1.0",
        "contributes": { "providers": ["community.provider-gemini/gemini"], "commands": [], "settings": false } }
    ],
    "capabilities": ["threads", "approvals", "plugins"] } }
```

- `protocol` is `MAJOR.MINOR`. A client refuses a core with a different major version and
  treats unknown minor features as absent. Before 1.0 (while the protocol is `0.x`) a
  minor bump may break, so both sides require the same `0.MINOR`.
- On a local transport `core.hello` is the first call. Any other method before it fails
  with `handshake_required`. A version the core cannot serve fails `core.hello` with
  `protocol_mismatch`.
- The client builds its UI from `plugins` and `capabilities`. A tab on a core without a
  feature shows no UI for it; it does not show a disabled stub.
- `plugins` lists the enabled plugins. Their `contributes` lists only what is available:
  a delegated provider whose backend is not enabled is left out. Provider ids are
  qualified (`<plugin id>/<local id>`, [05](05-providers.md#provider-ids)).
- When the enabled plugins or their available contributions change while connected, the
  core sends `core.pluginsChanged { plugins }` with the new list to every connection that
  has completed `core.hello`. Plugins becoming active or idle is not a change.

## Methods

Methods are namespaced. Kernel namespaces are fixed; plugins get their own.

| Namespace | Examples | Notes |
|---|---|---|
| `auth.*` | `pair`, `token`, `devices.list`, `devices.revoke` | Before `core.hello` |
| `core.*` | `hello`, `status`, `shutdown` | |
| `workspace.*` | `list`, `open`, `close`, `files.list`, `files.read` | Reads are scoped to the workspace root |
| `thread.*` | `list`, `create`, `get`, `send`, `cancel`, `respondApproval`, `archive` | |
| `provider.*` | `list`, `models` | Aggregated across provider plugins |
| `settings.*` | `get`, `set`, `schema` | Per plugin, validated against its schema |
| `secret.*` | `set`, `delete`, `list` | **Write-only**: a client can set a secret, never read one |
| `plugin.*` | `list`, `install`, `enable`, `disable`, `uninstall`, `permissions` | |
| `command.*` | `list`, `run` | Commands contributed by core plugins |
| `events.*` | `subscribe`, `unsubscribe` | See below |
| `<pluginId>/*` | `kumikit.git/status` | Plugin-contributed methods, routed to that plugin |

JSON field names are camelCase (`deviceName`, `inputTokens`); enum values are snake_case
strings (`awaiting_approval`, `allow_always`). Optional fields are omitted when absent,
and both sides ignore fields they do not know.

### Kernel methods

| Method | Params | Result |
|---|---|---|
| `core.hello` | `protocol`, `client { name, version }` | `protocol`, `core { version, name, os }`, `plugins`, `capabilities` |
| `workspace.list` | — | `workspaces` (open ones only) |
| `workspace.open` | `path` | `workspace`. The path is canonicalized; opening the same root again returns the same workspace, and reopens it if it was closed. |
| `workspace.close` | `workspace` | `{}`. Hides the workspace from `list`; its threads and events are kept. |
| `thread.list` | `workspace`, `includeArchived?` | `threads`, most recently updated first |
| `thread.create` | `workspace`, `provider?` (qualified id), `model?`, `title?` | `thread`. `provider` defaults to the only provider when exactly one exists (`unsupported` if there is none, `invalid_params` if there are several); `model` defaults to the provider's first model. A closed workspace gives `conflict`. |
| `thread.get` | `thread` | `thread`, `turns`, `pendingApprovals`, `seq`: a snapshot folded from the log up to `seq` |
| `thread.send` | `thread`, `input { text }` | `turn`, `queued`. One turn runs per thread; a message sent during a turn is queued in memory and starts when the turn ends. |
| `thread.cancel` | `thread` | `{}`. Cancels the running turn and drops queued messages; does nothing if no turn runs. |
| `thread.respondApproval` | `approval`, `decision` | `{}`. The first answer wins; later answers fail with `conflict`. |
| `thread.archive` | `thread` | `{}`. Sets `status` to `archived`; no thread event is appended (archiving is not thread content). Fails with `conflict` while a turn runs or waits; an archived thread refuses `thread.send`. |
| `events.subscribe` | `topic`, `after` | `head` (the topic's last `seq` at subscribe time) |
| `events.unsubscribe` | `topic` | `{}` |
| `plugin.list` | — | `plugins`: every installed plugin (`id`, `name`, `version`, `description?`, `repository?`, `state`, `requested`, `granted?`, `contributes`, `failure?`) |
| `plugin.install` | `path` (a `.kkp` file on the core's machine) | `plugin`, in state `installed`. Invalid packages and manifests fail with `invalid_params`, listing the problems; an id that is already installed fails with `conflict`. |
| `plugin.enable` | `plugin`, `grant` | `plugin`. `grant` must equal the plugin's requested permissions (they are granted as a whole), otherwise `conflict`. Compiles the component. |
| `plugin.disable` | `plugin` | `plugin`. Cancels the plugin's running turns and unloads it. |
| `plugin.uninstall` | `plugin`, `deleteData?` | `{}`. Disables it first; deletes its kv and data directory only if `deleteData` is true. |

`state` is `installed`, `enabled` or `disabled`. `requested` and `granted` are
`Permissions { spawn, network, files, secrets, tools, limits? }`, with `files` one of
`none`, `read_workspace`, `workspace` and `limits { memoryMb, callTimeoutS }` present only
when raised above the defaults. `failure { message, retryAt? }` describes the last trap or
limit breach while the plugin is in backoff.

### Notifications

| Notification | Params |
|---|---|
| `events.event` | One event on a subscribed topic (below) |
| `core.pluginsChanged` | `plugins`, as in `core.hello` |

### Errors

Errors are JSON-RPC error objects with a stable `data.code` string, so clients can react
without parsing messages:

| JSON-RPC code | `data.code` | Meaning |
|---|---|---|
| -32700 | `parse_error` | The line is not JSON |
| -32600 | `invalid_request` | Not a JSON-RPC 2.0 request (including batches) |
| -32601 | `method_not_found` | No such method |
| -32602 | `invalid_params` | Params do not match the method |
| -32000 | `not_found` | A referenced workspace, thread, turn or approval does not exist |
| -32000 | `permission_denied` | The caller or plugin lacks a permission |
| -32000 | `unsupported` | Known but not available on this core (for example a reserved topic) |
| -32000 | `conflict` | The request contradicts current state (approval already resolved, archived thread) |
| -32000 | `handshake_required` | A method was called before `core.hello` |
| -32000 | `protocol_mismatch` | The client's protocol version cannot be served |
| -32000 | `plugin_failed` | A plugin or provider failed while handling the request |
| -32000 | `internal` | A bug or an I/O failure in the core |

New codes are a minor change; clients treat an unknown `data.code` like `internal`.

## Events

Threads are event-sourced: the core appends every change to a thread as an event with a
per-thread sequence number. Clients read state by subscribing.

```jsonc
// → subscribe from a cursor (0 = from the start)
{ "id": 7, "method": "events.subscribe",
  "params": { "topic": "thread:01J…", "after": 1042 } }

// ← notifications
{ "method": "events.event",
  "params": { "topic": "thread:01J…", "seq": 1043, "time": "2026-11-15T10:00:00Z",
              "kind": "text.delta", "data": { "turn": "t3", "text": "Looking at…" } } }
```

Topics: `thread:<id>` is defined. `workspace:<id>` (threads created, renamed, archived),
`core` (plugins, status) and `approvals` (all pending approvals on this core) are
reserved: subscribing to them fails with `unsupported` until their delivery rules are
specified.

Delivery rules:
- The `events.subscribe` result is sent before any `events.event` for that topic.
- A connection has at most one subscription per topic; subscribing again replaces it.
  `events.unsubscribe { topic }` ends it, and closing the connection ends all of them.
- `seq` starts at 1 for each thread; `after: 0` replays from the start.
- Events after the cursor are replayed, then live events follow, with no gap or duplicate.
- A reconnecting client resubscribes with the last `seq` it saw.
- `text.delta` events may be coalesced for a slow client; other kinds never are.
- The log is the record: there is no separate "current state" API that can disagree with it.
  `thread.get` returns a snapshot computed from the log plus the `seq` it reflects.

Event kinds are listed in [03-kernel.md](03-kernel.md#thread-events).

## Versioning

- The method table and event kinds are versioned together as the protocol version.
- Adding a method, an optional field or an event kind is a minor change.
- Removing or changing anything is a major change and needs an ADR.
- Contract tests in `kumikit-protocol` replay recorded message fixtures; a change that
  breaks a fixture is a breaking change.
