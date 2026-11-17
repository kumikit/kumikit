# 00 — Overview

## Goals

1. **Clear client/core split.** The core is a headless Rust process that owns every piece
   of state. The client renders and sends commands. The same client can point at any core,
   local or remote, and at several at once.
2. **Pluggable on both sides.** The community can add core plugins (agent providers, tools,
   bots, integrations) and client plugins (panels, widgets, renderers, themes) without
   forking.
3. **A small default.** Only a handful of plugins are preinstalled; everything else is the
   user's choice to add.
4. **Small footprint.** Rust core, WASM plugins that load lazily, a system webview rather
   than a bundled browser engine.
5. **Private by default.** No accounts, no telemetry, no network traffic the user did not
   start or opt into.

## Non-goals (for 1.0)

- A bundled browser engine (Electron, CEF).
- Native-code plugins (`.so`/`.dylib`/`.dll`) loaded into the core.
- A hosted service. Kumikit runs on the user's machines; remote means "another machine
  the user runs a core on".
- A built-in agent loop that competes with existing agent CLIs. Kumikit drives agents
  through provider plugins; a native loop can be a plugin later.
- Mobile clients. The protocol must allow them; building one is out of scope for 1.0.

## Principles

These are the rules that make the architecture hold. They are repeated as working rules
in [AGENTS.md](../../AGENTS.md).

- **The kernel knows no plugin by name.** No provider, bot, theme or tool is mentioned in
  kernel or UI-kernel code. If a feature needs a special case for one plugin, the plugin
  API is missing something.
- **First-party plugins use only the public SDK.** They are built and installed the same
  way as community plugins. Being first-party grants no private API.
- **The protocol is the contract.** The client and the core share nothing but
  `kumikit-protocol`. A bot plugin can do anything a client can, through the same surface.
- **Default deny.** A plugin can use a host capability (files, network, processes,
  secrets) only if its manifest declares it and the user approved it.
- **Lazy everything.** Nothing is instantiated until something needs it; idle plugins are
  unloaded.
- **Every surface has an instant path.** UI never waits on an animation, a plugin or the
  network to accept input.

## Footprint budgets

Measured on a release build, idle, with the preinstalled plugins only. The core's
budgets are checked by `sh scripts/footprint.sh` (median of 5 runs, each from a fresh
data directory; memory is sampled one second after a client's `core.hello`), which CI
runs on Linux: cold start and idle memory without plugins and with provider-acp and a
declarative plugin enabled, the time and memory of the first activation, and the size of
the core binary. The others become checks when their parts exist (see
[12-roadmap.md](12-roadmap.md)).

| Measure | Budget |
|---|---|
| Core resident memory, idle | ≤ 30 MB |
| Core cold start to accepting connections | ≤ 300 ms |
| Activating a typical WASM plugin (the first in a core) | ≤ 50 ms, ≤ 16 MB ([ADR 0012](../decisions/0012-activation-memory-budget.md)) |
| Core binary, release build, uncompressed | ≤ 32 MiB ([ADR 0012](../decisions/0012-activation-memory-budget.md)) |
| Desktop shell anonymous memory, excluding the webview's processes; Linux, gated (macOS and Windows reported) | ≤ 60 MB ([ADR 0015](../decisions/0015-shell-memory-budget.md)) |
| UI kernel JavaScript, compressed | ≤ 150 KB |
| Installer size, per platform (`.deb`, `.rpm`, `.dmg`, NSIS `.exe`; an AppImage bundles WebKitGTK and is not counted) | ≤ 20 MB |

## Glossary

| Term | Meaning |
|---|---|
| **Core** | The headless `kumikit` process: kernel plus core plugins. One per machine (or per user profile). |
| **Kernel** | The non-plugin part of the core: protocol server, domain model, storage, secrets, plugin host. |
| **Client** | Anything that speaks the protocol to a core. Usually the desktop app; also a browser, a bot or a script. |
| **Shell** | The desktop app's native part (Tauri): window, local core discovery, client secret storage. |
| **UI kernel** | The Svelte application in the webview: connections, tabs, slots, commands, theme engine. |
| **Plugin** | A versioned package that extends the core or the client. "Piece" is an informal name for it; code and docs say plugin. |
| **Core plugin** | A plugin that runs inside the core, as a WASM component (or, as an escape hatch, a sidecar process). |
| **UI plugin** | A plugin that runs in the client's webview. |
| **Theme** | A data-only UI plugin: design tokens and optional fonts, no code. |
| **Provider** | A core plugin that drives an agent (usually an agent CLI) and turns its output into kernel events. |
| **Connection** | A client's link to one core, local or remote. |
| **Tab** | A client view bound to one `(connection, workspace)` pair. |
| **Workspace** | A project directory known to a core. |
| **Thread** | A conversation with an agent inside a workspace; an append-only list of events. |
| **Contribution** | Something a plugin's manifest adds: a provider, command, setting, slot entry, tool, theme. |
| **Slot** | A named place in the UI where plugins can contribute, such as `inspector.tab`. |
