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

**Status:** built. The exit test runs three ways: on the core through the CLI and the
protocol (`crates/kumikit-core/tests/integration/exit_m4.rs`), in the browser build
(`web/e2e/tests/exit-m4.spec.ts`), and on the real desktop app over WebDriver
(`web/e2e-desktop/exit-m4.test.mjs`, Linux and Windows). Each one uses a third-party
publisher whose key is made for the test, a fixture registry on loopback that its own
checker accepts, and a fresh machine; the permissions are shown before the grant, and
calls outside it fail with `permission_denied`. The official registry
(`github.com/kumikit/registry`) is live: its CI runs `kumikit-registry check` on pull
requests and publishes `index.json` to `https://kumikit.github.io/registry/index.json`;
the `kumikit` publisher is its only (early) publisher so far. Plan:
[M4 — Ecosystem](../plans/2026-11-18-m4-ecosystem.md).

## M5 — Plugins

- General APIs first, each in its own step: secrets and plugin settings over the protocol;
  WIT 0.2.0 ([ADR 0022](../decisions/0022-wit-0-2-and-dropping-0-1.md)); plugin methods and
  core commands; outgoing HTTP ([ADR 0023](../decisions/0023-outgoing-http-for-plugins.md));
  a client interface with linked workspaces; schedules
  ([ADR 0024](../decisions/0024-schedules.md)); the tool gateway
  ([ADR 0025](../decisions/0025-tool-gateway.md)).
- Providers: Antigravity, GitHub Copilot CLI, Cursor CLI and OpenCode as declarative ACP
  manifests; Claude Code as a WASM adapter over its `stream-json` protocol, also a provider
  backend (DeepSeek through it as a community example).
- `kumikit.git` and its panel `kumikit.ui-git`, `kumikit.github`, `kumikit.schedules`.
- The first-party plugins live in their own repositories: `kumikit/kumikit-plugin-agents`
  (every agent provider), `kumikit/kumikit-plugin-git`, `kumikit/kumikit-plugin-github`
  and `kumikit/kumikit-plugin-schedules`. A Telegram bot comes in a later milestone.

Each is a separate plugin and none needs a kernel special case; where one needs something
the kernel lacks, a general API lands first.

**Exit:**
1. Each new provider replays recorded output of its real agent and runs a turn with an
   approval end to end from its package, against a fake agent.
2. An agent turn uses a plugin tool through the gateway (`kumikit.git`'s `status`); a write
   tool waits for an approval; a provider without the `tools` grant gets no gateway; a
   wrong or expired token is refused.
3. The `ui-git` panel shows the workspace's changes; an agent creates a GitHub pull request
   through the gateway after an approval, against a fake GitHub API; the token reaches no
   log, event or client.
4. A scheduled prompt runs at its time; `schedule.list` shows it; pausing stops it; missed
   runs follow their policy; with no schedules no timer is armed.
5. In the desktop app, a user installs `kumikit.git` and `kumikit.ui-git` from a registry,
   asks an agent about the workspace's changes, and the agent answers with the git tool
   while the panel shows the same changes.

**Status:** done (`v0.1.0-m5`, merged to `main`). All CI checks pass across Linux, macOS
and Windows: exit tests on the core (`crates/kumikit-core/tests/integration/exit_m5.rs`), in the
browser build (`web/e2e/tests/exit-m5.spec.ts`, Chromium and WebKit), and on the real
desktop app over WebDriver (`web/e2e-desktop/exit-m5.test.mjs`, Linux and Windows), and
each first-party plugin repository passes its own CI (`kumikit-plugin-agents`,
`kumikit-plugin-git`, `kumikit-plugin-github`, `kumikit-plugin-schedules`). All nine
signed first-party plugins are live in `github.com/kumikit/registry`
(`https://kumikit.github.io/registry/index.json`) and its starter pack, with automated
per-repository signing, GitHub Releases and registry PR workflows. The core release binary
is 33.7–33.9 MiB (allowed past its 32 MiB budget during M5; reported by
`scripts/footprint.sh`, not gated); optimising it is a separate task after M5. Plan:
[M5 — Plugins](../plans/2026-11-19-m5-plugins.md); interface freeze review:
[freeze review](../plans/2026-11-20-freeze-review.md).

## M6 — UI design and usability

A refined, minimal workspace shell and everyday usability polish, built as an amalgamation of
Zed's thin, keyboard- and icon-first chrome with Claude web's calm, distraction-free chat
experience, plus Orca-inspired workspace and session organization:

- **Thin toolbar and status bar**: compact top toolbar and bottom status bar with single-click
  icon controls and descriptive tooltips (with keybindings), replacing bulky text buttons.
- **Adjustable sidebars and plugin docks**: resizable, collapsible left and right sidebars
  persisted per client.
  - Left sidebar: Orca-inspired project/workspace and thread organization (tree and active
    threads views, quick single-click `+` thread creation per workspace, clear live status
    dots, and docked `sidebar.section` UI plugins).
  - Right sidebar (inspector dock): hosts `inspector.tab` UI plugins (such as `kumikit.ui-git`)
    with a thin tab switcher and one-click dock toggle from the toolbar and status bar.
- **Main panel splits**: recursive horizontal and vertical resizable splits in the main area,
  each pane with its own tab strip, active focus indicator, draggable split divider, and
  keyboard/toolbar controls so multiple threads or views run side by side.
- **Calm conversation UI**: centered readable transcript column, subtle user bubbles, clean
  markdown/code/diff presentation, compact collapsible tool calls and reasoning, and a focused
  composer with icon send/stop controls.
- **Iterative usability fixes**: interactive design passes to tighten workflows (quick thread
  creation, remembered provider/model, thread management, settings/plugins/cores visual
  consistency, keyboard navigation across splits).

**Status:** done (merged to `main`). Plan: [M6 — UI design and usability](../plans/2026-11-22-m6-ui-design.md).

**Exit:**
1. Left and right sidebars resize smoothly, collapse/expand via toolbar icons, status bar dock
   toggles and keyboard shortcuts, and persist their dimensions and state.
2. `inspector.tab` UI plugins (e.g., `kumikit.ui-git`) mount, switch and follow the active
   workspace in the right sidebar without special cases in the UI kernel.
3. The main workspace supports horizontal and vertical resizable splits with independent tabs
   and threads, persisted across reloads.
4. All existing accessibility (WCAG AA axe checks in light and dark themes), UI rule checks,
   bundle size budget (≤ 150 KB gzipped), unit tests and desktop/browser exit tests pass.

## M7 — Threads

The chat panel as a full working surface for agent threads, built as general kernel
features that every provider and client gets:

- A durable, editable message queue that is held, not dropped, after a stop, a failure or
  a restart.
- Steering a running agent through a host input inbox (`kumikit:turn@0.1.0`) and a `steer`
  model capability, falling back to the queue.
- Switching provider in an idle thread with a bounded handoff of the conversation, and
  per-turn provider attribution.
- Deferred messages: send at a time, after another thread's turn, or after a command
  exits or succeeds (a build, CI); agent tools for them in a first-party `kumikit.threads`
  plugin (`kumikit:threads@0.1.0`).
- Forks, edit-and-resend, `/btw` side threads, provider slash commands and usage limits
  (`kumikit:session@0.1.0`), `@` mentions, turn cards, per-turn changes, find in thread,
  usage-limit holds and opt-in turn checks.

**Status:** done. Plan:
[M7 — Threads](../plans/2026-11-24-m7-threads.md).

**Exit:** see the plan's exit criteria: queue held across restart; a steer lands mid-turn
on Claude Code; a Claude Code thread hands off to an ACP agent with its history; a
wake-up fires when a build command succeeds; edit-and-resend forks; all existing checks
pass.

## M8 — Application updates and SSH remote sync

Signed, idle-safe application updates, matching remote core installation over SSH, and
distribution/desktop polish:

- Unified Ed25519-signed `release.json` manifest for desktop installers and core archives
  ([ADR 0031](../decisions/0031-signed-release-manifest-and-updates.md)), with build-time
  release version stamping.
- Settings → Updates page displaying current version, update availability, manual install, and
  opt-in automatic updates that wait until all agents are idle (`core.activity` and
  `core.shutdown { ifIdle: true }`, protocol `0.10`).
- `kumikit update` CLI subcommand for headless and TLS-paired cores (also replacing `$APPIMAGE`
  when run via AppImage CLI forwarding, and syncing bundled `core-plugins/*.kkp`).
- Verified client-side download and SSH streaming installation of matching `kumikit` binaries
  into `~/.local/bin/kumikit` (plus `core-plugins/*.kkp` into `~/.local/share/kumikit/core-plugins/`),
  prompted by default with optional per-profile or global auto-sync
  ([ADR 0032](../decisions/0032-ssh-remote-core-sync.md)).
- Follow-up polish (`v0.1.0-m8.1` and after): bundled `kumikit.threads` alongside
  `kumikit.provider-acp` in `core-plugins/` across desktop installers, standalone CLI
  archives, and SSH sync (with auto-upgrade/restore on startup and idle local core
  rollover), AppImage CLI forwarding via `AppRun`, borderless window chrome (native macOS
  overlay traffic lights, Linux tiling WM detection, and **Window buttons: Auto / Show /
  Hide** in Settings → General), finished turn summary cards with a per-turn diff overlay
  and Send/Stop swap in `ui-chat`, and a unified model picker with recent models, provider
  icons, and per-model settings (`kumikit:models@0.1.0`, protocol `0.11`, UI SDK `0.1.6`,
  [ADR 0033](../decisions/0033-model-settings-and-provider-icons.md)).

**Status:** done (`v0.1.0-m8` and `v0.1.0-m8.1`, merged to `main` and released on GitHub
Releases with desktop installers, CLI archives, signed `release.json` / `release.json.sig`,
and the `release-manifest` branch). Plan:
[M8 — Application updates and SSH remote sync](../plans/2026-11-25-m8-updates-and-remote-sync.md).

**Exit:** see the plan's exit criteria: signed update check, download, idle-gated auto-install,
`kumikit update` CLI, and SSH matching core install all pass deterministic tests alongside all
existing workspace checks.

## M9 — Agent orchestration

An open milestone: features that let agents coordinate other agents, added as they come.

- Agents start threads with a chosen provider, model, model settings and permission mode
  (`start_thread` and `list_models` in `kumikit.threads`, `kumikit:threads@0.2.0`,
  [ADR 0034](../decisions/0034-agents-start-threads.md)). A plugin never creates a
  `full_access` thread, nor one looser than the thread it acts for.

**Status:** in progress. Plan:
[M9 — Agent orchestration](../plans/2026-11-28-m9-agent-orchestration.md).

**Exit:** set when the milestone closes; each feature ships with its tests and passes all
existing checks.

## Open questions

| Question | Decide by |
|---|---|
| WASI 0.3 (native async) adoption timing | Decided for now: WASI 0.2 ([ADR 0010](../decisions/0010-wasi-p2-and-session-instances.md)); revisit when wasmtime ships WASI 0.3 as stable (Rust's `wasm32-wasip3` target is tier 2 since 1.100) |
| SQL storage for plugins beyond key-value | When a plugin needs it |
| Whether themes may ship restricted CSS in v2 | After M4, from theme authors' feedback |
| Sidecar plugins: ship in 1.0 or later | Decided: not in 1.0 ([ADR 0021](../decisions/0021-no-sidecars-in-1-0.md)) |
| Freezing WIT and the UI SDK at 1.0 | After M5 ([ADR 0020](../decisions/0020-interface-freeze-after-m5.md)) |
| Mobile client over the same protocol | After 1.0 |
