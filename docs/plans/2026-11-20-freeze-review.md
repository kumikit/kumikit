# Interface freeze review (ADR 0020)

Review of WIT `0.2.0` (`kumikit:host@0.2.0`, `kumikit:plugin@0.2.0`,
`kumikit:provider@0.2.0`), `kumikit:tool@0.1.0`, and UI SDK `0.1.2` after building M5's
plugins, before any 1.0.0 freeze. This is a review, not the freeze itself.

## What M5 exercised in practice

Every plugin was built strictly against the public SDKs (`kumikit-plugin`,
`kumikit-provider`, `kumikit-tool`, `@kumikit/ui-sdk/panel`) in separate repositories:

- **Declarative ACP providers** (`kumikit/kumikit-plugin-agents`): Antigravity (`agy acp`),
  GitHub Copilot CLI (`copilot --acp`), Cursor CLI (`cursor-agent acp`), OpenCode
  (`opencode acp`).
- **WASM provider adapter & backend** (`kumikit/kumikit-plugin-agents`):
  `kumikit.provider-claude` over Claude Code's native `stream-json` stdio protocol, plus
  the `claude` backend and `community.provider-deepseek` example.
- **Tool + workspace RPC + Level 2 UI panel** (`kumikit/kumikit-plugin-git`): `kumikit.git`
  (five read-only MCP tools + three workspace RPC methods) paired with `kumikit.ui-git`
  (sandboxed `inspector.tab` Changes panel).
- **Outgoing HTTPS + secrets + write approvals** (`kumikit/kumikit-plugin-github`):
  `kumikit.github` (five read tools, two approval-gated write tools over `wasi:http` with
  `x-secret` settings).
- **Schedules + client interface + declarative commands**
  (`kumikit/kumikit-plugin-schedules`): `kumikit.schedules` (declarative form command,
  cron timers, `new_thread` and `reuse_thread` modes).

## Findings by area

### 1. Always-present plugin exports (`rpc`, `commands`, `timers`)

WIT has no optional exports, so `kumikit:plugin@0.2.0`, `kumikit:provider@0.2.0`, and
`kumikit:tool@0.1.0` all export `rpc`, `commands`, and `timers`, with the Rust SDK's
`Plugin` trait defaulting each to `not-found`.

In practice this worked cleanly: `kumikit.git` lives in the `tool` world and also serves
three workspace-scoped RPC methods for `kumikit.ui-git` with zero extra boilerplate, and
`kumikit.schedules` implements `commands`, `rpc`, and `timers` in the base `plugin` world.

**Recommendation:** keep this model unchanged for 1.0.0.

### 2. Tool gateway (`mcp-server` and `kumikit:tool@0.1.0`)

Passing a stdio MCP server (`kumikit mcp --socket <socket>` + `KUMIKIT_MCP_TOKEN`) to
providers worked without opening any TCP port across Antigravity, OpenCode, Cursor, and
Claude Code (`--mcp-config`). `kumikit:tool/tool.call` with JSON-schema-validated input
and `effect = "read" | "write"` approvals proved straightforward to implement in both
`kumikit.git` and `kumikit.github`.

Two limitations to note before 1.0:
- **API-only providers** (path 3 of [05-providers](../spec/05-providers.md)) cannot spawn
  a subprocess to speak stdio MCP unless granted `spawn`. Adding an optional host import
  (e.g. `kumikit:host/tools`) later is purely additive and does not break `mcp-server`.
- **Copilot CLI 1.0.114** accepted `mcpServers` in `session/new` without error but did not
  expose the server's tools to its model during recording (likely workspace-trust or MCP
  allowlist behaviour in Copilot CLI itself). No Kumikit protocol change is needed.

**Recommendation:** keep `mcp-server` and `kumikit:tool` as designed; add a direct host
tool-call import as a minor addition if/when API providers arrive.

### 3. `kumikit:host/files`

No M5 plugin needed `kumikit:host/files`: `kumikit.git` spawns `git`, `kumikit.github`
takes `owner`/`repo` in tool inputs, `kumikit.schedules` stores jobs in `kv`, and
providers spawn their agent CLI. `permissions.files` remains shown, stored and validated
in manifests, but the WIT import does not exist yet.

**Recommendation:** do **not** invent and freeze an untested `kumikit:host/files` for
1.0.0. Keep it out of 1.0.0 and introduce it as a minor addition when a concrete plugin
needs sandboxed workspace file I/O.

### 4. `kumikit:host/client`, `workspaces = "linked"`, and WASI 0.3

- `kumikit.schedules` exercised `workspaces.list`, `threads.create`, `threads.send`, and
  `workspaces = "all"` smoothly.
- Because the Telegram bot moved out of M5, `workspaces = "linked"`, `approvals.respond`,
  and the `wasi:io/poll` subscription + HTTP multi-wait loop have only been exercised by
  core integration tests (`test-plugins/client`), not yet by a production bot plugin.
- `kumikit_plugin::http::send` (blocking `wasi:http@0.2`) was sufficient for
  `kumikit.github`. WASI 0.3 native async would simplify a long-polling bot, though
  WASI 0.2 pollables already work.

**Recommendation:** hold `kumikit:bot` and the 1.0.0 freeze of `kumikit:host/client`
until at least one real bot plugin (Telegram/Slack/Matrix) exercises `linked` workspaces,
`subscribe` + HTTP polling, and `approvals.respond` end to end. If `wasmtime` stabilises
WASI 0.3 before that milestone, evaluate adopting 0.3 before freezing 1.0.

### 5. Process supervisor and `provider-acp`

Recording real CLIs exposed two real-world gaps:
- Cursor CLI requires ACP `authenticate` (`cursor_login`) after `initialize` -> solved in
  `provider-acp` with optional `auth-method` in `acp.schema.json`.
- Linux CLIs that read credentials from the desktop keyring hang when `DISPLAY` /
  `WAYLAND_DISPLAY` / `DBUS_SESSION_BUS_ADDRESS` are stripped -> solved in the kernel
  process supervisor's `ENV_ALLOWLIST`.

Neither required changing WIT, which is good evidence that `kumikit:host/process` and
`kumikit:provider` sit at the right abstraction boundary.

### 6. UI SDK `0.1.2` (`@kumikit/ui-sdk/panel`)

Building `kumikit.ui-git` as a Level 2 sandboxed iframe panel (`api = "^0.1.2"`,
`requires.core = ["kumikit.git@^0.1"]`) worked cleanly:
- `connectPanel()` delivered theme tokens (`--kk-diff-added`, `--kk-diff-removed`,
  `--kk-surface-*`, etc.) via a constructed stylesheet compatible with the strict CSP.
- `ctx.connection.call('kumikit.git/...')` routed to the core plugin with the active tab's
  workspace enforced by the UI kernel.
- `ctx.connection.subscribe('workspace:<id>', ...)` let the panel refresh on
  `thread.updated` without polling or timers.
- Core plugin `api` (`^0.2`) and UI plugin `api` (`^0.1.2`) already evolve independently;
  keep them separately versioned at 1.0.

### 7. Binary size follow-up (post-M5)

The core release binary grew from 30.4 MiB (M4) to 33.9 MiB (budget 32 MiB) due to
`wasmtime-wasi-http`, `jiff`, `jsonschema`, and the additional WIT worlds. All other
budgets (cold start, idle memory with M5 plugins enabled, activation time/memory, UI
kernel size) remain well within budget. Binary size optimisation (fat LTO, `opt-level = "s"`
for wasmtime/Cranelift, feature trimming) is tracked as a separate post-M5 task.

## Verdict for M5 merge

No breaking changes are needed in WIT `0.2.0`, `kumikit:tool@0.1.0`, or UI SDK `0.1.2`
before M5 merges. They are ready to be recorded in `wit/PUBLISHED` on close-out.
