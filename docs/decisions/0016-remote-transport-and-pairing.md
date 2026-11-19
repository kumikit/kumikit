# 0016 — Remote transport: WebSocket over TLS, pinned certificates, device tokens

- **Status:** Accepted
- **Date:** 2026-11-17

## Context

M3 lets a client use cores on other machines ([02-protocol](../spec/02-protocol.md),
[12-roadmap](../spec/12-roadmap.md#m3--multiple-cores)). The spec named the shape
(`wss://host:port/rpc`, a pairing code, a device token, a pinned self-signed
certificate) and left the rest open:

1. **TLS library.** rustls with `ring`, rustls with `aws-lc-rs` (its default), or the
   platform's TLS (OpenSSL on Linux). aws-lc-rs's licence includes the OpenSSL licence,
   which is not on the AGENTS.md list, and it needs CMake (and NASM on Windows) to build.
2. **HTTP and WebSocket.** The desktop needs only a WebSocket on `/rpc`; the browser client
   (06-client), which is in M3, needs a few HTTP routes on the same port.
3. **How remote access is turned on**, and whether anything stores that it is on.
4. **How `kumikit pair` gets a code** from a running core: the spec had no method for it.
5. **Where device tokens live.** 03-kernel said the secret store; the storage section listed
   a `devices` table.
6. **How a browser authenticates** without giving page scripts the token.

## Decision

- **TLS:** rustls with the `ring` provider (`Apache-2.0 AND ISC`), TLS 1.3 only, through
  tokio-rustls. One implementation on every OS, no C toolchain beyond what `ring` needs.
- **Certificate:** generated with rcgen (ECDSA P-256, ten years) into `<core data>/tls/` on
  the first start with remote access, unless the user passes `--tls-cert` and `--tls-key`.
  Clients pin the SHA-256 of the leaf certificate; names and dates are not checked. A
  changed certificate is refused until the user re-adds the core.
- **HTTP:** hyper's HTTP/1.1 server, with `/rpc` upgraded to a WebSocket by
  tokio-tungstenite. No web framework.
- **Turning it on:** only `kumikit serve --listen <host[:port]>` (default port 7463, no
  default address). Nothing is stored, so every start is local-only unless the flag is
  given, and no protocol method can turn remote access on.
- **The gate:** a native client's first message on `/rpc` must be `auth.token` or
  `auth.pair`; anything else gets `unauthenticated` and the connection closes. After that
  the usual handshake applies. Pre-auth connections have a 10 s deadline and a cap of 32.
- **Pairing codes** live in the running core's memory. `kumikit pair` asks for one with
  `auth.pairing.create`, a method allowed only on local connections. A code is 40 bits,
  valid 5 minutes, used once, burnt after 5 wrong tries, and a new code replaces the old.
- **Device tokens** are 256 random bits. The core keeps only their SHA-256 in the kernel's
  `devices` table, never in the secret store: it never needs a token back. Revoking a
  device deletes the row and closes its live connections.
- **Browsers:** a request with an `Origin` header is a browser. It is accepted only when the
  core serves the web UI (`--web`) and the origin is the core's own. A browser pairs with
  `POST /auth/pair`, which sets the token as an `HttpOnly; Secure; SameSite=Strict` cookie
  and never returns it in a body; the WebSocket upgrade is then authenticated by the
  cookie.
- **Keepalive:** the core pings remote connections and closes one that is silent for 60 s.

## Consequences

- The core binary grows by the TLS, HTTP and WebSocket code even with remote access off; it
  must stay within the 32 MiB budget, and nothing remote is initialised without `--listen`.
- Remote access can only be turned on by whoever starts the core. A settings page for it
  needs a persisted setting and a decision on which clients may change it.
- A user-supplied certificate that renews (for example every 60–90 days) breaks every
  paired client until it is re-added; pinning the public key or trusting CA roots is the
  next step if users need it.
- A stolen token stays usable until revoked; revocation is immediate.
- A web page on another origin cannot reach a core's socket or its pairing route.
