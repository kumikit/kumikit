# M1 — Plugin host

- **Goal:** reach the exit criteria of [M1](../spec/12-roadmap.md#m1--plugin-host): an ACP
  agent runs through `provider-acp` with no provider-specific code in the kernel; a plugin
  that calls an unpermitted host function gets `permission_denied`; idle footprint stays
  within the [budgets](../spec/00-overview.md#footprint-budgets). Nothing from M2 (no
  Tauri shell, `web/kernel`, UI plugins), no registry, no signing.
- **Status:** active (approved 2026-11-15).

## Scope

In:
- `crates/kumikit-plugin-host`: wasmtime (component model, async, pooling allocator,
  epoch interruption, `ResourceLimiter`), host imports, manifest, permissions, lifecycle,
  activation, idle unload, failure backoff, `.kkp` install.
- `wit/`: `kumikit:host`, `kumikit:plugin`, `kumikit:provider` at 0.1.0 (Q2).
- Kernel: process supervisor, plugin records, per-plugin kv, secret store (file backend),
  provider registry changes.
- `sdk/rust` (`kumikit-plugin` guest crate), `plugins/provider-acp`, test plugins and a
  fake ACP agent.
- Protocol: `plugin.list/install/enable/disable/uninstall`, `core.pluginsChanged`, real
  `core.hello` plugins.
- `kumikit plugin add/list/remove` (and `pack`, Choice H) for local `.kkp` files.
- Deleting the scripted provider; tests, CI, footprint.

Out (deferred, with the milestone that needs it):
- Host interfaces `settings` (+ `settings.*` methods, schema validation: M2 settings UI),
  `files` (workspace files service: M2), `http`/`wasi:http` (first network plugin: M5),
  `timers` (schedules: M5). No M1 plugin uses them. Adding an interface later is a minor
  WIT version (Q2).
- `kumikit:plugin` exports `run-command` and `handle-rpc` (`command.*`, `<pluginId>/*`
  methods: M2 or when a plugin needs them). `kumikit:bot`, `kumikit:tool`, tool gateway.
- `secret.set/delete/list` methods and the OS keychain backend (Q12).
- Activation triggers `command:`, `workspace`, `rpc`, `schedule:`: accepted by the
  manifest schema, never fired in M1 (nothing contributes them yet).
- `provider.list/models`, `plugin.permissions`, the `core` topic (Q4), `plugins.lock`,
  updates, signatures, registry (M4), sidecars (M4 decision), bundling provider-acp as
  preinstalled (Q10), logger redaction of secret values.

## Design

### Crates and dependency rules

```
kumikit-protocol ← kumikit-kernel ← kumikit-plugin-host ← kumikit-core
wit/ ← kumikit-plugin-host, sdk/rust (kumikit-plugin), plugins/*, test-plugins/*
```

- **Kernel** gains the services 03-kernel already lists as its own: process supervisor,
  per-plugin kv, secret store, plugin records (the `plugins` table is a core table), and a
  `Grants` type (the granted permissions) that the supervisor and kv/secrets check. It
  still names no plugin.
- **Plugin host** owns everything WASM and everything about the manifest: parsing,
  schema validation, package unpacking, the wasmtime engine, host imports, lifecycle, and
  `WasmProvider`, which implements the kernel's existing `Provider` trait. It registers
  providers with `Kernel::register_provider` (plus a new `unregister_provider`).
- **Core** opens the kernel, opens the plugin host on it, serves the protocol, and has
  the CLI. `serve.rs` loses its registration line.

### Runtime

- One lazily created `Engine`: nothing WASM-related exists until the first activation
  (or a `startup` trigger). Config: async support, component model, pooling allocator
  (slot count and memory reservation tuned so idle cost is virtual address space only;
  slots exhausted → `plugin_failed` "too many active plugin instances"), epoch
  interruption, `ResourceLimiter` (64 MB default, manifest `[limits]` up to a cap of
  256 MB), wasmtime's compilation cache in `<data>/cache/wasmtime`.
- **Compile at enable, not at activation.** Enabling a plugin compiles its component once
  (warming the cache) and checks with `instantiate_pre` that its imports are satisfiable
  and its exports match its declared worlds, so bad components fail at install. Activation
  loads compiled code from the cache. This is what makes "≤ 50 ms" reachable; Cranelift
  compilation of a component takes far longer. Fallback if the cache API does not fit:
  our own `.cwasm` files and `Component::deserialize_file` in one `unsafe` module (Choice
  C).
- **CPU limit.** An epoch ticker (100 ms) runs only while at least one instance exists.
  The store's call hook resets the deadline when wasm is entered and whenever a host
  import returns, so the limit is "10 s of guest execution without returning to the host";
  a breach traps. This is wall time spent in guest code, not CPU time (Q13).
- **WASI.** wasmtime-wasi p2 with: no args, no env, no stdin, stdout/stderr to the log
  (debug level, with the plugin id), clocks and random allowed, no sockets
  (`allow_tcp/udp/ip_name_lookup = false`), one preopen: the plugin's own data directory
  `<data>/plugin-data/<id>/`.
- **Instances** (Q1, ADR 0010). Each instance is an actor: a tokio task that owns its
  `Store` and runs one call at a time. A provider plugin gets one *control* instance for
  calls without a session (`list-models`) and one *session* instance per provider session.
  Dropping a caller's future never aborts a guest mid-call; the actor finishes or is
  discarded whole.
- **Traps and limits** fail the call, mark the plugin `failed`, kill its processes, drop
  all its instances, end their sessions (kernel forgets them, so the next turn starts with
  `resume`), and log at `warn`. The next activation trigger restarts it after a backoff of
  1 s, 2 s, 4 s … capped at 5 min; triggers inside the window fail fast with
  `plugin_failed` "retrying in N s". A plugin that stays up 10 minutes resets its count.

### Lifecycle and laziness

States: `installed` (unpacked, not granted) → `enabled` (granted; contributions visible,
providers registered, nothing instantiated) → `active` (≥ 1 instance) ⇄ `idle` →
unloaded (back to enabled with no instances); `disabled`; `failed` (with backoff).

- Activation: contributions activate their own plugin implicitly (a provider is used →
  its plugin activates), `activation.on` only adds extra triggers; M1 fires `startup` and
  the implicit `provider:` triggers (Q6).
- Idle unload: an instance with no call in progress, no running turn and no call for 10
  minutes is dropped (after `deactivate`, with a 2 s limit), and its processes are killed.
  "No running session" is read as "no running turn" (Q5): otherwise an ACP agent process
  would keep its instance alive forever. The kernel is told to forget the session, so the
  next turn re-runs `start-session` with `resume`. Timeout injectable; tests use tokio's
  paused clock (deterministic, no sleeps).
- **Startup does no WASM work.** `kumikit serve` reads the `plugins` rows and the
  enabled plugins' `plugin.toml` (small TOML files) to build `core.hello` contributions and
  register `WasmProvider` stubs. No engine, no compile, no instance, no ticker, unless a
  plugin declares `startup`.

### Footprint measurement

`scripts/footprint.sh` (release build, median of 5, fresh data dir each run) grows from
one phase to three:
1. No plugins (as today): cold start ≤ 300 ms, idle RSS ≤ 30 MB.
2. provider-acp and one declarative ACP plugin installed and enabled (installed
   offline with `kumikit plugin add --yes` before `serve` starts): same two budgets, so
   enabled-but-unused plugins are shown to cost nothing.
3. Activation: after phase 2's idle sample, `thread.create` on the declarative provider
   (instantiates provider-acp's control instance from the warm cache, calls `activate`
   and `list-models`). Round-trip time ≤ 50 ms and RSS one second later minus the idle
   sample ≤ 5 MB. Includes engine creation and a DB insert, so it is conservative.

Also recorded (not a budget yet): release binary size before and after wasmtime, since
the 20 MB installer budget (M2) includes the core. Wasmtime default features we don't use
are turned off (gc, threads, stack-switching, profiling, coredump, component-model-async
unless ADR 0010 changes) to keep it small.

### WIT

Packages, each in its own directory `wit/<name>@<version>/`, here `@0.1.0` (the version is part of the
path, so a new version is a new directory and a published one is never edited):

- `kumikit:host`:
  - `types`: `record error { code: error-code, message: string }`, `enum error-code {
    permission-denied, not-found, invalid-argument, conflict, cancelled, limit, io,
    internal }`.
  - `log`: `log(level, message)`. Always allowed.
  - `kv`: `get/set/delete/list(prefix)`, own namespace, values ≤ 1 MB. Always allowed.
  - `secrets`: `get(name) -> result<option<string>, error>`, checked against
    `permissions.secrets`.
  - `process`: the `child` resource from 04-plugin-host plus `spawn(spawn-request)`;
    `spawn-request { program, args, cwd: option<string>, env, secret-env:
    list<tuple<string, string>> }` (env var name, secret name).
  - `events`: `emit(provider-event)` (a variant mirroring the kernel's `ProviderEvent`;
    JSON payloads as JSON text, parsed and checked by the host) and
    `request-approval(kind, summary, detail) -> result<decision, error>`, which blocks
    until answered (Q1).
- `kumikit:plugin`: `interface lifecycle { activate, deactivate }`; world `plugin`
  (imports log, kv; exports lifecycle).
- `kumikit:provider`: `types` (`model`, `session-config`, `user-input` without
  attachments, `decision`, `provider-ref { id, plugin, config: option<string> }`), `interface
  provider { list-models(provider-ref), start-session(provider-ref, session-config),
  send(session, user-input), end-session(session) }`; world `plugin` imports host
  types, log, kv, events, process, secrets and exports lifecycle and provider.
  `respond-approval` and `cancel` are not exports (Q1); `provider-ref` exists for
  delegated providers (below).

A test in `kumikit-plugin-host` keeps `wit/PUBLISHED` (SHA-256 of every published package
directory) and fails if a published directory changes.

### Process supervisor (kernel)

`kumikit_kernel::process::Supervisor`, used only through the plugin host.
- `spawn(owner, request, grants, acting_workspaces)`: `program` must be a bare name listed
  in `grants.spawn` (no path separators: `/tmp/x/claude` is denied even if `claude` is
  listed); resolved on the core's `PATH` (with `PATHEXT` on Windows, via the `which`
  crate); the resolved path is logged at `info` and kept in the process table.
- `cwd` defaults to, and must be inside, a workspace the owner acts for (a session's
  thread workspace); with none, spawning is denied. Paths are canonicalized before the
  check.
- Environment: cleared, then an allowlist copied from the core (`PATH`, `HOME`, `USER`,
  `LOGNAME`, `SHELL`, `LANG`, `LC_*`, `TERM`, `TMPDIR`, `XDG_*` dirs; Windows:
  `SystemRoot`, `windir`, `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, `ProgramData`,
  `ProgramFiles*`, `PATHEXT`, `COMSPEC`, `TEMP`, `TMP`, `USERNAME`, `HOMEDRIVE`,
  `HOMEPATH`), then the plugin's explicit vars (loader variables `LD_*`, `DYLD_*` refused),
  then `secret-env` entries, each checked against `grants.secrets`.
- Lifetimes: Unix children start in their own process group (`process_group(0)`) and are
  killed with SIGTERM to the group, then SIGKILL after 2 s. Windows: each owner gets a job
  object with `KILL_ON_JOB_CLOSE`; the child is assigned right after spawn (a brief window
  in which a grandchild could escape; documented) and killed with `TerminateJobObject`.
  Owners are killed on cancel (after the grace period, Q5), session end, unload, failure
  and shutdown. Known gap: a SIGKILLed core on Unix leaves children behind.
- stdout/stderr are read by host tasks into bounded buffers, so a plugin that reads only
  stdout cannot deadlock on a full stderr pipe.
- `unsafe` for the job object lives in one module (`process/windows.rs`) with `SAFETY:`
  comments, like the M0 pipe module.

### Delegated providers (declarative ACP) and provider ids

Proposed general API (ADR 0011), no special case for ACP:
- A plugin may declare a **provider backend**: `[contributes] provider-backends = [{ key =
  "acp", schema = "acp.schema.json" }]`.
- Any plugin's provider entry may carry exactly one backend key (`acp = { … }`). Such an
  entry is served by the enabled plugin that declares that key; the entry's table is
  validated against the backend's schema at enable and passed to every call as
  `provider-ref.config` (JSON text). The backend never reads other manifests; it only
  sees the entries routed to it.
- A provider entry without a backend key is served by its own plugin's component. A
  plugin with no `[runtime]` (a declarative plugin) may only contain delegated entries.
- **Permissions follow the contributor.** A delegated session runs in the backend's code
  but with the *contributing* plugin's grants (the user approved "can start `gemini`" on
  that plugin). provider-acp itself requests no permissions. The install text of a
  declarative plugin says "runs through kumikit.provider-acp".
- If the backend is missing or disabled, the contributor stays enabled and its providers
  are unavailable (not listed in `core.hello`); they appear (with `core.pluginsChanged`)
  when the backend is enabled.
- Provider ids are **qualified on the wire**: `<plugin id>/<local id>`, for example
  `community.provider-gemini/gemini`, owned by the contributing plugin. Plugin ids are
  unique, so provider ids never collide; manifests use local ids. `Thread.provider`
  stores the qualified id.

### provider-acp

- Sans-IO core: an ACP client state machine that takes lines from the agent and returns
  actions (write a line, emit an event, ask for approval, turn finished). Handles
  `initialize` (protocol version 1, no `fs`/`terminal` client capabilities in M1),
  `session/new` (cwd = workspace root, no MCP servers) or `session/load` when resuming
  and the agent supports it, `session/prompt`, `session/update` (`agent_message_chunk` →
  `text.delta`, accumulated into `text.done`; `agent_thought_chunk` → `reasoning.delta`;
  `tool_call`/`tool_call_update` → `tool.started/output/finished`, diffs →
  `file.changed`; `plan` and unknown updates → `plugin.event`), and
  `session/request_permission` (→ `request-approval`; allow → `allow_once`, allow-always →
  `allow_always`, deny → `reject_once`). Unknown fields and update kinds are tolerated.
- Thin WASM glue drives it with `process` and `events`. ACP types are a small hand-written
  serde subset (Choice F).
- Entry schema (`acp.schema.json`): `command`, `args`, `env`, `secret-env`, `models`
  (optional list; default one model `default` labelled with the provider name), since
  ACP gives no model list before a session.

### Tests and fixtures without the scripted provider

- Kernel tests keep their in-file fake provider (they never used the scripted one).
- `kumikit-core` unit tests (`rpc/tests.rs`) get a `#[cfg(test)]` fake provider in the
  test module; nothing provider-shaped is compiled into the core.
- `test-plugins/probe`: a WASM provider built with the public SDK whose models select a
  deterministic scenario: `echo`, `tool-approval`, `fail` (the scripted scenarios), and
  for host tests `trap`, `spin`, `oom`, `spawn:<program>`, `secret:<name>`, `kv`,
  `fs-escape`, `socket`. `exit_m0` moves onto it, unchanged in what it checks.
- `test-plugins/fake-acp-agent`: a native binary. `record -- <agent cmd>` proxies stdio
  and writes a transcript; `replay <transcript>` plays the agent side, matching each
  client message by method and order (rewriting response ids), and a `spawn-tree` mode
  for supervisor kill tests on all three platforms.
- `test-plugins/` sits outside `crates/` and `plugins/`: test plugins are neither kernel
  code nor shipped plugins, and the name check (below) does not have to exempt them.
- Building guests (Choice E): a `kumikit-test-plugins` dev-only crate whose `build.rs`
  runs a nested `cargo build` (own target dir `target/kumikit-test`) for the probe and
  provider-acp (`wasm32-wasip2`) and the fake agent (host), and exports their paths plus
  a `pack()` helper that makes `.kkp` files in temp dirs.
- Provider testing rule: recorded real ACP transcripts in
  `plugins/provider-acp/tests/transcripts/` are replayed natively through the sans-IO
  core and compared with expected event files (bless with `KUMIKIT_BLESS=1`); the same
  transcripts drive the fake agent end to end; one `#[ignore]`d live smoke test runs the
  real agent (Q16).

### CLI

`kumikit plugin add <file.kkp> [--yes]`, `list`, `remove <id> [--yes] [--keep-data]`,
`pack <dir> [-o file]`, all with `--socket`/`--data-dir`.
- One code path: the CLI talks `plugin.*` to the running core. If none runs, it opens the
  kernel and plugin host in-process on the data directory and calls the same functions.
  `serve` and the offline CLI take an exclusive lock on `<data>/core.lock`
  (`std::fs::File::try_lock`), so they never write the same database together, and a
  second `serve` on one data dir now fails cleanly.
- `add`: `plugin.install` (unpack, validate, state `installed`), prints the permissions in
  plain words ("can start the `gemini` program, which runs with your full user access
  outside the sandbox"; "can read the secret `x`"; limits above the defaults), the
  publisher and repository, and "This package is not signed; Kumikit cannot check who
  made it." Then asks "Grant and enable? [y/N]"; yes → `plugin.enable { id, grant }`
  with the exact set shown. Without a terminal and without `--yes` it leaves the plugin
  installed and says how to finish.
- `remove`: asks before deleting plugin data (spec: "after confirmation").
- Unsigned packages before M4: `SIGNATURE` is neither required nor checked; the SHA-256
  of the `.kkp` is recorded for the M4 lockfile. Installing an id that is already
  installed fails with `conflict` (updates are M4).
- Unpacking is defensive: regular files and directories only (no symlinks, hard links,
  devices), no absolute or `..` paths, size and entry-count caps, into a temp dir renamed
  into `<data>/plugins/<id>/<version>/`.

## Protocol changes

All additive, so all **minor** per AGENTS.md. During 0.x both sides need the same minor,
so the protocol version goes `0.1` → `0.2`. TypeScript regenerated, fixtures added.

| Change | Class |
|---|---|
| `core.pluginsChanged { plugins }` notification, sent after `core.hello` whenever the enabled set or the available contributions change (not on active/idle) | minor (new notification) |
| `core.hello.plugins` lists enabled plugins with *available* contributions; provider ids qualified | no schema change; values change |
| `capabilities` gains `plugins` | minor (new string) |
| `plugin.list` → `{ plugins: [InstalledPlugin] }` with `id, name, version, description?, repository?, state (installed, enabled, disabled), requested, granted?, contributes, failure? { message, retryAt }` | minor (new method) |
| `plugin.install { path }` (a `.kkp` on the core's machine) → `{ plugin }` | minor |
| `plugin.enable { plugin, grant }` → `{ plugin }`; `conflict` if `grant` differs from the requested set | minor |
| `plugin.disable { plugin }` → `{ plugin }`; cancels the plugin's running turns, unloads it | minor |
| `plugin.uninstall { plugin, deleteData? }` → `{}` | minor |
| `Permissions { spawn, network, files (none, read_workspace, workspace), secrets, tools }` and `Limits` types | minor (new types) |
| `Thread.provider` holds a qualified id | no schema change |
| `plugin.event.plugin` is the emitting plugin's id (today the sink fills in the provider id; for a delegated provider this is the backend) | fix of a value, no schema change |

No new error codes: package and manifest problems are `invalid_params` with the list of
problems in the message; plugin failures stay `plugin_failed`.

## Steps

Each step ends with the Verify checks that apply (`sh scripts/check-docs.sh`, `cargo fmt
--check && cargo clippy --workspace --all-targets -- -D warnings`, `cargo test
--workspace`, `cargo deny check licenses bans sources` when dependencies change, the
TypeScript check when protocol types change) and one commit on branch `m1-plugin-host`.
Steps marked *(CI)* contain macOS/Windows code that is only compiled and run in CI.

- [x] **1. Decisions and spec (docs only).** Apply the answers below. ADR 0010 (WASI 0.2,
  instance per session, approvals and cancel through imports, guest tooling), ADR 0011
  (plugin-provided providers: registration, qualified ids, provider backends); ADRs 0008
  and 0009 status (Q18); `decisions/README.md` index; 02 (plugin methods,
  `core.pluginsChanged`, version 0.2), 03 (supervisor rules, kv, secret file backend,
  plugin records), 04 (execution model, WIT shape, lifecycle details, limits, delegated
  providers, compile at enable, CPU limit wording), 05 (provider world, ids, ACP backend),
  10 (local install in M1, unsigned packages), 12 (WASI open question resolved). Verify:
  check-docs. Commit: `docs: settle M1 plugin host design`.
- [x] **2. Toolchain spike and skeleton.** `rust-toolchain.toml` gains `targets =
  ["wasm32-wasip2"]`. New members: `crates/kumikit-plugin-host`, `sdk/rust`
  (`kumikit-plugin`), `test-plugins/kumikit-test-plugins` (nested-build crate), a minimal
  guest. A host test instantiates the guest with the pooling allocator, cache, epoch call
  hook and limiter, and prints activation time. Confirms the open technical points
  (wasmtime cache API, `call-hook`, wit-bindgen multi-package paths, native compile of
  generated bindings) and updates this plan if any fails. CI: add `cargo clippy --target
  wasm32-wasip2 -p kumikit-plugin … -- -D warnings`. Verify: all, plus `cargo deny`. *(CI)*
  Commit: `chore: add plugin host crate and WASM guest build`.
- [x] **3. WIT packages and SDK bindings.** `wit/kumikit-host@…`, `wit/kumikit-plugin@…`,
  `wit/kumikit-provider@…`, `wit/PUBLISHED` + guard test; `kumikit-plugin` generates
  bindings with wit-bindgen and adds safe helpers (`Child` with a line reader,
  `emit`/`request_approval`, `export_provider!`). Verify: guard test fails on an edit;
  guest compiles for wasm32-wasip2 and natively. Commit: `feat: add kumikit WIT packages
  and guest SDK`.
- [x] **4. Process supervisor.** `crates/kumikit-kernel/src/process/` (`mod.rs`, `unix.rs`,
  `windows.rs`), `Grants`. Tests: unlisted program, path-qualified program, cwd outside
  the workspace and via a symlink, no acting workspace, env allowlist and loader-var
  refusal, secret-env undeclared, resolved path recorded, kill on owner kill reaches a
  grandchild (`fake-acp-agent spawn-tree`), kill on shutdown. *(CI: Windows job objects,
  macOS groups)* Commit: `feat: add process supervisor`.
- [x] **5. Plugin records, kv, secrets.** Migration `0002_plugins.sql` (`plugins`,
  `plugin_kv`), kernel kv API (writer thread, 1 MB cap, prefix list), secret store with a
  0600 file backend (`<data>/secrets.json`), `Kernel::unregister_provider`,
  `forget_session`, sink carries the plugin id, `core.lock`. Tests: migration from schema
  1, kv namespacing and limits, secrets file mode, lock refusal. Commit: `feat: add plugin
  records, kv and secret store`.
- [x] **6. Manifest and packages.** `kumikit-plugin-host/src/manifest/` (TOML → JSON →
  JSON Schema at `sdk/schema/plugin-manifest.schema.json` → typed `Manifest`; semantic
  checks: id format, semver, `api` range, worlds known, runtime file present, unique
  provider ids, backend keys, bare spawn names, limits within caps, trigger syntax),
  permission descriptions, `.kkp` unpacking, `PluginManager` install/enable/disable/
  uninstall over the kernel records (enable without WASM yet). Tests: valid and invalid
  manifests (one per rule), traversal/symlink/oversize archives, grant mismatch,
  conflict on reinstall. Commit: `feat: add manifest validation and local package install`.
- [x] **7. WASM runtime and host imports.** Engine, instance actor, WASI context, linker
  with every import gated by `Grants`, logging of each denial with plugin id and
  operation, compile at enable. The probe plugin. Tests, one per permission (denied path
  returns `permission-denied` and is logged): spawn unlisted, spawn path-qualified, spawn
  cwd outside workspace, secret undeclared, secret-env undeclared; sandbox: filesystem
  outside the data dir fails, sockets fail, a component importing `wasi:http` is refused
  at enable; limits: `trap`, `spin` (short injected timeout), `oom`; kv round-trip.
  Commit: `feat: run WASM plugins with permission-checked host imports`.
- [x] **8. Lifecycle and the provider adapter.** States, activation (implicit, `startup`),
  idle unload, failure backoff, `WasmProvider` (control and session instances, approvals
  through the blocking import with early answers kept, cancel with grace then kill,
  `end-session`), delegated providers and qualified ids. Tests (paused tokio clock): idle
  unload drops the instance and kills its processes, then the next turn resumes; trap →
  failed → backoff window → restart; cancel during an approval and during a blocked read;
  disable cancels running turns; backend enabled after contributor. Commit:
  `feat: add plugin lifecycle and WASM providers`.
- [ ] **9. Protocol.** Types and table entries from the protocol table above, version
  0.2, regenerated TypeScript, fixtures for every new method and notification (hand-written
  first, replaced by scrubbed recordings in step 13). Verify: TS check. Commit:
  `feat: add plugin methods to the protocol`.
- [ ] **10. Serve providers from plugins; delete the scripted provider.** `serve.rs` opens
  the plugin host, dispatches `plugin.*`, sends `core.pluginsChanged`, fills `core.hello`;
  `scripted_provider.rs` and its registration deleted; `rpc/tests.rs` uses a test-module
  fake; `exit_m0` runs on the probe plugin; ADR 0009 → `Superseded by 0011`; AGENTS.md
  architecture rule loses its exception; agent reference updated. Commit:
  `refactor: serve providers from plugins and remove the scripted provider`.
- [ ] **11. CLI.** `kumikit plugin add/list/remove/pack`, online and offline paths,
  prompts. Tests: add/list/remove against a running core (sees `core.pluginsChanged`) and
  offline; non-interactive without `--yes` leaves it installed; remove asks about data.
  Commit: `feat: add kumikit plugin add, list and remove`.
- [ ] **12. provider-acp.** `plugins/provider-acp` (manifest, `acp.schema.json`, sans-IO
  core, glue), `fake-acp-agent record/replay`, recorded and scrubbed transcripts (text
  reply; tool call with approval allowed; denied; cancelled mid-turn), native replay
  tests, `#[ignore]`d live smoke test. Verify: also `cargo clippy --target wasm32-wasip2`.
  Commit: `feat: add provider-acp`.
- [ ] **13. Exit test.** `crates/kumikit-core/tests/exit_m1.rs`: `serve` with the fake
  agent on `PATH`; `kumikit plugin add --yes` for provider-acp and a manifest-only
  `.kkp`; a subscribed client sees `core.pluginsChanged`; `thread.create` with the
  qualified id; send; events match the transcript's expected events; approval answered;
  `turn.ended done`. Then the probe's unpermitted spawn reports `permission_denied`
  through the thread. `scripts/check-plugin-names.sh`: reads every `plugin.toml` under
  `plugins/` and `test-plugins/` and fails if a plugin id, provider id or backend key
  appears in `crates/` (or `web/kernel` later); runs in CI. Run the exit tests 50× locally.
  Commit: `test: add M1 exit-criteria test`.
- [ ] **14. Footprint.** Three phases above; `footprint.sh` builds the release guests
  first; record numbers (including binary size) in Notes. Commit: `chore: measure plugin
  footprint`.
- [ ] **15. Close out.** Spec matches the code, roadmap M1 done, AGENTS.md status and
  Verify (below), agent reference, plan `done`. Commit: `docs: close M1`.

### Verify additions for AGENTS.md (step 2 onward, as each exists)

- One-time setup: the `wasm32-wasip2` target (rustup installs it from
  `rust-toolchain.toml`; see Q15 for this machine).
- `cargo clippy --target wasm32-wasip2 -p kumikit-plugin -p kumikit-provider-acp -p
  kumikit-test-probe -- -D warnings`
- `sh scripts/check-plugin-names.sh`
- `cargo test -p kumikit-provider-acp -- --ignored` (live smoke, needs the agent; manual)
- `KUMIKIT_BLESS=1 cargo test -p kumikit-provider-acp` regenerates expected events.

### CI

- `rust` job (3 OS): unchanged commands; the toolchain file brings `wasm32-wasip2`, the
  nested build compiles the guests. Plus the wasm32 clippy line.
- `docs` job: plus `check-plugin-names.sh`.
- `footprint` job: builds release guests, three phases.
- `licenses`: unchanged command; covers the new dependencies and guest crates.
- Cache: rust-cache also keeps `target/kumikit-test`.

## Choices the spec leaves open

| # | Choice | Options | Recommendation |
|---|---|---|---|
| A | WASI 0.2 or 0.3 (roadmap open question, "by M1") | 0.2 sync exports; component-model async / 0.3 | **0.2.** ADR 0003 chose it; Rust's `wasm32-wasip2` is a tier-2 target with a shipped std and builds components directly, `wasm32-wasip3` is tier 3. Concurrency comes from one instance per session (Q1), which also works unchanged under 0.3 later. Revisit when wasip3 is tier 2. **ADR 0010.** |
| B | Guest tooling | cargo-component; `wasm32-wasip2` + wit-bindgen | **`wasm32-wasip2` + wit-bindgen** (`generate!` in `kumikit-plugin`): plain `cargo build --target wasm32-wasip2`, no extra cargo subcommand to install. In ADR 0010. |
| C | Compiled-code cache | wasmtime `cache` feature; own `.cwasm` + unsafe `deserialize` | **wasmtime cache** (safe API, keyed by engine version); fallback decided in the step 2 spike. |
| D | Provider id namespacing | global ids (first wins / conflict); qualified `<plugin>/<id>` | **Qualified on the wire**, local in manifests. **ADR 0011.** |
| E | Building guests for tests | prebuilt artifacts checked in; tests skip without the toolchain; nested build in `build.rs` | **Nested build, required target.** `wasm32-wasip2` becomes part of the pinned toolchain like rustfmt. Pure-logic tests (manifest, supervisor, kernel, ACP replay) need no WASM; host and exit tests do, and fail with the setup command rather than skip. The nested build is cached in its own target dir; two small guests. |
| F | ACP types | `agent-client-protocol-schema` crate; hand-written subset | **Hand-written subset** (≈10 message types): smaller component, tolerant of unknown fields, no schemars. |
| G | kv storage | SQLite table; files in the plugin data dir | **SQLite `plugin_kv` table** through the writer thread (one store, transactional, prefix scans by index). |
| H | Building `.kkp` files | test-only helper; `kumikit plugin pack` | **`kumikit plugin pack <dir>`**: tests, CI and plugin authors need it before M4 templates; ~50 lines. |
| I | Manifest schema validator | `jsonschema` (MIT, default features off: no HTTP/file resolving); `boon`; serde-only | **`jsonschema`** with default features off, schema embedded from `sdk/schema/`. Check binary size in step 14. |
| J | Other crates | — | wasmtime, wasmtime-wasi (Apache-2.0 WITH LLVM-exception), toml, semver, tar, zstd, sha2, which (MIT/Apache/BSD); wit-bindgen in guests. All within `deny.toml`; `cargo deny` per step. |
| K | Where test plugins live | `plugins/`; `crates/…/tests/`; `test-plugins/` | **`test-plugins/`** (see Tests). |

## Questions (unclear or possibly wrong in the spec)

Resolved 2026-11-15: every recommendation below is accepted. Q2: WIT packages are
0.1.0. Q15: the Arch `rust-wasm` package is installed (ships `wasm32-wasip2`). Q16:
transcripts are recorded from `copilot --acp` (GitHub Copilot CLI), which is also the
live smoke test's agent.

1. **Serialized calls block approvals, cancel and concurrent sessions.** 04 says calls into
   an instance are serialized, but `send` runs for the whole turn, so `respond-approval`
   and `cancel` (exports in the 05 sketch) could never reach the plugin during a turn, and
   two threads on one provider could not run turns at once. Recommend: one instance per
   provider session plus a control instance; approvals answered by a blocking
   `events.request-approval` import (the host keeps early answers); cancel signalled
   through imports (Q5); drop `respond-approval` and `cancel` from the provider exports.
   The kernel's `Provider` trait keeps both methods; `WasmProvider` implements them
   host-side. Cost: a few MB per concurrent session.
2. **WIT version 1.0.0 freezes the interfaces on merge** (spec README: they freeze at
   1.0), and the manifest's `api = "^1.0"` with them. With Q1 changing the sketch and M2
   still to come, I recommend **0.1.0** for all three packages and `api = "^0.1"`, like
   the protocol's 0.x, freezing at 1.0 before M4 lets third parties publish. The
   never-edit-a-published-version rule applies either way (the `PUBLISHED` test). If you
   prefer 1.0.0 now, the plan is otherwise unchanged.
3. **`deactivate` and per-instance lifecycle.** With several instances, `activate` runs
   once per instance. Recommend saying so in 04.
4. **"Emits a `core` event" on failure,** but the `core` topic is reserved and
   unsupported. Recommend for M1: log at `warn`, show `failure` in `plugin.list`, fail the
   running turn with the error; specify the `core` topic in M2 with the UI that needs it.
5. **Idle and cancel vs long-lived agent processes.** "No running session" would keep any
   ACP instance alive forever, and "processes terminated when a turn is cancelled" kills
   the agent and its in-memory context on every cancel. Recommend: idle means no running
   turn; on cancel the kernel ends the turn immediately (as now), the plugin's blocked
   import returns `cancelled` once so it can send ACP `session/cancel`, and the session's
   processes are killed only if `send` has not returned within a 2 s grace. Next turns
   resume through `resume`/`session/load`.
6. **Activation triggers for contributions.** The 05 declarative example has no
   `[activation]`, and a manifest that forgets `provider:<id>` would never run.
   Recommend: contributions activate their plugin implicitly; `activation.on` adds
   `startup` and the other triggers. `provider:<id>` stays valid (local id) and redundant.
7. **"Installed plugins" in `core.hello`.** Recommend enabled plugins only, with only
   their available contributions (matching "a tab shows no UI for a missing feature");
   `plugin.list` shows everything.
8. **M0 data.** Threads created by the scripted provider keep `provider: "scripted"` and
   fail with `plugin_failed` on send. Fine before 1.0; no migration.
9. **`plugin.install { path }` reads a file on the core's machine.** Right for M1 (local
   only); remote install (upload or registry id) is an added optional field in M3/M4.
10. **Preinstalled provider-acp.** 10-distribution lists it as preinstalled, but bundling
    the component inside the core binary would make every core build need the WASM
    toolchain. Recommend: M1 installs it with `kumikit plugin add`; the M2 installer
    ships and preinstalls it.
11. **Updates in M1.** "An update that widens permissions stays on the old version" needs
    side-by-side versions. Recommend: installing an installed id is a `conflict` in M1;
    updates come with M4.
12. **Secrets without a way to set them.** The provider world imports `secrets`, but
    `secret.set` is not in M1. Recommend: implement the import and the file backend now
    (tests seed the store through the kernel), add `secret.*` methods and the keychain
    backend in M2 with the settings UI. Alternative: add the three methods now (small).
13. **CPU limit wording.** "10 s of CPU per call" is implemented with epochs as 10 s of
    wall time spent in guest code between host calls. Recommend rewording 04 to say so.
14. **`files`, `network`, `tools` permissions** have no host interface in M1. Recommend
    they are accepted, shown at install and stored as granted, and enforced when their
    interfaces arrive; a component importing `wasi:http` is refused at enable until then.
15. **Local WASM toolchain.** `/usr/bin/cargo` (Arch) comes before rustup on `PATH` and
    has no `wasm32-wasip2` std. Either install Arch's `rust-wasm` package (if it ships
    wasip2 for 1.99.1) or let me run `rustup target add wasm32-wasip2 --toolchain 1.99.1`
    and build through the rustup toolchain. Which do you prefer?
16. **Recording real ACP output.** Recording costs tokens on your account and runs an agent
    in a temp workspace. Installed here and ACP-capable: `opencode acp`, `gemini
    --experimental-acp`, `kimi` (ACP mode). Recommend opencode or Gemini. May I run one,
    and which? Transcripts get scrubbed (session ids, paths, host and user names) before
    commit.
17. **Plugin host as a kernel service.** 03 lists the plugin host among kernel services,
    01 puts it in its own crate above the kernel. Recommend 03 says "kernel and plugin
    host" for that row; no code consequence.
18. **ADR 0008 and 0009 are still `Proposed`** although merged; the ADR README says they
    become `Accepted` on merge. Recommend 0008 → Accepted, 0009 → Superseded by 0011 in
    step 1/10.
19. **Kill on SIGKILL of the core (Unix).** Children survive a hard kill of the core
    (process groups don't help; `PR_SET_PDEATHSIG` needs `unsafe` `pre_exec` and is
    Linux-only). Recommend documenting the gap for M1.

## Notes

- Fixtures already use the provider id `example`, not `scripted`, so the scripted
  provider's removal changes no fixture by itself; qualified ids (`example.plugin/example`
  style) do.
- Local machine: only `x86_64-unknown-linux-gnu` std is installed for both the Arch and
  rustup toolchains. macOS and Windows code (job objects, process groups on macOS) is
  verified only in CI; each such step says so in its commit.
- Step 2 spike (Linux, debug build): compiling the hello component takes 233 ms; loading
  it from wasmtime's cache, instantiating and calling `activate` takes 1.4 ms (median of
  5). The safe cache API works, so Choice C needs no `unsafe` fallback. The guest is 15 KB.
- Step 2: wasmtime's call hook gets no access to the epoch deadline, so the call-time
  limit is the hook recording when the guest last entered wasm or returned from the host,
  plus an epoch deadline callback (every tick) that traps once the limit has passed and
  otherwise yields. Same behavior as planned.
- Step 2: wit-bindgen needs `generate_all` for imports from other packages, and a
  `cdylib` exporting `kumikit:plugin/lifecycle@0.1.0#activate` does not link natively
  (the symbol name breaks the linker's version script). The SDK's `export_plugin!`
  therefore expands to nothing off `wasm32`, which keeps native builds and unit tests of
  plugin crates working.
- Step 3: one SDK crate per world. wit-bindgen's export macros are `#[macro_export]`
  and named after the world and each exported interface, so two worlds that both export
  `lifecycle` cannot be generated in one crate, and a cargo feature would be unified
  across the workspace. `kumikit-plugin` (`sdk/rust`) has the base world;
  `kumikit-provider` (`sdk/rust-provider`) has the provider world and reuses the base
  crate's `types`, `log` and `kv` through `with:`. A test checks that the hello component
  imports no provider interface and that a provider linking both crates is one valid
  component. The approval `decision` enum lives in `kumikit:host/events` (the host
  package cannot depend on the provider package). WIT keywords `result` and `list` are
  escaped as `%result` and `%list`.
- Step 4: the supervisor keeps one process tree per child (a process group, or a job
  object on Windows), not per owner, so `child.kill` and dropping a child's last handle
  kill just that tree; killing an owner kills each of its trees concurrently. The
  Windows job-object module was not compiled locally; CI is its first build.
- Step 5: the kernel owns the supervisor (`Kernel::supervisor`, killed in `close`) and
  the secret store (`Kernel::secrets`, `secrets.json` next to the database by default).
  `Provider::plugin_id` (default: the provider id) names the plugin in `plugin.event`.
  Sessions remember their provider, so `unregister_provider` can cancel that provider's
  turns and forget its sessions without a database read. The data-directory lock is in
  `kumikit-core` (`lock.rs`), taken before the socket is bound.
- Step 6: manifest parsing is TOML, then the JSON Schema, then strict types (unknown
  fields refused), then the semantic rules, each with a test. The schema
  (`sdk/schema/plugin-manifest.schema.json`) covers structure only (names, types,
  enums); patterns and ranges are semantic checks, because their messages are clearer
  and they report together with the other rules. The validator (`jsonschema`) needs
  `fluent-uri`, licensed MIT-0, accepted 2026-11-15 and added to `deny.toml` and the
  AGENTS.md list. `check_backend_entry` validates `acp = { … }` tables against a
  backend's schema (used from step 8). Packages pack reproducibly and unpack
  defensively; `Manager` installs, enables, disables and removes plugins.
- 2026-11-15: WASI 0.3 reconsidered after Rust 1.100 made `wasm32-wasip3` tier 2. The
  host side is not ready (wasmtime 50.0.1 and 51.0.0-rc.1 mark `wasmtime-wasi` p3
  experimental, without security patch releases), so WASI 0.2 stays; ADR 0010 records
  the trigger to revisit. The toolchain pin moved to Rust 1.100.0 (this machine sets
  `RUSTUP_TOOLCHAIN=1.100.0`; its `wasm32-wasip2` std was added with rustup).
- Step 7: the runtime starts the engine and linkers on first use; enabling a plugin
  compiles and links its component (`Manager::prepared`), installing and opening start
  nothing (tested). Host imports check the instance's grants and log refusals; the probe
  plugin covers each permission, the sandbox (data directory as `/data`, no sockets, a
  `wasi:http` import refused at enable) and traps, memory and time limits. Gotcha found:
  the epoch deadline callback is a host call to the call hook (agent reference).
- Step 8: `PluginHost` (`lifecycle.rs`) registers providers from manifests and
  reconciles them on every change; idle unload and backoff use an injected clock and
  `unload_idle(now)`, so tests never sleep. Found and fixed a race: `disable` during a
  session's start left an instance behind. Not tested: the race itself (no
  deterministic way to hit the window).
