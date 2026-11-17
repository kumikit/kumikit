# 06 — Client

The client is the Kumikit desktop app: a Tauri 2 **shell** around a Svelte 5 **UI kernel**
running in the system webview. It holds no project state; everything it shows comes from
cores over the protocol.

## Shell (Rust, `kumikit-shell`)

The shell's jobs, and its only commands ([ADR 0013](../decisions/0013-desktop-shell.md)):

- **Window and UI.** Creates the window and serves the UI kernel's files from the custom
  protocol `kumikit-app://localhost/`, and installed UI plugins from
  `kumikit-plugin://<id>/`. Where the webview exposes custom schemes as
  `http://<scheme>.localhost/` (WebView2 on Windows), that form is used. Every response
  carries the CSP: `default-src 'none'`, with scripts, styles, fonts and images only from
  the app and plugin schemes (images also from `data:`), no inline scripts or styles, no
  `eval`, no frames, and no network: `fetch` reaches only the app and plugin schemes and
  the shell's IPC. A path outside the plugin's
  directory is refused.
- **Local core.** Looks for the core on the local address ([02](02-protocol.md#local-transport-details)).
  If no core answers, it starts the core binary bundled with it (never one from `PATH`) as
  `kumikit serve --bundled-plugins <dir>`, detached so agents keep running when the window
  closes, and waits for its `ready` line. If two windows start a core at once, the second
  core fails on the data-directory lock and its window connects to the first.
- **Connection bridge:** `bridge_connect(profile, channel)` returns a connection id,
  `bridge_send(connection, line)` sends one JSON-RPC message, and `bridge_close(connection)`
  closes it. Incoming messages arrive in order on the channel, followed by a `closed`
  notice with a reason. The shell does not parse messages; it enforces the 8 MiB limit.
  Profiles are names, not addresses: the webview cannot open arbitrary sockets. Remote TLS,
  certificate pinning and SSH tunnels live here, not in JavaScript (M3).
- **Client plugin store:** `client_plugins_list`, `client_plugins_install(path)` and
  `client_plugins_remove(id)`. Installs UI plugin and theme packages into
  `<client data>/ui-plugins/<id>/<version>/`, checks the structure of their manifest
  ([07](07-ui-plugins.md#manifest)), refuses core plugin packages, and lists the plugins
  bundled with the app (read-only) next to the installed ones.
- **Client secrets:** device tokens for paired cores, stored in the OS keychain or a 0600
  file. They never reach the webview; the shell attaches them when connecting (M3).
- **File picker:** the native open dialog, used to pick packages and workspace folders.
- No other native commands. Features belong in the core or in plugins.

The shell holds the client data directory ([01](01-architecture.md#data-directories)) and
puts the webview's storage inside it. For tests and development, environment variables
override the defaults: `KUMIKIT_SOCKET` (the local address), `KUMIKIT_CORE_DATA_DIR`,
`KUMIKIT_CLIENT_DATA_DIR`, `KUMIKIT_CORE_BIN` (the core binary to start), and the app's
resource directories `KUMIKIT_UI_DIR`, `KUMIKIT_BUNDLED_UI_PLUGINS` and
`KUMIKIT_BUNDLED_CORE_PLUGINS`.

Builds with the `test-hooks` cargo feature (never released) add what end-to-end tests
need: the open dialog answers from a queue file (`KUMIKIT_TEST_PICKS`, one path per line),
and the shell prints `ui-connected` once the UI kernel's first connection is up.
When a WebDriver starts the app on Windows (msedgedriver sets `WEBVIEW2_USER_DATA_FOLDER`
or `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`), the shell leaves the webview's folder and
arguments to the driver, so the driver can attach.

## UI kernel (Svelte, `web/kernel`)

Svelte 5 with runes, TypeScript strict, Vite
([ADR 0004](../decisions/0004-svelte-ui-kernel.md)). Responsibilities:

| Part | Does |
|---|---|
| Connection manager | Profiles (local, paired remotes), connect/reconnect with backoff (0.5 s doubling to 30 s), per-connection status; after a reconnect it repeats `core.hello` and resubscribes every topic with its last `seq` |
| Tabs | Each tab is `(connection, workspace)`; tabs from different cores sit side by side |
| Thread store | Owns every wire subscription (one per topic per connection), folds thread events into view state, and shares it with plugins (`ctx.threads`) |
| Slot registry | Places where UI plugins contribute ([07](07-ui-plugins.md)) |
| Command registry | Commands from the UI kernel, UI plugins and core plugins; one palette |
| Declarative renderer | Renders core plugins' declarative UI: settings forms, status items, views |
| Theme engine | Applies the active theme's tokens as CSS custom properties ([08](08-themes.md)) |
| Plugin manager | Lists, installs and configures plugins on the client and on each core |
| Sidebar | Workspaces and their threads with status, kept current by the `workspace:<id>` topic |

The plugin manager is part of the UI kernel, not a plugin, so the user can always remove
a plugin that broke the UI. **Safe mode** (`kumikit-desktop --safe`) loads no UI plugins; themes
still load.

- **Core plugins:** installed from a `.kkp` or a folder with a `plugin.toml`, picked with
  the file picker and passed to `plugin.install`. The requested permissions are shown in
  plain words (the same wording as `kumikit plugin add`), with the warning that the
  package is unsigned, and granted as a whole.
- **Client plugins:** installed through the shell's client plugin store. A third-party UI
  plugin with inline code asks for trust first: it runs with full access to the app and to
  every connected core. Themes run no code and need no trust. A disabled UI plugin's
  contributions disappear at once; its module stays loaded until the app restarts.
- **Bundled plugins.** The app's resources hold the bundled UI plugins and a data file,
  `bundled.json`, which lists them and names the fallback theme. The UI kernel names no
  plugin in code. If the fallback theme is missing, the base styles fall back to system
  colors.

### Client settings

The theme and appearance, open tabs, which UI plugins are enabled and trusted, and each
plugin's `ctx.storage` live in the webview's storage, inside the client data directory.
Nothing secret is stored there.

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

The same `web/kernel` bundle runs in a plain browser when a core serves it (M3). Without the
shell, the bundle uses a browser transport (WebSocket from the page, token in an HttpOnly
cookie set at pairing) and loads UI plugins over HTTPS from that core. Only one connection
is available in this mode.

The UI kernel reaches the shell or the browser through a platform adapter (connect, the
file pickers, plugin URLs). Tests use a test-only adapter that speaks the bridge's line
protocol over a WebSocket to a relay in front of a real core; it is never part of the
desktop bundle.

## UI rules

- Input is never gated on motion, plugins or the network. Every animated surface has an
  instant path; `prefers-reduced-motion` removes motion.
- No literal colors, radii or font sizes in UI-kernel components: only theme tokens.
- Keyboard first: every action reachable from the command palette; focus is always visible.
- Accessibility: semantic HTML first, ARIA where needed, contrast checked by theme
  validation, forced-colors respected.
- Enter and Escape handlers in text inputs ignore IME composition.
- Links in agent output and plugin markdown are not opened by the app; it offers to copy
  them. Raw HTML in markdown is shown as text.
