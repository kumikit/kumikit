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
| [0009](0009-temporary-scripted-provider.md) | A temporary scripted provider compiled into the core (M0 only) | Proposed |
| [0010](0010-wasi-p2-and-session-instances.md) | WASI 0.2, one instance per session, blocking host imports | Proposed |
| [0011](0011-providers-from-plugins.md) | Providers come from plugins: qualified ids and provider backends | Proposed |
