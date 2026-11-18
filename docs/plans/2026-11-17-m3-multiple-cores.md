# M3 — Multiple cores

- **Goal:** reach the exit criteria of [M3](../spec/12-roadmap.md#m3--multiple-cores): one
  window shows tabs from a local core and a remote core; killing the remote's network
  shows "reconnecting" only in its tabs, and the local tabs keep streaming. Nothing from M4
  (registry, signing, updates, lockfiles, level-2 iframe panels, theme tooling, author
  templates) or M5 (new provider or bot plugins).
- **Status:** active. Approved 2026-11-17 with the answers under Questions; the browser
  client is in scope (Q5).

## Scope

In:
- **Core remote transport:** WebSocket over TLS (`wss://host:port/rpc`), off unless
  `kumikit serve --listen <addr>` is given; a self-signed certificate kept in the core data
  directory, or one the user supplies; keepalive.
- **Pairing and devices:** `auth.pair`, `auth.token`, `auth.devices.list`,
  `auth.devices.revoke`, plus `auth.pairing.create` for the CLI (Q3); the `devices` table;
  revocation closing live connections; `by` on `approval.resolved`; `kumikit pair`,
  `kumikit devices list|revoke`.
- **SSH:** the shell starts the system `ssh`, which runs `kumikit bridge` on the remote
  machine: JSON-RPC lines over SSH's stdio to the remote core's local socket (Choice D, Q1).
- **Shell:** remote profiles (TLS and SSH) behind the existing bridge API, certificate
  pinning, device tokens in the OS keychain or a 0600 file, profile storage in the client
  data directory, connect and handshake timeouts, keepalive.
- **UI kernel:** several connections, tabs bound to any of them, a sidebar grouped by core,
  per-connection status shown only in that core's tabs, a Cores view (add by address and
  pairing code with the fingerprint to confirm, add by SSH host, remove, reconnect now), the
  plugin manager asking which core, upload of a `.kkp` to a remote core, `requires.core`
  gating.
- **Browser client** (Q5): the core serves the UI bundle over HTTPS on its remote
  listener when started with `--web <dir>`; pairing from the page sets an HttpOnly
  cookie; a browser platform adapter; and a Vite dev mode against a core for fast UI work.
- **Protocol 0.4** (all minor, listed below).
- **Tests:** core, kernel, shell, Vitest, Playwright with two real cores through the relay,
  the desktop exit test extended to two cores, and a cuttable loopback proxy to simulate
  the network failing.
- Footprint with remote access off (gated as today) and on (measured, gated by the same
  core budgets); binary and installer sizes.

Out (deferred, with the reason):
- **Installing UI plugins in the browser client:** it shows the UI plugins served with
  its bundle (read-only). A client plugin store on the core is a later design.
- **Shipping the web bundle with a core-only install:** `--web <dir>` takes a directory
  (`target/bundle/ui-browser` from `node scripts/bundle.mjs`, or the desktop app's
  resources). Packaging it for servers comes with release packaging.
- **Settings → Remote access in the UI, and turning remote access on for a running core**
  (02-protocol, Pairing step 1): remote access is a `serve` flag in M3, so nothing new can
  turn it on over the protocol (Q4). `kumikit pair` is the way to get a code.
- **Devices UI** (list and revoke from the app): the CLI does it on the core, and a paired
  client can call `auth.devices.*`. A page in the app can come with the remote-access
  settings page.
- **`kumikit plugin add --core devbox`** (10-distribution): the CLI would need its own
  profiles and token store. The CLI keeps talking to the local core; the app installs on
  remote cores.
- **`approvals` and `core` topics:** the merged sidebar uses one `workspace:<id>` topic per
  workspace per connection, which already shows "awaiting approval". Nothing in the exit
  criteria needs a core-wide topic.
- **Secret Service (Linux keychain)** for client tokens: Linux uses the 0600 file in M3
  (Choice F, Q9).
- **Accepting a changed certificate in place:** a changed fingerprint is refused; the user
  removes and re-adds the core (Q6).
- **`kumikit bridge` starting a core** on the SSH host when none runs: it reports that no
  core runs; servers start `kumikit serve` themselves (systemd unit, tmux).

## Design

### Core: remote transport

- **Turning it on.** `kumikit serve --listen <host[:port]>` (default port **7463**,
  arbitrary; step 2 checks it against the IANA registry). There is no default bind
  address: the user names the interface (`--listen 0.0.0.0`, `--listen 100.64.0.5`,
  `--listen 127.0.0.1:0` in tests). Without the flag the core binds nothing but the local
  socket, and no TLS code runs: no certificate is read or generated. Nothing about remote
  access is stored, so "off" is the default for every start, including the cores the
  desktop app starts (Choice E, Q4).
- **Certificate.** `--tls-cert <pem> --tls-key <pem>` uses the user's certificate.
  Otherwise the core reads `<core data>/tls/cert.pem` and `key.pem` (directory 0700, key
  0600), generating them on the first `--listen` start with `rcgen`: ECDSA P-256, subject
  and SAN the host name plus `localhost`, valid for 10 years (clients pin the certificate,
  so expiry and names are not what is checked). Deleting the files makes a new certificate
  on the next start, which every paired client then refuses (documented in 11-security as
  how to rotate).
- **Fingerprint.** SHA-256 of the leaf certificate's DER, shown as
  `SHA256:` + 64 hex digits in groups of four (`SHA256:3f2a 9c01 …`) so it can be read
  aloud and compared. Formatting and parsing are pure functions in
  `kumikit_protocol::remote`, so core, CLI and shell print the same text.
- **Startup order and errors.** Lock the data directory → bind the local socket → bind the
  TCP port → open the core → print `remote wss://<bound addr>/rpc <fingerprint>` → print
  `ready <local address>`. `ready` stays the last line, so scripts that wait for it keep
  working, and tests read the bound port from the `remote` line. A busy port fails `serve`
  before `ready` with "cannot listen on <addr>: address in use" and removes the socket it
  bound. A second core on the same data directory still fails on the lock first.
- **Stack.** `tokio::net::TcpListener` → `tokio-rustls` (rustls with the `ring` provider,
  **TLS 1.3 only**, ALPN `http/1.1`) → `hyper` (HTTP/1.1 server only) → `GET /rpc` upgraded
  to a WebSocket with `tokio-tungstenite`. Other routes exist only with `--web` (below).
  A `/rpc` request without an `Origin` header is a native client and authenticates with
  its first message. One with an `Origin` header is a browser: refused unless `--web` is on
  and the origin is the core's own (`https://` + the request's `Host`), and then
  authenticated by its cookie (below). `max_message_size` and `max_frame_size` are
  8 MiB. A binary frame gets `invalid_request` and the connection closes. Pings are
  answered by tungstenite.
- **Limits before authentication.** TLS handshake plus WebSocket upgrade plus the first
  message must finish within 10 s, or the connection is dropped. At most 32 connections may
  be unauthenticated at once; more are accepted and closed at once. These guard the
  pre-auth stage against slow or many clients; an authenticated device has no extra limit.
- **Keepalive.** The core pings every authenticated remote connection every 20 s and closes
  it if nothing (pong or message) arrives for 60 s, so subscriptions of a vanished client
  do not live forever. Test builds pass shorter values through hidden `serve` flags; unit
  tests use tokio's paused clock.
- **One connection loop for every transport.** `rpc::serve_connection` today reads lines
  from a byte stream. It becomes generic over a small message trait (`recv() ->
  Option<Result<String>>`, `send(String)`, `close(reason)`) with two implementations:
  newline-delimited (socket, pipe, `kumikit bridge`) and WebSocket text frames. The
  8 MiB rule, ordering and subscription code stay in one place. This refactor is its own
  step with no behaviour change.
- **The authentication gate** is a field of the connection state, checked before dispatch:
  - Local (socket, pipe, and SSH, which arrives on the remote machine's socket): as today;
    `core.hello` first. `auth.pair` and `auth.token` give `unsupported` ("local connections
    need no authentication").
  - Remote: the first message must be a request for `auth.token` or `auth.pair`. Anything
    else, including a notification, a batch or a parse error, gets `unauthenticated` and
    the connection closes. A failed `auth.*` closes it too: one attempt per connection.
    After success the connection is bound to a device id and behaves as a local one: it
    must call `core.hello` before anything else.
  - The gate runs on the raw envelope; params of a method other than `auth.*` are never
    parsed before authentication. A test walks `METHOD_NAMES` and proves each one is
    refused before auth.

### Core: pairing and devices

- **Pairing codes** live only in the running core's memory. `kumikit pair` calls
  `auth.pairing.create` over the local socket (Q3) and prints the code, its expiry, the
  fingerprint and the `remote` URL. `auth.pairing.create` is refused on remote
  connections (`permission_denied`) and when remote access is off (`unsupported`, "start
  the core with --listen"), and fails if no core runs ("no core is running").
- A code is 8 characters of Crockford base32 (40 bits), shown as `ABCD-EFGH`; input is
  case-insensitive and ignores `-` and spaces. It is valid for 5 minutes, works once, and
  is burnt after 5 wrong attempts (so a guess succeeds with probability at most 5 in
  2^40). Only one code is live: creating a new one replaces the old. Wrong, expired, used
  and burnt codes all give the same `unauthenticated` "wrong or expired pairing code".
  The clock is injected, so expiry tests do not wait.
- **`auth.pair { code, deviceName }`** → `{ token, device }`. The token is 32 bytes from
  the OS random source, base64url, prefixed `kkd_` so it is recognisable if it leaks (for
  example to a secret scanner). The connection is authenticated as the new device right
  away. `deviceName` is 1–64 characters, control characters refused.
- **Storage.** The kernel's `devices` table (migration 0004): `id` (ULID), `name`,
  `token_hash` (SHA-256 of the token, unique index), `created`, `last_seen`. Tokens are
  256 bits of randomness, so a plain SHA-256 is enough (no password hashing), and the core
  never stores or can show a token after `auth.pair` returns it. This replaces 03-kernel's
  "the secret store holds … device tokens of paired clients" (Q2): the core never needs a
  token back, only to recognise one.
- **`auth.token { token }`** → `{ device }`, after a lookup by hash; `last_seen` is updated
  (at most once a minute per device, to avoid a write per reconnect storm).
- **`auth.devices.list`** → `{ devices: [{ id, name, created, lastSeen?, current? }] }`
  and **`auth.devices.revoke { device }`** → `{}`: allowed after `core.hello` on local
  connections and on authenticated remote connections (a device can revoke itself, which
  the app does when the user removes a core) (Q7).
- **Revocation closes live connections.** The kernel deletes the row, then publishes the
  device id on a broadcast channel. Every remote connection selects on it next to its next
  message and closes with WebSocket close code 4001, reason "this device was revoked". A
  later `auth.token` with that token gets `unauthenticated`.
- **`by` on `approval.resolved`.** `thread.respondApproval` passes the connection's device
  id to the kernel (`Kernel::respond_approval(…, by: Option<DeviceId>)`), which writes it
  into the event. Absent for local and SSH connections, as 03-kernel already says.
- **Logging.** `auth.*` params and results are never logged at any level (the rpc layer
  logs method names only for them). A test captures trace-level logs through a pairing and
  a token login and checks that neither the code nor the token appears.
- **CLI.** `kumikit pair`, `kumikit devices list`, `kumikit devices revoke <id>` (all with
  `--socket`), and `kumikit bridge [--socket]` for SSH (below). Device commands need a
  running core; when none runs they work on the data directory like `plugin` does.

### SSH (Choice D, ADR 0017)

- A profile of kind `ssh` holds a destination (`devbox`, `me@10.0.0.5`, anything in the
  user's `~/.ssh/config`) and an optional remote command (default `kumikit bridge`).
- The shell runs the system `ssh`: `ssh -T -o BatchMode=yes -o ServerAliveInterval=15
  -o ServerAliveCountMax=3 -- <destination> kumikit bridge`. `BatchMode` means no
  password or host-key prompt can hang a window with no terminal: an unknown host or a key
  that needs a passphrase fails with ssh's own message, which the UI shows ("run `ssh
  devbox` once in a terminal first"). The destination is checked (no leading `-`, no
  whitespace or control characters) and passed after `--`.
- `ssh` is found on `PATH` (on Windows, the built-in OpenSSH client); `KUMIKIT_SSH`
  overrides it for tests. The shell does not use a shell to start it, and kills it when
  the connection closes.
- `kumikit bridge` on the remote machine connects to that machine's local socket (or pipe)
  and copies lines both ways, enforcing the 8 MiB limit. The core sees a local connection
  of the same user: SSH authenticated the user, and the local transport's trust model is
  "same user". So SSH profiles need no pairing and no device token (Q1: the spec says
  "SSH, then a device token" and draws "WebSocket over SSH tunnel").
- Why not a Rust SSH library: users expect their `~/.ssh/config`, agent, `ProxyJump`,
  hardware keys and `known_hosts` to work, which a library would have to reimplement.
  `russh` (Apache-2.0) is pure Rust but brings its own crypto stack and none of that
  configuration. A port forward to `wss` (`ssh -L`) would need remote access turned on,
  a free local port and pairing on top of SSH.

### Shell (`crates/kumikit-shell`, ADR 0017)

- **Profiles** are stored by the shell in `<client data>/profiles.json` (0600; written
  atomically): `{ name, label, kind: "tls" | "ssh", address | destination, fingerprint,
  deviceId, command? }`. `local` is built in and reserved. Names are the connection ids
  in addresses (`<connection>/<workspace>/<thread>`), so they are unique, never reused
  while a tab names them, and match `[a-z0-9-]{1,32}`.
- **New commands** (the bridge API itself does not change: `bridge_connect(profile,
  channel)` gains remote kinds behind the same name):
  - `profiles_list` → `[{ name, label, kind }]`. No addresses, hosts, fingerprints or
    tokens: after a profile is added, the webview refers to it only by name.
  - `profile_probe(address)` → `{ fingerprint }`: a TLS handshake that records the
    certificate and sends nothing (the handshake signature is still checked), with a 10 s
    timeout.
  - `profile_pair({ name, label, address, fingerprint, code, deviceName? })` → profile.
    Connects pinned to `fingerprint`, sends `auth.pair`, reads the token itself, stores it,
    then saves the profile. The webview sees the code the user typed, never the token.
    `deviceName` defaults to the client's host name.
  - `profile_add_ssh({ name, label, destination, command? })` → profile. Checks the
    destination by connecting once and calling nothing (the first `core.hello` is the
    webview's).
  - `profile_remove(name)`: best effort `auth.devices.revoke` of its own device (2 s
    timeout), then deletes the token and the profile. If the core was unreachable the UI
    says to run `kumikit devices revoke <id>` there.
  - All need capability grants in `capabilities/main.json` and `build.rs`.
- **TLS client:** rustls (ring, TLS 1.3 only) with a custom `ServerCertVerifier` that
  compares the SHA-256 of the end-entity certificate with the profile's fingerprint in
  constant time, ignores names and dates (the certificate is pinned, not trusted through a
  CA), and verifies the handshake signature with the provider's `verify_tls13_signature`.
  A mismatch fails the connection with "the core's certificate changed (now SHA256:…);
  remove and re-add the core if you expected this" and the UI does not retry it.
- **Remote bridge connections.** For a `tls` profile, `bridge_connect` opens TCP (10 s
  timeout), TLS, the WebSocket to `/rpc`, sends `auth.token` with the stored token, waits
  for its result (10 s) and only then attaches the stream to the bridge, so the webview's
  first message is `core.hello`, as on a local connection. `unauthenticated` (revoked or
  unknown device) fails `bridge_connect` with a typed error `unauthorized`, which the UI
  shows as "needs pairing" and does not retry. The shell pings every 20 s and closes the
  connection after 45 s without traffic, reporting `closed { reason: "the core stopped
  answering" }`. A close frame with code 4001 is reported as "this device was revoked".
- **Client secrets** (Choice F): a `TokenStore` with two backends. macOS Keychain and
  Windows Credential Manager through `keyring` (native features only; service
  `dev.kumikit.client`, account = profile name); Linux uses `<client data>/secrets.json`
  (0600, atomic writes, as the core's file backend). `KUMIKIT_CLIENT_SECRETS=file` forces
  the file anywhere (tests, or a keychain that does not work). Tokens are never logged, and
  never cross the IPC boundary: no command returns one.
- The bridge code stays transport-agnostic: it gets a message stream (lines or WebSocket
  frames) and keeps enforcing the 8 MiB limit.
- The new code lives in the shell library (`profiles.rs`, `remote.rs`, `ssh.rs`,
  `secrets.rs`), testable without a window, and reused by the test relay.

### UI kernel (`web/kernel`)

- **Platform adapter** gains `profiles: { list, probe, pair, addSsh, remove }` (tauri:
  the new commands; relay: HTTP endpoints of the relay backed by the same shell library).
  `connect(profile, on)` is unchanged; it may now reject with `{ code: "unauthorized" |
  "certificate_changed" | "unreachable", message }`.
- **Connections.** `App` keeps a `Map<profile, CoreConnection>` built from
  `profiles.list()` (`local` first). Each connection starts on its own; none waits for
  another. Each has its own `ThreadStore` (the store is already per connection in shape),
  workspace watches and backoff. New statuses: `unauthorized` and `certificate_changed`
  stop retrying; `reconnecting` keeps the existing backoff (0.5 s doubling to 30 s). A
  "Reconnect now" action resets the backoff (users want it after fixing their network; the
  exit tests use it so they never wait out a 16 s backoff).
- **Tabs and addressing.** Tabs already store `connection`. `openTab`, the thread
  commands, `ctx.threads.watch(address)` and `ctx.connection` (the active tab's
  connection; SDK unchanged) route by the address's connection. A tab whose connection is
  not ready shows the connection's state (reconnecting, needs pairing, certificate changed)
  over its content; tabs of other connections are unaffected. A tab naming a removed
  profile shows "This core was removed" with a close action. A copied address with an
  unknown connection opens nothing and says why.
- **Sidebar:** one group per connection (label and status), with its workspaces and their
  threads; "Open a project folder" and "New thread" act on the group they are in (or the
  active tab's connection from the palette).
- **Cores view** (kernel UI, reached from the palette and the sidebar): lists connections
  with label, kind, status and the core's `core.hello` name and version. Add a core:
  1. by address: address, pairing code, label → "Check" calls `profile_probe` and shows the
     fingerprint with "Compare it with the one `kumikit pair` printed on the core" → the
     user confirms → `profile_pair`;
  2. by SSH: destination, optional remote command, label → `profile_add_ssh`.
  Remove a core (with the revoke note above). A remote core of another protocol version
  shows the mismatch for that connection only, and "Restart the core" is offered only for
  `local` (stopping a remote core would not restart it).
- **Plugin manager:** core plugins are listed per connection with a core picker (default:
  the active tab's). Installing on `local` keeps the path pickers (file and folder).
  Installing on a remote core takes a `.kkp` through a standard `<input type="file">`: the
  webview reads the bytes the user picked, with no new native command, and sends
  `plugin.install { package }` (base64). Folders are offered only for `local` ("pack it
  first with `kumikit plugin pack`" otherwise). Permissions and the unsigned warning are
  shown as today, naming the core.
- **`requires.core` gating:** entries are `<plugin id>@<semver range>` (07's example
  `kumikit.git@^1`); an entry is met when the connection's current plugin list (from
  `core.hello`, updated by `core.pluginsChanged`) has that id enabled at a version in the
  range. A UI plugin's module is loaded and activated once, as today. Its slot
  contributions render, and its commands are offered, only while the active tab's
  connection meets every entry; `thread.view` and `transcript.renderer` contributions are
  chosen per tab, so two tabs side by side can mount different thread views. Plugins
  without `requires.core` are unaffected. A pure function decides it, tested in Vitest.
- **Addressing holds across cores:** the thread store, workspace watches and the fold are
  keyed by connection, so the same workspace or thread id on two cores never mixes. A
  Vitest test opens the same ids on two fake cores.

### Protocol changes (0.3 → 0.4)

All additive, so all **minor** per AGENTS.md. During 0.x both sides need the same minor,
so the version goes to 0.4. A remote core must therefore match the app's version (Q8).
TypeScript is regenerated, and fixtures are added for every new method, error and the
`by` field.

| Change | Class |
|---|---|
| `auth.pair { code, deviceName }` → `{ token, device: Device }`; remote, first message only | minor (new method) |
| `auth.token { token }` → `{ device: Device }`; remote, first message only | minor (new method) |
| `auth.devices.list` → `{ devices: Device[] }` (`Device { id, name, created, lastSeen?, current? }`) | minor (new method) |
| `auth.devices.revoke { device }` → `{}` | minor (new method) |
| `auth.pairing.create` → `{ code, expires, fingerprint, url }`; local connections only (Q3) | minor (new method) |
| Error code `unauthenticated` (-32000) | minor (new code; old clients read it as `internal`) |
| `plugin.install { package }`: base64 `.kkp` bytes, as an alternative to `path` (exactly one) | minor (new optional field; `path` becomes optional on the wire only as an alternative) |
| `capabilities` gains `devices` when remote access is on | minor (new string) |
| WebSocket transport details: path `/rpc`, text frames, close code 4001 (revoked), keepalive | minor (new transport, already in the table) |
| `approval.resolved.by` written for remote answers | none (already in 03-kernel; first time it is set) |

Not changed: `core.hello` (a remote client calls it after `auth.*`, with the same rules),
thread, workspace, provider and event methods.

### Browser client (Q5)

The same `web/kernel` bundle, built for the browser, served by a core and talking to it
over the WebSocket from the page (06-client, Browser client). One connection only.

- **Turning it on.** `kumikit serve --listen <addr> --web <dir>`. `<dir>` holds the browser
  bundle as `scripts/bundle.mjs` writes it (`target/bundle/ui-browser`: the kernel built
  with `--mode browser`, `bundled.json`, and the bundled UI plugins under `ui-plugins/`).
  `--web` without `--listen` is an error. Without `--web` the listener serves only `/rpc`.
- **Routes** on the TLS listener, all under one origin:
  - `GET /` and the bundle's files; `GET /ui-plugins/<id>/<path>`. Same path checks, MIME
    types and CSP as the shell (with `connect-src 'self'`, which covers `wss:` to the
    same host, plus `frame-ancestors 'none'`). To share that code, the shell's asset
    responder (path safety, MIME, CSP builder; no Tauri types) moves to a new crate,
    `kumikit-assets`, used by the shell, the core and the relay (01's dependency rules
    gain it).
  - `GET /auth/status` → `{ paired: bool, core: { name } }`.
  - `POST /auth/pair` with `{ code, deviceName }` (JSON only): the same pairing code rules
    as `auth.pair`. On success it sets the cookie and returns `{ device }`; the token is
    never in the body.
  - `POST /auth/logout`: revokes the cookie's device and clears the cookie.
  - The two `POST`s require `Content-Type: application/json` and an `Origin` equal to the
    core's own origin (so no other site can post a form or a fetch to them).
- **Cookie:** `kk_device=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=`
  one year. Page scripts cannot read it (an XSS can still act in the page, which the
  strict CSP guards, but cannot take the token away). A browser device is an ordinary row
  in `devices`, named from `deviceName` (default "Browser"), listed and revoked like any
  other, and revocation closes its socket.
- **`/rpc` from the page:** the browser sends the cookie with the upgrade request. With the
  matching `Origin` and a valid cookie the connection is authenticated before its first
  message, so the page's first message is `core.hello`, as on every other path. Without a
  valid cookie the upgrade gets 401 and the page shows the pairing screen.
- **Browser platform adapter** (`web/kernel/src/platform/browser.ts`, in the `browser`
  build only): `connect` opens `wss://<location.host>/rpc`; one profile (the core,
  connection id `core`); `profiles` reports just that one and cannot add or remove;
  `pluginUrl` → `/ui-plugins/…`; `clientPlugins.list` reads `bundled.json`, install and
  remove are unavailable (hidden in the plugin manager); `pickFile` uses a file input (so
  core plugins install by upload, as on any remote core); `pickFolder` becomes a text
  field for a path on the core's machine. A pairing screen (code and device name) shows
  when `/auth/status` says unpaired or the upgrade gets 401; "Sign out" calls
  `/auth/logout`.
- **The certificate in a browser.** A self-signed certificate gets the browser's warning;
  the user accepts it once and can compare the fingerprint in the browser's certificate
  viewer with the one `kumikit pair` prints (11-security says so). A user-supplied
  certificate avoids the warning.
- **Dev mode for UI work:** `npm run dev:browser -- --core https://127.0.0.1:7463` runs
  Vite's dev server (hot reload) on loopback with the `browser` platform and proxies
  `/rpc`, `/auth` and `/ui-plugins` to the core, rewriting `Origin` to the core's origin
  so the core's same-origin rule holds. The proxy accepts the core's self-signed
  certificate only when the `--fingerprint` given matches (the script checks it before
  starting). Dev-only; it is never part of a build.

## Security: new surfaces against 11-security

| Threat | Control | Proved by |
|---|---|---|
| Someone on the network connects to a core | Remote access is off unless `--listen` is given; nothing is stored that turns it on | `serve_without_listen_is_local_only` (no `remote` line, no `tls/` directory created, `auth.pairing.create` gives `unsupported`); footprint run with remote off |
| An unauthenticated client reaches kernel methods | The gate refuses everything but `auth.pair`/`auth.token` and closes | `every_method_is_refused_before_auth` (walks `METHOD_NAMES`), `notification_and_batch_before_auth_close` |
| Guessing a pairing code | 40-bit code, 5 attempts then burnt, 5 min expiry, single use, one live code | `pair_wrong_code`, `pair_code_burnt_after_five_failures`, `pair_code_expires` (injected clock), `pair_code_single_use`, `new_code_replaces_old` |
| Slow or many pre-auth connections | 10 s deadline, 32 unauthenticated at most | `pre_auth_deadline_closes` (paused clock), `pre_auth_connection_cap` |
| A web page opens the socket (cross-site WebSocket) | A browser `Origin` must be the core's own origin and `--web` must be on; `SameSite=Strict` cookie | `foreign_origin_refused`, `origin_refused_without_web` |
| Cross-site form or fetch to `/auth/pair` or `/auth/logout` | JSON content type and same `Origin` required | `pair_post_needs_same_origin_json` |
| Page script steals the browser's token | HttpOnly cookie; token never in a response body; strict CSP; `frame-ancestors 'none'` | Playwright `cookie_is_http_only` (`document.cookie` lacks it), core `pair_post_body_has_no_token`, CSP header test |
| Path traversal in the web bundle or UI plugins | The shell's checks, shared through `kumikit-assets` | moved tests, plus core `web_path_traversal_refused` |
| Man in the middle on first pairing | The user compares the fingerprint from `kumikit pair` before the code is sent; the code is sent only over the pinned connection | shell `pair_sends_code_only_to_pinned_certificate` (a second core with another certificate behind the same address gets nothing) |
| Man in the middle later, or a replaced core | Certificate pinned by SHA-256; a change is refused, not retried | shell `changed_fingerprint_is_refused`, Playwright "certificate changed" state |
| Weak TLS | rustls, TLS 1.3 only, ring provider | shell and core tests negotiate 1.3; `tls12_client_refused` |
| A stolen device token | Revocable; revocation closes live connections; tokens are 256-bit | `revoke_closes_live_connection`, `revoked_token_refused` |
| Token theft from the core's disk or database | Only SHA-256 hashes are stored | kernel `devices_store_only_hashes` (reads the row) |
| Token theft from the client | Keychain on macOS and Windows, 0600 file on Linux; never sent to the webview | shell `token_file_is_0600`, `keychain_round_trip` (CI macOS, Windows), `no_command_returns_a_token` (the IPC types have no token field; checked by a test over the command result types) |
| Tokens or codes in logs | `auth.*` params never logged | `auth_secrets_never_logged` (trace-level capture) |
| A remote core runs code in the webview | Unchanged: declarative UI only (ADR 0007); a remote `.kkp` upload goes to the core, not the client | existing tests; `upload_installs_on_core_only` |
| Uploading a malicious package | Same defensive unpack, size and entry caps as a path install; permissions shown and granted as a whole | core `install_from_package_bytes`, `oversized_package_refused` |
| SSH: injection through the destination | Destination validated, passed after `--`, no shell | shell `ssh_destination_validation`, `ssh_args_exact` |
| SSH: a prompt hangs the app | `BatchMode=yes`; ssh's error shown | shell test with a fake `ssh` that exits with ssh's message |
| A compromised trusted inline plugin adds profiles | Inline plugins already have full access (trust prompt says so); profile commands are not exposed to level-2 panels (M4) | — (documented in 11-security) |
| A vanished client keeps subscriptions alive | Keepalive closes it after 60 s | `silent_client_is_closed` (paused clock) |

README privacy section: no new automatic traffic. Remote connections and SSH run only to
cores the user added; the keepalive is traffic on a connection the user opened (stated in
the same sentence).

## Tests

All deterministic, loopback only, temp directories, no sleeps for timing (paused tokio
clocks and injected clocks; tests wait on conditions).

- **Kernel:** devices (create, lookup by hash, list, revoke publishes, `last_seen`
  throttle), migration 0004, `by` written on `approval.resolved`.
- **Core (Rust, all three OSs):** TLS and WebSocket over `127.0.0.1:0` (a test client built
  from the shell library, so both sides are real): handshake rules, the gate (above),
  pairing (expiry, wrong code, reuse, burn, replace, rate), token auth, revocation closing
  a live connection, `by` on a remote approval, `Origin` refused, wrong path 404, binary
  frame and oversized message, keepalive, busy port, `--tls-cert/--tls-key`, certificate
  generated once and reused, `remote` line before `ready`, `auth.pairing.create` refused
  remotely and when remote is off, `plugin.install { package }`, `kumikit pair`,
  `kumikit devices`, `kumikit bridge` (lines both ways, limit, no core running).
- **Shell (Rust, all three OSs):** pinning (a changed fingerprint refused; the probe
  reports the right fingerprint), pairing through `profile_pair` against a real core,
  token storage (file 0600; keychain round trip in CI on macOS and Windows), profiles file,
  `bridge_connect` for a `tls` profile (auth before the webview's first line;
  `unauthorized` after revocation; 4001 reported), keepalive (paused clock), SSH through a
  fake `ssh` (`test-plugins/fake-ssh`, which ignores the destination and runs the command
  locally) to a real core, destination validation. One ignored live test: `ssh localhost`
  to a real core (manual; needs a local sshd).
- **Vitest:** the multi-connection app state against two fake cores (one reconnecting
  leaves the other's tabs and streaming alone; statuses `unauthorized` and
  `certificate_changed` stop retries; Reconnect now), routing by address with the same ids
  on two cores, sidebar grouping, `requires.core` matching, the plugin manager's core
  choice and upload encoding, the Cores view flows (probe → confirm → pair; SSH; remove).
- **Playwright through the relay, two real cores.** `test-plugins/web-relay` gains
  `--remote`: it starts a second `kumikit serve --listen 127.0.0.1:0` in its scratch
  directory, puts a cuttable proxy in front of it, and serves the profile endpoints with
  the shell library, so pairing, pinning and token auth are the real code. Test endpoints:
  `POST /test/pair-code` (runs `kumikit pair` on the remote core and returns code and
  fingerprint), `POST /test/cut`, `POST /test/restore`. Specs: add a remote core through
  the UI (fingerprint shown, then paired); a tab on each core; the multi-core exit flow
  (below); a changed certificate refused; revocation shows "needs pairing"; a
  `requires.core` plugin appears only in the tab whose core has the required plugin;
  uploading a `.kkp` to the remote core; axe on the Cores view and the add-core dialog in
  both themes; keyboard-only add-core.
- **Browser client (Playwright, Chromium and WebKit):** a core with `--listen 127.0.0.1:0
  --web target/bundle/ui-browser` (`ignoreHTTPSErrors` in the test config): the pairing
  screen, pairing with a code from `kumikit pair`, the cookie is HttpOnly, the M2 exit flow
  (install by upload, thread, approval, theme switch) through the browser adapter,
  revocation returns to the pairing screen, Sign out, axe on the pairing screen. Core tests
  cover the routes, origin rules and cookie attributes.
- **The cuttable proxy** (`test-plugins/net-cut`, library and binary): forwards TCP from a
  loopback port to a target. `cut` makes it a black hole: live connections stay open but
  no bytes pass in either direction, and new connections are accepted and held, which is
  what a lost network looks like (no FIN, no RST), so the client must notice by itself
  through keepalive and timeouts. `restore` closes the held connections and forwards new
  ones. The binary reads `cut` and `restore` lines on stdin and prints `ready <addr>`; the
  relay embeds the library. Test builds shorten the keepalive (shell: 1 s ping, 3 s dead;
  core: hidden flags) so a cut is noticed in seconds; tests wait on the UI state with a
  deadline, never a fixed sleep.
- **The exit test** (Playwright and desktop, same flow):
  1. Start: the local core (started by the app, or the relay's) and the remote core
     (`--listen 127.0.0.1:0` behind the proxy), both with the fake ACP agent replaying
     recorded Claude Code transcripts, the recorded provider installed on both.
  2. Add the remote core in the Cores view with the code from `kumikit pair`; the
     fingerprint shown equals the one printed; confirm.
  3. Open a workspace and a thread on each core in two tabs. On the remote thread, send the
     `claude-tool-allow` message; it waits on the approval.
  4. **Cut** the remote's network. The remote tab and its sidebar group show
     "reconnecting"; the local tab does not.
  5. In the local tab send a message: its text streams in, its approval is allowed, the turn
     ends `done`, all while the remote is cut. The remote tab still shows "reconnecting".
  6. **Restore**, then "Reconnect now". The remote tab is ready, shows the same pending
     approval (the remote core kept running), allow it, the turn ends `done` with its file
     change. The thread store's gap and duplicate check stayed silent.
  The desktop version (`web/e2e-desktop/exit-m3.test.mjs`) starts the remote core and
  `net-cut` from Node, and adds the core through the real shell (so pinning and the token
  store are the shipped code; the token goes to the file store on Linux and to Credential
  Manager on Windows). It runs after the M2 test in `scripts/desktop/e2e.sh`, in the
  container on Linux and in CI on Windows. macOS has no WebDriver; the Playwright WebKit
  run covers the flow there, and the shell's Rust tests cover TLS and the keychain.
- **Live test:** the existing ignored live tests stay; M3 adds only the ignored
  `ssh localhost` shell test.

## Footprint

| Measure | How | Gate |
|---|---|---|
| Core, remote off: cold start, idle memory, activation | `scripts/footprint.sh` as today | existing budgets (≤ 300 ms, ≤ 30 MB, ≤ 50 ms / 16 MB) |
| Core, remote on (`--listen 127.0.0.1:0`, certificate already generated): cold start to `ready`, idle memory with one authenticated remote client after `core.hello` | new phase in `crates/kumikit-core/tests/footprint.rs`; also the first start with certificate generation, reported | the same cold start and idle budgets |
| Core binary | as today | ≤ 32 MiB |
| Shell anonymous memory | `scripts/desktop/footprint.sh` with the local connection, as today; a second run with a TLS connection to a second core, reported only | ≤ 60 MB (first run) |
| Installers | `installers` CI job, `scripts/check-installer-size.mjs` | ≤ 20 MB (AppImage exempt) |

Expected growth: rustls and ring, tokio-rustls, tungstenite and rcgen add roughly 1.5–3
MiB to the core (25.4 MiB now) and about the same, plus `keyring` on macOS and Windows, to
the shell. Step 2 measures it before any feature code. If the core would pass 30 MiB or an
installer 18 MB, I stop and report with a per-crate size breakdown rather than change a budget. Release
profile changes measured in M1 (per-package `opt-level = "s"` for Cranelift and wasmtime)
remain the first lever, and I would ask before using them.

## CI

| Job | Change |
|---|---|
| `rust` (3 OSs) | none in commands; the new core, kernel and shell tests run in `cargo test --workspace` (TLS on loopback, keychain on macOS and Windows) |
| `e2e-browser` (Linux, macOS) | build adds `-p kumikit-test-net-cut`; the multi-core specs run in the same `npx playwright test` |
| `e2e-desktop` (Linux, Windows) | `scripts/desktop/e2e.sh` (and the Windows step) also runs `exit-m3.test.mjs`; build adds `-p kumikit-test-net-cut -p kumikit-test-fake-ssh` |
| `footprint` (Linux) | the remote-on phase is part of `scripts/footprint.sh`; the shell's second run is reported |
| `installers` (3 OSs) | none; the size check catches growth |
| `licenses` | `cargo deny` over the new crates (rustls, ring, tokio-rustls, rcgen, keyring and their trees) |

AGENTS.md Verify changes:
- The browser e2e line becomes `node scripts/bundle.mjs && cargo build -p kumikit-core -p
  kumikit-web-relay -p kumikit-test-net-cut`, then `npx playwright test` in `web/e2e`.
- The desktop exit test line says it runs the M2 and M3 exit tests.
- New manual line: `cargo test -p kumikit-shell --test ssh -- --ignored` (live SSH to
  `localhost`; needs a local sshd and `kumikit` on its `PATH`).
- The status paragraph says M3 is built, at the end.

## Steps

Each step ends with the Verify checks that apply and one commit on branch
`m3-multiple-cores`. *(CI)* marks what only runs in CI (macOS, Windows). Pushes are
batched: the first push (opening a draft PR) after step 8, when the TLS, keychain and SSH
code first needs macOS and Windows; the next after step 13; the last after step 15.

- [x] **1. Decisions and spec (docs only).** Apply the answers to the questions. ADR 0016
  (remote transport and pairing: rustls/ring, TLS 1.3, tokio-tungstenite, rcgen,
  certificate pinning, `--listen`, device token hashes, `auth.pairing.create`, hyper, the
  browser client's cookie and origin rules), ADR 0017
  (shell: remote profiles and their commands, the system `ssh` with `kumikit bridge`,
  client secrets with `keyring`; extends ADR 0013's command list and its process
  exception). Spec: 01 (`kumikit-assets`, `net-cut`, `fake-ssh` in the layout and dependency rules; the SSH row in the diagram),
  02 (transports table, WebSocket details, pairing, handshake and the auth gate, `auth.*`,
  `unauthenticated`, `plugin.install { package }`, `serve` lines, 0.4), 03 (devices,
  secrets text, `by`), 06 (profiles, commands, statuses, Cores view, plugin manager,
  client secrets, the browser client and its dev mode), 07 (`requires.core` semantics), 10 (CLI:
  `pair`, `devices`, `bridge`), 11 (threat rows), 12 (M3 bullets gain the browser client). Verify: check-docs. Commit: `docs: settle M3 multiple-cores design`.
- [x] **2. Spike: dependencies and sizes.** Add rustls (ring), tokio-rustls,
  tokio-tungstenite, rcgen to the core and rustls, tokio-rustls, tokio-tungstenite,
  keyring to the shell, behind a minimal loopback handshake test. `cargo deny`; any licence
  outside the list stops the step and I ask. Measure the core binary and the `.deb` (in
  the container) before and after. Check that WebKitWebDriver and msedgedriver can fill a
  file input (for the upload test) *(Windows: CI)*. Check port 7463. Commit: `chore: add
  TLS and WebSocket dependencies`.
- [x] **3. Kernel: devices and `by`.** Migration 0004, the devices API with the revocation
  broadcast, `respond_approval(…, by)`, tests. Commit: `feat: store paired devices in the
  kernel`.
- [x] **4. Protocol 0.4.** `auth.*` types and methods, `unauthenticated`, `Device`,
  `plugin.install { package }`, `kumikit_protocol::remote` (fingerprint format, code
  format), fixtures, TypeScript regenerated (`tsc` check of `web/protocol`). Routing stubs
  in the core return `unsupported` so `every_method_is_routed` passes. Commit: `feat: add
  authentication methods to the protocol`.
- [x] **5. Core: one message loop for every transport.** The message trait; lines
  implementation; no behaviour change; all existing tests pass. Commit: `refactor: serve
  connections over a message stream`.
- [x] **6. Core: remote transport, pairing, revocation.** `--listen`, `--tls-cert/key`,
  certificate generation, TLS, WebSocket, limits, the gate, pairing codes, tokens,
  revocation, keepalive, `by`, logging rules, `remote` line, busy port, `plugin.install {
  package }`. Tests above. *(CI: Windows and macOS loopback TLS.)* Commit: `feat: serve
  paired remote clients over WebSocket and TLS`.
- [x] **7. Core CLI.** `kumikit pair`, `kumikit devices list|revoke`, `kumikit bridge`,
  tests. Commit: `feat: pair devices and bridge SSH from the CLI`.
- [x] **8. Shell: profiles, pinning, tokens, SSH.** Library modules and commands,
  capabilities, `test-plugins/fake-ssh`, tests *(CI: keychain on macOS and Windows,
  Windows OpenSSH path)*. Commit: `feat: connect the shell to remote cores`. **First push,
  draft PR.**
- [x] **9. Test tooling.** `test-plugins/net-cut` (library, binary, its own tests); the
  relay's `--remote`, profile endpoints and test endpoints. Commit: `test: add a cuttable
  network and a second core to the relay`.
- [x] **10. UI kernel: several cores.** Connections map, per-connection stores, statuses,
  Reconnect now, routing, tabs, grouped sidebar, Cores view, plugin manager core choice and
  upload, `requires.core` gating; `npm run check`, `npm test`, size budget. Commit:
  `feat: show tabs from several cores in one window`.
- [x] **11. Playwright multi-core suite.** Specs above, axe, keyboard. Commit: `test:
  drive two cores from the browser build`.
- [x] **12. Browser client.** `kumikit-assets` (moved from the shell, no behaviour
  change, as its own commit), `--web`, the routes and cookie, the browser adapter and
  pairing screen, `bundle.mjs` writes `ui-browser`, `dev:browser`, core tests and the
  Playwright browser specs. Commits: `refactor: share asset serving with the core`, `feat:
  serve the UI to browsers from the core`.
- [x] **13. Desktop exit test.** `exit-m3.test.mjs`, `e2e.sh`, CI step; Linux in the
  container, Windows *(CI)*. Commit: `test: add M3 exit-criteria test`. **Second push.**
- [x] **14. Footprint and installers.** Remote-on phase, shell second run, numbers in
  Notes. Commit: `chore: measure the remote transport's footprint`.
- [ ] **15. Close out.** Spec matches code, roadmap M3 done, AGENTS.md status and Verify,
  agent reference (the message trait, the gate, the proxy, test keepalive values), README
  privacy sentence, plan `done`. Commit: `docs: close M3`. **Last push**; mark the PR ready.

## Choices the spec leaves open

| # | Choice | Options | Recommendation |
|---|---|---|---|
| A | TLS library | rustls with `ring`; rustls with `aws-lc-rs` (its default); native-tls/OpenSSL | **rustls + ring, TLS 1.3 only.** ring is `Apache-2.0 AND ISC`, both on the list. aws-lc-rs's licence includes the OpenSSL licence (not on the list) and its build needs CMake (and NASM on Windows). native-tls means OpenSSL on Linux and three behaviours. Pure Rust, one behaviour on every OS. |
| B | HTTP and WebSocket | tokio-tungstenite alone; hyper + tokio-tungstenite; axum | **hyper (HTTP/1.1 server) + tokio-tungstenite** (both MIT / MIT OR Apache-2.0, already in the lockfile through the test relay). The browser client needs a few HTTP routes; hyper gives them without a framework's router and extractors. |
| C | Certificate generation | rcgen; shell out to openssl; ship none and require one | **rcgen** (MIT OR Apache-2.0) with its `ring` feature only; ECDSA P-256. |
| D | SSH | system `ssh` + `kumikit bridge` (stdio); `ssh -L` to `wss`; russh in the shell | **System `ssh` + `kumikit bridge`** (ADR 0017): the user's config, agent and known hosts work; no open port, no pairing; no crate. Costs: `kumikit` must be on the remote `PATH` (or set the remote command), and the spec's "SSH then device token" changes (Q1). |
| E | Where "remote access on" lives | `serve --listen` flag; a persisted setting in the core data directory; both | **The flag only** (Q4). Off is the default for every start, nothing over the protocol can turn it on, and a server runs it from its unit file. A persisted setting comes with a Remote access settings page. |
| F | Client token store | `keyring` everywhere; `keyring` on macOS/Windows and a 0600 file on Linux; file everywhere | **`keyring` (MIT OR Apache-2.0), native stores only, on macOS and Windows; 0600 file on Linux** (Q9). Its Linux Secret Service backends need either `libdbus` or a large zbus tree and an unlocked collection, which fails in the headless setups M3 tests in. |
| G | Remote plugin install | upload in one message (base64 in `plugin.install`); chunked upload methods; path only (copy the file yourself) | **One message, `plugin.install { package }`**, read through a standard file input, so no native file-reading command. The 8 MiB message limit allows packages up to about 6 MiB; provider-acp is 90 KB. Chunking can come if a real plugin needs it (it would be another minor change). |
| H | Pairing method for the CLI | `auth.pairing.create` over the local socket; the CLI writes a code into the database | **`auth.pairing.create`**, local connections only: the code lives in the running core's memory, and the database is the core's while it runs. |
| I | Liveness | WebSocket ping/pong on both sides; TCP keepalive (needs `socket2`); none | **WebSocket ping/pong** both ways (core 20 s/60 s, shell 20 s/45 s, ssh `ServerAliveInterval`). Without it a lost network is noticed only by TCP, minutes later, and the exit criterion depends on noticing. |
| J | Simulating a dead network | close the relay's sockets; kill the remote core; a loopback proxy with a black hole | **The `net-cut` black-hole proxy**: closing sockets or killing the core is a clean close, not a dead network, and would not test keepalive. |

New dependencies, all inside the AGENTS.md list unless step 2's `cargo deny` says otherwise:
rustls, tokio-rustls (Apache-2.0 OR ISC OR MIT / MIT OR Apache-2.0), ring (Apache-2.0 AND
ISC), rustls-webpki (ISC) and rustls-pki-types (MIT OR Apache-2.0) through rustls,
tokio-tungstenite and tungstenite (MIT OR Apache-2.0), hyper and hyper-util (MIT) (all
moved from test-only to shipped),
rcgen (MIT OR Apache-2.0) with yasna and pem (MIT), keyring (MIT OR Apache-2.0) with
security-framework on macOS. `getrandom`, `sha2` and `base64` are already in the
lockfile. No new npm dependencies.

## Questions (unclear or possibly wrong in the spec)

Answered 2026-11-17: Q1 yes (stdio over SSH, SSH is the authentication), Q2 yes (hashes in
`devices`), Q3 yes, Q4 yes (flag only), Q5 **no: the browser client stays in M3**, because
it speeds up the UI design work planned next (designed above, with a dev mode), Q6 yes
(pin the certificate; re-add on change), Q9 yes (Secret Service later). Q7, Q8, Q10 and
Q11 take the recommendation unless you say otherwise.

1. **SSH transport.** 02-protocol says "WebSocket over an SSH tunnel" with "SSH, then a
   device token", and 01's diagram shows it. I recommend stdio over SSH to the remote's
   local socket through `kumikit bridge`, with SSH as the only authentication, because SSH
   already proves the user is the core's user, which is the local transport's trust model.
   The spec's row and diagram change. Agree?
2. **Where device tokens live on the core.** 03-kernel's Secrets says the secret store
   holds "device tokens of paired clients"; Storage lists a `devices` table. I recommend
   only SHA-256 hashes in the `devices` table and dropping device tokens from the secret
   store text, since the core never needs a token back.
3. **Which `auth.*` methods run before `core.hello`.** 02's table says the whole namespace
   is "before `core.hello`". I recommend: `auth.pair` and `auth.token` are the first
   message on remote connections only; `auth.devices.*` and a new `auth.pairing.create`
   (local only, used by `kumikit pair`) come after `core.hello`. Pairing needs some method
   for the CLI to get a code from the running core; the spec has none.
4. **Remote access as a flag.** 02 mentions "Settings → Remote access" for showing a code.
   I recommend M3 has only `serve --listen` (nothing persisted) and `kumikit pair`, and a
   settings page comes later with a persisted setting. Agree, or do you want the setting
   (and which clients may change it) in M3?
5. **Browser client.** 06 says "(M3)"; the roadmap's M3 bullets and exit criteria do not
   include it. I recommend moving it to after M3 (an open question in the roadmap with a
   milestone to pick), for the reasons under "Browser client".
6. **A changed certificate.** Pinning the certificate means a user-supplied certificate
   that renews (Let's Encrypt every 60–90 days) breaks every client until it is re-added.
   Options: pin the certificate (spec's wording; recommended for M3, re-add on change);
   pin the public key (survives renewals that keep the key); or verify user-supplied
   certificates through the system's CA roots (needs `rustls-platform-verifier`, licence to
   check). I recommend pinning the certificate now and revisiting with real users.
7. **Who may list and revoke devices.** 02 says the user can list and revoke "on the core".
   I recommend local connections and any paired device (a paired device can already run
   agents as the user, so revoking another device gives it nothing new; revoking itself is
   how the app cleans up when a core is removed).
8. **Version skew between cores.** While the protocol is 0.x, both sides need the same
   minor, so after an app update a remote core of the old version shows "different
   versions" until it is updated, and only for its tabs. "Restart the core" is offered only
   for the local core. Acceptable for 0.x?
9. **Linux token store.** 06 says "the OS keychain or a 0600 file". I recommend the 0600
   file on Linux in M3 and the Secret Service later (with its own dependency and licence
   review). Do you want Secret Service in M3?
10. **`requires.core` format and scope.** 07 shows `kumikit.git@^1` but never defines it or
    what "activate only in tabs whose core has it" means for a module loaded once. I
    recommend `<id>@<semver range>` against the core's enabled plugins, with contributions
    and commands gated per active tab (not per-tab module instances).
11. **The `ready` line.** 02 says `serve` prints one line `ready <address>`. With
    `--listen` I recommend one more line before it, `remote <url> <fingerprint>`, so
    `ready` stays the last line and tests learn the bound port.
12. **Installs on this machine.** None needed: `ssh` is installed; Playwright's cached
    browsers and the desktop container are enough. The ignored live SSH test needs a local
    sshd, which I will not set up; it stays a manual test you run if you want.

## Notes

- 2026-11-17: baseline from M2. Core binary 25.4 MiB (release), provider-acp `.kkp`
  90 KB, largest installer 13.3 MB (`.deb`). `tokio-tungstenite` 0.29, `sha2`,
  `getrandom` and `base64` are already in `Cargo.lock`; rustls, ring, rcgen and keyring
  are not. `/usr/bin/ssh` is present. The UI SDK already has `ctx.connection` as the
  active tab's connection and passes `connection` in `thread.view` props, so M3 needs no
  SDK change.
- Step 2 (2026-11-17): rustls 0.23 (ring, TLS 1.3), tokio-rustls 0.26, hyper 1 with
  hyper-util, tokio-tungstenite 0.29 (the version axum already uses, so no duplicate),
  rcgen 0.14 (ring, pem) and futures-util in the core; rustls, tokio-rustls and
  tokio-tungstenite in the shell. `keyring` 4's default features add a Linux Secret
  Service backend, so the shell uses `keyring-core` 1 with `apple-native-keyring-store`
  (keychain) on macOS and `windows-native-keyring-store` (no default features) on Windows
  only: all MIT OR Apache-2.0. `cargo deny check licenses bans sources` passes over every
  target. A loopback test (`tests/tls_spike.rs`) proves a TLS 1.3 WebSocket round trip with
  a pinned rcgen certificate and a refused wrong pin. Port 7463 is unassigned (IANA lists
  7444–7470 as unassigned). The binary size is measured in step 6, once the code is linked:
  unused dependencies add nothing to the binary. Whether the WebDrivers can fill a file
  input is checked with the upload test in step 11/13.
- Step 3: the kernel already wrote `by` on `approval.resolved` (with a test) since M0, so
  only the core has to pass the device id. Revocation listeners that lag check
  `device_exists` instead of trusting the broadcast.
- Step 4: `auth.*` on local connections is final already (pair and token `unsupported`,
  devices list and revoke through the kernel, `auth.pairing.create` `unsupported` until
  `--listen` exists), so they are implemented rather than stubbed. `plugin.install
  { package }` is implemented too (written to a private temporary file, then the same
  install as a path). Tests that hard-coded `"0.3"` now say `"0.4"`.
- Step 6 (Linux): the release core is **28.0 MiB** (25.4 before; budget 32). With remote
  access off: cold start 3 ms, idle 11.1 MB (10.5 before), 12.1 MB with plugins,
  activation 4.3 ms / +7.9 MB. The gate is not a state of the JSON-RPC server: the remote
  module reads and answers the first message itself and builds the server only after it
  authenticates. Revocation is a `stop` future passed to `serve_messages`, subscribed
  before the token is checked. `tests/remote.rs` (16 tests, about 1 s) uses hidden
  `serve` flags to shorten the deadlines and waits on the core closing; stable over 6
  runs. The TLS 1.2 refusal test needs rustls's `tls12` feature as a dev-dependency
  only. Windows and macOS run these only in CI.
- Step 8: `keyring-core` is a dependency on every platform, so the keychain code path
  compiles and runs on Linux against its mock store; only the macOS and Windows store
  constructors (and the real-keychain test) are checked in CI alone. The profile
  commands live in `connections.rs`, shared by Tauri's commands and (step 9) the relay.
  A child's stdin does not close on tokio's `shutdown`, and `io::join` plus `split` kept it
  open until the read side dropped too, so the remote `kumikit bridge` never saw EOF
  (found by the SSH test hanging): the bridge now takes separate read and write halves
  (`attach_halves`) and drops the write side when it ends. ssh's stderr can arrive after
  its stdout closes, so explaining a failure waits up to 2 s for it. The shell's
  keepalive test freezes a core with SIGSTOP (Unix only), which is what a dead network
  looks like to the client.
- CI on the first push (PR #4, steps 1–8): every job passes on Linux, macOS and Windows,
  including the real-keychain round trip on macOS and Windows.
- Step 9: `net-cut` uses epochs. Restoring bumps the epoch and closes every connection
  from an earlier one, even if its task never saw the cut (a current-thread test cut and
  restored before the task ran, which hung the first version). While cut it stops
  accepting, so new connections wait in the kernel's backlog, as on a dead network. The
  relay's bridge WebSocket now starts with `{"type":"open"}` or `{"type":"error","code",
  "message"}`, so the test adapter can report `bridge_connect`'s typed errors.
- Step 10: the UI kernel bundle is **57.3 KB** gzip -9 (51.7 before; budget 150 KB). The
  `Kernel` keeps a list of `CoreConnection`s (home first, then the shell's profiles, each
  connecting on its own); `kernel.connection` is now the active tab's connection, so the
  single-core code paths (new thread, palette `when`, plugin context) follow the tab.
  The thread store is keyed by the whole address. `requires.core` is checked by a small
  semver matcher (`plugins/requires.ts`; `^`, `~`, `>=`, exact, prefix, `*`), applied to
  slot contributions, renderers and plugin commands per active tab. Accessible names the
  M2 tests (and the desktop test) rely on are kept: the sidebar is still the
  "Workspaces" navigation, each core a labelled section in it, and the plugin manager's
  region is still "On this core", now with a core picker.
- Step 11: `web/e2e/tests/cores.spec.ts` (4 tests): the exit flow with two cores and the
  cut network, revocation and certificate rotation, `requires.core` gating, and axe plus a
  keyboard-only pairing in the Cores view. Chromium locally, 16 of 16 over 4 repeats;
  WebKit only in CI. The suite found two bugs: the open-folder dialog read its props after
  closing itself (Svelte props are gone once a component unmounts), and the add-core forms
  did not move focus into themselves. On the fingerprint step focus lands on Cancel, so a
  reflexive Enter never confirms an unchecked certificate. The palette helper now looks
  inside the palette dialog, since the plugin manager has a second combobox.
- Step 12: `kumikit-assets` holds the shell's asset code unchanged, plus the browser
  client's CSP. The core serves `--web <dir>` on its remote port; the bundled UI plugins
  are listed at `/ui-plugins/index.json`, which the core builds from their `plugin.toml`
  (no TOML parser in `bundle.mjs`, and no new npm dependency for one). A non-WebSocket
  request frees its connection's unauthenticated place at once, so browsers' parallel
  keep-alive connections do not use up the 32. Tests: `tests/web.rs` (routes, CSP, path
  traversal, same-origin and JSON-only POSTs, cookie attributes, token never in a body,
  cookie-authenticated socket, sign-out closing it with 4001), and
  `web/e2e/tests/browser.spec.ts` against a core with no relay (pairing, HttpOnly cookie,
  the M2 flow by upload and typed path, reload, sign-out, wrong code, axe). Dev mode
  (`npm run dev:browser`) was checked by hand against a core: a wrong fingerprint is
  refused; with the right one the UI, `/auth/status` and pairing work through the proxy.
  The WebSocket through the dev proxy is not covered by an automated test.
- Step 13: `web/e2e-desktop/exit-m3.test.mjs` passes on Linux in the container, 3 of 3
  runs (about 12 s each, after the M2 test). The app starts the local core; the test starts
  the remote core (with `--bundled-plugins`, which the recorded provider's backend needs:
  the first run failed without it) behind the `net-cut` binary, installs the recorded
  provider on each core with `kumikit plugin add`, and pairs through the real shell (token
  in the 0600 file on Linux). Windows runs it in CI with `node --test
  --test-concurrency=1`, since both tests use tauri-driver's port.
- Step 14 (2026-11-18, Linux x86_64, release):
  - Core, remote off: cold start 3 ms, idle 11.4 MB (12.3 MB with plugins), activation
    5.0 ms / +7.9 MB. Remote on with one paired client past `core.hello`: cold start 3 ms
    (the first start, generating the certificate: 4 ms), idle **14.0 MB**. Budgets 300 ms
    and 30 MB; `scripts/footprint.sh` gates both phases.
  - Core binary **28.4 MiB** (25.4 before M3; budget 32 MiB).
  - Desktop shell, container under Xvfb, median of 5: **36.8 MiB anonymous** (36.5
    before; budget 60 MB); resident 157 MiB.
  - Not done: the planned second shell run with a TLS connection. It needs a paired
    profile and token set up from outside the app, which the scripts cannot do today; the
    gated run is unchanged.

