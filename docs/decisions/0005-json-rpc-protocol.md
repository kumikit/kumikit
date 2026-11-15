# 0005 — JSON-RPC 2.0 protocol, schema in Rust

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

The client–core protocol must work over Unix sockets, named pipes and WebSockets, be easy
to debug, and be usable from a browser, a bot or a script. Agent tooling already uses the
JSON-RPC family (LSP, MCP, ACP).

## Decision

- JSON-RPC 2.0 with notifications for events; newline-delimited on sockets and pipes, one
  message per frame on WebSockets.
- The schema is Rust types in `kumikit-protocol`; TypeScript types are generated from
  them (`ts-rs` or `specta`) into `@kumikit/protocol` and never edited by hand.
- Recorded message fixtures are contract tests.

## Consequences

- Human-readable traffic and simple clients in any language.
- JSON costs more bytes and CPU than a binary format; acceptable at agent-event rates.
  Text deltas may be coalesced for slow clients.
- The two sides cannot drift: a Rust change regenerates the TypeScript, and CI fails if
  the generated files are stale.
