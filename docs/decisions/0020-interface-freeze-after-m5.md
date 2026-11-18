# 0020 — Freeze WIT and the UI SDK after M5; early publishers until then

- **Status:** Proposed
- **Date:** 2026-11-18

## Context

[ADR 0010](0010-wasi-p2-and-session-instances.md) and [ADR 0014](0014-ui-sdk-versioning.md)
say the WIT packages and the UI SDK freeze at 1.0 "before third parties publish". M4 builds
publishing (signing, the registry). M5 builds the first plugins that press on these
interfaces: git and GitHub (plugin methods, so `handle-rpc`), a bot (`http`, a client
world), the tool gateway. Those will add host interfaces and may reshape `kumikit:plugin`.
The level-2 panel bridge ([ADR 0019](0019-sandboxed-panels.md)) is new in M4.

Freezing in M4 would lock in interfaces before the plugins that test them exist; changes
after 1.0 need a new major version that the host supports alongside the old one.

## Decision

- WIT stays at 0.1.0 through M4 (M4 needs no WIT change); the UI SDK goes to 0.1.1.
- The formats M4 introduces are versioned from the start: `SIGNATURE` v1, the registry
  `format = 1`, the lockfile `format = 1`, and the manifest JSON Schemas as published files.
- Until 1.0, the official registry accepts only publishers marked `early = true` in their
  publisher file (the `kumikit` publisher and testers the maintainers add, who accept
  breaking changes). Anyone may run their own registry.
- After M5, WIT and the UI SDK are reviewed with M5's plugins in hand, published as 1.0.0,
  and the official registry opens. Items queued for that review: `handle-rpc` and
  `run-command` in `kumikit:plugin`; `settings`, `http`, `files`, `timers` in
  `kumikit:host`; whether core and UI `api` ranges stay separate; level-3
  `ctx.connection` limits; which slots accept panels.

## Consequences

- "Before third parties publish" in ADRs 0010 and 0014 holds: the official registry is not
  open to them until the freeze.
- Early publishers rebuild when a 0.x interface changes.
- M4's exit test simulates a third-party publisher with a fixture registry.
