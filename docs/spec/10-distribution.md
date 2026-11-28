# 10 — Distribution

## Preinstalled plugins

Kumikit ships with as little as works out of the box. Each preinstalled plugin can be
disabled; the UI kernel works (with an empty state) without any of them.

| Plugin | Side | Why it is preinstalled |
|---|---|---|
| `kumikit.provider-acp` | core | Runs any ACP agent from a manifest; gives a working agent with no code. Shipped in `core-plugins/` with desktop installers and CLI archives (below). |
| `kumikit.threads` | core | Agent tools for wake-ups, thread coordination, queues, and title suggestions via the MCP tool gateway. Shipped in `core-plugins/` alongside `provider-acp` (waits for the user's permission grant). |
| `kumikit.ui-chat` | client | Transcript and composer |
| `kumikit.ui-settings` | client | Settings pages for the kernel and declarative plugin settings |
| `kumikit.ui-usage` | client | Thread header plan-limit meter and Agent usage dialog (statistics, heatmap, refresh) |

Bundled themes live in `themes/*.json` as plain resource files, not plugins ([08](08-themes.md)).

### Bundled core plugins

`kumikit serve --bundled-plugins <dir>` keeps the core plugins shipped with a distribution
in sync. Desktop installers pass their bundled `core-plugins/` directory when starting the
local core; standalone CLI archives (`kumikit-v*-<target>.tar.gz`/`.zip`) and SSH remote
sync also ship `core-plugins/` alongside the `kumikit` binary. When `--bundled-plugins` is
omitted (and `--no-bundled-plugins` is not passed), `kumikit serve` checks
`<exe dir>/core-plugins` and `<exe dir>/../share/kumikit/core-plugins` automatically. At
start the core checks every `.kkp` in the directory:

- If the plugin is not installed, it installs it with `source = bundled` and enables it
  only if it requests no permissions; any other stays installed until the user grants it.
  Bundled core plugins can be disabled (their disabled state persists across restarts and
  upgrades), rather than permanently removed, matching bundled client plugins.
- If the plugin is already installed from the bundle (`source = bundled`) and its version
  or package hash differs from the bundled `.kkp`, or its installed copy failed to load,
  the bundled package replaces it at start like an update ([Updates](#updates)): it keeps
  the grant and enabled/disabled state if it asks for no more; otherwise it is installed
  without a grant and waits, disabled, until the user grants the new permissions.
- A copy the user installed from elsewhere (`file` or a registry) is left alone.

The core names no plugin: the directory decides. Pass `--no-bundled-plugins` to disable
auto-discovery.

### Starter pack

On a client's first run, a welcome screen offers the registry's **starter pack**: common
plugins (agent providers, git, a bot) listed in the registry's `starter` list.

- Nothing is fetched until the user chooses "Browse the starter pack"; the screen says that
  this contacts the registry and names its host. "Not now" is remembered, and the screen is
  reachable later from the palette.
- Each entry shows its publisher, description and permissions and has its own Install
  button, which goes through the normal permission dialog. Nothing is preselected and there
  is no "install all": nothing is installed without the user choosing it.
- The list is registry data; the UI kernel names no plugin. The browser client shows only
  core plugins (it cannot install UI plugins).

## Desktop installers

| Platform | Installers |
|---|---|
| Linux | `.deb`, `.rpm`, and an AppImage that runs on glibc 2.35 or newer (built on Ubuntu 22.04) |
| macOS | `.dmg` |
| Windows | NSIS `.exe`, with the WebView2 bootstrapper |

The AppImage carries WebKitGTK and GTK, so it is larger than the other installers and
outside the installer budget ([00](00-overview.md#footprint-budgets)). It uses the host's
Wayland and EGL libraries rather than bundled ones, which do not work with current Mesa. It
starts on X11 (or Xwayland) by default; `GDK_BACKEND=wayland` selects GTK's Wayland backend.
Running the AppImage with no arguments (or `--safe`) opens the desktop app; passing any
other arguments (such as `--help`, `serve`, `plugin`, or `update`) forwards directly to the
bundled `kumikit` CLI without loading GTK or WebKit libraries, and `update` replaces the
AppImage file itself.

## Package format

`<id>-<version>.kkp`: a tar.zst archive with `plugin.toml`, the component or UI bundle,
assets (such as provider icons referenced by `contributes.providers[].icon`: `.svg` or
`.png`, at most 32 KiB) and, when signed, a `SIGNATURE` file ([04](04-plugin-host.md#signatures)): ed25519
over the list of the other files' SHA-256 hashes, made with the publisher's key
([ADR 0018](../decisions/0018-package-signing-and-registry.md)).

### Publisher keys

- `kumikit plugin keygen [--out <file>] [--force]` makes a key pair. The private key is a
  PKCS#8 file (PEM, `-----BEGIN PRIVATE KEY-----`) with mode 0600, by default in
  `<user config dir>/kumikit/keys/` (Linux `$XDG_CONFIG_HOME/kumikit/keys`, macOS
  `~/Library/Application Support/dev.kumikit/keys`, Windows `%APPDATA%\kumikit\keys`). It
  prints only the public key id (`ed25519:` and the key in base64url). `--out` inside a git
  work tree is refused unless `--force` is given.
- Signing reads the private key from `--key <file>` or from the `KUMIKIT_SIGNING_KEY`
  environment variable (the PEM text, for a CI secret). On Unix a key file readable by
  group or others is refused. Key material is never printed or logged.
- `kumikit plugin pack <dir> [-o <file.kkp>] [--key <file>]` packs, and signs with
  `--key`. `kumikit plugin sign <file.kkp> --key <file>` adds or replaces the signature of
  an existing package. `kumikit plugin verify <file.kkp> [--key <id>]` prints who signed it
  or that it is not signed, and fails on a bad signature or another key than `--key`.

## Registry

A registry is a git repository of plugin descriptions, and the `index.json` its CI builds
from them. The official one is `github.com/kumikit/registry`, whose index is served at
`https://kumikit.github.io/registry/index.json`.

```
publishers/<publisher>.toml       # the publisher's key and who may change its files
plugins/<publisher>/<name>.toml   # one plugin: metadata and versions
starter.toml                      # ids offered on the first-run screen
allow-confusable.toml             # reviewed exceptions to the confusable-id check
```

```toml
# publishers/acme.toml
name = "Acme"
key = "ed25519:Hx3k…"
github = ["acme-bot", "jane"]     # GitHub logins that may change this publisher's files
early = true                      # accepted before 1.0 (ADR 0020)
```

```toml
# plugins/acme/probe.toml
id = "acme.probe"
name = "Probe"
description = "…"
repository = "https://github.com/acme/kumikit-probe"
license = "MIT"
kind = ["provider"]

[[versions]]
version = "0.1.0"
url = "https://github.com/acme/kumikit-probe/releases/download/v0.1.0/acme.probe-0.1.0.kkp"
sha256 = "…"
api = "^0.1"                                   # copied from the manifest
permissions = { spawn = ["fake-acp-agent"] }   # copied from the manifest (core plugins)
yanked = false
```

- UI plugins and themes copy `[ui] network`, `isolation` and `requires.core` instead of
  `permissions`.
- **Index.** On its default branch the registry's CI builds `index.json` (`format`, `name`,
  `generated`, `starter`, `publishers` with their keys, `plugins` with their versions) and
  publishes it. Clients read only the index; they never clone the repository.
- **Publishing** is a pull request. CI (`kumikit-registry check`) downloads each new
  version and checks its SHA-256, that it is signed by the publisher's key, its manifest
  (the same validation as install), that the copied fields equal the manifest, that the id
  starts with the publisher's name, that the publisher key did not change, that no
  published version was changed or removed (a version can only be marked `yanked`), that
  the author of the pull request is in the publisher's `github` list, and that a new id is
  not confusingly close to an existing one. Reserved publishers: `kumikit`, `test`, `local`.
- **Confusable ids.** Ids are ASCII, so the check compares skeletons: lowercase, `-`
  removed, `0→o 1→l i→l 5→s rn→m vv→w`. A new id is refused if its skeleton equals an
  existing one's, if it is one edit away from an existing id of 5 or more characters, or
  if its publisher is confusable with a reserved one. Exceptions are listed in
  `allow-confusable.toml`, so a maintainer reviews them.
- **Before 1.0** the official registry accepts only publishers marked `early`
  ([ADR 0020](../decisions/0020-interface-freeze-after-m5.md)). Anyone can run their own.

### Fetching

- A registry is a URL to an index: `https://…` (verified with the operating system's
  trusted certificates), `http://` only for a loopback host (`127.0.0.1`, `::1`,
  `localhost`), or `file:` for a mirror on the same machine. Redirects are followed up to
  5 times and only to `https` (or loopback `http` from loopback). Limits: index 16 MiB,
  package 64 MiB, 10 s to connect, 30 s per request.
- **When.** Only when the user browses or searches the registry, installs or updates from
  it, imports a lockfile from it, or checks for updates. Nothing at startup, first run,
  connect or in the background; there are no periodic checks. A search reuses a cached
  index younger than 10 minutes; installing and checking for updates revalidate it
  (`If-None-Match`).
- **Who.** Each side fetches what it installs: a core fetches core plugins (a remote core
  downloads on its own machine), the desktop app's shell fetches UI plugins.
- **Cache:** `<core data>/registry-cache/` and `<client data>/registry-cache/` (the index
  and its ETag, named by the SHA-256 of the URL).
- **Several registries.** Each core and each client keeps an ordered list
  (`<core data>/registries.toml`, `<client data>/registries.toml`), by default the
  official one. `KUMIKIT_DEFAULT_REGISTRY` replaces the default (tests, development). An id
  resolves to the first registry that lists it, and search shows which registry each
  result comes from. A plugin is updated only from the registry it was installed from.
- Installing from a local `.kkp` file works offline.

## Install

```sh
kumikit plugin add acme.probe[@0.1.0] [--registry <url>] [--yes]   # from a registry
kumikit plugin add ./my-plugin-0.1.0.kkp [--yes]                   # a local file
kumikit plugin search <words>
kumikit plugin update [<id>] [--yes] [--allow-wider]
kumikit plugin registry list | add <url> | remove <url>
kumikit plugin lock export [-o <file>] | import <file> [--yes]
kumikit plugin list | remove <id> [--yes] [--keep-data]
```

The argument of `add` is a file when it names an existing file or ends in `.kkp`, and an
id otherwise. The desktop plugin manager, and Settings → Agents for agents
([06](06-client.md#agents)), do the same through the protocol
(`plugin.install` with `registry`, `path` or `package`) for core plugins, and through the
shell's client plugin store for UI plugins ([06](06-client.md#shell-rust-kumikit-shell)).

Install steps:

1. Resolve the id in the registry: the latest version that is not yanked and whose `api`
   this Kumikit satisfies, or the version asked for.
2. Download, and check the SHA-256 against the index (and against the lockfile, when
   importing one).
3. Unpack into a staging directory, defensively ([04](04-plugin-host.md#package-contents)).
4. Check the signature against the registry's publisher key, and against the key pinned
   for this id if one is installed. A registry package must be signed.
5. Validate the manifest, and check that its id, version, `api` and permissions equal the
   index's copy.
6. Show the name, publisher, key id, registry, repository and permissions in plain words.
7. Install (move into place, record the hash, key and source); enable only after the user
   grants the permissions.

A failure in steps 2–5 installs nothing and is reported as untrusted
([02](02-protocol.md#errors)).

### Local packages

- `kumikit plugin add <file.kkp> [--yes]`, `kumikit plugin list`, `kumikit plugin remove
  <id> [--yes] [--keep-data]` and `kumikit plugin pack`. All take `--socket` and
  `--data-dir`.
- The commands talk to the running core through `plugin.*`. When no core runs, they open
  the data directory themselves (holding its lock) and do the same work.
- `add` unpacks and validates the package, then prints the plugin's name, publisher,
  repository and permissions in plain words. A signed file shows "signed by key ed25519:…,
  not checked against a registry", and that key is pinned. An unsigned one shows that the
  package is not signed, so its publisher cannot be verified. It asks before granting the
  permissions and enabling the plugin; `--yes` grants without asking. Without a terminal
  and without `--yes`, the plugin stays installed but not enabled.
- `remove` asks before deleting the plugin's data; `--keep-data` keeps it.
- The CLI talks to the core on its own machine. To install on another machine, run it
  there (for example `ssh devbox kumikit plugin add <id>`), or use the desktop app, which
  installs on any core it is connected to (by registry id, or by uploading the package with
  `plugin.install { package }`).
- The SHA-256 of the package is recorded for the lockfile. Adding an id that is already
  installed fails; use `update`. Adding the same version of a plugin that is installed but
  not enabled asks again for the grant.

### Remote access commands

- `kumikit serve --listen <host[:port]> [--tls-cert <pem> --tls-key <pem>] [--web <dir>]`
  turns on the remote transport for that run, and with `--web` serves the browser client
  ([02](02-protocol.md#remote-transport), [06](06-client.md#browser-client)).
- `kumikit pair [--json]` prints a pairing code, its expiry, the certificate fingerprint and
  the URL (`--json`: the `auth.pairing.create` result). It needs a running core with
  remote access on.
- `kumikit devices list` and `kumikit devices revoke <id>` manage paired devices, through
  the running core or, when none runs, on the data directory.
- `kumikit bridge` copies JSON-RPC lines between stdin/stdout and the local core; the
  desktop app runs it over SSH.

All take `--socket` (and the device commands `--data-dir`).

## Updates

- Updates are proposed, never applied silently. `kumikit plugin update` and the plugin
  manager's "Check for updates" list installed registry plugins whose registry has a
  higher version that is not yanked and that this Kumikit can run; never a lower one.
- An update is downloaded and checked like an install, then replaces the old version.
  If its permissions are no wider, the plugin keeps its state (and gets the new, possibly
  narrower, set). Wider permissions (a new program, host or secret, more file access,
  tools, raised limits) are shown and must be granted first; until then the plugin stays
  on the old version. An update fails while one of the plugin's turns is running.
- `--yes` applies updates that do not widen permissions; widening needs an interactive yes
  or `--allow-wider`.
- For UI plugins, wider means new network hosts or a change from a sandboxed panel to an
  inline module (which needs trust again).

## Lockfiles

- Each core keeps `plugins.lock` in its data directory, rewritten after every install,
  update, enable, disable and uninstall; the client keeps its own in the client data
  directory for installed UI plugins.

  ```toml
  # Written by Kumikit. Export it to reproduce these plugins on another core.
  format = 1
  [[plugin]]
  id = "acme.probe"
  version = "0.1.0"
  sha256 = "…"
  source = "registry+https://kumikit.github.io/registry/index.json"   # or "file", "bundled"
  key = "ed25519:…"                                                    # when signed
  state = "enabled"

  [plugin.granted]                       # the protocol's permission words
  spawn = ["fake-acp-agent"]
  network = []
  files = "none"                         # none, read_workspace or workspace
  secrets = []
  tools = false
  ```

  The client's lock has no `granted`; when the UI kernel exports it, it adds `enabled` and
  `trusted` (trust is the UI plugin's grant) from the client settings.
- **Export:** `kumikit plugin lock export`, or "Export lock file" in the plugin manager.
- **Import:** `kumikit plugin lock import <file>`, or "Import lock file". Entries already
  installed at that version are skipped; registry entries are installed at the locked
  version and must have the locked SHA-256; the registry must already be configured
  (import does not add registries); `file` and `bundled` entries are reported. All the
  permissions (and trust, on the client) are shown together and confirmed once, then the
  entries that were enabled are enabled.

## Application updates

Kumikit publishes a signed `release.json` manifest (with a sibling `release.json.sig` in the
`kumikit-signature 1` Ed25519 format) alongside every release
([ADR 0031](../decisions/0031-signed-release-manifest-and-updates.md)).

- **Verification.** Clients and the `kumikit update` CLI verify `release.json.sig` against the
  pinned Kumikit release public key (overridable via `KUMIKIT_UPDATE_KEY`) before reading any
  asset URL or SHA-256 hash, and verify every downloaded installer or archive against its hash in
  the signed manifest.
- **Desktop app (Settings → About).** Displays the build-stamped current version, checks for
  updates on demand, and downloads/installs verified updates (in-place on Linux AppImage, macOS
  `.app`, and Windows NSIS; downloads the verified package for system-managed `.deb`/`.rpm`
  installs). An opt-in **Automatic updates** setting (off by default) periodically checks and
  downloads updates in the background, waiting until `core.activity` reports `idle: true`
  (zero active turns and zero unpaused ready queued messages) before shutting down the local core
  and installing.
- **Core CLI (`kumikit update`).** `kumikit update [--check] [--yes] [--wait-idle] [--force]`
  checks for and installs a newer signed `kumikit` binary (and any bundled `core-plugins/*.kkp`
  in the archive) on headless and TLS-paired machines, refusing to interrupt active agents
  unless `--force` is passed (or waiting until idle with `--wait-idle`). When invoked through
  AppImage CLI forwarding (`APPIMAGE` set and the running binary lives under `APPDIR`),
  `kumikit update` verifies and atomically replaces the AppImage file itself.
- **SSH remote sync.** When connecting over SSH to a machine where `kumikit` is missing or on a
  different version ([ADR 0032](../decisions/0032-ssh-remote-core-sync.md)), the desktop app can
  download and verify the matching `kumikit` archive locally and stream it over `ssh` into
  `~/.local/bin/kumikit` (installing bundled `core-plugins/*.kkp` into
  `~/.local/share/kumikit/core-plugins/`), checking remote `core.activity` before restarting an
  older running core. Prompted by default, with optional per-profile or global auto-sync.

## Versioning

Plugins use semver. `api` in the manifest is a range over the kernel plugin API version.
The kernel refuses to enable a plugin whose `api` range it does not satisfy and says which
version the plugin needs.
