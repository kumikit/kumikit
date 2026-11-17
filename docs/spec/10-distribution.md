# 10 — Distribution

## Preinstalled plugins

Kumikit ships with as little as works out of the box. Each preinstalled plugin can be
disabled; the UI kernel works (with an empty state) without any of them.

| Plugin | Side | Why it is preinstalled |
|---|---|---|
| `kumikit.provider-acp` | core | Runs any ACP agent from a manifest; gives a working agent with no code. Shipped with the desktop app and preinstalled through `--bundled-plugins` (below). |
| `kumikit.ui-chat` | client | Transcript and composer |
| `kumikit.ui-settings` | client | Settings pages for the kernel and declarative plugin settings |
| `kumikit.theme-default` | client | Light and dark themes; the fallback for missing tokens |

### Bundled core plugins

`kumikit serve --bundled-plugins <dir>` preinstalls the core plugins shipped with an
installer. The desktop app passes its own directory when it starts the core. At start the
core installs every `.kkp` in the directory whose id it has not seeded before, and records
the id and version as seeded, so a plugin the user removed is not installed again. It
enables a seeded plugin only if it requests no permissions; any other stays installed
until the user grants it. The core names no plugin: the directory decides. A core started
without the flag (for example from the CLI) preinstalls nothing.

On first run (from M4, with the registry), a "starter pack" screen offers common plugins (agent providers, git, a bot)
to install. Nothing is installed without the user choosing it.

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

## Package format

`<id>-<version>.kkp`: a tar.zst archive with `plugin.toml`, the component or UI bundle,
assets and a `SIGNATURE` file. The signature is ed25519 over a list of file hashes, made
with the publisher's key.

## Registry

- The registry is a git repository, `github.com/kumikit/registry`, with one file per
  plugin: `plugins/<publisher>/<name>.toml` listing versions, download URLs (usually
  GitHub release assets), SHA-256 hashes and the publisher's public key.
- Publishing is a pull request to the registry. CI checks the manifest, the hash, the
  signature, that the publisher key matches earlier versions, and that the id is not
  confusingly close to an existing one.
- The registry is fetched only when the user browses, searches, installs or checks for
  updates, or when they turn on periodic update checks. No other traffic.
- Other registries can be added by URL; installing from a local `.kkp` file works offline.

## Install

```sh
kumikit plugin add kumikit.provider-claude          # on the local core
kumikit plugin add kumikit.provider-claude --core devbox
kumikit plugin add ./my-plugin-0.1.0.kkp            # local file
kumikit plugin list | update | disable <id> | remove <id>
```

The desktop plugin manager does the same through the protocol (`plugin.install`, with a
`.kkp` or a folder holding a `plugin.toml`) for core plugins, and through the shell's
client plugin store for UI plugins and themes ([06](06-client.md#shell-rust-kumikit-shell)).

Install steps: download → check hash → check signature against the registry's publisher
key → validate manifest → show permissions → unpack → enable.

### Local packages (before signing)

Until signing and the registry exist (M4), `kumikit plugin add <file.kkp>` is the only way
to install a core plugin:

- `kumikit plugin add <file.kkp> [--yes]`, `kumikit plugin list`, `kumikit plugin remove
  <id> [--yes] [--keep-data]`, and `kumikit plugin pack <dir> [-o <file.kkp>]` to build a
  package. All take `--socket` and `--data-dir`.
- The commands talk to the running core through `plugin.*`. When no core runs, they open
  the data directory themselves (holding its lock) and do the same work.
- `add` unpacks and validates the package, then prints the plugin's name, publisher,
  repository and permissions in plain words, and that the package is not signed so its
  publisher cannot be verified. It asks before granting the permissions and enabling the
  plugin; `--yes` grants without asking. Without a terminal and without `--yes`, the
  plugin stays installed but not enabled.
- `remove` asks before deleting the plugin's data; `--keep-data` keeps it.
- The SHA-256 of the package is recorded for the lockfile. Installing an id that is
  already installed fails until updates exist; remove it first. Adding the same version
  of a plugin that is installed but not enabled asks again for the grant.

## Lockfiles

- Each core keeps `plugins.lock` in its data directory; the client keeps its own.
- The lock records id, version, hash and granted permissions. Exporting and importing it
  reproduces a setup on another machine.
- Updates are proposed, not applied silently; an update that widens permissions waits for
  approval.

## Versioning

Plugins use semver. `api` in the manifest is a range over the kernel plugin API version.
The kernel refuses to enable a plugin whose `api` range it does not satisfy and says which
version the plugin needs.
