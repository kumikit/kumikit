# M9 — Agent orchestration

- **Goal:** Let agents coordinate other agents across providers and models, starting with
  agents that start threads with a chosen model and settings
  ([ADR 0034](../decisions/0034-agents-start-threads.md),
  [roadmap M9](../spec/12-roadmap.md#m9--agent-orchestration)).
- **Status:** active (open milestone: features are added as they come)

## Steps

### Feature 1 — Agents start threads (ADR 0034)

- [x] **Step 1.0 — Decision and docs:** ADR 0034; `03-kernel.md` (client interface),
  `04-plugin-host.md` (WIT table), `12-roadmap.md` (M9), `AGENTS.md` status. Verify with
  `sh scripts/check-docs.sh`.
- [x] **Step 1.1 — WIT `kumikit:threads@0.2.0`:** `wit/kumikit-threads@0.2.0/threads.wit`,
  the 0.1.0 functions plus `list-models` and `create-thread` (a `create-request` record).
  0.1.0 stays unchanged and linked.
- [x] **Step 1.2 — Host:** `crates/kumikit-plugin-host/src/runtime/bindings.rs` (a
  `threads_client_v2` module), `runtime/client.rs` (0.2.0 `Host`: the old functions
  delegate to 0.1.0; `list_models`; `create_thread` with the mode rules in a pure,
  unit-tested function), `runtime/mod.rs` (link 0.2.0 for base and tool worlds). Kernel
  unchanged (`thread_create_opts`, `provider_models`).
- [x] **Step 1.3 — Rust SDK:** `sdk/rust/src/lib.rs` `threads` module generated from
  0.2.0 and re-exporting `list_models`, `create_thread` and `CreateRequest`;
  `sdk/rust-tool` re-exports unchanged.
- [x] **Step 1.4 — Tests through the real core:** `test-plugins/client` gains `models` and
  `create-with` commands; `crates/kumikit-core/tests/integration/client_interface.rs`
  covers the probe's effort setting, invalid settings (`invalid_params`), `full_access`
  refused, looser-than-origin refused, the default clamped to a `plan` origin, an origin
  in another workspace refused, and both new commands denied without `threads.create`.
- [x] **Step 1.5 — `kumikit.threads` tools:** `plugins/threads` gains `list_models`
  (`read`) and `start_thread` (`write`) with schemas, the `threads.create` grant, input
  parsing in `format.rs` with unit tests, an updated README, and plugin version 0.2.0.
- [x] **Step 1.6 — Verify:** fmt, clippy (host and wasm), `cargo test --workspace`,
  `check-docs.sh`, `check-plugin-names.sh`, `check-no-keys.sh`.
- [ ] **Step 1.7 — Publish:** add `kumikit-threads@0.2.0` to `wit/PUBLISHED` when it ships
  in a release.

## Notes

- The protocol doesn't change: `thread.create` already takes `options` and
  `permissionMode` (protocol 0.11).
- `origin` is used only for checks (same workspace, mode bound). It isn't the kernel's
  `parent` link: started threads are ordinary sidebar threads, not side threads.
- `kumikit.threads` 0.2.0 widens its grant (`threads.create`), so a bundled upgrade waits,
  disabled, for the user to grant it again.
- Through `command.run`, a plugin's `invalid-argument` reaches clients as
  `invalid_params`.
