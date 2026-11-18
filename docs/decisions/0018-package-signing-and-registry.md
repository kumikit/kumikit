# 0018 — Package signing and the plugin registry

- **Status:** Proposed
- **Date:** 2026-11-18

## Context

M4 lets third parties publish plugins ([12-roadmap](../spec/12-roadmap.md#m4--ecosystem)).
[10-distribution](../spec/10-distribution.md) named the shape: an ed25519 `SIGNATURE` over
the package's file hashes, a git registry with one file per plugin, the publisher's key,
CI checks on publishing, and fetching only on the user's request. It left open:

1. **The signature format and crate.** minisign, signify, SSH signatures and JWS each sign
   one blob and bring a crate or a tool; `ed25519-dalek` is a new crate. `ring` 0.17
   already ships with the core and the shell for TLS ([ADR 0016](0016-remote-transport-and-pairing.md))
   and does ed25519.
2. **How clients read a git registry.** Cloning needs `git` or `gix` in the core; reading
   one raw file per id cannot search.
3. **Trust roots for HTTPS.** `webpki-roots` is CDLA-Permissive-2.0, outside the AGENTS.md
   licence list; `rustls-native-certs` (Apache-2.0 OR ISC OR MIT) reads the OS store.
4. **Who fetches.** Core plugins are installed by a core, possibly on another machine; UI
   plugins and themes by the shell's client plugin store ([ADR 0013](0013-desktop-shell.md)),
   which so far makes no network requests.
5. **Where a publisher's key lives**, so that one key covers one id namespace.

## Decision

- **`SIGNATURE` v1** is a small text format: a `kumikit-signature 1` line, the publisher's
  key (`ed25519:` + base64url), one `file <sha256> <path>` line per regular file except
  `SIGNATURE` (sorted, `/` separators), and a `sig` line: the ed25519 signature over every
  byte before it. Verification requires the unpacked files to equal the list exactly. The
  code lives in `kumikit-package` and uses `ring`.
- **Keys** are PKCS#8 files (PEM, mode 0600) made by `kumikit plugin keygen`, kept in the
  user's config directory, never in a work tree, never logged. Signing takes `--key` or
  `KUMIKIT_SIGNING_KEY`.
- **Pinning.** The key of each installed plugin is recorded; a later version must be signed
  by it, and an unsigned package never replaces a signed one. A changed key is refused
  (there is no rotation in format v1). Signed local files pin their key too; unsigned local
  files install with the "not signed" warning, as before.
- **Registry layout:** `publishers/<publisher>.toml` (key, GitHub logins that may change
  it), `plugins/<publisher>/<name>.toml` (metadata and versions, with the manifest's `api`
  and permissions copied and checked), `starter.toml`. Registry CI runs `kumikit-registry
  check` (hash, signature, manifest, copied fields, key continuity, immutable versions,
  confusable ids) and, on `main`, builds **`index.json`**, which is all a client reads.
  Default: `https://kumikit.github.io/registry/index.json`.
- **Fetching:** HTTPS with the OS trust store (`rustls-native-certs`), `http` only to
  loopback, `file:` for local mirrors; a small HTTP/1.1 client on `hyper`; size caps,
  timeouts, https-only redirects. Only on browse, search, install, lockfile import or an
  update check. Registry packages must be signed by the publisher key the index gives.
- **Each side fetches what it installs.** The core fetches core plugins (so a remote core
  downloads on its own machine); the shell fetches UI plugins and themes. Both use the
  `kumikit-registry` crate. This extends ADR 0013's shell command list with registry
  search, registry settings, install from a registry, updates and the client lockfile.
- **Updates** come only from the registry a plugin was installed from, only to a higher
  version, only on an explicit request; wider permissions need a new grant first.
- **Not in v1:** a signed index (HTTPS plus signed packages and pinned keys instead), key
  rotation, periodic update checks.

## Consequences

- No new crypto crate; one small new dependency (`rustls-native-certs`) for trust roots.
- A compromised registry can add new plugins or change the key of a plugin nobody has
  installed, but cannot replace an installed one. A signed index can close that later.
- A publisher who loses their key must publish under a new id, or wait for a rotation
  format; users must remove and reinstall to accept a new key.
- The shell now makes network requests, only on the user's action, listed in the README.
- The registry's own repository and its CI live outside this repository; this repository
  tests the same checker against fixture registries.
