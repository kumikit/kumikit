# 0031 — Signed release manifest and idle-gated application updates

- **Status:** Accepted
- **Date:** 2026-11-25

## Context

Kumikit publishes desktop installers (`.AppImage`, `.deb`, `.rpm`, `.dmg`, Windows `-setup.exe`)
and standalone `kumikit` core archives on GitHub Releases, but neither the desktop app nor the
core CLI can check for updates, display the exact release version (`v0.1.0-m7.4` vs `0.1.0`),
or download and install new releases.

We need application updates that respect Kumikit's rules:

1. **Zero unasked network traffic:** checks happen only when the user clicks "Check for updates",
   runs `kumikit update`, or explicitly turns on opt-in automatic updates.
2. **Never interrupt active agents:** automatic updates must wait until every workspace on the
   core has no running turns, no turns awaiting approval, and no unpaused queued messages waiting
   to start, and must shut down the detached local core cleanly before applying the update.
3. **Cryptographic verification:** neither desktop installers nor core archives may be trusted
   from an unsigned checksum file alone.
4. **Minimal footprint and one signing stack:** `kumikit-shell` and `kumikit-core` already share
   `kumikit-registry::fetch::Fetcher` (HTTPS over rustls with OS roots, redirect rules, caps and
   loopback/file test support) and `kumikit-package::signature` (`kumikit-signature 1` Ed25519
   signatures via `ring`). Adding `tauri-plugin-updater` would pull in a second HTTP stack, a
   second signature format (minisign), and a second signing key, while still leaving `kumikit`
   CLI updates and SSH remote sync separate — and would conflict with `scripts/repack-appimage.mjs`.

## Decision

1. **Unified signed release manifest (`release.json` + `release.json.sig`).**
   Each release publishes `release.json` (format 1: semver `version`, `tag`, `publishedAt`,
   `notesUrl`, optional `notes`, per-target `core` archives and per-platform `desktop` installers,
   each with `url`, `sha256`, `size` and `format`) alongside `release.json.sig` in the existing
   `kumikit-signature 1` Ed25519 format. `kumikit-registry` verifies the signature against the
   pinned Kumikit release public key (`KUMIKIT_UPDATE_KEY` overrides it for tests and mirrors)
   before trusting any URL or SHA-256 hash, and checks the SHA-256 of every downloaded asset.
   `release.yml` uploads both files to the GitHub Release and updates the `release-manifest`
   branch so pre-1.0 milestone releases and 1.0+ releases resolve from one URL
   (`KUMIKIT_UPDATE_URL` overrides it).

2. **Build-time version stamping.**
   `kumikit-protocol` embeds `KUMIKIT_RELEASE_VERSION` (falling back to `CARGO_PKG_VERSION`) as
   `APP_VERSION`, used by `kumikit --version`, `core.hello.core.version`, `kumikit-shell`, and
   semver update comparisons.

3. **Protocol 0.10 idle checks.**
   - `core.activity` returns `{ activeTurns, readyQueued, idle }` across every workspace
     (including side threads and scheduled turns).
   - `core.shutdown` accepts optional `{ ifIdle?: boolean }`; when `ifIdle` is true and any turn
     is running or any unpaused queued message is ready to start, it refuses with `conflict`.

4. **Shell update commands (refining [ADR 0013](0013-desktop-shell.md)).**
   `kumikit-shell` gains four update commands: `app_update_info`, `app_update_check`,
   `app_update_download`, and `app_update_apply`. In-place install and relaunch is supported for
   Linux AppImage (atomic replace of `$APPIMAGE`), macOS `.app` (from `.app.tar.gz`), and Windows
   NSIS (`-setup.exe /S`). System-packaged Linux installs (`.deb`, `.rpm`) cannot overwrite
   `/usr/bin` unprivileged: the shell downloads and verifies the package and shows the upgrade
   command.

5. **Opt-in automatic updates and `kumikit update` CLI.**
   Automatic updates in Settings default to **off**. When turned on, the UI kernel periodically
   checks the signed manifest, downloads and verifies newer releases in the background, waits for
   `core.activity` to report `idle: true`, and applies the update with `ifIdle: true`. Headless
   and TLS-paired cores get the `kumikit update [--check] [--yes] [--wait-idle] [--force]` CLI
   subcommand built on the same verified release client.

## Consequences

- No new HTTP client or signature crate is added; binary size and memory budgets stay intact.
- Both desktop installers, CLI self-updates, and SSH remote sync ([ADR 0032](0032-ssh-remote-core-sync.md))
  share one signed manifest and one verification path.
- `release.yml` must sign `release.json` with `KUMIKIT_SIGNING_KEY` and publish macOS
  `.app.tar.gz` alongside `.dmg`.
- [02-protocol](../spec/02-protocol.md), [06-client](../spec/06-client.md),
  [10-distribution](../spec/10-distribution.md), and the `README.md` Privacy section are updated
  in the same change.
