# 04 — Plugin host (core plugins)

Core plugins are **WASM components** run by wasmtime
([ADR 0003](../decisions/0003-wasm-first-core-plugins.md)). A sidecar process speaking the
same interfaces over JSON-RPC is a later escape hatch for code that cannot target WASM.
Native libraries are never loaded into the core.

## Package contents

```
kumikit.provider-claude-0.3.0.kkp      # tar.zst
├── plugin.toml                        # manifest
├── plugin.wasm                        # component (core plugins)
├── settings.schema.json               # optional
├── assets/                            # icons, README, licenses
└── SIGNATURE                          # ed25519 over the other files' hashes
```

## Manifest

```toml
id = "kumikit.provider-claude"         # <publisher>.<name>, lowercase, [a-z0-9-]
name = "Claude Code"
version = "0.3.0"                      # semver
description = "Run Claude Code as a Kumikit provider."
license = "MIT"
repository = "https://github.com/kumikit/provider-claude"
kind = ["provider"]                    # provider | tool | bot | integration | ui | theme
api = "^1.0"                           # kernel plugin API it targets

[runtime]
wasm = "plugin.wasm"                   # or: sidecar = { command = "node", args = ["main.js"] }
worlds = ["kumikit:provider/plugin@1.0.0"]

[activation]                           # when to instantiate; default: never at startup
on = ["provider:claude", "command:claude.login"]

[permissions]
spawn = ["claude"]                     # executables it may start
network = []                           # outbound hosts, e.g. ["api.telegram.org"]
files = "workspace"                    # none | read-workspace | workspace
secrets = ["anthropic-api-key"]        # secret names it may read
tools = false                          # may call tools contributed by other plugins

[contributes]
providers = [{ id = "claude", name = "Claude Code" }]
commands = [{ id = "claude.login", title = "Sign in to Claude Code" }]
settings = "settings.schema.json"
```

The manifest is validated at install against a JSON Schema published with the SDK.

## Interfaces (WIT)

WIT packages live in `wit/` and are versioned independently. A published version is never
changed in place; changes ship as a new version and the host supports the previous major
version for at least one kernel major release.

| Package | What it is |
|---|---|
| `kumikit:host` | Host imports every plugin may call, each gated by permissions: `log`, `kv`, `secrets`, `settings`, `events`, `process`, `http`, `files`, `timers` |
| `kumikit:plugin` | Exports every plugin implements: `activate`, `deactivate`, `run-command`, `handle-rpc` |
| `kumikit:provider` | The provider world ([05](05-providers.md)) |
| `kumikit:bot` | The bot world ([09](09-bots.md)) |
| `kumikit:tool` | Tool contribution: `list-tools`, `call-tool` |

Sketch of the host interface:

```wit
package kumikit:host@1.0.0;

interface process {
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
  get: func(key: string) -> result<option<list<u8>>, error>;
  set: func(key: string, value: list<u8>) -> result<_, error>;
  delete: func(key: string) -> result<_, error>;
  list: func(prefix: string) -> result<list<string>, error>;
}
```

Outgoing HTTP uses `wasi:http` with the host enforcing `permissions.network`. Plugins have
no WASI filesystem preopens except their own data directory; workspace access goes through
`kumikit:host/files`, which enforces `permissions.files`.

## Execution model

- Each activated plugin is one component instance on its own tokio task. Calls into an
  instance are serialized; a plugin that needs concurrency (several sessions) multiplexes
  inside its own code or asks for more instances (a manifest setting, later).
- wasmtime runs with async support, the pooling allocator, epoch interruption and a
  `ResourceLimiter`. Defaults: 64 MB linear memory, 10 s per call of CPU time without
  yielding to a host import. Both are manifest-adjustable up to a user-visible cap.
- WASI 0.2 has no native async; long-running work is host-driven: a plugin blocks inside
  host imports (`read-stdout`, `http` responses, `timers.sleep`), which are async on the
  host side. Moving to WASI 0.3 is a future ADR.
- A trap or limit breach fails the current call, marks the plugin `failed`, emits a
  `core` event, and terminates its processes. The kernel restarts it on the next
  activation trigger, with backoff.

## Lifecycle

```
installed → enabled → active ⇄ idle → unloaded
                 ↘ disabled            ↘ failed
```

- **installed**: files verified and unpacked, permissions not yet granted.
- **enabled**: the user granted the permissions. The manifest's contributions are visible
  to clients (a provider appears in the model picker) without instantiating anything.
- **active**: instantiated because an activation trigger fired (`startup`,
  `provider:<id>`, `command:<id>`, `workspace`, `rpc`, `schedule:<cron>`).
- **idle → unloaded**: after 10 minutes without calls and with no running session or
  process, the instance is dropped. State that must survive goes to `kv`.

## Permissions

- Declared in the manifest, shown at install in plain words ("can start the `claude`
  program, which runs with your full user access"), granted or refused as a whole.
- An update that widens permissions stays on the old version until the user approves.
- Every host import checks the grant; a denied call returns `permission_denied` and is
  logged. There is no API to ask for more permissions at runtime.
- `spawn` is the widest permission: a spawned CLI is outside the WASM sandbox. The UI says
  so at install and in the plugin's detail page.

## Sidecar plugins (later)

`runtime.sidecar` starts a process that speaks the same WIT-defined interfaces as JSON-RPC
over stdio. It exists for plugins that need a runtime WASM cannot host yet. Sidecars are
not sandboxed: they are installed only with an explicit "unsandboxed" confirmation and are
marked as such everywhere they appear.
