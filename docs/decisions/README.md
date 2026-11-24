# Architecture decision records

One file per decision that shapes the architecture or is expensive to reverse. Agents and
humans read these to learn *why* the code is the way it is before changing it.

## When to write one

- Choosing or replacing a technology, runtime, format or protocol.
- Changing a published interface (protocol major version, a WIT package major version,
  the manifest format, the token set).
- Adding an exception to a rule in [AGENTS.md](../../AGENTS.md) or the spec's principles.

Small, local choices do not need one; a clear commit message is enough.

## How

1. Copy [template.md](template.md) to `NNNN-short-title.md` with the next number.
2. Status starts as `Proposed`; it becomes `Accepted` when merged.
3. Never rewrite an accepted record. To change course, write a new record and set the old
   one's status to `Superseded by NNNN`.
4. Update the spec in the same change so the two agree.

## Index

| # | Decision | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](0002-separate-core-and-client.md) | Separate headless core and thin client | Accepted |
| [0003](0003-wasm-first-core-plugins.md) | Core plugins are WASM components first | Accepted |
| [0004](0004-svelte-ui-kernel.md) | Svelte 5 for the UI kernel | Accepted |
| [0005](0005-json-rpc-protocol.md) | JSON-RPC 2.0 protocol, schema in Rust | Accepted |
| [0006](0006-kernel-owns-domain-model.md) | The kernel owns the domain model | Accepted |
| [0007](0007-core-plugins-declarative-ui.md) | Core plugins contribute declarative UI only | Accepted |
| [0008](0008-typescript-generation-with-ts-rs.md) | Generate TypeScript protocol types with ts-rs | Accepted |
| [0009](0009-temporary-scripted-provider.md) | A temporary scripted provider compiled into the core (M0 only) | Superseded by 0011 |
| [0010](0010-wasi-p2-and-session-instances.md) | WASI 0.2, one instance per session, blocking host imports | Accepted |
| [0011](0011-providers-from-plugins.md) | Providers come from plugins: qualified ids and provider backends | Accepted |
| [0012](0012-activation-memory-budget.md) | Budgets for plugin activation memory and the core binary | Accepted |
| [0013](0013-desktop-shell.md) | The desktop shell: own asset protocol, raw-line bridge, client plugin store | Accepted |
| [0014](0014-ui-sdk-versioning.md) | UI SDK 0.1: versioning, published lists, thread views | Accepted |
| [0015](0015-shell-memory-budget.md) | Shell memory budget: anonymous memory, gated on Linux | Accepted |
| [0016](0016-remote-transport-and-pairing.md) | Remote transport: WebSocket over TLS, pinned certificates, device tokens | Accepted |
| [0017](0017-shell-remote-profiles-and-ssh.md) | The shell's remote profiles, SSH through the system `ssh`, client secrets | Accepted |
| [0018](0018-package-signing-and-registry.md) | Package signing and the plugin registry | Accepted |
| [0019](0019-sandboxed-panels.md) | Sandboxed panels: iframes, a CSP per panel, a narrow bridge | Accepted |
| [0020](0020-interface-freeze-after-m5.md) | Freeze WIT and the UI SDK after M5; early publishers until then | Accepted |
| [0021](0021-no-sidecars-in-1-0.md) | No sidecar plugins in 1.0 | Accepted |
| [0022](0022-wit-0-2-and-dropping-0-1.md) | WIT 0.2.0, and the host drops 0.1.0 | Accepted |
| [0023](0023-outgoing-http-for-plugins.md) | Outgoing HTTP for plugins: wasi:http with the host's policy | Accepted |
| [0024](0024-schedules.md) | Schedules: kernel-owned timers that wake plugins | Accepted |
| [0025](0025-tool-gateway.md) | The tool gateway: MCP over stdio through the core's socket | Accepted |
| [0026](0026-first-party-plugins-in-their-own-repositories.md) | First-party plugins live in their own repositories | Accepted |
| [0027](0027-durable-queue-and-deferred-messages.md) | A durable thread queue, held after stops, and deferred messages | Proposed |
| [0028](0028-steering-a-running-turn.md) | Steering a running turn through a host input inbox | Proposed |
| [0029](0029-provider-switch-and-handoff.md) | Switching provider in a thread, with a bounded handoff | Proposed |
| [0030](0030-thread-agent-tools-plugin.md) | Agent tools for threads in a first-party plugin | Proposed |
