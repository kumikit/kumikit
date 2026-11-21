# 0023 — Outgoing HTTP for plugins: wasi:http with the host's policy

- **Status:** Accepted
- **Date:** 2026-11-19

## Context

The Telegram bot and the GitHub plugin talk to web APIs. [04](../spec/04-plugin-host.md#wasi)
said outgoing HTTP would use `wasi:http` with the host enforcing `permissions.network`, and
that a component importing it is refused until then. The options were `wasi:http`
(through `wasmtime-wasi-http`, Apache-2.0 WITH LLVM-exception) or an own
`kumikit:host/http` interface. `wasmtime-wasi-http`'s default client brings
`webpki-roots` (CDLA-Permissive-2.0, not on the licence list); the crate lets the
embedder supply the request sender instead.

Tests must not reach the real services, yet should run the real manifests and the real
grant check.

## Decision

- Plugins use **`wasi:http/outgoing-handler`** (WASI 0.2), linked through
  `wasmtime-wasi-http` without its default sender. Kumikit's own sender runs every
  request through the policy:
  - `https` only (the OS trust store, as for registries);
  - the host, and the port when not 443, must be in `permissions.network` (`host` or
    `host:port`, no wildcards);
  - the addresses the name resolves to must be public, unless the entry itself is a
    loopback or private IP literal;
  - no automatic redirects (a guest that follows one sends a new, checked request);
  - limits per plugin: 8 requests in flight, request bodies 8 MiB, responses 16 MiB,
    10 s to connect, a guest-chosen timeout up to 120 s per request.
- Refusals reach the guest as wasi:http errors and are logged with the plugin id. Logs
  name the scheme and host only: never the path (which may hold a token), headers or
  bodies.
- **Tests:** in debug builds only, `KUMIKIT_TEST_HTTP_ROUTES=<host>=<http://127.0.0.1:port>[,…]`
  sends requests for a granted host to a loopback fake after the policy has accepted
  them. Release builds ignore the variable and log that they did.

## Consequences

- Guests can use ordinary WASI HTTP code; the Rust SDK wraps it in a small blocking
  client.
- A new host dependency and its bindings grow the core binary (measured in M5's step 2).
- The README privacy section lists each plugin that makes automatic requests.
