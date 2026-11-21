# CI pipeline optimization and integration test consolidation

- **Goal:** Cut `kumikit/kumikit` CI wall-clock time (~15–22 min → ~6–8 min) and runner minutes (>50%) without dropping coverage.
- **Status:** active

## Steps
- [x] Fix the un-cleared 90 s `setTimeout` in `scripts/desktop/panel-smoke.mjs` (saves ~88 s per OS).
- [x] Add `KUMIKIT_BIN` / `KUMIKIT_PROVIDER_ACP_WASM` overrides to `scripts/bundle.mjs` and `KUMIKIT_SKIP_BUILD` to `scripts/desktop/e2e.sh` and `scripts/check-templates.sh` so CI stops compiling `kumikit-core` twice per job.
- [x] Consolidate integration test binaries per crate (`crates/kumikit-core`, `kumikit-plugin-host`, `kumikit-protocol`, `kumikit-registry`, `kumikit-shell`) from 34 binaries down to 10 while keeping explicitly documented `--test` targets (`footprint`, `exit_m0`, `exit_m1`, `remote`) intact; update doc references.
- [x] Restructure `.github/workflows/ci.yml`:
  - Add PR concurrency cancellation and `dorny/paths-filter` path filtering (all jobs still run on `main`).
  - Fix 10 GB GitHub cache thrashing with `shared-key` (`ci-debug` / `ci-release`) and `save-if: ${{ github.ref == 'refs/heads/main' }}`.
  - Add a single `build-linux` job that builds `target/bundle` once (shared across all OSes), builds Linux debug test binaries once (shared across Linux `e2e-browser`, `e2e-desktop`, `panel-smoke`, `templates`), and runs `theme check --strict plugins/theme-default`.
  - Use `mold` linker on Linux + `cargo-nextest` (plus `cargo test --workspace --doc`) in `rust`.
  - Run `--project webkit` only on `e2e-browser (macos-latest)` (Linux covers both Chromium and WebKit).
  - Run Linux `installers` on PRs touching installer paths and all 3 OSes on `main`.
- [ ] Verify locally (`check-docs`, `cargo fmt`, `cargo clippy`, `cargo test --workspace`) and measure real CI timings on the PR.

## Notes
- Prior to this change, `kumikit/kumikit` held 13.05 GB across 13+ `Swatinem/rust-cache` entries (past GitHub's 10 GB eviction limit), causing repeated cold Rust builds on `ubuntu-latest` and `windows-latest`.
- `scripts/desktop/panel-smoke.mjs` left a 90 s `setTimeout` referenced after `Promise.race` completed, idling every `panel-smoke` job for ~88 s.
