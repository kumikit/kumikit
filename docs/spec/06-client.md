# 06 — Client

The client is the Kumikit desktop app: a Tauri 2 **shell** around a Svelte 5 **UI kernel**
running in the system webview. It holds no project state; everything it shows comes from
cores over the protocol.

## Shell (Rust, `kumikit-shell`)

- Creates the window and serves the bundled UI from a custom protocol.
- **Local core:** looks for the socket; if no core answers, starts `kumikit serve` as a
  detached process (so agents keep running when the window closes) and connects.
- **Connection bridge:** the UI talks to cores through the shell (`connect`, `send`,
  receive notifications). Remote TLS, certificate pinning and SSH tunnels live here, not
  in JavaScript.
- **Client secrets:** device tokens for paired cores, stored in the OS keychain or a 0600
  file. They never reach the webview; the shell attaches them when connecting.
- **UI plugin files:** serves installed UI plugins from `kumikit-plugin://<id>/` with a
  strict CSP.
- No other native commands. Features belong in the core or in plugins.

## UI kernel (Svelte, `web/kernel`)

Svelte 5 with runes, TypeScript strict, Vite
([ADR 0004](../decisions/0004-svelte-ui-kernel.md)). Responsibilities:

| Part | Does |
|---|---|
| Connection manager | Profiles (local, paired remotes), connect/reconnect with backoff, per-connection status |
| Tabs | Each tab is `(connection, workspace)`; tabs from different cores sit side by side |
| Thread store | Subscribes to thread events from a cursor and folds them into view state |
| Slot registry | Places where UI plugins contribute ([07](07-ui-plugins.md)) |
| Command registry | Commands from the UI kernel, UI plugins and core plugins; one palette |
| Declarative renderer | Renders core plugins' declarative UI: settings forms, status items, views |
| Theme engine | Applies the active theme's tokens as CSS custom properties ([08](08-themes.md)) |
| Plugin manager | Lists, installs and configures plugins on the client and on each core |

The plugin manager is part of the UI kernel, not a plugin, so the user can always remove
a plugin that broke the UI.

### State per connection

Each connection has its own store: handshake result, workspaces, threads, subscriptions
and last-seen `seq` per thread. A failure on one connection affects only its tabs, which
show a reconnecting state; other tabs keep working.

### Addressing

Everything the UI shows is addressed as `<connection>/<workspace>/<thread>`. The sidebar
can therefore list threads from every connection, grouped by core, and a link copied in
one tab opens in the right place.

## Multiple cores

- The user adds a core with its address and a pairing code (or an SSH host). The profile
  stores the address, certificate fingerprint and a display name; the token goes to the
  shell's secret store.
- UI plugins that need a core-side plugin declare it (`requires.core`); they activate only
  in tabs whose core has it.
- Installing a core plugin is always done on a chosen core: the plugin manager asks which
  one.

## Browser client

The same `web/kernel` bundle runs in a plain browser when a core serves it. Without the
shell, the bundle uses a browser transport (WebSocket from the page, token in an HttpOnly
cookie set at pairing) and loads UI plugins over HTTPS from that core. Only one connection
is available in this mode.

## UI rules

- Input is never gated on motion, plugins or the network. Every animated surface has an
  instant path; `prefers-reduced-motion` removes motion.
- No literal colors, radii or font sizes in UI-kernel components: only theme tokens.
- Keyboard first: every action reachable from the command palette; focus is always visible.
- Accessibility: semantic HTML first, ARIA where needed, contrast checked by theme
  validation, forced-colors respected.
