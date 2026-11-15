# 01 — Architecture

## Processes

```
┌──────────────── Client: Kumikit desktop (Tauri shell + Svelte UI kernel) ──────┐
│  Tabs: [local · ~/proj-a] [devbox · ~/svc] [vm-3 · ~/x]                        │
│  UI kernel: connections, tabs, slots, commands, theme engine, plugin manager   │
│  UI plugins (installed per client): panels, widgets, renderers, themes         │
└───────┬────────────────────────────┬────────────────────────────┬──────────────┘
        │ Unix socket / named pipe   │ WebSocket over TLS         │ WebSocket over SSH tunnel
┌───────▼─────────┐         ┌────────▼────────┐          ┌────────▼────────┐
│ kumikit serve   │         │ kumikit serve   │          │ kumikit serve   │
│ (this machine)  │         │ (devbox)        │          │ (vm-3)          │
│ kernel          │         │                 │          │                 │
│ core plugins    │         │                 │          │                 │
│  └ spawned CLIs │         │                 │          │                 │
└─────────────────┘         └─────────────────┘          └─────────────────┘
```

- **`kumikit` (core binary).** Headless. `kumikit serve` runs the core; other subcommands
  (`kumikit plugin add`, `kumikit pair`, `kumikit status`) are thin clients of a running
  core, or operate on its data directory when none runs. The same binary is used on a
  laptop and on a server.
- **Kumikit desktop (client).** A Tauri 2 app using the system webview (WKWebView,
  WebView2, WebKitGTK). It finds a local core on its socket or starts one, and connects to
  remote cores the user has paired with. It never opens project files or runs agents
  itself.
- **Browser client.** A core can optionally serve the same UI bundle over HTTPS, so a
  plain browser is a client. Off by default.

The desktop shell and the core are separate processes even on one machine. Closing the
window does not stop running agents; the core keeps working and the next client to
connect catches up from the event log.

## Repository layout

```
crates/
  kumikit-protocol/      # protocol types and method table; generates TypeScript
  kumikit-kernel/        # domain model, event log, storage, secrets, processes, permissions
  kumikit-plugin-host/   # wasmtime component host, WIT bindings, sidecar host (later)
  kumikit-core/          # the `kumikit` binary: transports, auth, CLI
  kumikit-shell/         # Tauri desktop shell
wit/                     # WIT packages: kumikit:host, kumikit:provider, kumikit:bot, …
web/
  kernel/                # Svelte 5 UI kernel
  sdk/                   # @kumikit/ui-sdk: types and helpers for UI plugin authors
  protocol/              # @kumikit/protocol: generated from kumikit-protocol, never edited
sdk/
  rust/                  # kumikit-plugin crate: guest-side bindings and helpers
plugins/                 # first-party plugins, built with the public SDKs only
  provider-acp/
  ui-chat/
  ui-settings/
  theme-default/
docs/
  spec/                  # this specification
  decisions/             # architecture decision records
  plans/                 # working plans for multi-step changes
```

## Dependency rules

```
kumikit-protocol  ←  kumikit-kernel  ←  kumikit-plugin-host  ←  kumikit-core
kumikit-protocol  ←  kumikit-shell
@kumikit/protocol ←  web/kernel, web/sdk
wit/              ←  kumikit-plugin-host, sdk/rust, plugins/*
```

- `kumikit-protocol` depends on nothing in the workspace. It has no I/O.
- `kumikit-shell` and `web/` never depend on `kumikit-kernel`: the client knows the core
  only through the protocol.
- `plugins/*` depend only on `wit/`, `sdk/`, `web/sdk` and `@kumikit/protocol`.
- No crate or package outside `plugins/` names a plugin.

## Technology

| Part | Choice | Why (ADR) |
|---|---|---|
| Core, shell | Rust stable, edition 2024, tokio | Footprint, one language for native code |
| Plugin runtime | wasmtime, WASM component model, WIT, WASI 0.2 | [ADR 0003](../decisions/0003-wasm-first-core-plugins.md) |
| Protocol | JSON-RPC 2.0, schema in Rust, TS generated | [ADR 0005](../decisions/0005-json-rpc-protocol.md) |
| Storage | SQLite (WAL) via rusqlite | Single file, no server, proven |
| Desktop | Tauri 2, system webview | [ADR 0002](../decisions/0002-separate-core-and-client.md) |
| UI kernel | Svelte 5 (runes), TypeScript strict, Vite | [ADR 0004](../decisions/0004-svelte-ui-kernel.md) |
| Plugin packages | tar.zst + manifest + ed25519 signature | [10-distribution.md](10-distribution.md) |

## Data directories

| Platform | Core data | Client data |
|---|---|---|
| Linux | `$XDG_DATA_HOME/kumikit/core` | `$XDG_DATA_HOME/kumikit/client` |
| macOS | `~/Library/Application Support/dev.kumikit/core` | `…/dev.kumikit/client` |
| Windows | `%APPDATA%\kumikit\core` | `%APPDATA%\kumikit\client` |

Core data holds the database, installed core plugins, plugin data and the secret store.
Client data holds connection profiles, layout, installed UI plugins and themes. Neither
directory is shared between the two.
