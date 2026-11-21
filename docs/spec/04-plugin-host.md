# 04 — Plugin host (core plugins)

Core plugins are **WASM components** run by wasmtime
([ADR 0003](../decisions/0003-wasm-first-core-plugins.md)) on WASI 0.2
([ADR 0010](../decisions/0010-wasi-p2-and-session-instances.md)). Sidecar processes are
not part of 1.0 ([ADR 0021](../decisions/0021-no-sidecars-in-1-0.md)). Native libraries are
never loaded into the core.

The plugin host is the `kumikit-plugin-host` crate. It sits above the kernel: it uses the
kernel's services (records, kv, secrets, process supervisor, provider registry) and the
kernel never calls into it by plugin name.

## Package contents

```
kumikit.provider-claude-0.3.0.kkp      # tar.zst
├── plugin.toml                        # manifest
├── plugin.wasm                        # component (core plugins)
├── settings.schema.json               # optional
├── assets/                            # icons, README, licenses
└── SIGNATURE                          # optional: ed25519 over the other files' hashes
```

A package holds only regular files and directories. The host refuses absolute paths,
`..` components, symlinks, hard links and device files, and caps the unpacked size and
entry count. `kumikit plugin pack <dir> [--key <file>]` builds a `.kkp` from a directory,
signed with `--key` ([10](10-distribution.md#publisher-keys)).

### Signatures

`SIGNATURE` (format v1, [ADR 0018](../decisions/0018-package-signing-and-registry.md)) is a
UTF-8 text file with LF line endings:

```
kumikit-signature 1
key ed25519:<public key, 32 bytes, base64url without padding>
file <sha256, 64 lowercase hex digits> <path>
file …
sig <signature, 64 bytes, base64url without padding>
```

- `file` lines list every regular file in the package except `SIGNATURE`, sorted bytewise
  by path, with `/` separators, one line each.
- The signature is ed25519 over every byte of the file before the `sig` line (from
  `kumikit-signature 1` up to and including the newline that ends the last `file` line).
- **Verification**, after the package is unpacked into staging: the file must parse
  exactly (no other lines, no duplicate or unsafe paths); the unpacked files must equal
  the `file` lines (nothing missing or extra, every hash equal); the signature must verify
  with the key. Any failure refuses the package as untrusted.
- The **key id** shown to people is `ed25519:` and the key in base64url, in groups of four.
- **Pinning.** The core records each installed plugin's key (`publisher_key`) and where it
  came from (`source`: a registry URL, `file` or `bundled`). A later version must be signed
  by the pinned key; an unsigned package cannot replace a signed one. A plugin without a
  pinned key (installed unsigned, or bundled) pins the first key it is updated with.
- A registry package must be signed by the publisher key the registry gives
  ([10](10-distribution.md#install)); a local file may be unsigned, and installing it says
  that its publisher cannot be verified.

## Manifest

```toml
id = "kumikit.provider-claude"         # <publisher>.<name>, lowercase, [a-z0-9-]
name = "Claude Code"
version = "0.3.0"                      # semver
description = "Run Claude Code as a Kumikit provider."
license = "MIT"
repository = "https://github.com/kumikit/provider-claude"
kind = ["provider"]                    # provider | tool | bot | integration | ui | theme
api = "^0.2"                           # kernel plugin API it targets

[runtime]
wasm = "plugin.wasm"                   # or: sidecar = { command = "node", args = ["main.js"] }
worlds = ["kumikit:provider/plugin@0.2.0"]

[activation]                           # extra triggers; contributions activate implicitly
on = ["startup"]

[permissions]
spawn = ["claude"]                     # executables it may start
network = []                           # outbound hosts, e.g. ["api.telegram.org"]
files = "workspace"                    # none | read-workspace | workspace
secrets = ["anthropic-api-key"]        # secret names it may read
tools = false                          # may hand tools contributed by other plugins to its agent
client = []                            # client operations, e.g. ["threads.read", "threads.send"]
workspaces = "none"                    # none | linked | all: where client operations may act
schedules = false                      # may run on a schedule

[limits]                               # optional; defaults below, capped by the core
memory-mb = 64
call-timeout-s = 10

[contributes]
providers = [{ id = "claude", name = "Claude Code" }]
commands = [{ id = "login", title = "Sign in to Claude Code", scope = "core" }]
settings = "settings.schema.json"
methods = [{ name = "status", scope = "workspace" }]   # routed as <plugin id>/status
tools = [{ name = "status", title = "Status", description = "…", input = "status.schema.json", effect = "read" }]
```

- `commands`: `scope` is `core` (default) or `workspace` (the command acts for the
  workspace it is run in); an optional `when` is a UI condition
  ([07](07-ui-plugins.md#level-1-declarative-contributions)).
- `methods`: plugin methods clients call as `<plugin id>/<name>`; `scope` as for
  commands. A `workspace` method gets the workspace in its call context, and the plugin
  acts for it.
- `tools`: tools for the tool gateway ([03](03-kernel.md#tool-gateway)).

The manifest is validated at install against the JSON Schema published with the SDK
(`sdk/schema/plugin-manifest.schema.json`), then checked for what a schema cannot express:
the `api` range is satisfied by the kernel, every world is one the host supports, the
runtime file exists in the package, provider ids are unique within the plugin, `spawn`
entries are bare program names (no path separators), limits are within the caps, and
activation triggers are well formed, names (commands, methods, tools) are unique and well
formed, files they name exist, a `schedule:` trigger comes with `schedules = true`, and a
settings field's `x-secret` names a secret in `permissions.secrets`.

`[runtime]` is optional: a plugin without one contributes only delegated providers
(below). The kernel plugin API version is `0.2` ([ADR 0022](../decisions/0022-wit-0-2-and-dropping-0-1.md)).

## Interfaces (WIT)

WIT packages live in `wit/`, one directory per published version
(`wit/kumikit-host@0.2.0/`). They are versioned independently. A published version is
never changed in place; changes ship as a new version. From 1.0 the host supports the
previous major version for at least one kernel major release; before 1.0 it links only the
current 0.x versions (0.2.0 since M5; 0.1.0 plugins must be rebuilt,
[ADR 0022](../decisions/0022-wit-0-2-and-dropping-0-1.md)). `wit/PUBLISHED` records a hash of
each published directory, and a test fails if one changes. The packages stay `0.x`
through M5 and freeze at 1.0.0 after it, before the official registry opens to third
parties ([ADR 0020](../decisions/0020-interface-freeze-after-m5.md)).

Rust guests use one SDK crate per world: `kumikit-plugin` (`sdk/rust`, the base world)
and `kumikit-provider` (`sdk/rust-provider`, the provider world, sharing the base crate's
types). A component imports only the interfaces its code calls.

| Package | What it is |
|---|---|
| `kumikit:host@0.2.0` | Host imports, each gated by permissions: `types`, `log`, `kv`, `secrets`, `events`, `process` (as in 0.1.0), and `settings` (own settings), `timers` (`permissions.schedules`), `client` (`permissions.client` and `workspaces`). `files` comes later. |
| `kumikit:plugin@0.2.0` | Exports every plugin world has: `lifecycle` (`activate`, `deactivate`), `rpc` (`handle`), `commands` (`run`) and `timers` (`fire`). The Rust SDK's `Plugin` trait answers `not-found` for the ones a plugin does not implement. |
| `kumikit:provider@0.2.0` | The provider world ([05](05-providers.md)) |
| `kumikit:tool@0.1.0` | Tool contribution: `tool.call` ([03](03-kernel.md#tool-gateway)) |
| `kumikit:bot` | The bot world ([09](09-bots.md)), planned for after M5 |

Shape of the host interface (abridged; `wit/` is the source):

```wit
package kumikit:host@0.2.0;

interface types {
  enum error-code { permission-denied, not-found, invalid-argument, conflict, cancelled, limit, io, internal }
  record error { code: error-code, message: string }
}

interface process {
  use types.{error};
  record spawn-request {
    program: string, args: list<string>, cwd: option<string>,
    env: list<tuple<string, string>>,
    secret-env: list<tuple<string, string>>,   // (variable, secret name)
  }
  resource child {
    write-stdin: func(data: list<u8>) -> result<_, error>;
    close-stdin: func();
    read-stdout: func(max: u32) -> result<option<list<u8>>, error>;  // none = EOF
    read-stderr: func(max: u32) -> result<option<list<u8>>, error>;
    wait: func() -> result<exit-status, error>;
    kill: func();
  }
  spawn: func(req: spawn-request) -> result<child, error>;  // checked against permissions.spawn
}

interface kv {
  use types.{error};
  get: func(key: string) -> result<option<list<u8>>, error>;
  set: func(key: string, value: list<u8>) -> result<_, error>;
  delete: func(key: string) -> result<_, error>;
  list: func(prefix: string) -> result<list<string>, error>;
}
```

`log` and `kv` (the plugin's own namespace) are always allowed; every other import checks
the grants of the plugin, or of the contributing plugin for a delegated session.

### WASI

Plugins get WASI 0.2 with no arguments, no environment variables and no stdin; their
stdout and stderr go to the core's log at debug level, tagged with the plugin id. Clocks
and random numbers are available. Sockets are disabled. The only filesystem preopen is the
plugin's own data directory, which the guest sees as `/data`; workspace access will go
through `kumikit:host/files`, which enforces `permissions.files`.

### Outgoing HTTP

Decided in [ADR 0023](../decisions/0023-outgoing-http-for-plugins.md). Plugins send requests
with `wasi:http/outgoing-handler` (WASI 0.2); the host sends them itself, after checking:
- the scheme is `https` (verified with the OS trust store);
- the host, and the port when it is not 443, is in `permissions.network` (`host` or
  `host:port`; no wildcards);
- every address the name resolves to is public, unless the entry is itself a loopback or
  private IP literal;
- redirects are not followed (a guest that follows one sends a new, checked request);
- per plugin: at most 8 requests in flight, request bodies up to 8 MiB, responses up to
  16 MiB, 10 s to connect, and a guest-chosen timeout up to 120 s.

A refused request fails with a wasi:http error and is logged with the plugin id. Logs give
only the scheme and host of a request, never its path, headers or body. In debug builds
only, `KUMIKIT_TEST_HTTP_ROUTES=<host>=<http://127.0.0.1:port>,…` sends requests for a
granted host to a loopback test server after the checks; release builds ignore it.

## Execution model

- An activated plugin runs as one or more component instances. Each instance is driven by
  its own task and handles one call at a time. A provider plugin has a control instance
  (calls outside a session) and one instance per provider session
  ([ADR 0010](../decisions/0010-wasi-p2-and-session-instances.md)). Method, command, timer and
  tool calls use the plugin's control instance, one call at a time.
- **Acting workspace.** A call acts for at most one workspace: a session's or a gateway
  call's thread workspace, or the workspace of a `workspace`-scoped method or command.
  `process.spawn` needs one (its working directory must be inside it).
- Something that must reach a plugin during a call does so through the host import it is
  blocked in: approval answers return from `events.request-approval`; a cancel makes the
  blocked import return `cancelled` once.
- wasmtime runs with async support, the pooling allocator, epoch interruption and a
  `ResourceLimiter`. Defaults: 64 MB linear memory per instance and 10 s per call of
  guest execution without returning to a host import. A manifest can raise both up to
  the core's caps (256 MB, 60 s), and install shows raised limits. The time limit is
  measured as wall time spent in guest code (an epoch tick every 100 ms; the deadline is
  reset whenever the guest enters wasm or a host import returns), not as CPU time.
- The pooling allocator has a fixed number of slots; when all are in use, activating
  another instance fails with `plugin_failed`.
- **Compile at enable.** Enabling a plugin compiles its component into the core's
  compilation cache and checks that its imports can be satisfied and its exports match
  its declared worlds. Activation loads compiled code from the cache.
- WASI 0.2 has no native async; long-running work is host-driven: a plugin blocks inside
  host imports (`read-stdout`, `request-approval`), which are async on the host side.
- A trap or limit breach fails the current call, marks the plugin `failed`, terminates its
  processes, discards all its instances (their sessions resume on the next turn) and logs
  a warning with the plugin id. The kernel restarts it on the next activation trigger,
  with backoff: 1 s after the first failure, doubling up to 5 minutes. A trigger inside
  the backoff window fails at once with `plugin_failed`. A plugin that runs for 10
  minutes without failing starts again from 1 s. `plugin.list` shows the failure.

## Lifecycle

```
installed → enabled → active ⇄ idle → unloaded
                 ↘ disabled            ↘ failed
```

- **installed**: files verified and unpacked, permissions not yet granted.
- **enabled**: the user granted the permissions. The manifest's contributions are visible
  to clients (a provider appears in the model picker) without instantiating anything.
- **active**: instantiated because an activation trigger fired. A contribution activates
  its plugin when it is used (a thread uses one of its providers, a client calls one of its
  methods or commands, an agent calls one of its tools, one of its schedules is due);
  `activation.on` adds `startup` and `schedule:<cron>` ([03](03-kernel.md#schedules));
  `command:<id>`, `rpc` and `workspace` are accepted and add nothing beyond the implicit
  triggers.
  `activate` runs once per instance. `startup` activation runs in the background, so it
  never delays the core accepting connections.
- **idle → unloaded**: an instance with no call in progress, no running turn and no call
  for 10 minutes is dropped, after `deactivate` (given 2 s). Its processes are killed and
  its session, if any, resumes on the next turn. State that must survive goes to `kv`.
- Nothing WASM-related exists at startup: the core reads enabled plugins' manifests to
  list contributions and creates the engine on the first activation.
- Disabling or uninstalling a plugin cancels its running turns, unloads its instances and
  removes its contributions.

## Permissions

- Declared in the manifest, shown at install in plain words ("can start the `claude`
  program, which runs with your full user access"), granted or refused as a whole.
- An update that widens permissions stays on the old version until the user grants the
  new set ([10](10-distribution.md#updates)). Installing an id that is already installed
  fails with `conflict`; updates go through `plugin.update`.
- Every host import checks the grant; a denied call returns `permission-denied` and is
  logged with the plugin id and the operation. A turn that fails because the plugin
  returns such an error ends with an error that starts `permission_denied: `. There is no API to ask for more
  permissions at runtime.
- `spawn` is the widest permission: a spawned CLI is outside the WASM sandbox. The UI says
  so at install and in the plugin's detail page.
- `network` is enforced on every outgoing request (above).
- `tools` lets a provider's sessions get the tool gateway; without it they get none.
- `client` lists the operations of `kumikit:host/client` the plugin may use:
  `workspaces.list`, `threads.read` (list, get, subscribe), `threads.create`,
  `threads.send`, `threads.cancel`, `approvals.respond`. `workspaces` limits where:
  `none`, `linked` (the workspaces the user linked to it) or `all`.
- `schedules` lets a plugin run on a schedule (manifest triggers and `timers`).
- `files` is shown and stored as granted, and enforced by `kumikit:host/files` when it
  arrives.

## Delegated providers

A plugin can serve providers declared by other plugins
([ADR 0011](../decisions/0011-providers-from-plugins.md)):

```toml
# the backend plugin
[contributes]
provider-backends = [{ key = "acp", schema = "acp.schema.json" }]
```

- A provider entry that carries a backend key (`acp = { … }`) is served by the enabled
  plugin that declares that key. The entry's table is validated against the backend's
  schema when the contributing plugin is enabled and passed to the backend with every call.
- A delegated session runs the backend's code with the contributing plugin's grants.
- An entry without a backend key is served by its own plugin's component.
- If no enabled plugin serves the key, or the entry's table fails the backend's schema
  (logged as a warning), the provider is unavailable and not listed.

## Sidecar plugins (not in 1.0)

`runtime.sidecar` is reserved and refused at install
([ADR 0021](../decisions/0021-no-sidecars-in-1-0.md)). A plugin that needs another runtime
starts it with the `spawn` permission from a WASM component, which the user approves
explicitly. Sidecars may be reconsidered after 1.0 if a real plugin cannot be built that
way.
