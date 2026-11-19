# 0021 — No sidecar plugins in 1.0

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

[ADR 0003](0003-wasm-first-core-plugins.md) allowed sidecar processes speaking the WIT
interfaces over JSON-RPC as a later, unsandboxed escape hatch, and the roadmap asked to
decide by M4 whether they ship in 1.0.

A sidecar is native code outside the sandbox: none of the permissions apply to it. A WASM
plugin with the `spawn` permission can already start any listed program (an agent CLI, a
Node script), and install tells the user it runs with their full access. Shipping sidecars
means a second host, a JSON-RPC mapping of every WIT interface and a second permission
story, maintained before 1.0.

## Decision

- Sidecar plugins are not part of 1.0. `runtime.sidecar` stays reserved in the manifest
  and refused at install.
- Revisit after 1.0 if a real plugin cannot be built as a WASM component plus `spawn`.

## Consequences

- Every core plugin in 1.0 runs in the WASM sandbox with enforced permissions.
- Plugins that need another runtime use `spawn` from a WASM component, which the user
  approves explicitly.
