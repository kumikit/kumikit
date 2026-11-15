# 0008 — Generate TypeScript protocol types with ts-rs

- **Status:** Proposed
- **Date:** 2026-11-15

## Context

[ADR 0005](0005-json-rpc-protocol.md) puts the protocol schema in Rust and generates
TypeScript from it with "`ts-rs` or `specta`". The choice has to be made before the first
protocol type exists.

- **ts-rs**: a derive macro per type that reads the serde attributes (`rename_all`,
  `tag`, `content`, `skip_serializing_if`). Stable releases, no runtime registry, MIT.
- **specta** + **specta-typescript**: a type registry that can also export to other
  languages. Its 2.x line spent a long time in release candidates, and the exporter is a
  separate crate. MIT.

## Decision

- Use **ts-rs**.
- Generation is a test, not a build side effect: a test in `kumikit-protocol` exports
  every type into a temporary directory and compares it with `web/protocol/src/`. It fails
  when they differ. Running it with `KUMIKIT_BLESS=1` rewrites the files.
- The method table is declared once in Rust (a `methods!` macro). The same test turns it
  into a TypeScript map from method name to params and result types.

## Consequences

- A normal `cargo test` never writes into the source tree; CI catches stale types.
- ts-rs maps `u64` to `bigint`, which `JSON.parse` never produces. Protocol integers
  (`seq`, token counts) are annotated as `number`; they stay below 2^53.
- If Kumikit later needs generated types for another language, revisit specta in a new ADR.
