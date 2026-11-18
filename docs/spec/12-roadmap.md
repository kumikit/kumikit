# 12 — Roadmap

Each milestone ends with exit criteria that can be checked. M0–M2 deliver the pluggable
framework; later milestones add plugins on top of it.

## M0 — Protocol and kernel

**Status:** done. The exit test (`crates/kumikit-core/tests/exit_m0.rs`) passes in CI on
Linux, macOS and Windows.

- Cargo workspace with `kumikit-protocol`, `kumikit-kernel`, `kumikit-core`.
- `kumikit serve` on the local socket (Linux, macOS) and named pipe (Windows); `core.hello`, `workspace.*`, `thread.*`,
  `events.subscribe` with replay from a cursor.
- SQLite storage with migrations; event log.
- A deterministic scripted provider compiled into the core behind an internal trait, so
  tests need no real agent (temporary; deleted in M1, see
  [ADR 0009](../decisions/0009-temporary-scripted-provider.md)).
- TypeScript generation for `@kumikit/protocol`.

**Exit:** a test client script creates a thread, sends a message, receives streamed
events, disconnects, reconnects from its cursor and sees no gap or duplicate.

## M1 — Plugin host

- `kumikit-plugin-host` with wasmtime, `kumikit:host`, `kumikit:plugin`, `kumikit:provider`.
- Manifest validation, permissions, lifecycle, activation triggers, idle unload, limits.
- `sdk/rust` guest crate; `plugins/provider-acp` built with it.
- The M0 compiled-in provider is deleted.
- `kumikit plugin add/list/remove` for local `.kkp` files.

**Status:** done. The exit tests (`crates/kumikit-core/tests/exit_m1.rs`) pass in CI on
Linux, macOS and Windows, replaying recorded Claude Code transcripts through
provider-acp; the live smoke test against real Claude Code passed on Linux. Plan:
[M1 — Plugin host](../plans/2026-11-15-m1-plugin-host.md).

**Exit:** an ACP agent runs through `provider-acp` with no provider-specific code in the
kernel; a plugin that calls an unpermitted host function gets `permission_denied`; idle
footprint is within the budget in [00-overview.md](00-overview.md#footprint-budgets).

## M2 — Client

- `kumikit-shell` (Tauri 2) with local core discovery and the connection bridge.
- `web/kernel` (Svelte 5): connection manager, tabs, thread store, slots, commands, theme
  engine, declarative renderer, plugin manager.
- `plugins/ui-chat`, `plugins/ui-settings`, `plugins/theme-default`.
- `@kumikit/ui-sdk` with inline modules (level 3) and declarative contributions (level 1).

**Exit:** from the desktop app, a user installs an ACP provider, starts a thread, approves
a tool call and switches themes; closing and reopening the window resumes the running turn.

**Status:** built. The exit test (`web/e2e-desktop/exit.test.mjs`) drives the real app over
WebDriver with a recorded Claude Code transcript and passes on Linux and Windows (CI). macOS has no WebDriver for its webview, so the same flow runs there in the browser
build (`web/e2e`, Playwright WebKit). A live run against real Claude Code passed. Plan:
[M2 — Client](../plans/2026-11-16-m2-client.md).

## M3 — Multiple cores

- WebSocket/TLS transport, pairing, device tokens, revocation, SSH tunnels.
- Tabs bound to different connections; merged sidebar; per-connection reconnect.
- `requires.core` gating for UI plugins.
- The browser client: a core serves the UI over HTTPS, with pairing from the page.

**Exit:** one window shows tabs from a local core and a remote core; killing the remote's
network shows "reconnecting" only in its tabs, and the local tabs keep streaming.

**Status:** built. The exit test (`web/e2e-desktop/exit-m3.test.mjs`) drives the real app
over WebDriver with a remote core behind a cuttable network (`test-plugins/net-cut`) and
recorded Claude Code transcripts; it passes on Linux and Windows (CI). The same flow runs
in the browser build with Playwright (`web/e2e/tests/cores.spec.ts`), WebKit included on
macOS. Plan: [M3 — Multiple cores](../plans/2026-11-17-m3-multiple-cores.md).

## M4 — Ecosystem

- Package signing, the git registry and its CI, `kumikit plugin add <id>` from the
  registry, updates, lockfiles.
- Sandboxed iframe panels (level 2) and the postMessage bridge.
- Theme validation tooling (`kumikit theme check`).
- Plugin author templates: core plugin (Rust), UI plugin (Svelte), theme.
- The starter pack on first run.
- WIT and the UI SDK stay 0.x; they freeze at 1.0 after M5, when the official registry
  opens to third parties ([ADR 0020](../decisions/0020-interface-freeze-after-m5.md)).

**Exit:** a plugin published to the registry by a third party installs on a fresh machine,
with its permissions shown and enforced.

**Status:** in progress. Plan: [M4 — Ecosystem](../plans/2026-11-18-m4-ecosystem.md).

## M5 — Plugins

Adapters for popular agent CLIs, the tool gateway (MCP), git and GitHub, a Telegram bot,
the VS Code theme importer, schedules. Each is a separate plugin and none needs a kernel
change; if one does, the kernel change lands first as a general API.

## Open questions

| Question | Decide by |
|---|---|
| WASI 0.3 (native async) adoption timing | Decided for now: WASI 0.2 ([ADR 0010](../decisions/0010-wasi-p2-and-session-instances.md)); revisit when wasmtime ships WASI 0.3 as stable (Rust's `wasm32-wasip3` target is tier 2 since 1.100) |
| SQL storage for plugins beyond key-value | When a plugin needs it |
| Whether themes may ship restricted CSS in v2 | After M4, from theme authors' feedback |
| Sidecar plugins: ship in 1.0 or later | Decided: not in 1.0 ([ADR 0021](../decisions/0021-no-sidecars-in-1-0.md)) |
| Freezing WIT and the UI SDK at 1.0 | After M5 ([ADR 0020](../decisions/0020-interface-freeze-after-m5.md)) |
| Mobile client over the same protocol | After 1.0 |
