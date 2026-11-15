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
frame, or newline-delimited JSON on sockets and pipes. A core listens only on the local
socket unless the user enables remote access.

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
  "params": { "protocol": "1.0", "client": { "name": "kumikit-desktop", "version": "0.1.0" } } }

// ← core
{ "jsonrpc": "2.0", "id": 1, "result": {
    "protocol": "1.0",
    "core": { "version": "0.1.0", "name": "devbox", "os": "linux" },
    "plugins": [
      { "id": "kumikit.provider-acp", "version": "0.1.0",
        "contributes": { "providers": ["acp"], "commands": [], "settings": true } }
    ],
    "capabilities": ["threads", "approvals", "files.read"] } }
```

- `protocol` is `MAJOR.MINOR`. A client refuses a core with a different major version and
  treats unknown minor features as absent.
- The client builds its UI from `plugins` and `capabilities`. A tab on a core without a
  feature shows no UI for it; it does not show a disabled stub.
- When plugins change while connected, the core sends `core.pluginsChanged` with the new
  list.

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

Errors use JSON-RPC error objects with stable `data.code` strings
(`permission_denied`, `not_found`, `plugin_failed`, `unsupported`, …) so clients can react
without parsing messages.

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

Topics: `thread:<id>`, `workspace:<id>` (threads created, renamed, archived), `core`
(plugins, status), `approvals` (all pending approvals on this core).

Delivery rules:
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
