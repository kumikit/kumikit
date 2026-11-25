# M8 — Application updates and SSH remote core sync

- **Goal:** Add Ed25519-verified application updates in Settings (with opt-in automatic updates
  that apply only when all agents are idle), a `kumikit update` CLI subcommand, and verified
  matching `kumikit` installation/sync over SSH ([ADR 0031](../decisions/0031-signed-release-manifest-and-updates.md),
  [ADR 0032](../decisions/0032-ssh-remote-core-sync.md), [10-distribution](../spec/10-distribution.md)).
- **Status:** done

## Steps

- [x] **Step 0 — Decisions and spec:** Record [ADR 0031](../decisions/0031-signed-release-manifest-and-updates.md)
  and [ADR 0032](../decisions/0032-ssh-remote-core-sync.md); update `02-protocol.md`,
  `06-client.md`, `10-distribution.md`, and `12-roadmap.md`. Verify with `sh scripts/check-docs.sh`.
- [x] **Step 1 — Shared release manifest, signing, and verification:**
  - Add `sign_file_bytes` / `verify_file_bytes` to `crates/kumikit-package/src/signature.rs`.
  - Add `crates/kumikit-registry/src/release.rs` (`ReleaseManifest`, `ReleaseAsset`, `ReleaseClient`,
    Ed25519 signature + SHA-256 verification, semver comparison, `.tar.gz` extraction, target helpers).
  - Add `kumikit-registry release-manifest` subcommand in `crates/kumikit-registry-tool` to build
    and sign `release.json` + `release.json.sig` from staged release artifacts.
  - Unit tests: valid signature, tampered JSON, wrong signing key, bad SHA-256, semver ordering
    across pre-release milestone tags (`0.1.0-m7.4` < `0.1.0-m8` < `0.1.0`), path traversal rejection.
- [x] **Step 2 — Version stamping, protocol 0.10 idle checks, and `kumikit update` CLI:**
  - Add `build.rs` in `crates/kumikit-protocol` exposing `APP_VERSION` (`KUMIKIT_RELEASE_VERSION`
    or `CARGO_PKG_VERSION`).
  - Bump `PROTOCOL_VERSION` to `0.10`; add `core.activity` (`CoreActivityResult`) and optional
    `ifIdle` on `core.shutdown` (`ShutdownParams`); update fixtures and regenerate `web/protocol/`.
  - Implement `Kernel::activity` in `crates/kumikit-kernel` (counts running/awaiting turns across
    all workspaces + unpaused ready queued entries) and `ifIdle` guard in `crates/kumikit-core/src/rpc.rs`.
  - Use `APP_VERSION` in `kumikit` CLI `--version` and `ServerInfo::current()`.
  - Add `kumikit update [--check] [--yes] [--wait-idle] [--force]` in `crates/kumikit-core/src/cli/update.rs`.
  - Tests: `core.activity` idle vs busy (running turn, queued entry), `core.shutdown { ifIdle: true }`
    refuses when busy and succeeds when idle, `kumikit update --check` and self-replace against a
    loopback signed release fixture.
- [x] **Step 3 — Desktop shell updater and SSH remote sync:**
  - Add `crates/kumikit-shell/src/updater.rs`: install-kind detection (AppImage, macOS `.app`,
    Windows NSIS, Linux package-managed, unsupported/dev), check, verified download/stage, idle-gated
    local core shutdown (`core.shutdown { ifIdle }`), and platform apply/relaunch.
  - Add `auto_sync: bool` to `Kind::Ssh` in `crates/kumikit-shell/src/profiles.rs` and
    `profile_update_ssh` in `crates/kumikit-shell/src/connections.rs`.
  - Add `crates/kumikit-shell/src/ssh_sync.rs`: `ssh_probe` and `ssh_sync` (local verified download
    + cache, remote idle check + `core.shutdown { ifIdle }`, `tar -xzf -` stream over SSH `stdin`
    to `~/.local/bin/kumikit`, detached remote core start + `core.hello` check).
  - Wire Tauri commands in `crates/kumikit-shell/src/main.rs` and mirror endpoints in
    `test-plugins/web-relay/src/main.rs` for E2E testing.
  - Tests: shell updater unit tests (AppImage atomic replace, busy refusal, signature verification),
    SSH sync tests using a fake `ssh` script.
- [x] **Step 4 — UI kernel: Settings Updates page, idle-gated auto-updater, and SSH sync UX:**
  - Extend `web/kernel/src/platform/index.ts`, `tauri.ts`, `relay.ts`, `browser.ts` with `updates`
    and `profiles.probeSsh` / `syncSsh` / `updateSsh`.
  - Extend `web/kernel/src/settings/settings.svelte.ts` with `autoUpdate` (default `false`) and
    `autoSyncSsh` (default `false`).
  - Add `web/kernel/src/updates/updates.svelte.ts`: reactive update state, manual check/download/install,
    opt-in periodic check + background download + `core.activity` idle watcher (applies only when
    `idle: true` with debounce and `ifIdle: true`).
  - Add `web/kernel/src/ui/UpdatesSettings.svelte` and wire it in `SettingsView.svelte` (plus subtle
    status indicator in `StatusBar.svelte` when an update is ready/waiting for idle).
  - Enhance `web/kernel/src/ui/CoresView.svelte` with SSH install/sync prompt on missing/mismatched
    host, per-profile "Auto-sync version" toggle, and manual "Install matching version" button.
  - Unit tests (Vitest) for `UpdateManager` (opt-in off by default, idle wait while turn running,
    auto-apply once idle, busy retry) and `Settings` parsing.
- [x] **Step 5 — Release workflow, README privacy section, and docs:**
  - Update `.github/workflows/release.yml` to pass `KUMIKIT_RELEASE_VERSION`, package macOS
    `.app.tar.gz`, generate & sign `release.json` + `release.json.sig`, upload them to the release,
    and update the `release-manifest` branch.
  - Update `README.md` Privacy section and `docs/remote.md`.
- [x] **Step 6 — Verification:**
  - `sh scripts/check-docs.sh`, `sh scripts/check-no-keys.sh`, `sh scripts/check-plugin-names.sh`
  - `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `npx -p typescript@7.0.2 tsc --noEmit -p web/protocol`
  - `npm run check && npm test`
  - `node scripts/bundle.mjs` (verify UI bundle stays within 150 KB gzipped budget)
  - Playwright E2E tests covering Settings Updates (version display, manual check, download, idle-gated
    auto-update waiting for active agent then applying when idle) and SSH sync prompt.

## Exit criteria

1. Settings → Updates displays the stamped version, detects a newer signed release, downloads and
   verifies it, and refuses tampered or unsigned manifests.
2. With automatic updates enabled, a downloaded update waits while any agent turn is running or
   queued and applies automatically once all agents are idle.
3. With automatic updates disabled (the default), zero update requests are made on startup.
4. `kumikit update --check` and `kumikit update` verify signatures and refuse to interrupt running
   agents unless `--force` is passed (or wait with `--wait-idle`).
5. Connecting over SSH to a host missing `kumikit` or running a different version offers to
   download the verified archive locally, stream it over SSH to `~/.local/bin/kumikit`, start the
   matching core without interrupting busy remote agents, and connect.
6. All workspace Rust, TypeScript, UI rule, bundle size, doc, and E2E checks pass.

## Notes

- Keep `kumikit-registry` and `kumikit-package` shared so desktop shell, core CLI, and SSH sync
  use the exact same Ed25519 manifest format and verification logic.
