# 02 — Protocol

The protocol is the only contract between a client and a core. It is defined in Rust in
`kumikit-protocol`; TypeScript types in `@kumikit/protocol` are generated from it and never
edited by hand.

## Transports

| Transport | Used for | Authentication |
|---|---|---|
| Unix domain socket (`$XDG_RUNTIME_DIR/kumikit/core.sock`, mode 0600) | Local client on Linux/macOS | File permissions plus a peer-credential check (same user) |
| Named pipe (`\\.\pipe\kumikit-<user>`) | Local client on Windows | Pipe ACL restricted to the user |
| WebSocket over TLS (`wss://host:port/rpc`) | Remote clients, browsers | Device token from pairing (below) |
| Stdio over SSH (`ssh <host> kumikit bridge`) | Remote clients without an open port | SSH; the core sees a local connection of the same user ([ADR 0017](../decisions/0017-shell-remote-profiles-and-ssh.md)) |

Every transport carries the same framing: one JSON-RPC 2.0 message per WebSocket text
frame, or newline-delimited JSON on sockets, pipes and SSH's stdio. A message is at most 8 MiB; a
longer line gets an `invalid_request` error and the connection is closed. JSON-RPC batches (arrays) are not supported and get a
single `-32600` error. A core listens only on the local socket unless it is started with
`--listen` (below).

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
- **Readiness.** Once it accepts connections, `kumikit serve` prints `ready <address>` on
  stdout as its last startup line, so scripts can wait for it without polling. With remote
  access on, a line `remote <url> <fingerprint>` comes before it.

### Remote transport

Decided in [ADR 0016](../decisions/0016-remote-transport-and-pairing.md).

- **Off by default.** `kumikit serve --listen <host[:port]>` turns it on for that run
  (default port 7463; there is no default address, so `--listen 0.0.0.0` is how to listen
  on every interface). Nothing about it is stored, and no method turns it on. Without the
  flag no TCP port is bound and no certificate is read or made. A busy port fails `serve`
  before `ready`.
- **TLS.** TLS 1.3 only. The core uses `--tls-cert <pem> --tls-key <pem>` if given;
  otherwise `<core data>/tls/cert.pem` and `key.pem` (0600), generated on the first start
  with `--listen` (self-signed, ECDSA P-256). Deleting them makes a new certificate, which
  every paired client then refuses. The **fingerprint** is the SHA-256 of the certificate
  (DER), written `SHA256:` and 64 hex digits in groups of four.
- **WebSocket.** `GET /rpc` over HTTP/1.1 is upgraded. One JSON-RPC message per text
  frame, at most 8 MiB; a binary frame gets `invalid_request` and the connection closes.
  When a device is revoked the core closes its connections with close code 4001. The core
  pings each connection and closes one that sends nothing (not even a pong) for 60 s.
- **Native clients** send no `Origin` header. Their first message must be a request for
  `auth.token` or `auth.pair`; anything else gets `unauthenticated` and the connection
  closes, as does a failed `auth.*`. The TLS handshake, the upgrade and that first
  message must finish within 10 s, and at most 32 connections may be unauthenticated at
  once.
- **Browsers** send an `Origin` header. It must be the core's own origin (`https://` and the
  request's `Host`) and the core must serve the web UI (`--web`, [06](06-client.md#browser-client));
  otherwise the upgrade is refused. The upgrade is authenticated by the `kk_device` cookie
  set at pairing (401 without a valid one), so the page's first message is `core.hello`.
- **Web routes** (only with `--web`): the UI bundle and its UI plugins (listed at
  `GET /ui-plugins/index.json`, read from their manifests), `GET /auth/status` →
  `{ paired, core: { name } }`, `POST /auth/pair { code, deviceName }` → `{ device }` and the
  cookie `kk_device=<token>; HttpOnly; Secure; SameSite=Strict; Path=/`, and
  `POST /auth/logout`, which revokes the cookie's device. Both `POST`s need
  `Content-Type: application/json` and the core's own `Origin`.

### Stdio over SSH

`kumikit bridge` connects to the local socket (or pipe) of the machine it runs on and
copies newline-delimited messages between it and its stdin and stdout, with the 8 MiB
limit. The desktop shell runs it through the system `ssh`; the core sees an ordinary local
connection.

### Pairing

1. On the core: `kumikit pair` shows a one-time code, the core's certificate fingerprint
   and its URL. It asks the running core with `auth.pairing.create`, which works only on
   local connections and only while remote access is on.
2. The client connects, shows the fingerprint for the user to compare, and sends
   `auth.pair { code, deviceName }` over that connection, pinned to that certificate.
3. The core returns a long-lived device token; the connection is now authenticated as the
   new device. The client stores the token in its secret store.
4. Later connections send `auth.token { token }` as the first message. Devices are listed
   and revoked with `auth.devices.*` or `kumikit devices`.

- A code is 8 characters of Crockford base32, shown as `ABCD-EFGH` (case, `-` and spaces
  are ignored). It is valid for 5 minutes and works once; 5 wrong attempts burn it; a new
  code replaces the previous one. Wrong, expired, used and burnt codes give the same
  `unauthenticated` error.
- A token is 32 random bytes, base64url, prefixed `kkd_`. The core stores only its SHA-256
  ([03](03-kernel.md#storage)) and cannot show it again.
- Pairing codes, tokens and `auth.*` params are never logged.
- Clients pin the certificate's fingerprint; a changed certificate is refused until the
  user re-adds the core.

## Handshake

The first call after authentication is `core.hello`:

```jsonc
// → client
{ "jsonrpc": "2.0", "id": 1, "method": "core.hello",
  "params": { "protocol": "0.6", "client": { "name": "kumikit-desktop", "version": "0.1.0" } } }

// ← core
{ "jsonrpc": "2.0", "id": 1, "result": {
    "protocol": "0.6",
    "core": { "version": "0.1.0", "name": "devbox", "os": "linux" },
    "plugins": [
      { "id": "kumikit.provider-acp", "version": "0.1.0",
        "contributes": { "providers": [], "commands": [], "settings": false } },
      { "id": "community.provider-gemini", "version": "0.1.0",
        "contributes": { "providers": ["community.provider-gemini/gemini"], "commands": [], "settings": false } }
    ],
    "capabilities": ["threads", "approvals", "plugins", "providers", "workspace_events"] } }
```

- `protocol` is `MAJOR.MINOR`. A client refuses a core with a different major version and
  treats unknown minor features as absent. Before 1.0 (while the protocol is `0.x`) a
  minor bump may break, so both sides require the same `0.MINOR`.
- On a local transport `core.hello` is the first call; on a remote one it is the first call
  after authentication. Any other method before it fails with `handshake_required`. A version the core cannot serve fails `core.hello` with
  `protocol_mismatch`.
- `capabilities` lists what this core serves: `threads`, `approvals`, `plugins`,
  `providers`, `workspace_events`, `registry`, and `devices` while remote access is on.
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
| `auth.*` | `pair`, `token`, `devices.list`, `devices.revoke`, `pairing.create` | `pair` and `token` authenticate a remote connection (its first message); the others come after `core.hello` |
| `core.*` | `hello`, `status`, `shutdown` | |
| `workspace.*` | `list`, `open`, `close`, `files.list`, `files.read` | Reads are scoped to the workspace root |
| `thread.*` | `list`, `create`, `get`, `send`, `cancel`, `respondApproval`, `archive` | |
| `provider.*` | `list`, `models` | Aggregated across provider plugins |
| `settings.*` | `get`, `set`, `schema` | Per plugin, validated against its schema |
| `secret.*` | `set`, `delete`, `list` | **Write-only**: a client can set a secret, never read one |
| `plugin.*` | `list`, `install`, `enable`, `disable`, `uninstall`, `updates`, `update`, `lock` | |
| `registry.*` | `list`, `add`, `remove`, `search` | The core's plugin registries ([10](10-distribution.md#registry)) |
| `command.*` | `list`, `run` | Commands contributed by core plugins |
| `events.*` | `subscribe`, `unsubscribe` | See below |
| `<pluginId>/*` | `kumikit.git/status` | Plugin-contributed methods, routed to that plugin |

JSON field names are camelCase (`deviceName`, `inputTokens`); enum values are snake_case
strings (`awaiting_approval`, `allow_always`). Optional fields are omitted when absent,
and both sides ignore fields they do not know.

### Kernel methods

| Method | Params | Result |
|---|---|---|
| `auth.pair` | `code`, `deviceName` (1–64 characters, no control characters) | `token`, `device`. Remote connections only, as the first message; `unsupported` on local ones. |
| `auth.token` | `token` | `device`. Remote connections only, as the first message; `unsupported` on local ones. |
| `auth.pairing.create` | — | `code`, `expires`, `fingerprint`, `url`. Local connections only (`permission_denied` on remote ones); `unsupported` when remote access is off. |
| `auth.devices.list` | — | `devices`: `{ id, name, created, lastSeen?, current? }` (`current` marks the caller's own device) |
| `auth.devices.revoke` | `device` | `{}`. Deletes the device and closes its connections, including the caller's if it revokes itself. |
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
| `plugin.list` | — | `plugins`: every installed plugin (`id`, `name`, `version`, `description?`, `repository?`, `state`, `requested`, `granted?`, `contributes`, `signed`, `publisher?`, `source?`, `failure?`, `error?`) |
| `core.shutdown` | — | `{}`, sent before the core stops. Running turns end as on any stop: the next start ends them as `core_restarted`. |
| `provider.list` | — | `providers`: `{ id (qualified), name, plugin }` for every available provider, read from manifests without activating anything |
| `provider.models` | `provider` (qualified id) | `models`: `{ id, label, capabilities }`. Activates the provider's plugin; an unknown provider gives `not_found`. |
| `plugin.install` | Exactly one of `path` on the core's machine (a `.kkp` file, or a directory holding a `plugin.toml`, packed as `kumikit plugin pack` would, and the packed archive's SHA-256 recorded), `package`, the bytes of a `.kkp` in base64 (so a client can install on a core on another machine; limited by the 8 MiB message), and `registry { id, version?, sha256?, registry? }` (`registry`: one index URL to use instead of the core's list), fetched by the core from its registries ([10](10-distribution.md#install)) | `plugin`, in state `installed`. Invalid packages and manifests fail with `invalid_params`, listing the problems; an id that is already installed fails with `conflict`; a hash, signature, pinned-key or index mismatch fails with `untrusted`; a registry that cannot be reached with `unavailable`; an id no registry lists with `not_found`. |
| `plugin.updates` | `refresh?` (revalidate the indexes now) | `updates`: `{ plugin, from, to, registry, requested, widens }` for each installed registry plugin whose registry offers a higher version. Applies nothing. |
| `plugin.update` | `plugin`, `version`, `grant?` | `plugin`. Downloads and checks the version like an install, then replaces the old one. When `requested` widens the current grant, `grant` must equal the new `requested`, otherwise `conflict` and nothing changes; `conflict` also while one of the plugin's turns runs. |
| `plugin.lock` | — | `lock`: the text of the core's `plugins.lock` ([10](10-distribution.md#lockfiles)) |
| `registry.list` | — | `registries`: `{ url, name? }` in resolution order (`name` once the index was read) |
| `registry.add` | `url` | `{}`. Appends it; `invalid_params` for a URL that is not `https`, loopback `http` or `file`. |
| `registry.remove` | `url` | `{}` |
| `registry.search` | `query?`, `refresh?` | `results`: `{ id, name, description?, kind, publisher, key, registry, repository?, version, requested? }` (core plugins only, the latest version this core can run), and `fetched` (when the indexes were read) |
| `plugin.enable` | `plugin`, `grant` | `plugin`. `grant` must equal the plugin's requested permissions (they are granted as a whole), otherwise `conflict`. Compiles the component. |
| `plugin.disable` | `plugin` | `plugin`. Cancels the plugin's running turns and unloads it. |
| `plugin.uninstall` | `plugin`, `deleteData?` | `{}`. Disables it first; deletes its kv and data directory only if `deleteData` is true. |

### Methods added in 0.6

| Method | Params | Result |
|---|---|---|
| `secret.set` | `name`, `value` | `{}`. Stores a secret on the core. Values are never returned, logged or included in any result. |
| `secret.delete` | `name` | `{}` |
| `secret.list` | — | `secrets`: `{ name, set, plugins }` for every name an installed plugin declares in `permissions.secrets` and every name that is set (`plugins`: the installed plugins that may read it) |
| `settings.schema` | `plugin` | `schema`: the plugin's settings JSON Schema, or `not_found` if it has none |
| `settings.get` | `plugin` | `values`: the stored values (defaults from the schema filled in). Fields marked `x-secret` are never in it. |
| `settings.set` | `plugin`, `values` | `{}`. `invalid_params` listing the problems when `values` fails the schema. |
| `command.list` | — | `commands`: `{ plugin, id, title, scope, when? }` from enabled plugins' manifests |
| `command.run` | `plugin`, `command`, `args?`, `workspace?` (required when the command's scope is `workspace`) | `CommandResult { message?, view? }`: a line to show and/or a declarative view ([07](07-ui-plugins.md#level-1-declarative-contributions)) |
| `<pluginId>/<method>` | the method's own params; `workspace` when its scope is `workspace` | the plugin's result. Only methods declared in the manifest are routed (`method_not_found` otherwise); a plugin that is not enabled gives `not_found`. |
| `plugin.workspaces` | `plugin` | `workspaces`: the workspace ids linked to the plugin (for `permissions.workspaces = "linked"`) |
| `plugin.setWorkspaces` | `plugin`, `workspaces` | `{}`. Replaces the linked set. |
| `schedule.list` | — | `schedules`: `{ plugin, key, description, cron, timeZone, enabled, missed, nextRun?, lastRun?, lastOutcome?, source }` (`source`: `manifest` or `plugin`; `lastOutcome`: `ran`, `failed`, `missed`, `skipped`) |
| `schedule.setEnabled` | `plugin`, `key`, `enabled` | `{}` |
| `schedule.delete` | `plugin`, `key` | `{}`. Manifest schedules cannot be deleted (`conflict`); pause them instead. |
| `gateway.attach` | `token` | `{}`, then the connection carries MCP for that token's provider session ([03](03-kernel.md#tool-gateway)). Local connections only, as their first message (it needs no `core.hello`); remote ones get `permission_denied`, an unknown or void token `unauthenticated`. |

New capabilities: `secrets`, `settings`, `commands`, `schedules`, `gateway`.

`state` is `installed`, `enabled` or `disabled`. `requested` and `granted` are
`Permissions { spawn, network, files, secrets, tools, client, workspaces, schedules, limits? }`
(`client`: the client operations of [04](04-plugin-host.md#permissions); `workspaces`:
`none`, `linked` or `all`; `schedules`: a boolean), with `files` one of
`none`, `read_workspace`, `workspace` and `limits { memoryMb, callTimeoutS }` present only
when raised above the defaults. `failure { message, retryInSeconds? }` describes the last
trap or limit breach; `retryInSeconds` is present while the plugin is in backoff. `signed`
is true when the package's signature was verified. `publisher { key, registry? }` gives
the signing key's id and, for registry installs, the registry; `source` is `registry`,
`file` or `bundled`. `error` says why an installed package cannot be loaded (for example,
its files were removed).

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
| -32000 | `unauthenticated` | A remote connection did not authenticate, or its code or token was refused |
| -32000 | `protocol_mismatch` | The client's protocol version cannot be served |
| -32000 | `plugin_failed` | A plugin or provider failed while handling the request |
| -32000 | `untrusted` | A package failed verification: hash, signature, pinned key, or its manifest differs from the registry's entry |
| -32000 | `unavailable` | A registry could not be reached or answered badly |
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

Topics: `thread:<id>` and `workspace:<id>` are defined. `core` (plugins, status) and
`approvals` (all pending approvals on this core) are reserved: subscribing to them fails
with `unsupported` until their delivery rules are specified.

### Workspace topic

`workspace:<id>` tells a client about changes to a workspace's threads, so a sidebar can
show every thread's status without subscribing to each thread:

| Kind | Data |
|---|---|
| `thread.created` | `thread` |
| `thread.updated` | `thread` (its status or title changed; text and tool events do not count) |
| `thread.archived` | `thread` |

- The topic has no log: `after` must be 0 (otherwise `invalid_params`), `head` is 0, and
  only changes after the subscription follow. They arrive as `events.event` notifications
  whose `seq` counts from 1 within the subscription; it is not a cursor and is not kept.
- A client subscribes first, then calls `thread.list`, and keeps whichever copy of a
  thread has the later `updated`, so nothing is missed between the two calls.
- An unknown workspace gives `not_found`.

The rules below are for thread topics.

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
