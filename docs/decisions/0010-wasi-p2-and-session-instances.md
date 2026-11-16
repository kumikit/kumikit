# 0010 — WASI 0.2, one instance per session, blocking host imports

- **Status:** Proposed
- **Date:** 2026-11-15

## Context

[ADR 0003](0003-wasm-first-core-plugins.md) chose WASM components on WASI 0.2 and left
WASI 0.3 (native async) open; the roadmap asked for a decision by M1.

Under WASI 0.2 an export call runs to completion, and a component instance handles one
call at a time. A provider's `send` runs for the whole turn, blocked in host imports such
as `read-stdout`. The first sketch of `kumikit:provider` also exported
`respond-approval` and `cancel`, and assumed one instance per plugin. With one instance,
neither export could ever run during a turn, and two threads using the same provider
could not run turns at the same time.

Options:

| Option | Concurrency | Status in the toolchain (2026-11) |
|---|---|---|
| WASI 0.3 / component-model async exports | Concurrent calls into one instance | Rust's `wasm32-wasip3` target is tier 2 since Rust 1.100 (2026-11-12). On the host, wasmtime 50.0.1 calls its component-model async support "very incomplete", and `wasmtime-wasi`'s WASI 0.3 module (also in 51.0.0-rc.1) is "experimental, unstable and incomplete … not ready for production use", with no patch releases for bug and security fixes limited to it |
| WASI 0.2, one instance per plugin | None | Stable |
| WASI 0.2, one instance per provider session | One turn per session, sessions in parallel | Stable |

Guest tooling: `cargo-component` (an extra cargo subcommand) or the `wasm32-wasip2`
target, which links a component directly, plus `wit-bindgen`.

## Decision

- Core plugins target **WASI 0.2** (`wasm32-wasip2` for Rust). Guests use `wit-bindgen`
  through the `kumikit-plugin` SDK crate; no `cargo-component`.
- A provider plugin gets one **control instance** for calls outside a session
  (`list-models`) and one **session instance** per provider session. Each instance is
  driven by its own task, one call at a time.
- Anything that must reach a plugin during a running call does so through a **host import
  the plugin is blocked in**, not through another export:
  - approvals: `kumikit:host/events.request-approval` blocks until the approval is
    answered (or cancelled); the host keeps answers that arrive early;
  - cancel: the import the plugin is blocked in returns `cancelled` once; the plugin then
    has a short grace period to end its turn before its processes are killed and the
    instance is discarded.
- `kumikit:provider` therefore exports `list-models`, `start-session`, `send` and
  `end-session` only. The kernel's internal `Provider` trait keeps `respond_approval` and
  `cancel`; the WASM adapter implements them on the host side.
- WIT packages start at **0.1.0** and freeze at 1.0.0 before third parties publish (M4).

## Consequences

- Several threads can run turns on one provider at the same time, at the cost of one
  instance (a few MB) per active session. The pooling allocator makes instantiation
  cheap; its slot count caps concurrent instances.
- A guest is ordinary blocking code: no async runtime in plugins.
- Plugin-wide state is not shared between instances in memory; plugins keep shared state
  in `kv`.
- Moving to WASI 0.3 later is a new version of the WIT packages and a new ADR; the
  per-session model keeps working under it.
- **When to revisit:** when wasmtime ships WASI 0.3 as stable (no longer marked
  experimental, with security fixes in patch releases). Reconsidered on 2026-11-15, after
  Rust 1.100 made `wasm32-wasip3` tier 2: the guest side was ready, the host side was
  not, so the decision stands. A plugin sandbox meant for community plugins should not
  depend on a runtime layer that gets no security patch releases.
- **Keeping the move small:** guest code calls host imports only through the SDK crates,
  so most of a later move happens in the SDK and the host; plugins mostly rebuild. The
  host-side `Provider` trait in the kernel does not change.
