# 0003 — Core plugins are WASM components first

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

Core plugins must be safe to install from the community, cheap in memory, and writable in
more than one language. Options:

| Option | Isolation | Memory | Languages |
|---|---|---|---|
| Native dynamic libraries | None; unstable Rust ABI | Lowest | Rust/C |
| Sidecar processes (JSON-RPC over stdio) | OS process | Tens of MB each for managed runtimes | Any |
| WASM components (wasmtime, WIT) | Capability-based sandbox | A few MB per instance, lazy | Rust, Go/TinyGo, C, JS, Python |

## Decision

- Core plugins are WASM components run by wasmtime, with interfaces defined in WIT and
  WASI 0.2.
- A sidecar runtime speaking the same interfaces over JSON-RPC is allowed later as an
  explicitly unsandboxed escape hatch.
- Native libraries are never loaded into the core.

## Consequences

- Permissions can be enforced on every host call; plugins cannot reach files, network or
  processes without a grant.
- Rust and TinyGo plugins are small and start fast; JavaScript plugins carry an embedded
  engine (a few MB) and start slower. SDK and templates favour Rust.
- WASI 0.2 lacks native async; long-running work blocks inside async host imports. A move
  to WASI 0.3 needs its own ADR.
- Agent CLIs still run as normal processes via the `spawn` permission, outside the
  sandbox; the install UI must say so.
