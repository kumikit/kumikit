# 01 — Architecture

## Processes

```
┌──────────────── Client: Kumikit desktop (Tauri shell + Svelte UI kernel) ──────┐
│  Tabs: [local · ~/proj-a] [devbox · ~/svc] [vm-3 · ~/x]                        │
│  UI kernel: connections, tabs, slots, commands, theme engine, plugin manager   │
│  UI plugins (installed per client): panels, widgets, renderers, themes         │
└───────┬────────────────────────────┬────────────────────────────┬──────────────┘
        │ Unix socket / named pipe   │ WebSocket over TLS         │ stdio over SSH
┌───────▼─────────┐         ┌────────▼────────┐          ┌────────▼────────┐
│ kumikit serve   │         │ kumikit serve   │          │ kumikit serve   │
│ (this machine)  │         │ (devbox)        │          │ (vm-3)          │
│ kernel          │         │                 │          │                 │
│ core plugins    │         │                 │          │                 │
│  └ spawned CLIs │         │                 │          │                 │
└─────────────────┘         └─────────────────┘          └─────────────────┘
```

- **`kumikit` (core binary).** Headless. `kumikit serve` runs the core; other subcommands
  (`kumikit plugin add`, `kumikit pair`, `kumikit devices`, `kumikit bridge`) are thin clients of a running
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
  kumikit-protocol/      # protocol types, method table, local address; generates TypeScript
  kumikit-package/       # .kkp packing and defensive unpacking, shared by core and shell
  kumikit-assets/        # UI files: path checks, MIME types, CSPs, UI manifest structure; shell, core, registry checker
  kumikit-registry/      # registry index, fetching, resolution, install-time checks
  kumikit-registry-tool/ # `kumikit-registry check | build-index`, run by a registry's CI
  kumikit-theme/         # theme checks (token types, contrast, fonts) for `kumikit theme check`
  kumikit-kernel/        # domain model, event log, storage, secrets, processes, permissions
  kumikit-plugin-host/   # wasmtime component host, WIT bindings, sidecar host (later)
  kumikit-core/          # the `kumikit` binary: transports, auth, CLI
  kumikit-shell/         # Tauri desktop shell: window, bridge, client plugin store
wit/                     # WIT packages: kumikit:host, kumikit:provider, kumikit:bot, …
web/
  kernel/                # Svelte 5 UI kernel
  sdk/                   # @kumikit/ui-sdk: types and helpers for UI plugin authors
  e2e/                   # Playwright tests of the browser build against a real core
  e2e-desktop/           # WebDriver tests of the desktop app
  protocol/              # @kumikit/protocol: generated from kumikit-protocol, never edited
sdk/
  rust/                  # kumikit-plugin crate: guest bindings for the base world, helpers
  rust-provider/         # kumikit-provider crate: the provider world, on top of kumikit-plugin
  schema/                # JSON Schema of plugin.toml
plugins/                 # first-party plugins, built with the public SDKs only
  provider-acp/
  ui-chat/
  ui-settings/
  theme-default/
templates/               # plugin author templates: core plugin (Rust), UI plugin (Svelte), theme
test-plugins/            # plugins and helper programs used only by tests
docs/
  spec/                  # this specification
  decisions/             # architecture decision records
  plans/                 # working plans for multi-step changes
```

## Dependency rules

```
kumikit-protocol  ←  kumikit-kernel  ←  kumikit-plugin-host  ←  kumikit-core
kumikit-protocol  ←  kumikit-shell
kumikit-package   ←  kumikit-plugin-host, kumikit-registry, kumikit-core, kumikit-shell
kumikit-registry  ←  kumikit-core, kumikit-shell, kumikit-registry-tool
kumikit-plugin-host ← kumikit-registry-tool (to validate core manifests as install does)
kumikit-theme     ←  kumikit-core
kumikit-assets    ←  kumikit-core, kumikit-shell, kumikit-registry-tool
@kumikit/protocol ←  web/kernel, web/sdk
@kumikit/ui-sdk   ←  web/kernel, plugins/ui-*
wit/              ←  kumikit-plugin-host, sdk/rust*, plugins/*, test-plugins/*
```

- `kumikit-protocol` depends on nothing in the workspace. It has no I/O.
- `kumikit-package` and `kumikit-assets` depend on nothing in the workspace and know no
  kernel types.
- `kumikit-registry` depends only on `kumikit-package` and `kumikit-protocol`;
  `kumikit-theme` on nothing in the workspace. Neither knows kernel types.
- `templates/` are built like third-party projects: outside the cargo and npm workspaces,
  against the public SDKs only.
- `kumikit-shell` and `web/` never depend on `kumikit-kernel`: the client knows the core
  only through the protocol.
- `plugins/*` and `test-plugins/*` depend only on `wit/`, `sdk/`, `web/sdk` and
  `@kumikit/protocol` (test helpers may also use test-only crates).
- No crate or package outside `plugins/` and `test-plugins/` names a plugin;
  `scripts/check-plugin-names.sh` checks `crates/`, `web/kernel/src` and `web/sdk/src`.

## Technology

| Part | Choice | Why (ADR) |
|---|---|---|
| Core, shell | Rust stable, edition 2024, tokio | Footprint, one language for native code |
| Plugin runtime | wasmtime, WASM component model, WIT, WASI 0.2; guests built for `wasm32-wasip2` with wit-bindgen | [ADR 0003](../decisions/0003-wasm-first-core-plugins.md), [ADR 0010](../decisions/0010-wasi-p2-and-session-instances.md) |
| Protocol | JSON-RPC 2.0, schema in Rust, TS generated | [ADR 0005](../decisions/0005-json-rpc-protocol.md) |
| Remote transport | rustls (ring, TLS 1.3), hyper, tokio-tungstenite, rcgen; the system `ssh` | [ADR 0016](../decisions/0016-remote-transport-and-pairing.md), [ADR 0017](../decisions/0017-shell-remote-profiles-and-ssh.md) |
| Storage | SQLite (WAL) via rusqlite | Single file, no server, proven |
| Desktop | Tauri 2, system webview | [ADR 0002](../decisions/0002-separate-core-and-client.md) |
| UI kernel | Svelte 5 (runes), TypeScript strict, Vite | [ADR 0004](../decisions/0004-svelte-ui-kernel.md) |
| Plugin packages | tar.zst + manifest + ed25519 signature | [10-distribution.md](10-distribution.md) |
| Signing, registry | ed25519 through `ring`; `index.json` over HTTPS (hyper client, OS trust store via `rustls-native-certs`) | [ADR 0018](../decisions/0018-package-signing-and-registry.md) |
| Sandboxed UI panels | iframe sandbox, a CSP per panel, `MessagePort` bridge | [ADR 0019](../decisions/0019-sandboxed-panels.md) |

## Data directories

| Platform | Core data | Client data |
|---|---|---|
| Linux | `$XDG_DATA_HOME/kumikit/core` | `$XDG_DATA_HOME/kumikit/client` |
| macOS | `~/Library/Application Support/dev.kumikit/core` | `…/dev.kumikit/client` |
| Windows | `%APPDATA%\kumikit\core` | `%APPDATA%\kumikit\client` |

`kumikit serve --data-dir <path>` overrides the core data directory (created with mode
0700 on Linux and macOS). Logs go to stderr; `KUMIKIT_LOG` sets the level (`info`,
`kumikit_core=debug`, …, default `warn`), and message bodies are never logged at `info`
or above.

Core data holds the database, installed core plugins, plugin data and the secret store.
Client data holds connection profiles, layout, installed UI plugins and themes, and the
webview's own storage, where the UI kernel keeps client settings. Neither directory is
shared between the two.
