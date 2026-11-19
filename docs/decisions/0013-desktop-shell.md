# 0013 — The desktop shell: own asset protocol, raw-line bridge, client plugin store

- **Status:** Accepted
- **Date:** 2026-11-16

## Context

[ADR 0002](0002-separate-core-and-client.md) makes the desktop app a thin Tauri 2 shell
with no feature commands. Building it raised four questions the spec did not settle:

1. **Serving the UI.** Tauri normally embeds the frontend at compile time
   (`frontendDist`). Then `cargo build` of the shell needs a built `web/`, so Node becomes
   part of every Rust build and test. The shell must also serve UI plugins from
   `kumikit-plugin://<id>/` with a strict CSP.
2. **The bridge.** The UI reaches cores only through the shell. The options were Tauri
   events, commands with an `ipc::Channel`, or a localhost WebSocket.
3. **Installing UI plugins.** 10-distribution says UI plugins and themes are installed
   "locally", but only native code can write the client data directory, and 06 says the
   shell has no feature commands.
4. **Starting the core.** AGENTS.md says external programs run only through the kernel's
   process supervisor, but the shell cannot depend on the kernel, and it has to start
   `kumikit serve` when no core answers.

## Decision

- **UI from the shell's own protocol.** The UI kernel's built files are app resources,
  served from `kumikit-app://localhost/` by the same responder that serves plugins from
  `kumikit-plugin://<id>/`. Every response carries the CSP. Rust builds need no Node.
  Where a webview exposes custom schemes as `http://<scheme>.localhost/` (WebView2), the
  platform's form is used and the CSP lists it.
- **A raw-line bridge.** The bridge has three commands: `bridge_connect(profile, channel)`,
  `bridge_send(connection, line)` and `bridge_close(connection)`. Incoming lines arrive in
  order on the `ipc::Channel` given at connect. The shell does not parse JSON-RPC; it
  enforces the 8 MiB line limit and reports when a connection closes. Profiles are names,
  not addresses (M2 knows only `local`), so the webview cannot reach arbitrary sockets.
- **A client plugin store in the shell.** It has three commands, `client_plugins_list`,
  `client_plugins_install(path)` and `client_plugins_remove(id)`. They unpack UI and theme
  packages into the client data directory, check their manifest's structure and list the
  bundled plugins next to the installed ones. This is client file management, like serving
  plugin files, not a product feature. Enabled state and trust live in the UI kernel's
  client settings.
- **The shell starts its own core.** It is the one exception to "external programs run
  only through the kernel's process supervisor": the shell may start exactly one program,
  the core binary bundled with it, by absolute path, never from `PATH`. The core starts
  detached (its own process group on Unix; a new process group without a console on
  Windows), so it keeps running when the window closes.
- The only Tauri plugin is the file dialog, with only `dialog:allow-open` granted.

## Consequences

- `cargo test --workspace` builds and tests the shell without Node. Linux CI needs the
  WebKitGTK development packages.
- The UI kernel's files are on disk at runtime. Another process of the same user could
  change them, but a same-user attacker is outside the threat model ([11](../spec/11-security.md)).
- M3 adds remote profiles to `bridge_connect` without changing the bridge API.
- A UI plugin is never installed on a core, and a core plugin never on the client: each
  installer refuses the other's packages.
- AGENTS.md's supervisor rule names this exception.
