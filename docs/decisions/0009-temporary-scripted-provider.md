# 0009 — A temporary scripted provider compiled into the core (M0 only)

- **Status:** Proposed
- **Date:** 2026-11-15

## Context

M0 builds the protocol and kernel before the plugin host exists (M1), but its exit
criteria need a provider that streams events, calls a tool and asks for an approval.
Driving a real agent CLI would make the tests depend on an installed agent, the network
and timing. AGENTS.md forbids provider ids in `crates/`.

## Decision

- M0 ships one deterministic **scripted provider** in `crates/kumikit-core`, behind the
  kernel's internal `Provider` trait. The kernel itself never names it: the core binary
  registers it at startup.
- Its models select a scenario (`echo`, `tool-approval`, `fail`). It uses no timers and
  no randomness, so tests are deterministic; the approval is the point where a turn
  pauses.
- This is a time-limited exception to "no provider id in `crates/`". The provider and
  this exception are deleted in M1, when providers become WASM plugins.

## Consequences

- M0 tests run with no agent installed.
- The `Provider` trait mirrors the `kumikit:provider` WIT, so M1 replaces the scripted
  implementation with a WASM adapter behind the same trait.
- Until M1, a `kumikit serve` build always offers this provider, and it cannot run real
  agents.
- M1's exit includes deleting `scripted_provider.rs`, its registration and this ADR's
  exception (set this record to `Superseded`).
