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
  certificate pinning, token authentication and SSH live here, not in JavaScript. For a TLS
  profile the shell sends `auth.token` itself and attaches the connection only after it
  succeeds, so the webview's first message is always `core.hello`. `bridge_connect` and the
  profile commands fail with `{ code, message }`; the UI acts on the code: `unauthorized`
  (the token was refused: the device was revoked or the core forgot it),
  `certificate_changed`, `unreachable`, `unknown_profile`, `bad_address`, `invalid` (bad
  input such as a taken name), `protocol` or `internal`. The shell pings TLS
  connections every 20 s and closes one that is silent for 45 s.
- **Profiles** ([ADR 0017](../decisions/0017-shell-remote-profiles-and-ssh.md)), kept by the
  shell in `<client data>/profiles.json` (0600). `local` is built in. A profile's name
  (`[a-z0-9-]{1,32}`) is its connection id in addresses. Kinds:
  - `tls`: address, pinned fingerprint, device id; the token is in the client secrets.
  - `ssh`: a destination as `ssh` understands it (a `~/.ssh/config` host, `user@host`)
    and the remote command (default `kumikit bridge`). The shell runs the system `ssh`
    (`KUMIKIT_SSH` overrides it) as `ssh -T -o BatchMode=yes -o ServerAliveInterval=15
    -o ServerAliveCountMax=3 -- <destination> <command>`, with no shell between. A
    destination that starts with `-` or holds whitespace or control characters is refused.
    `BatchMode` means ssh never prompts: an unknown host key or a locked key fails with
    ssh's message, shown to the user.

  Commands: `profiles_list` (names, labels and kinds only), `profile_probe(address)` (a TLS
  handshake that returns the certificate's fingerprint and sends nothing),
  `profile_pair({ name, label, address, fingerprint, code, deviceName? })` (pinned to
  `fingerprint`; sends `auth.pair`, keeps the token, saves the profile; the device name
  defaults to this computer's host name), `profile_add_ssh({ name, label, destination,
  command? })` (checks it with a `core.hello` over its own connection), and `profile_remove(name)` (revokes its own
  device on the core if it can reach it within 2 s, then forgets the token and profile).
  After a profile is added the webview never sees its address again.
- **Client plugin store:** `client_plugins_list`, `client_plugins_install(path)` and
  `client_plugins_remove(id)`. Installs UI plugin and theme packages into
  `<client data>/ui-plugins/<id>/<version>/`, checks the structure of their manifest
  ([07](07-ui-plugins.md#manifest)), refuses core plugin packages, and lists the plugins
  bundled with the app (read-only) next to the installed ones.
- **Client secrets:** device tokens for paired cores. On macOS the Keychain and on Windows
  the Credential Manager (service `dev.kumikit.client`, account the profile name); on
  Linux, and wherever `KUMIKIT_CLIENT_SECRETS=file` is set, `<client data>/secrets.json`
  with mode 0600. No command returns a token; the shell attaches it when connecting.
- **File picker:** the native open dialog, used to pick packages and workspace folders.
- No other native commands. Features belong in the core or in plugins.

The shell holds the client data directory ([01](01-architecture.md#data-directories)) and
puts the webview's storage inside it. For tests and development, environment variables
override the defaults: `KUMIKIT_SOCKET` (the local address), `KUMIKIT_CORE_DATA_DIR`,
`KUMIKIT_CLIENT_DATA_DIR`, `KUMIKIT_CORE_BIN` (the core binary to start), `KUMIKIT_SSH`
(the `ssh` program), `KUMIKIT_CLIENT_SECRETS=file`, and the app's
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
| Connection manager | One connection per profile (local, paired remotes, SSH), each connecting on its own; reconnect with backoff (0.5 s doubling to 30 s) and a "Reconnect now" action; per-connection status (`connecting`, `ready`, `reconnecting`, `mismatch`, and `unauthorized` and `certificate_changed`, which stop retrying); after a reconnect it repeats `core.hello` and resubscribes every topic with its last `seq` |
| Tabs | Each tab is `(connection, workspace)`; tabs from different cores sit side by side |
| Thread store | Owns every wire subscription (one per topic per connection), folds thread events into view state, and shares it with plugins (`ctx.threads`) |
| Slot registry | Places where UI plugins contribute ([07](07-ui-plugins.md)) |
| Command registry | Commands from the UI kernel, UI plugins and core plugins; one palette |
| Declarative renderer | Renders core plugins' declarative UI: settings forms, status items, views |
| Theme engine | Applies the active theme's tokens as CSS custom properties ([08](08-themes.md)) |
| Plugin manager | Lists, installs and configures plugins on the client and on each core |
| Sidebar | One group per connection (label and status), with its workspaces and their threads, kept current by the `workspace:<id>` topic |
| Cores view | Lists connections; adds a core by address and pairing code (the fingerprint shown for the user to confirm before the code is sent) or by SSH destination; removes one |

The plugin manager is part of the UI kernel, not a plugin, so the user can always remove
a plugin that broke the UI. **Safe mode** (`kumikit-desktop --safe`) loads no UI plugins; themes
still load.

- **Core plugins:** installed on a chosen core (the active tab's by default). On the local
  core, from a `.kkp` or a folder with a `plugin.toml`, picked with the file picker and
  passed to `plugin.install { path }`. On a core on another machine (and in the browser
  client), from a `.kkp` chosen with a standard file input; the webview reads it and sends
  `plugin.install { package }`. The requested permissions are shown in
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
and last-seen `seq` per thread. A failure on one connection affects only its tabs and its
sidebar group, which show its state (reconnecting, needs pairing, certificate changed);
other tabs keep working. A protocol mismatch is shown per connection too; "Restart the
core" is offered only for the local core.

### Addressing

Everything the UI shows is addressed as `<connection>/<workspace>/<thread>`. The sidebar
can therefore list threads from every connection, grouped by core, and a link copied in
one tab opens in the right place.

## Multiple cores

- The user adds a core with its address and a pairing code (or an SSH destination). The
  profile stores the address, certificate fingerprint and a display name; the token goes to
  the shell's client secrets.
- UI plugins that need a core-side plugin declare it (`requires.core`,
  [07](07-ui-plugins.md#manifest)); their contributions show only in tabs whose core has it.
- Installing a core plugin is always done on a chosen core: the plugin manager asks which
  one.
- A tab or link naming a removed profile shows that the core was removed and opens
  nothing.

## Browser client

The same `web/kernel` bundle, built with `--mode browser`, runs in a plain browser when a
core serves it: `kumikit serve --listen <addr> --web <dir>`, where `<dir>` is the browser
bundle (`target/bundle/ui-browser` from `scripts/bundle.mjs`: the kernel, `bundled.json`
and the bundled UI plugins). The core serves it on its remote port with the same CSP as
the shell (plus `frame-ancestors 'none'`), and UI plugins from `/ui-plugins/<id>/`.

- **Pairing.** When `/auth/status` says the browser is not paired, or the WebSocket upgrade
  gets 401, the page shows a pairing screen (code and device name, default "Browser"). It
  posts to `/auth/pair`; the core sets the token as an HttpOnly cookie that page scripts
  cannot read ([02](02-protocol.md#remote-transport)). "Sign out" posts to `/auth/logout`,
  which revokes the device.
- **One connection**, to the serving core (connection id `core`). There are no profiles to
  add or remove.
- **Plugins.** UI plugins are the ones served with the bundle, read-only; installing UI
  plugins needs the desktop app. Core plugins install by upload (`plugin.install
  { package }`). Workspace folders are typed as paths on the core's machine.
- **Certificates.** With the core's self-signed certificate the browser warns once; the
  fingerprint in the browser's certificate viewer should match `kumikit pair`'s.
- **Dev mode** (development only, never in a build): `npm run dev:browser -- --core
  https://<host:port> --fingerprint <SHA256:…>` runs Vite's dev server with hot reload on
  loopback and proxies `/rpc`, `/auth` and `/ui-plugins` to that core, after checking its
  certificate's fingerprint, rewriting `Origin` to the core's own.

The UI kernel reaches the shell or the browser through a platform adapter (connect,
profiles, the file pickers, plugin URLs). Tests use a test-only adapter that speaks the
bridge's line protocol over a WebSocket to a relay in front of real cores; it is never part
of the desktop or browser bundle.

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
