# 11 — Security

## Assets

Source code in workspaces, provider and bot credentials, device tokens, the ability to run
programs as the user, and the conversation history.

## Threats and controls

| Threat | Control |
|---|---|
| A malicious or buggy core plugin reads files or secrets | WASM sandbox; no filesystem except its data directory; `files` and `secrets` permissions checked on every host call |
| A plugin exfiltrates data | No network unless `network` lists hosts; the host enforces the list for `wasi:http` |
| A plugin escapes through a spawned program | `spawn` is an explicit, prominently explained permission; only listed executables; cwd limited to workspaces; minimal environment |
| A plugin exhausts the core | Memory limits, epoch interruption, restart with backoff, per-plugin process accounting |
| A remote core runs code in the user's webview | Core plugins have declarative UI only; UI plugins are installed on the client by the user |
| A UI plugin reads other tabs or the core | Iframe sandbox by default; bridge limited to declared needs; inline only when trusted |
| A theme injects code or CSS | Themes are typed token values only; no raw CSS in v1 |
| Someone on the network connects to a core | Remote access only with `serve --listen`, nothing stored that turns it on; TLS 1.3 with a pinned certificate; the first message must authenticate; pairing code then device token; tokens revocable, closing live connections |
| Guessing a pairing code | 40-bit codes, valid 5 minutes, single use, burnt after 5 wrong tries, one live code at a time |
| Slow or numerous unauthenticated connections | 10 s to authenticate, at most 32 unauthenticated connections |
| A man in the middle at pairing | The user compares the fingerprint `kumikit pair` printed with the one the client shows before the code is sent over that pinned connection |
| A replaced certificate | Clients refuse a changed fingerprint and do not retry; the user re-adds the core. Rotating the core's own certificate (deleting `tls/`) needs every client re-added |
| A stolen device token | Tokens are random 256-bit values; the core stores only hashes; clients keep them in the OS keychain or a 0600 file, never in the webview; revocation is immediate |
| A web page reaches a core (cross-site WebSocket or form) | A browser `Origin` must be the core's own and the web UI must be on; the browser token is an `HttpOnly; Secure; SameSite=Strict` cookie; pairing and sign-out need same-origin JSON requests; `frame-ancestors 'none'` |
| SSH destinations used for injection | Validated, passed after `--`, no shell; `BatchMode` so nothing prompts |
| A trusted inline UI plugin adds or removes core profiles | Trust already grants full access to the app (the trust prompt says so); sandboxed panels (M4) get no profile commands |
| Another local user connects to the core | Socket 0600 and peer-credential check; per-user pipe ACL on Windows |
| A tampered or substituted package | SHA-256 in the registry, publisher signature, key pinned across versions |
| A typo-squatted plugin | Registry CI checks for confusable ids; install shows publisher and repository |
| Secrets leak to the UI or logs | Secrets are write-only over the protocol; the logger redacts values of known secrets |

## Rules

- The client never receives a secret value.
- No telemetry and no automatic network requests other than ones the user turned on. Any
  new automatic request is listed in the README's privacy section in the same change.
- Every permission check failure is logged with the plugin id and the operation.
- Security issues are reported privately ([SECURITY.md](../../SECURITY.md)).
