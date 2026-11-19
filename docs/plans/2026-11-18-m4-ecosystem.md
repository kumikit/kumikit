# M4 — Ecosystem

- **Goal:** reach the exit criteria of [M4](../spec/12-roadmap.md#m4--ecosystem): a plugin
  published to the registry by a third party installs on a fresh machine, with its
  permissions shown and enforced. Nothing from M5: no new provider, bot, git or GitHub
  plugin, no tool gateway, no VS Code theme importer, no new WIT host interfaces.
- **Status:** active (approved 2026-11-18).

## Scope

In:
- **Package signing:** the `SIGNATURE` file (format v1), publisher keys, `kumikit plugin
  keygen | sign | verify`, `pack --key`, verification at install on the core and in the
  shell's client plugin store, key pinning across versions, the "not signed" path kept for
  local files.
- **Registry:** its file format and the `index.json` clients read, a checker for its CI
  (manifest, hash, signature, key continuity, confusable ids), fetching on demand with a
  cache, more registries by URL, and a fixture registry for tests. The real
  `github.com/kumikit/registry` is planned here and created only after you say so
  (step R).
- **Install, search, update:** `kumikit plugin add <id>`, `search`, `update`, `registry`,
  `lock`; the matching protocol methods (0.5); the desktop and browser plugin managers;
  updates proposed and never applied silently; widening updates wait for approval.
- **Lockfiles:** `plugins.lock` per core and per client, export and import.
- **Starter pack:** a first-run screen, filled from registry data only after the user asks.
- **Level-2 panels:** sandboxed iframes, a CSP for each panel, the postMessage bridge, the
  asynchronous UI SDK for panels, the method limits, theme tokens pushed live.
- **`kumikit theme check`**, with rules shared with the UI kernel.
- **Author templates:** core plugin (Rust), UI plugin (Svelte, level 2), theme. Each one is
  built, packed, signed and installed in CI.
- **Decisions:** when WIT and the UI SDK freeze, and sidecars (recommend, don't build).
- **Tests:** the simulated third-party exit test in Rust, Playwright and the desktop app,
  plus a test for each control in the security table.

Out (deferred, with the reason):
- **`kumikit plugin add --core <name>`.** As in M3, the CLI would need its own profiles and
  token store. Two paths already cover it: `ssh devbox kumikit plugin add <id>` (the CLI
  on that machine), or the app, which installs on any core it is connected to. 10-distribution's
  example changes to say so (Q8).
- **Periodic update checks.** Updates are checked only when the user asks ("Check for
  updates", `kumikit plugin update`). An opt-in schedule would be the only automatic
  registry request; it can come later with its own README line (Q9).
- **Publisher key rotation and recovery.** A changed key is refused, not migrated (Q11).
- **A signed registry index** (TUF-style). Packages are signed and keys are pinned; the
  index itself is trusted through HTTPS (Q12).
- **Signing bundled plugins** with a project release key: that needs a secret in CI. The
  installer is what vouches for them (Q15).
- **`kumikit plugin new`** (scaffolding from the binary). The templates are directories
  you copy; a command can come later.
- **Installing UI plugins in the browser client** (unchanged from M3).
- **Freezing WIT and the UI SDK at 1.0** (recommended against for M4; see Decisions, Q4).
- **Publishing the SDK crates and `@kumikit/ui-sdk` to crates.io and npm.** That is
  outward-facing; the templates build against this repository until you decide (Q5).

## Design

### Package signing

**Format (`SIGNATURE`, v1).** A text file at the package root, UTF-8, LF line endings:

```
kumikit-signature 1
key ed25519:Hx3k…(43 characters, base64url, no padding)
file 3f2a…(64 hex digits) plugin.toml
file 9c01… plugin.wasm
file 77be… assets/icon.svg
sig 4m9Q…(86 characters, base64url of the 64-byte signature)
```

- The signed message is every byte of the file before the `sig` line, starting with
  `kumikit-signature 1`, so a signature from another protocol cannot be replayed as a
  package signature.
- `file` lines list every regular file in the package except `SIGNATURE`, sorted by path
  (bytewise), with `/` separators, and the SHA-256 of each file's contents.
- **Verification** happens after the existing defensive unpack into staging. The host
  hashes every file it unpacked and requires the set to equal the `file` lines exactly:
  nothing missing, nothing extra, no duplicates, and no path that `safe_path` refuses. Then
  it verifies the signature with the key. The result is `Unsigned`, `Signed { key }`, or
  an error (`untrusted`, below) for a malformed or failing signature.
- The **key id** shown to people is `ed25519:` followed by the key in base64url, in groups
  of four for reading aloud (as the certificate fingerprint is). The registry and pinning
  compare full keys.
- The code goes in `kumikit-package` (shared by the core and the shell), as a `signature`
  module. It uses `ring` 0.17, which the core and the shell already ship for TLS
  (`Apache-2.0 AND ISC`): `Ed25519KeyPair` for keygen and signing,
  `UnparsedPublicKey::new(&ED25519, …)` for verifying. No new crypto crate.

**Keys.**
- `kumikit plugin keygen [--out <file>] [--force]` writes a new private key (PKCS#8 v2 from
  `Ed25519KeyPair::generate_pkcs8`, PEM-armoured as `-----BEGIN PRIVATE KEY-----`, so
  secret scanners recognise it) with mode 0600. It prints only the public key id. The
  default location is `<user config dir>/kumikit/keys/<name>.key` (Linux
  `$XDG_CONFIG_HOME/kumikit/keys`, macOS `~/Library/Application Support/dev.kumikit/keys`,
  Windows `%APPDATA%\kumikit\keys`), never the working directory. It refuses an `--out`
  inside a git work tree (it looks for `.git` going up) unless `--force` is given.
- Signing reads the key from `--key <file>` or from the `KUMIKIT_SIGNING_KEY` environment
  variable (the PEM text, for a publisher's CI secret). Key bytes are never logged or
  printed, and errors name only the file. On Unix, `--key` refuses a file readable by group
  or others (as ssh does).
- `kumikit plugin pack <dir> [-o <file>] [--key <file>]` signs while packing.
  `kumikit plugin sign <file.kkp> --key <file>` adds or replaces `SIGNATURE` in an existing
  package. Packing stays deterministic. `kumikit plugin verify <file.kkp> [--key <id>]`
  prints `signed by <key id>` or `not signed`, and fails on a bad signature or a key other
  than `--key`.
- Repository hygiene: `.gitignore` gains `*.key`, and a new `scripts/check-no-keys.sh` (run
  in the `docs` CI job) fails if any tracked file contains `BEGIN PRIVATE KEY`. Test keys
  are generated at test time and never written into the tree.

**At install (core and shell).**
1. From a **registry**: the package's SHA-256 must equal the index entry's. The package
   must be signed, by exactly the publisher key the index gives. Its manifest's `id`,
   `version`, `api` and permissions must equal the entry's (the index copies them, so
   search can show them before download, and a mismatch means the entry lies). Any
   failure is `untrusted` and nothing is installed.
2. From a **local file**, signed: the signature is verified. Install says "signed by key
   ed25519:…, not checked against a registry", and the key is pinned.
3. From a **local file**, unsigned: as today, "not signed; Kumikit cannot check who made
   it". Nothing is pinned.
4. **Pinning:** the core records each plugin's key and source in its `plugins` table
   (migration 0005: `publisher_key`, `source`); the shell records them in its client
   lockfile. A later version of the same id must be signed by the pinned key. Otherwise it
   is refused: "the publisher's key changed; if you trust the new key, remove the plugin
   and install it again". An unsigned package can never replace a pinned one. A plugin with
   no pinned key (installed unsigned, or bundled) pins the first key it is updated with.
5. `InstalledPlugin.signed` becomes true for verified packages, and gains
   `publisher { key, registry? }` and `source` (protocol below).

### Registry

**Repository layout** (`github.com/kumikit/registry`; created in step R only after you
approve):

```
publishers/<publisher>.toml       # one per publisher: key and who may change it
plugins/<publisher>/<name>.toml   # one per plugin: metadata and versions
starter.toml                      # ids offered on the first-run screen
allow-confusable.toml             # reviewed exceptions to the confusable-id check
.github/workflows/check.yml       # runs `kumikit-registry check` and builds index.json
```

```toml
# publishers/acme.toml
name = "Acme"
key = "ed25519:Hx3k…"
github = ["acme-bot", "jane"]     # GitHub logins allowed to change this publisher's files
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
# The rest is copied from the package by the checker, which fails on any difference:
api = "^0.1"
permissions = { spawn = ["fake-acp-agent"] }   # core plugins; UI plugins: network, isolation, requires-core
yanked = false
```

Q2: this puts the key in a publisher file rather than in each plugin file (as 10-distribution
says), so one key covers a publisher's namespace and nobody else can publish under its id
prefix.

**What clients read: `index.json`.** Clients don't clone git. The registry's CI checks
each pull request, and on `main` it builds one `index.json` (`{ format: 1, name,
generated, starter, publishers: { <p>: { key } }, plugins: [ … ] }`) and publishes it at a
stable URL. The recommendation is GitHub Pages: `https://kumikit.github.io/registry/index.json`
(Q1). A registry is a URL to such an index:
- `https://…`: verified with the operating system's trust store (`rustls-native-certs`,
  `Apache-2.0 OR ISC OR MIT`; checked in the step 2 spike). `webpki-roots` is
  CDLA-Permissive-2.0, which is not on the list, so it is not used.
- `http://` only for a loopback host (`127.0.0.1`, `::1`, `localhost`). This is for tests
  and local mirrors (Q20). Packages are still hash- and signature-checked.
- `file:` for a directory or file on the core's own machine (offline mirrors).

Redirects (GitHub release assets redirect to another host) are followed up to 5 times,
and only to `https` (or loopback `http` from loopback). Limits: index 16 MiB, package
64 MiB, 30 s per request, 10 s to connect. The client is `hyper`'s HTTP/1.1 client
connection (`hyper::client::conn::http1`, already in the tree as a server) over
`tokio-rustls`, so no `reqwest`.

**When it is fetched.** Only when the user browses or searches the registry, installs or
imports from it, or checks for updates. Nothing at startup, first run, connect or idle. A
search reuses a cached index younger than 10 minutes. An install revalidates it
(`If-None-Match`) so the hash is current. The cache is
`<core data>/registry-cache/<sha256 of url>.{json,etag}`, and the shell's is under
`<client data>/registry-cache/`. Nothing about the index is kept in memory between calls.
README privacy gains: "Plugin registries are contacted only when you browse, search,
install from them, import a lock file or check for updates. The core contacts them for
core plugins, the app for UI plugins and themes. Nothing is checked in the background."

**Several registries.** Each side keeps an ordered list. On the core it is
`<core data>/registries.toml`; in the app it is `<client data>/registries.toml`. Both
default to the official URL. `KUMIKIT_DEFAULT_REGISTRY` replaces the default for tests and
development (listed with the other overrides in 06-client). An id resolves to the first
registry that lists it. Search shows which registry each result came from. A plugin is
updated only from the registry it was installed from (its `source`), so adding a second
registry cannot take over another plugin's updates.

**Who fetches.** Each side fetches what it installs (Choice A):
- The **core** fetches core plugins: `kumikit plugin add <id>`, the `registry.*` methods
  and `plugin.install { registry }`. A remote core downloads from its own machine, so
  installing on devbox needs no upload, and the browser client can install from the
  registry too.
- The **shell** fetches UI plugins and themes for its client plugin store. This extends
  ADR 0013's command list (new ADR 0018). The shell already links rustls; the fetch code is
  shared.

The shared code is a new crate, **`kumikit-registry`**: index and plugin-file types,
fetching with the cache, resolving id and version, and the install-time checks on top of
`kumikit-package`. It depends on `kumikit-package` and `kumikit-protocol` (for
`Permissions`). It also has a binary, `kumikit-registry`, with `check` and `build-index`
for the registry's CI. Those stay out of the core binary.

**Registry CI** (`kumikit-registry check <registry dir> [--base <dir of main>]`). For each
changed plugin file it downloads each new version and checks:
- the hash;
- the signature, by the publisher's key;
- the manifest, with the same validation as install (core plugins through
  `kumikit-plugin-host::manifest`; UI plugins and themes through the shared UI manifest
  check, below);
- that the copied fields equal the manifest;
- that the id's publisher prefix matches the directory;
- that the publisher key is unchanged from `--base` (key continuity);
- that a published version is never changed or removed (it may only be marked
  `yanked = true`);
- that a new id is not confusable with an existing one (below).

The workflow checks that the pull request's author is in the publisher's `github` list.
Reserved publishers: `kumikit` (project maintainers only), `test`, `local`.

**Confusable ids.** Ids are ASCII `[a-z0-9-.]`, so Unicode homoglyphs can't occur. The
check works on a skeleton: lowercase, drop `-`, map `0→o 1→l i→l 5→s rn→m vv→w`. A new
id is refused if its skeleton equals an existing one's, if it is one edit (Damerau) from
an existing id of 5 or more characters, or if its publisher is confusable with a reserved
one (`kumikit` vs `kumikti`, `kumik1t`). An exception needs an entry in
`allow-confusable.toml`, so a maintainer reviews it. Install also shows the publisher,
the key id and the repository, which is 11-security's second control.

### Install, search and updates (core)

- **`plugin.install { registry: { id, version?, sha256? } }`** is a third source next to
  `path` and `package`. The steps follow 10-distribution: resolve the id (latest version
  that isn't yanked and whose `api` the kernel satisfies, unless `version` is given) →
  download → check the hash (and `sha256`, when a lockfile gives it) → check the signature
  against the registry's publisher key and any pinned key → check the manifest against
  the entry → unpack → `installed`. Enabling stays a separate `plugin.enable { grant }`,
  so permissions are always shown first.
- **`plugin.updates { refresh? }`** → `updates: [{ plugin, from, to, registry, requested,
  widens }]`. These are registry plugins whose source lists a higher, non-yanked version
  the kernel can run. It never lists a lower version. `widens` comes from a pure function
  (`Permissions::widens`): a new `spawn`, network host or secret, a higher `files` level,
  `tools` turned on, or a raised limit.
- **`plugin.update { plugin, version, grant? }`.** It downloads and verifies like an
  install, then replaces the old version atomically: the new directory is staged, the
  record switched, the providers reconciled, and the old directory removed. If the new
  version's permissions are no wider, the plugin keeps its state and gets the new
  requested set (which may be narrower). If they are wider, `grant` must equal the new
  requested set, otherwise `conflict` ("this update asks for more permissions") and
  nothing changes: the plugin stays on the old version. While one of the plugin's turns is
  running, it fails with `conflict` rather than cancelling the turn.
- **Never silent.** No code path calls `plugin.update` except an explicit user action
  (CLI or UI), and there is no scheduler. A test runs a core whose registry has a newer
  version, past checks for updates, and the plugin stays where it was.
- **`registry.list`, `registry.add { url }`, `registry.remove { url }`** edit the core's
  list (any client that may install plugins may edit it). **`registry.search { query?,
  refresh? }`** → `{ results, fetched }`: core-side kinds only, each with the registry
  name, publisher, key id, latest version, permissions and repository.

### Lockfiles

- **Core:** `<core data>/plugins.lock`, rewritten atomically after every install, update,
  enable, disable and uninstall, from the `plugins` table (which stays the truth). TOML:

  ```toml
  # Written by Kumikit. Export it to reproduce these plugins on another core.
  format = 1
  [[plugin]]
  id = "acme.probe"
  version = "0.1.0"
  sha256 = "…"
  source = "registry+https://kumikit.github.io/registry/index.json"   # or "file", "bundled"
  key = "ed25519:…"                                                    # if signed
  state = "enabled"
  granted = { spawn = ["fake-acp-agent"] }
  ```

- **`plugin.lock`** → `{ lock }` (the text) for export. Import is driven by the client
  (CLI or UI), so the user sees everything before anything is granted:
  - Entries already installed at that version are skipped.
  - Registry entries are installed with `plugin.install { registry: { id, version, sha256 } }`.
    The registry must already be in the core's list: an import does not add registries.
  - `file` and `bundled` entries are reported ("install … from its file").
  - Then all requested permissions are shown at once, one confirmation is asked, and the
    entries that were `enabled` are enabled with `plugin.enable`.
  - A hash that differs from the lock is refused.
- **CLI:** `kumikit plugin lock export [-o <file>]` and
  `kumikit plugin lock import <file> [--yes]`.
- **Client:** the shell writes `<client data>/plugins.lock` for installed (not bundled) UI
  plugins and themes: id, version, sha256, source and key. Trust, the UI-plugin grant,
  stays in the UI kernel's settings. The UI kernel adds `trusted` and `enabled` when it
  exports, and asks for trust again when it imports (Choice F, Q19).
- **UI:** in the plugin manager, per core and for this app: "Export lock file" shows the
  text with Copy (no save dialog, so no new Tauri permission). "Import lock file" reads a
  file through a standard `<input type="file">` (which M3 showed works under both
  WebDrivers) and then shows the plan and the permissions.

### CLI

```sh
kumikit plugin add <id>[@<version>] [--registry <url>] [--yes]   # an id unless it is an existing file or ends in .kkp
kumikit plugin add ./my-plugin-0.1.0.kkp [--yes]                 # unchanged; now verifies a SIGNATURE if present
kumikit plugin search <words>
kumikit plugin update [<id>] [--yes] [--allow-wider]
kumikit plugin registry list | add <url> | remove <url>
kumikit plugin lock export [-o <file>] | import <file> [--yes]
kumikit plugin keygen [--out <file>] [--force]
kumikit plugin pack <dir> [-o <file>] [--key <file>]
kumikit plugin sign <file.kkp> --key <file>
kumikit plugin verify <file.kkp> [--key <id>]
kumikit theme check <dir | file.kkp> [--json]
```

- All of these work through the running core or, when none runs, on the data directory,
  as today.
- `add` prints the publisher, key id, registry, repository and permissions in plain words,
  then asks.
- `update` lists the available updates, then asks per update. Updates that widen
  permissions show the difference and are applied only with an interactive yes or
  `--allow-wider`: `--yes` alone never widens (Q17).
- Without a terminal and without `--yes`, nothing is applied.
- `keygen`, `pack`, `sign`, `verify` and `theme check` need no core.

### Protocol changes (0.4 → 0.5)

All additive, so all **minor** per AGENTS.md. During 0.x both sides need the same minor, so
the version goes to 0.5 (a 0.4 remote core shows "different versions" in its tabs, as in
M3). TypeScript is regenerated, and fixtures are added for every new method, field and
error code.

| Change | Class |
|---|---|
| `plugin.install { registry: { id, version?, sha256? } }`, a third exclusive source | minor (new optional field) |
| `plugin.updates { refresh? }` → `{ updates: [{ plugin, from, to, registry, requested, widens }] }` | minor (new method) |
| `plugin.update { plugin, version, grant? }` → `{ plugin }` | minor (new method) |
| `plugin.lock` → `{ lock }` | minor (new method) |
| `registry.list` → `{ registries: [{ url, name? }] }`, `registry.add { url }`, `registry.remove { url }` | minor (new methods) |
| `registry.search { query?, refresh? }` → `{ results, fetched }` | minor (new method) |
| `InstalledPlugin` gains `publisher? { key, registry? }` and `source?` (`registry`, `file`, `bundled`); `signed` is now meaningful | minor (new optional fields) |
| Error code `untrusted`: hash, signature, pinned-key or manifest-vs-index mismatch | minor (new code) |
| Error code `unavailable`: a registry could not be reached or answered badly | minor (new code) |
| `capabilities` gains `registry` | minor (new string) |

Unchanged: the handshake, threads, events, `auth.*`. The panel bridge is client-side, not
protocol.

### Starter pack

- On a client's first run (no `firstRun` record in client settings) the UI kernel shows a
  welcome screen. It explains that Kumikit starts small and offers two choices: "Browse
  the starter pack", with the line "This contacts the plugin registry at <host>", and "Not
  now". Until a choice is made nothing is fetched. "Not now" is remembered; the screen
  stays reachable from the palette ("Starter pack").
- "Browse" fetches the index (the shell's for UI plugins and themes, the local core's
  `registry.search` for core plugins) and lists the ids in the index's `starter` list.
  Each entry shows its publisher, description and permissions. Each has its own Install
  button, which goes through the normal install and permission dialog. Nothing is
  preselected, and there is no "install all".
- The list is data from the registry, so the UI kernel names no plugin. In the browser
  client only core-plugin entries are shown.
- A UI plugin whose `requires.core` names a starter core plugin is shown with it ("needs
  Git on this core").

### Level-2 panels (07-ui-plugins)

**Manifest.** `isolation = "iframe"` is accepted. `[ui] entry` names an HTML file. A new
optional `[ui] network = ["api.example.com"]` lists hosts the panel may contact
(`https://` only), shown at install. A panel contributes through
`[contributes] slots = [{ slot, id, title }]`, which can be `sidebar.section`,
`inspector.tab`, `view` and `settings.page` (Q16). Panels are not allowed in
`transcript.renderer`, `composer.accessory`, `statusbar.item`, `thread.header` or
`thread.view` (an iframe per item is too expensive, and the kernel's own surfaces stay
kernel-drawn). The panel's code loads only when its slot is shown.

**The frame.** `<iframe sandbox="allow-scripts" src="<plugin url>/<entry>" referrerpolicy="no-referrer" allow="">`.
- There is no `allow-same-origin`, so the panel has an opaque origin. It cannot read the
  parent's DOM, storage or cookies, nor the app's IndexedDB.
- No `allow-top-navigation`, `allow-popups`, `allow-forms` or `allow-modals`.
- The app CSP's `frame-src 'none'` becomes `frame-src <plugin scheme>` (desktop) or
  `frame-src 'self'` (browser client).
- If the frame loads a second document (it navigated itself), the kernel removes the
  frame and closes its bridge, and the slot shows "This panel navigated away and was
  closed". A panel can therefore never hand its port to another page.

**A CSP for each panel**, built in `kumikit-assets` and sent with every response from a
panel plugin's directory (shell and core):

```
default-src 'none'; script-src <own dir>; style-src <own dir>; img-src <own dir> data:;
font-src <own dir>; connect-src <declared https hosts, or 'none'>; frame-src 'none';
form-action 'none'; base-uri 'none'; object-src 'none'
```

- `<own dir>` is `kumikit-plugin://<id>`, `http://kumikit-plugin.localhost/<id>/` on
  WebView2, or `/ui-plugins/<id>/` in the browser client.
- Module scripts from an opaque origin need CORS. The panel's own responses then carry
  `Access-Control-Allow-Origin: null`. Step 2's spike decides whether that is needed and
  safe on each webview. The fallback is classic scripts only.

**The bridge.**
- When the frame loads, the kernel creates a `MessageChannel` and sends port 2 to
  `iframe.contentWindow` with `{ type: "kumikit.panel", version: 1 }`. The target origin
  has to be `*` because the frame's origin is opaque. That is safe because it goes to that
  window object, once, at its first load.
- Every later message travels on the port. Messages on `window` from any other source are
  ignored.
- Messages are JSON-RPC-shaped (`{ id, method, params }` → `{ id, result | error }`, plus
  notifications), with a 1 MiB limit and at most 64 calls in flight.

**The panel SDK** (`@kumikit/ui-sdk/panel`, framework-agnostic, about 3 KB):
`connectPanel(): Promise<PanelContext>`. It is the level-3 API made asynchronous:
- `ctx.slot`, `ctx.props` (the tab's address);
- `ctx.connection.call(method, params)` and `ctx.connection.subscribe(topic, after, fn)`
  (fanned out from the kernel's subscriptions, as at level 3);
- `ctx.threads.watch(address, fn)`;
- `ctx.theme.tokens`, `appearance` and `onChange(fn)`;
- `ctx.storage.get/set/delete` (async, per plugin);
- `ctx.notify(message)`;
- `ctx.commands.run(id, args)`, limited to the plugin's own commands.

The SDK applies the tokens to the panel's document as `--kk-*` custom properties through
a constructed stylesheet, so the panel's CSP needs no inline styles. Tokens are pushed
again on every theme or appearance change.

**What a panel may call**, enforced in the UI kernel by a pure policy function tested in
Vitest:
- Only the tab's own connection. No other core, no profiles or shell commands, no
  `plugin.*`, `settings.*`, `secret.*`, `auth.*` or `core.shutdown`, and no thread
  writes.
- Read-only kernel methods: `workspace.list`, `thread.list`, `thread.get`,
  `provider.list`, `provider.models`, `plugin.list`, and subscriptions to `thread:` and
  `workspace:` topics.
- Thread and workspace reads are limited to the tab's workspace (Q10). The kernel's thread
  store already knows which workspace each thread is in.
- `<pluginId>/*` methods for each plugin in its `requires.core`. No core plugin answers
  these until a later WIT adds `handle-rpc` (M5), so today they reach the core and get
  `method_not_found`. The forwarding is built and tested now.

**Panels get no shell commands.** Tauri's IPC must not be reachable from a panel. The
spike checks this on every webview: `__TAURI_INTERNALS__` is absent from the frame, and
`ipc://` fetches and `chrome.webview.postMessage` fail. A permanent test in the desktop
exit run proves it. Tauri's capability file also limits the commands to the app's own
origin (`local: true`, no remote URLs), so the opaque origin does not match.

**Trust.** Iframe plugins need no trust prompt: they show their slots, the methods they
may call and their network hosts. Inline (level 3) keeps the trust prompt. A
third-party plugin can't switch from iframe to inline in an update without being asked
again (the client's "widening").

**UI SDK version.** Panels add API but no slots or tokens. The SDK goes to **0.1.1**
(additive, so `^0.1` plugins keep working). A panel plugin declares `api = "^0.1.1"`. The
published lists don't change, so `web/sdk/api/` gets no new directory.

### `kumikit theme check` (08-themes)

The rules are in TypeScript today (`web/kernel/src/theme/validate.ts`, `contrast.ts`). The
CLI is Rust. Options: port the rules to Rust and share them as data and test vectors;
compile Rust to WASM for the webview; embed a JS engine in the CLI; or make the check an
npm script. Recommend **port, with the data and vectors shared** (Choice H):
- The token list stays in `web/sdk/api/<version>/tokens.json` (read by both). The contrast
  pairs move from `contrast.ts` into `web/sdk/theme-rules.json`, also read by both. The
  file is not published, so it can be tuned without a new SDK version.
- A new crate, `kumikit-theme` (`serde_json` only), implements type checks, contrast and
  font checks. The font check reads the `wOF2`/`wOFF` magic and requires the file to be in
  the package.
- A shared vector file, `web/sdk/tests/theme-vectors.json` (values with their type and
  expected verdict, colour pairs with expected ratios to two decimals), must pass in both
  Vitest and `cargo test`. So the two implementations cannot drift without a test failing,
  as with `status.json` for the thread fold.
- `kumikit theme check <dir|kkp>` also checks the manifest's structure. That check moves
  from `kumikit-shell/src/store.rs` into a module shared with the CLI and the registry
  checker (`kumikit-package::ui_manifest`), and `sdk/schema/ui-plugin-manifest.schema.json`
  is written (07 names it, but it doesn't exist: Q6).
- Contrast failures are warnings (exit 0, with `--strict` they fail); invalid values and
  missing fonts are errors. The first-party theme's existing contrast test moves to the
  shared vectors plus a `kumikit theme check plugins/theme-default --strict` in CI.

### Author templates

`templates/` (excluded from the cargo and npm workspaces, so they build like a third
party's project):
- **`core-plugin-rust/`**: a minimal provider (`kumikit:provider/plugin@0.1.0`) that
  echoes, plus `package/plugin.toml`, `build.sh` (cargo build for `wasm32-wasip2`, copy to
  `package/plugin.wasm`, `kumikit plugin pack package --key …`) and a README.
- **`ui-plugin-svelte/`**: Svelte 5 + Vite, an `inspector.tab` iframe panel using
  `@kumikit/ui-sdk/panel` (the level third parties should start with), `plugin.toml`,
  `build.sh`, README.
- **`theme/`**: `plugin.toml`, `light.json`, `dark.json`, README. `kumikit theme check .`
  passes.

The SDK dependency is a path in this repository, rewritten by the check script. For
authors the README gives the git form, until Q5 is answered.
`scripts/check-templates.sh` copies each template to a temp directory outside the
workspace, points its SDK dependency at this checkout, builds it, generates a key, packs
and signs. Then:
- the core template is installed and enabled in a fresh core through `kumikit plugin add
  --yes` (offline path), and one echo turn runs through `kumikit`'s test client;
- the UI plugin and the theme are installed into a fresh client store through
  `test-plugins/client-store` (a tiny binary over `kumikit_shell::store`);
- the theme passes `kumikit theme check --strict`.

It runs in a new CI job, `templates` (Linux).

### Decisions on interfaces

- **Freezing (Q4, ADR 0020).** ADRs 0010 and 0014 say WIT and the UI SDK freeze at 1.0
  "before third parties publish". M4 builds publishing, but M5 is where the first real
  plugins (git, GitHub, bot, tool gateway) will press on these interfaces. They need
  `handle-rpc`, `run-command`, `settings`, `http`, `files` and `timers`. Most of that is
  additive, but `handle-rpc` and the bot `client` world will likely reshape
  `kumikit:plugin` and the method-routing story. The level-2 bridge is also brand new.
  Recommendation:
  - **Do not freeze WIT and the UI SDK at 1.0 in M4.** WIT stays at 0.1.0 (M4 needs no WIT
    change); the UI SDK goes to 0.1.1.
  - **Do freeze the ecosystem formats M4 creates**, versioned from the start: `SIGNATURE`
    v1, registry `format = 1`, lockfile `format = 1`, and the manifest schemas as
    published files. These are what third-party tooling depends on.
  - **Until 1.0, the official registry accepts only allowlisted publishers**: `kumikit`,
    plus anyone you add as an early tester who accepts breakage. The checker enforces it
    with an `early = true` flag in the publisher file. Anyone can still run their own
    registry.
  - **Freeze after M5:** review WIT and the UI SDK with M5's plugins in hand, publish
    1.0.0, then open the registry. ADR 0020 records this, and ADRs 0010 and 0014 get a
    "see 0020" line. Changes queued for that review: `handle-rpc` and `run-command` in
    `kumikit:plugin`; `settings`, `http`, `files` and `timers` in `kumikit:host`; whether
    `api` ranges stay separate for core and UI; `ctx.connection` limits at level 3; the
    final slot list (a `panel`-capable flag per slot).
  - If you'd rather freeze in M4, it becomes a step before step 15: a WIT and SDK review,
    then 1.0.0 directories, `api = "^1"` everywhere, and the two ADRs superseded.
- **Sidecars (roadmap open question, ADR 0021).** Recommend: **not in 1.0.** The
  `runtime.sidecar` key stays refused by the validator, and the open question is closed
  as "after 1.0, if a real plugin cannot be built as WASM plus `spawn`". The reasons: a
  sidecar is unsandboxed native code, which breaks default deny. A WASM plugin with
  `spawn` already runs any program, and the user is told so at install. It would add a
  second host, a JSON-RPC mapping of every WIT interface, and a second permission story to
  maintain before 1.0. Nothing is built.

## Security: new surfaces against 11-security

| Threat | Control | Proved by |
|---|---|---|
| A tampered package (in transit or on the host) | SHA-256 from the index checked before unpacking anything into place | `registry_install_refuses_hash_mismatch` (core), shell `client_install_refuses_hash_mismatch` |
| A forged or partial signature | Ed25519 over the domain-separated file list; every file must match the list, none extra or missing | `signature_round_trip`, `extra_file_refused`, `missing_file_refused`, `changed_file_refused`, `duplicate_or_unsafe_path_in_list_refused`, `wrong_key_refused`, `malformed_signature_refused` (`kumikit-package`) |
| An unsigned package through a registry | Registry packages must be signed by the publisher's key | `registry_package_must_be_signed` (core, shell), checker `unsigned_version_fails` |
| A registry entry that lies about permissions | The index copy must equal the package's manifest | `install_refuses_index_manifest_mismatch`, checker `copied_fields_must_match` |
| Key substitution for an installed plugin | Key pinned per plugin; registry CI checks continuity | `update_refuses_changed_key`, `unsigned_cannot_replace_pinned`, checker `publisher_key_change_fails` |
| A second registry hijacks updates | Updates come only from the install's source registry | `update_only_from_source_registry` |
| A downgrade served as an update | Updates only to a higher, non-yanked version | `updates_never_downgrade`, `yanked_versions_are_not_offered` |
| A typo-squatted id | Confusable check in registry CI; install shows publisher, key id, repository | checker `confusable_ids_refused` (vectors: `kumikit`/`kumikti`, `acme.git`/`acme.gjt`, `rn`/`m`), Playwright and CLI tests assert the dialog text |
| A network attacker on registry traffic | HTTPS with the OS trust store; `http` only to loopback; https-only redirects | `plain_http_refused_off_loopback`, `redirect_to_http_refused`, `https_fetch_with_test_root` (test CA injected through the library API, loopback) |
| An oversized or slow registry or package | Size caps, timeouts, redirect limit | `index_size_cap`, `package_size_cap`, `slow_server_times_out` (paused clock), `redirect_loop_stops` |
| Silent updates | Nothing applies an update without an explicit call; widening needs a matching grant | `updates_are_never_applied_without_a_call`, `update_widening_needs_grant`, `narrowing_update_keeps_state`, Vitest `update_dialog_shows_new_permissions` |
| Automatic registry traffic | Fetch only on browse, search, install, import, update check; no scheduler | `serve_makes_no_registry_request` and Playwright `first_run_contacts_nothing_until_asked` (fixture server counts requests) |
| The starter pack installs something | Each entry is installed by its own click, through the permission dialog | Playwright `starter_pack_installs_only_what_is_chosen` |
| A lockfile import installs something else | Hash pinned in the lock; registries not added by import | `lock_import_refuses_hash_mismatch`, `lock_import_needs_known_registry` |
| A publisher's private key leaks from Kumikit | 0600 key files outside the work tree; never logged or printed; `check-no-keys.sh` in CI | `keygen_writes_0600_outside_repo`, `keygen_refuses_git_work_tree`, `signing_key_never_logged` (trace capture), `group_readable_key_refused` (Unix) |
| A panel reads other tabs or the app | Opaque-origin sandbox; bridge limited to the tab's connection and workspace | Playwright and desktop `panel_cannot_read_parent`, Vitest `policy_*` (write methods, other connection, other workspace, undeclared plugin methods) |
| A panel reaches the shell (profiles, tokens) | No IPC in the frame; capabilities limited to the app origin; no profile methods on the bridge | desktop exit `panel_cannot_reach_shell` (Linux container, Windows CI), macOS smoke (test hooks, CI) |
| A panel exfiltrates data | Per-panel CSP `connect-src` = declared hosts only | Playwright `panel_network_blocked` (CSP violation event, no request reaches a loopback counter), `declared_host_allowed` |
| A panel navigates or pops up | Sandbox without top navigation or popups; a second load tears it down | Playwright `panel_cannot_navigate_top`, `panel_cannot_open_popup`, `panel_navigating_away_is_closed` |
| Forged bridge messages | Only the transferred port is read; `window` messages from other sources ignored | Vitest `bridge_ignores_foreign_messages`, Playwright `sibling_panel_cannot_drive_bridge` |
| A malicious theme | Unchanged: typed tokens; the same rules in the CLI and the kernel | shared `theme-vectors.json` in Vitest and `kumikit-theme` |

11-security gains rows for publisher keys, registry traffic, updates, lockfiles and
panels, and drops "(M4)" from the panel row.

## Tests

All deterministic: temp directories, loopback only, keys generated at test time, injected
clocks, a deadline on every async wait (M3 lesson), and test clients that read
notifications while waiting.

- **`kumikit-package`:** the signature format, sign and verify, and every refusal above.
  Deterministic packing with a signature. `sign` replacing an old signature.
- **`kumikit-registry`:** parsing (index, plugin and publisher files, `format` refusal),
  resolution (latest compatible, yanked, explicit version), fetch over loopback HTTP and
  HTTPS with an injected root, the cache and `If-None-Match`, limits and redirects, and
  the checker against fixture registries (each refusal). Fixture registries are built by
  `test-plugins/registry-fixture` (a library plus a small binary for the Node tests). It
  writes publisher and plugin files, packs and signs test plugins with fresh keys, builds
  `index.json` and serves a directory over loopback HTTP, counting requests.
- **Kernel:** migration 0005 (`publisher_key`, `source`), and record round trips.
- **Core:**
  - install from the registry, every `untrusted` path, pinning;
  - updates (list, apply, widen, narrow, refused while a turn runs, never silent, no
    downgrade, source pinning);
  - the lock written after each change, export, and import (CLI with `--yes`);
  - the registry methods, and `unavailable`;
  - CLI `add <id>`, `search`, `update`, `registry`, `lock`, `keygen`, `pack --key`,
    `sign`, `verify`;
  - `serve_makes_no_registry_request`;
  - `every_method_is_routed`, which covers the new methods.
- **Shell:** client install from a registry (hash, signature, pin, `untrusted`), the
  client lock, the registries file, the iframe manifest accepted, the panel CSP on
  responses (`kumikit-assets` unit tests for each scheme form), and that no command
  returns key material.
- **`kumikit-theme`:** the shared vectors, fonts, manifest; `kumikit theme check` on good
  and bad fixtures.
- **Vitest:**
  - the panel policy;
  - the bridge (a fake frame and port);
  - token push on theme change;
  - navigation tear-down logic;
  - the updates view model (widening shown);
  - the starter-pack state (nothing fetched before the choice);
  - lock export and import plans;
  - the theme vectors.
- **Playwright** (the relay serves the fixture registry on loopback and points
  `KUMIKIT_DEFAULT_REGISTRY` at it for its cores and its client store):
  - `registry.spec.ts`: search, details, install on the local and on the remote core,
    update with widening, lock export and import;
  - `panels.spec.ts`: a test panel plugin (`test-plugins/ui-panel`) showing tokens, calling
    allowed methods, refused calls, the network block, navigation and popups;
  - `starter.spec.ts`;
  - `exit-m4.spec.ts` (below);
  - the browser client: a core-plugin install from the registry, and a panel from its
    bundle;
  - axe on the new screens in both themes;
  - Chromium locally; WebKit in CI.
- **Desktop exit test** (`web/e2e-desktop/exit-m4.test.mjs`): the same exit flow through
  the real shell. Linux in the container; Windows in CI.
- **macOS panel smoke test.** macOS has no WebDriver for WKWebView, so the shell's
  `test-hooks` build gets one hook: `KUMIKIT_TEST_REPORT=1` makes the UI kernel send test
  results to stdout through a test-only command. A test UI plugin with a panel checks, in
  WKWebView, that the custom scheme serves the frame with its CSP, that the bridge works,
  and that a blocked fetch and IPC from the frame fail. It then reports. This runs in CI
  on macOS (and on Linux and Windows too, for parity). It is the only automated check of
  the panel CSP in WKWebView.
- **Templates:** `scripts/check-templates.sh` (above).
- **Live:** unchanged. M4 adds no live test, since everything is local.

### The exit test (simulated third party)

1. **Publisher, at test time:** `kumikit plugin keygen` into a temp directory (publisher
   `acme`). The probe test plugin's component is packed with an `acme.probe` manifest
   (`spawn = ["fake-acp-agent"]`, no secrets) and signed with that key. The fixture
   registry gets `publishers/acme.toml`, `plugins/acme/probe.toml` and
   `starter = ["acme.probe"]`. The test runs `kumikit-registry check` on it (it passes),
   then `build-index`, and serves it on loopback.
2. **Fresh machine:** a new core data directory and a new client data directory, with
   `KUMIKIT_DEFAULT_REGISTRY` set to the fixture URL. The fixture server has seen **zero
   requests** after the app has started and shown the welcome screen.
3. "Browse the starter pack" → `acme.probe` is listed with publisher Acme, its key id
   and repository → Install, on the local core → the dialog shows **"can start the
   `fake-acp-agent` program, which runs with your full user access"**, "signed by Acme" and
   the key id → Allow → it is enabled. `plugin.list` says `signed: true` with the
   publisher key and the registry source.
4. Open a workspace, create a thread with the probe provider:
   - `spawn fake-acp-agent …` succeeds: the granted permission works;
   - `secret anthropic-api-key` makes the turn fail with **`permission_denied`**, shown in
     the transcript: a host call outside the grant is refused;
   - `spawn sh` also fails with `permission_denied`.
5. Export the lock: it lists `acme.probe` with its hash, key and grant.
6. Tamper: the fixture publishes 0.2.0 signed by another key → "Check for updates" shows
   no installable update; the CLI's update reports the key change. Then 0.2.0 signed by the
   right key and requesting `secrets = ["x"]` → shown as widening; the plugin stays on
   0.1.0 until allowed.

The Rust version (`crates/kumikit-core/tests/exit_m4.rs`) drives steps 1–6 through the CLI
and the protocol (with `serve_makes_no_registry_request` for step 2). Playwright and the
desktop drive steps 2–5 through the UI, plus a UI panel plugin from the same publisher
installed into the client store. The roadmap's M4 status names all three.

## Footprint

Budgets unchanged. Expected growth:
- core: the HTTP client connection, `rustls-native-certs`, ed25519, the registry and theme
  code: about +0.5–1 MiB, to about 29–29.5 MiB of 32;
- shell: about the same;
- installers: under +1 MB (largest 15.6 of 20 MB);
- UI kernel: browse, updates, lock, starter and the panel host, about +10–15 KB gzip, to
  about 70 KB of 150 KB.

The panel SDK ships in each panel plugin, not the kernel.

| Measure | How | Gate |
|---|---|---|
| Core cold start and idle (remote off and on) | `scripts/footprint.sh` as today | existing budgets; plus a check that startup opens no registry file |
| Core idle after a registry search and an install | new phase in `crates/kumikit-core/tests/footprint.rs` (loopback fixture) | the same idle budget (≤ 30 MB) |
| Core binary | `scripts/footprint.sh` | ≤ 32 MiB |
| Shell anonymous memory | `scripts/desktop/footprint.sh`; a second run with one panel open, reported only | ≤ 60 MB (first run) |
| Installers | `installers` job, `scripts/check-installer-size.mjs` | ≤ 20 MB (AppImage exempt) |
| UI kernel | `web/kernel/scripts/size.mjs` in the `footprint` job | ≤ 150 KB |

Step 2 measures the dependencies before any feature code. If the core would pass 30 MiB
or an installer 18 MB, I stop and report with a per-crate breakdown rather than touch a
budget.

## CI

| Job | Change |
|---|---|
| `docs` | add `sh scripts/check-no-keys.sh` |
| `rust` (3 OSs) | none in commands: the new crates, `exit_m4.rs`, signing and the registry tests run in `cargo test --workspace` |
| `web` | the theme vectors and panel policy run in `npm test`; `kumikit theme check --strict plugins/theme-default` in a step |
| `e2e-browser` (Linux, macOS) | build adds `-p kumikit-registry -p kumikit-test-registry-fixture`; new specs run in the same `npx playwright test` |
| `e2e-desktop` (Linux, Windows) | `e2e.sh` (and the Windows step) also runs `exit-m4.test.mjs` |
| `panel-smoke` (new; macOS, Linux and Windows) | builds the shell with `test-hooks`, runs the app with `KUMIKIT_TEST_REPORT=1` and the test panel plugin, and checks the report |
| `templates` (new, Linux) | `sh scripts/check-templates.sh` |
| `footprint` (Linux) | the new phase is in `scripts/footprint.sh` |
| `installers` (3 OSs), `licenses` | none; size and `cargo deny` catch growth (`rustls-native-certs` and its platform crates, `schannel` on Windows, `security-framework` already present on macOS) |

**Registry CI** lives in `kumikit/registry` (step R, only with your approval). It runs on
pull requests: `cargo install --locked --git https://github.com/kumikit/kumikit --tag <tag>
kumikit-registry`, then `kumikit-registry check . --base <main checkout>` and a
publisher-login check. On `main` it runs `build-index` and deploys `index.json` to Pages.
Until that repository exists, this repo's tests run the same checker on fixture registries,
so it can't rot.

AGENTS.md Verify gains:
- `sh scripts/check-no-keys.sh`
- `sh scripts/check-templates.sh` (needs the `wasm32-wasip2` target and npm)
- `cargo run -p kumikit-core -- theme check --strict plugins/theme-default`
- the browser e2e line adds `-p kumikit-registry -p kumikit-test-registry-fixture` to the
  build
- the desktop exit line says M2, M3 and M4
- the clippy wasm line adds the template's crate if it is built there (it isn't a workspace
  member, so `check-templates.sh` runs its clippy)
- the status paragraph says M4 is built, at the end

## Steps

Each step ends with the Verify checks that apply and one commit on branch `m4-ecosystem`.
*(CI)* marks what only runs in CI (macOS, Windows, and WebKit). Pushes are batched:
- first push (draft PR) after step 2, because the panel spike needs WebView2 and WKWebView;
- second push after step 13 (desktop exit test on Windows, macOS smoke);
- last push after step 16.

- [x] **1. Decisions and spec (docs only).** ADR 0018 (signing and the registry: format v1,
  ring, publisher files, `index.json`, OS roots, loopback `http`, pinning, no rotation, no
  signed index, each side fetches what it installs, the shell's new commands extending
  ADR 0013). ADR 0019 (level-2 panels: sandbox, per-panel CSP, bridge, method policy,
  allowed slots, no shell IPC). ADR 0020 (interface freeze timing and early publishers;
  "see 0020" lines in 0010 and 0014). ADR 0021 (sidecars after 1.0). Spec changes:
  - 01: new crates, `templates/`, dependency rules;
  - 02: 0.5 methods, `untrusted` and `unavailable`, `registry` capability, `InstalledPlugin`
    fields;
  - 03: `plugins` columns, lockfile;
  - 04: `SIGNATURE` format, pinning, update rules, sidecar status;
  - 06: the shell's registry commands, `KUMIKIT_DEFAULT_REGISTRY`, plugin manager
    browse/updates/lock, starter screen;
  - 07: iframe panels in full, SDK 0.1.1, `[ui] network`;
  - 08: `theme check`, shared rules;
  - 10: registry layout, index, fetch rules, CLI, updates, lockfiles, starter pack, no
    `--core`;
  - 11: the rows above;
  - 12: the open question closed, M4 bullets.

  Verify: check-docs. Commit: `docs: settle M4 ecosystem design`.
- [x] **2. Spike: dependencies, sizes, panels on every webview.**
  - Add the HTTP client connection, `rustls-native-certs` and ring's ed25519 use behind
    one loopback test. Run `cargo deny`: anything outside the list stops the step and I
    ask. Measure the core binary and the `.deb` (container) before and after.
  - Panel spike: a sandboxed iframe from the plugin scheme, with its CSP. Module vs classic
    scripts (CORS from an opaque origin), the `MessageChannel` handshake, and IPC
    unreachable from the frame. Run it on WebKitGTK (container), Chromium and WebKit
    (Playwright, local Chromium), and *(CI)* WebView2 and WKWebView (the smoke hook, in
    its first form).
  - Record the results in Notes; anything that forces a design change comes back to you
    before step 3.

  Commit: `chore: add registry fetch dependencies and a panel spike`. **First push, draft
  PR.**
- [x] **3. Signing.** `kumikit-package::signature`. CLI `keygen`, `pack --key`, `sign`,
  `verify`. Core install verifies a signature when present and pins the key (migration
  0005). `check-no-keys.sh`, `.gitignore`. Tests above. Commit: `feat: sign and verify
  plugin packages`.
- [x] **4. Registry crate and checker.** `kumikit-registry`: types, fetch and cache,
  resolve, install-time checks, and the `check`/`build-index` binary with confusable ids
  and key continuity. `test-plugins/registry-fixture`. Commit: `feat: read and check plugin
  registries`.
- [x] **5. Protocol 0.5.** The types and methods above, error codes, fixtures, TypeScript
  regenerated, `tsc` check of `web/protocol`; the core routes the new methods (`unsupported`
  stubs where step 6 fills them). Commit: `feat: add registry and update methods to the
  protocol`.
- [x] **6. Core: registry install, updates, lockfile, CLI.** The `registry.*` methods,
  `plugin.install { registry }`, `plugin.updates`/`update`, `plugin.lock`; the registries
  file; `plugins.lock` writing; CLI `add <id>`, `search`, `update`, `registry`, `lock`;
  README privacy sentence. Commit: `feat: install and update core plugins from registries`.
- [x] **7. Core exit test.** `tests/exit_m4.rs` (steps 1–6 of the exit flow) and
  `serve_makes_no_registry_request`. Commit: `test: add the M4 exit test for the core`.
- [ ] **8. Theme check.** `kumikit-theme`, `web/sdk/theme-rules.json`, shared vectors in
  Vitest and cargo, `kumikit-package::ui_manifest` (moved from the shell's store, no
  behaviour change, as its own commit), `sdk/schema/ui-plugin-manifest.schema.json`,
  `kumikit theme check`. Commits: `refactor: share the UI manifest check`, `feat: check
  themes from the CLI`.
- [ ] **9. Shell: client registry, signatures, lockfile.** Install from a registry with
  verification and pinning, the client lockfile, the registries file. New commands
  (`client_registry_search`, `client_registries_get/set`, `client_plugins_install` taking
  `{ path } | { registry }`, `client_plugins_updates`, `client_plugins_update`,
  `client_lock`) with capabilities and `build.rs`. Commit: `feat: install UI plugins and
  themes from registries`.
- [ ] **10. UI kernel: browse, updates, lock, starter pack.**
  - plugin manager browse and search (core and app), the details and permission dialog
    with publisher and key, Check for updates with the widening approval, lock export and
    import, Settings → Registries;
  - the welcome screen;
  - `npm run check`, `npm test`, size.

  Commit: `feat: browse, install and update plugins from the app`.
- [ ] **11. Level-2 panels.**
  - `kumikit-assets` panel CSP and `frame-src`; the shell and the core serve it;
  - the store and the shared manifest check accept `iframe` and `[ui] network`;
  - the UI kernel's panel host, bridge and policy;
  - `@kumikit/ui-sdk/panel` and SDK 0.1.1;
  - `test-plugins/ui-panel`;
  - Vitest.

  Commit: `feat: run UI plugins in sandboxed panels`.
- [ ] **12. Playwright suites.** Relay serves the fixture registry; `registry`, `panels`,
  `starter`, `exit-m4` specs; browser-client additions; axe. *(CI: WebKit.)* Commit: `test:
  drive registry installs and panels from the browser build`.
- [ ] **13. Desktop exit test and macOS smoke.** `exit-m4.test.mjs` in `e2e.sh` and the
  Windows step; `panel-smoke` job. Linux in the container; *(CI: Windows, macOS.)* Commit:
  `test: add M4 exit-criteria test`. **Second push.**
- [ ] **14. Templates.** `templates/*`, `test-plugins/client-store`,
  `scripts/check-templates.sh`, `templates` CI job, README pointers. Commit: `feat: add
  plugin author templates`.
- [ ] **15. Footprint and installers.** The new phase, the panel run (reported), numbers in
  Notes. Commit: `chore: measure the M4 footprint`.
- [ ] **16. Close out.** Spec matches the code; roadmap M4 status; AGENTS.md status and
  Verify; agent reference (signature format pitfalls, the registry cache, the panel
  bridge, the smoke hook); README (privacy, installing plugins); plan `done`. Commit:
  `docs: close M4`. **Last push**; mark the PR ready.
- [ ] **R. The real registry (only after you approve, any time after step 6).** Create
  `github.com/kumikit/registry` with the layout above, its README (how to publish), the
  workflow, `publishers/kumikit.toml` (with a key you generate and keep: I don't hold the
  project's private key), and Pages for `index.json`. Nothing is created, pushed or
  published without your go-ahead, and the exit criteria don't depend on it.

## Choices the spec leaves open

| # | Choice | Options | Recommendation |
|---|---|---|---|
| A | Who fetches from the registry | core for everything (UI plugins passed to the shell as bytes); shell for everything (uploads core plugins); each side for what it installs | **Each side for what it installs.** A remote core downloads its own plugins (no upload, works from the browser client). The shell gets UI plugins without going through a core. The code is shared in `kumikit-registry`. Cost: the shell gains an HTTP fetch and a few commands (ADR 0018). |
| B | Signature crate | `ring` (already shipped); `ed25519-dalek` (BSD-3-Clause) | **`ring`**: no new crate, licence already accepted. |
| C | Signature format | minisign; signify; SSH signatures; JWS; a small own text format | **Own v1 text format** (above): readable, a few dozen lines to write and parse, no new dependency, domain-separated. minisign would need its crate, and it signs one blob, not a file list. |
| D | How clients read the registry | git clone (gix or the git binary); raw files per id; one generated `index.json` | **`index.json`** built by registry CI: one small HTTPS GET with an ETag, search works offline from the cache, no git in the core. |
| E | Trust roots for HTTPS | `webpki-roots` (CDLA-Permissive-2.0, not on the list); `rustls-native-certs`; `rustls-platform-verifier` | **`rustls-native-certs`** (within the list): the OS store, loaded only when a fetch happens. |
| F | Client lockfile and trust | trust in the lock written by the shell (move trust into the shell); trust stays in UI-kernel settings and is merged at export | **Merged at export**: no change to where trust lives; the import asks again. |
| G | Updates with wider permissions | stage the new version beside the old; refuse until a matching grant | **Refuse until the grant matches** (`conflict`); the old version keeps running. Nothing is half-installed. |
| H | Sharing theme rules | Rust port with shared data and vectors; Rust→WASM in the webview; JS engine in the CLI; npm script | **Rust port with shared data and test vectors**. |
| I | Signing command | `sign` only; `pack --key` only; both | **Both**: `pack --key` for the common path, `sign` for packages built elsewhere. |
| J | Panel slots | every slot; a fixed set | **`sidebar.section`, `inspector.tab`, `view`, `settings.page`**. |
| K | Panel method scope | the whole core; the tab's workspace | **The tab's workspace** for thread and workspace reads (Q10). |
| L | Registry CI tool | subcommand of `kumikit`; a separate binary | **Separate `kumikit-registry` binary**: keeps maintainer tooling out of the core binary's size. |

New dependencies, all inside the AGENTS.md list unless step 2's `cargo deny` says otherwise:
- `rustls-native-certs` (Apache-2.0 OR ISC OR MIT), and on Windows `schannel` (MIT) through
  it;
- `hyper`'s `client` feature (MIT) and `http-body-util` (already present);
- `ring` and `base64` moved into `kumikit-package`.

No new npm dependencies; the templates use only packages the workspace already has
(Svelte, Vite).

## Questions (unclear or possibly wrong in the spec)

Answered 2026-11-18: Q4 yes (no 1.0 freeze in M4; v1 formats; allowlisted publishers until
1.0; ADR 0020), Q3 yes (the shell fetches UI plugins), Q1 and Q2 yes (`index.json` on
`kumikit.github.io/registry`; publisher files), Q5 git dependencies for now, sidecars not in
1.0 (ADR 0021). The other questions take the recommendation.

1. **Registry hosting.** 10-distribution says the registry is a git repository. I
   recommend clients read a generated `index.json`, served from GitHub Pages at
   `https://kumikit.github.io/registry/index.json`, and never clone it. Is Pages under the
   `kumikit` organisation the URL you want as the default?
2. **Where the publisher key lives.** 10-distribution puts it in each plugin's file. I
   recommend a `publishers/<publisher>.toml` holding the key and the GitHub logins that may
   change it, so a namespace has one key and one set of owners. Agree?
3. **The shell fetching from the network.** Choice A gives the shell an HTTPS fetch and new
   commands for UI plugins (amending ADR 0013 through ADR 0018). Agree, or should UI
   plugins come through the local core?
4. **Freezing.** I recommend not freezing WIT and the UI SDK at 1.0 in M4. Instead:
   versioned v1 formats for signatures, the registry and lockfiles; the official registry
   limited to allowlisted publishers until 1.0; and the freeze after M5 (ADR 0020). This
   changes what ADRs 0010 and 0014 imply about M4. Agree, or freeze in M4?
5. **SDKs for third parties.** A third party needs `kumikit-plugin`/`kumikit-provider` and
   `@kumikit/ui-sdk`. Publishing them to crates.io and npm is outward-facing. Until you
   decide, the templates' READMEs use git dependencies on this repository (cargo supports
   that; for npm, a GitHub release tarball). Do you want publishing planned (as part of
   step R, with your approval), or git dependencies for now?
6. **A missing schema.** 07 says UI manifests are checked against
   `sdk/schema/ui-plugin-manifest.schema.json`, which doesn't exist: the shell checks the
   structure in Rust. I plan to write the schema (step 8) so the spec is true.
7. **"Show permissions → unpack → enable".** 10-distribution lists that order. Today the
   core unpacks into staging to read and validate the manifest before anything is shown,
   and the plan keeps that: nothing goes into place or runs before the grant. I'll reword
   the steps as "download → check hash → unpack into staging → check signature and manifest
   → show permissions → install → enable". Agree?
8. **`--core devbox`.** I recommend dropping it from 10-distribution's example and
   documenting `ssh devbox kumikit plugin add <id>` and the app instead (deferred twice
   now). Agree?
9. **Periodic update checks.** 10-distribution allows "when they turn on periodic update
   checks". I recommend leaving them out of M4 (manual checks only), so M4 adds no
   automatic request at all. Agree?
10. **Panel read scope.** 07 says panels may call "read-only kernel methods". I recommend
    also limiting thread and workspace reads to the tab's workspace, since 11-security's
    threat is "a UI plugin reads other tabs". Agree?
11. **Lost or rotated publisher keys.** Not in M4: a changed key is refused, and the user
    must remove and reinstall the plugin, which the message says. A signed hand-over from
    the old key can come later as `SIGNATURE`/registry format v2. Acceptable?
12. **The index is not signed.** I rely on HTTPS for the index, plus a signature and a
    pinned key for each package. A compromised registry could add a new plugin under a new
    publisher, or swap the key of a plugin nobody has installed, but cannot replace an
    installed plugin. Acceptable for M4?
13. **Unsigned packages in the registry.** The plan refuses them in the checker and at
    install. Agree that a signature is mandatory there?
14. **Signed local files.** These show "signed by key …, not checked against a registry"
    and pin the key. Agree?
15. **Bundled plugins stay unsigned** (the installer vouches for them). If `kumikit.provider-acp`
    is later published to the registry, its first registry update pins that key. Signing
    them needs a project key held as a CI secret, which is yours to set up. Leave it for
    later?
16. **Panel slots.** Is the set `sidebar.section`, `inspector.tab`, `view`, `settings.page`
    right, or do you want `thread.view` panels too? (An iframe cannot mount other plugins'
    renderers inline.)
17. **`update --yes`.** I recommend that `--yes` never widens permissions; that needs an
    interactive yes or `--allow-wider`. `add --yes` keeps granting what it shows. Agree?
18. **Who may edit a core's registries.** Any client that may install plugins (local
    connections and paired devices), since installing from bytes is already allowed. Agree?
19. **Client lock and trust.** Trust stays in the UI kernel and is merged into exports
    (Choice F); importing asks for trust again. Agree?
20. **`http://` registries on loopback.** These are allowed (tests, local mirrors); every
    other host needs `https`. Agree?
21. **Installs on this machine.** None planned. The spike's macOS and Windows checks run in
    CI. `rustls-native-certs` needs no system package on Linux beyond the CA bundle that is
    already there.

## Notes

- 2026-11-18: baseline from M3: core binary 28.4 MiB, largest installer 15.6 MB (`.deb`,
  `.rpm`), UI kernel 57.8 KB gzip, protocol 0.4.
  - `ring` 0.17.14 is in the tree (core and shell, through rustls and rcgen) and does
    ed25519.
  - `hyper` 1.12 is present with only server features; `reqwest` is in `Cargo.lock` only
    through Tauri, and isn't built for the host target with the features the shell uses.
  - `semver` and `toml` are already shell and core dependencies.
  - The probe test plugin already returns host errors unchanged in its `probe` model,
    which the exit test uses for `permission_denied`.
  - `<pluginId>/*` methods are not routed by the core yet (no `handle-rpc` in WIT 0.1),
    so `requires.core` method forwarding in the bridge reaches `method_not_found` until M5.
  - `settings.*`, `secret.*`, `command.*` and `workspace.files.*` are listed in 02's
    namespace table but not implemented; panels' read-only list uses only implemented
    methods.
- Step 2 (Linux): `kumikit-registry` has its fetch module already (hyper's HTTP/1.1
  client connection over tokio-rustls; `https` with the OS store, loopback `http`,
  `file:`, https-only redirects, size caps, timeouts), with 15 tests over loopback,
  including HTTPS through a test certificate authority. `rustls-native-certs` 0.8.5 brings
  `openssl-probe` (it only finds certificate directories; no OpenSSL is linked),
  `schannel` on Windows and `security-framework` on macOS; `cargo deny check licenses bans
  sources` passes. Nothing links the crate into the core yet, so the binary and installer
  sizes are measured in step 6, as in M3.
- Step 2, panels: the real pieces went in now (they are step 11's anyway): the app CSP
  allows frames from the plugin scheme, `kumikit-assets::panel_csp`, the store accepts
  `isolation = "iframe"` with `[ui] network`, and the shell and the core serve a panel's
  files with its own CSP and `Access-Control-Allow-Origin: null`.
  `test-plugins/panel-smoke` (a stand-in page and a panel that probes its sandbox) runs in
  the shell through `scripts/desktop/panel-smoke.mjs` (stdout reports, no WebDriver) and
  in browsers through `web/e2e/tests/panel-smoke.spec.ts`. Results:
  - **WebKitGTK** (container) and **Chromium** (Playwright): the panel's origin is
    `null`; the parent's DOM is unreachable; `__TAURI_INTERNALS__` is absent; fetches to
    `ipc:`, `http://ipc.localhost` and the network are blocked by the panel's CSP (no
    request leaves the browser); popups and top navigation are blocked; a constructed
    stylesheet applies tokens; **module scripts run** (with `Access-Control-Allow-Origin:
    null`), so the classic-only fallback is not needed there.
  - **Finding:** on WebKitGTK, `window.webkit.messageHandlers.ipc` *is* visible inside the
    panel, and a forged invoke can be posted to it. Tauri refused it (nothing reached
    `test_report`), so Tauri's own check, not the frame, is what stops it. The smoke test
    keeps posting forged calls on every webview and fails if one ever lands.
  - The browser client served every file with `X-Frame-Options: DENY` and
    `frame-ancestors 'none'`, which would block the page's own panels. Panel files now get
    `frame-ancestors <the core's origin>` and `SAMEORIGIN`; everything else is unchanged
    (core test `a_panels_files_carry_its_own_csp_and_only_this_core_may_frame_them`).
  - WebView2, WKWebView and Playwright WebKit: CI only (`panel-smoke` job, `e2e-browser`).
- Checks after step 2: clippy, fmt, `cargo test --workspace`, docs and plugin names pass;
  Playwright Chromium 15 passed (1 live test skipped); both desktop exit tests pass in
  the container.
- Step 3: `kumikit-package::signature` (format v1), `pack_signed`, `sign_package`,
  `verify_package`; migration 0005 (`publisher_key`, `source`; plugins already seeded
  become `bundled`); the host verifies a `SIGNATURE` after unpacking into staging and
  records the key and source; `kumikit plugin keygen | pack --key/--sign | sign | verify`;
  `add` says who signed a package (the CLI verifies the file itself to say so until the
  protocol carries `publisher`, step 5). The error codes `untrusted` and `unavailable`
  were added here, since the host needed `untrusted`; their fixtures and the 0.5 bump
  come in step 5. `scripts/check-no-keys.sh` runs in the `docs` job. `cli::pack` reads a
  core-plugin manifest, so packing a UI plugin or theme with `kumikit plugin pack` fails
  today; step 8 (shared UI manifest check) fixes that for the templates.
- CI on the first two pushes (PR #6): the panel smoke passes on **WKWebView (macOS)**,
  **WebView2 (Windows)** and WebKitGTK. Findings:
  - On WebView2, Tauri's init script runs in every frame, so a sandboxed panel finds
    `__TAURI_INTERNALS__`, `window.ipc` and `chrome.webview`. Calling
    `__TAURI_INTERNALS__.invoke('test_report' | 'profiles_list')` from the panel gets **no
    answer** (3 s), and nothing reaches the shell: Tauri drops IPC from the frame's
    opaque (`null`) origin. On WebKitGTK and WKWebView the script is not in the frame at
    all. The smoke test now tries a real invoke on every webview and fails if one is
    answered; ADR 0019's "no shell IPC" holds by Tauri's origin check, which this test
    guards.
  - Module scripts run from the panel's opaque origin on all three webviews (with
    `Access-Control-Allow-Origin: null`).
  - The first Windows run failed only at cleanup: WebView2 still held the client data
    directory, so the script now checks the results before removing it, with retries.
- Step 4: the checker lives in its own crate, `kumikit-registry-tool` (binary
  `kumikit-registry`), because it validates core manifests through
  `kumikit-plugin-host`, which `kumikit-registry` (used by the shell) must not depend on.
  `test-plugins/registry-fixture` builds fixture registries with keys made at test time
  and serves them on loopback, counting requests; it uses the checker's own
  `facts`/`version_entry`, so fixtures and the checker cannot disagree. The checker
  reads a UI plugin's manifest only for the fields it copies; full validation of UI
  manifests comes with the shared check in step 8. `Manifest::requested()` gives a core
  manifest's permissions in protocol form.
- Step 5: protocol 0.5 as planned. The update record is `AvailableUpdate` (the method
  markers already use the name `PluginUpdate`). The new methods answer `unsupported`, and
  `plugin.install { registry }` too, until step 6. Hard-coded `"0.4"` in tests became
  `"0.5"`; the UI kernel's mismatch test still uses `0.4` as the other version.
- Step 6:
  - The lockfile format is `kumikit_protocol::lockfile` (pure parse and render), since
    the core and the shell both use it; `granted` is written in the protocol's words
    (`files = "read_workspace"`), as a `[plugin.granted]` table.
  - The plugin manager rewrites `plugins.lock` after every install, update, enable,
    disable and removal, so the CLI's offline path writes it too.
  - An update refuses while a turn runs: the kernel answers `turns_running_on(providers)`
    and the host counts a backend's delegated providers as well as the plugin's own.
  - An update disables the plugin around the swap and enables it with the new grant; if
    the swap fails, the old version is enabled again with its old grant.
  - `RegistryRef` gained an optional `registry` (an index URL used instead of the list),
    for `kumikit plugin add <id> --registry <url>` and lockfile imports.
  - `plugin.updates` uses a recent index unless `refresh` is set; the CLI and "Check for
    updates" pass `refresh: true`.
  - The `registry` capability is reported by cores started with `serve` (unit-test cores
    have no registries and answer `unsupported`).
  - Tests: `tests/registry.rs` (7 tests: no request at startup, install by id with
    publisher and lockfile, four ways a registry can lie, updates proposed and wider ones
    waiting, key change and a second registry, an update waiting for a running turn, and
    the CLI end to end with a lockfile moved to a fresh core).
- Step 7: `tests/exit_m4.rs` runs the exit flow through the CLI and the protocol in about
  3.5 s. A turn's error used to carry only the host's message ("secret … is not in
  permissions.secrets"), so a refused permission could not be told apart from other
  failures; the host now keeps the code in front for denied calls (`permission_denied:
  …`), and the test asserts it exactly. Other error codes keep their text. 04-plugin-host
  says so.

