# Prebuilt release binaries and `setup-kumikit` action

- **Goal:** Publish prebuilt `kumikit`, `kumikit-registry` and `kumikit-test-kit` release archives on tags and provide a `setup-kumikit` GitHub Action so plugin CI/release workflows and `kumikit/registry` do not compile `kumikit` from source.
- **Status:** active

## Steps
- [x] Add opt-in `test-hooks` Cargo feature to `kumikit-plugin-host` and `kumikit-core` so `kumikit-test-kit` ships a small stripped `kumikit` binary that honors `KUMIKIT_TEST_HTTP_ROUTES` while production release builds still ignore it; update spec and agent reference.
- [x] Add `[package.metadata.binstall]` metadata to `kumikit-core` and `kumikit-registry-tool`.
- [x] Add `.github/workflows/release.yml` in `kumikit/kumikit` to build, attest, checksum (`SHA256SUMS.txt`) and publish `kumikit`, `kumikit-registry` and `kumikit-test-kit` archives across Linux (`x86_64`, `aarch64`), macOS (`aarch64`) and Windows (`x86_64`) on `v*` tags.
- [x] Add `.github/actions/setup-kumikit/action.yml` composite action to download, SHA-256 verify, cache and configure `kumikit`, `kumikit-registry` and `test-kit` (`KUMIKIT_DIR`, `KUMIKIT_UI_SDK`).
- [x] Document prebuilt binaries and `setup-kumikit` in `docs/building.md`.
- [x] Verify locally, merge PR to `kumikit/kumikit`, tag `v0.1.0-m5.1` and verify the `Release` workflow publishes all assets cleanly.
- [ ] Update `kumikit/registry` (`check.yml`) and all four first-party plugin repos (`kumikit-plugin-agents`, `kumikit-plugin-git`, `kumikit-plugin-github`, `kumikit-plugin-schedules`) to use `setup-kumikit@v0.1.0-m5.1`, and verify all CI workflows pass.

## Notes
- `kumikit-plugin-github` e2e tests require `KUMIKIT_TEST_HTTP_ROUTES`, which was previously `cfg!(debug_assertions)` only. Mirroring `kumikit-shell`'s `test-hooks` Cargo feature keeps the production `kumikit` release binary free of test hooks while letting `release.yml` build the test-kit `kumikit` in ~15 s reusing release dependency artifacts.
- `kumikit-test-kit` mirrors the `KUMIKIT_DIR` layout (`target/debug/kumikit`, `target/kumikit-test/...`, `plugins/provider-acp/`, `test-plugins/probe/`, `web/sdk/`, `tsconfig.base.json`) so existing plugin e2e harnesses and Vite UI builds work unchanged against either a local checkout or the prebuilt archive.
