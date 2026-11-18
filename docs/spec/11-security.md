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
| A UI plugin reads other tabs or the core | Third-party UI plugins run as sandboxed panels: an opaque-origin iframe, a bridge limited to the tab's connection, read-only kernel methods in the tab's workspace and the `requires.core` plugins' methods; inline only when trusted |
| A panel reaches the shell (profiles, tokens, the plugin store) | No shell IPC in the frame; the bridge exposes no shell command |
| A panel sends data out | A CSP per panel: `connect-src` only to the hosts it declares, shown at install |
| A panel navigates, opens popups or hands its bridge to another page | Sandbox without top navigation or popups; a frame that loads a second document is removed and its port closed |
| A theme injects code or CSS | Themes are typed token values only; no raw CSS in v1 |
| Someone on the network connects to a core | Remote access only with `serve --listen`, nothing stored that turns it on; TLS 1.3 with a pinned certificate; the first message must authenticate; pairing code then device token; tokens revocable, closing live connections |
| Guessing a pairing code | 40-bit codes, valid 5 minutes, single use, burnt after 5 wrong tries, one live code at a time |
| Slow or numerous unauthenticated connections | 10 s to authenticate, at most 32 unauthenticated connections |
| A man in the middle at pairing | The user compares the fingerprint `kumikit pair` printed with the one the client shows before the code is sent over that pinned connection |
| A replaced certificate | Clients refuse a changed fingerprint and do not retry; the user re-adds the core. Rotating the core's own certificate (deleting `tls/`) needs every client re-added |
| A stolen device token | Tokens are random 256-bit values; the core stores only hashes; clients keep them in the OS keychain or a 0600 file, never in the webview; revocation is immediate |
| A web page reaches a core (cross-site WebSocket or form) | A browser `Origin` must be the core's own and the web UI must be on; the browser token is an `HttpOnly; Secure; SameSite=Strict` cookie; pairing and sign-out need same-origin JSON requests; `frame-ancestors 'none'` |
| SSH destinations used for injection | Validated, passed after `--`, no shell; `BatchMode` so nothing prompts |
| A trusted inline UI plugin adds or removes core profiles | Trust already grants full access to the app (the trust prompt says so); sandboxed panels get no profile commands |
| Another local user connects to the core | Socket 0600 and peer-credential check; per-user pipe ACL on Windows |
| A tampered or substituted package | SHA-256 in the registry, publisher signature over every file, key pinned across versions; registry packages must be signed |
| A registry entry that misstates permissions | The entry's copy of the manifest must equal the package's (registry CI and install) |
| A second registry takes over a plugin's updates | A plugin updates only from the registry it was installed from |
| A network attacker on registry traffic | HTTPS with the OS's trusted certificates; plain `http` only to loopback; size and time limits |
| A silent or downgrading update | Updates are listed only on request, applied only by the user, never to a lower version; wider permissions need a new grant |
| A lockfile import installs something else | Each entry's SHA-256 is pinned; an import adds no registry |
| A publisher's private key leaks | Key files are 0600 and kept outside work trees; never printed or logged; CI refuses committed private keys |
| The registry is contacted behind the user's back | Only on browse, search, install, lockfile import or an update check; the first-run screen asks first |
| A typo-squatted plugin | Registry CI checks for confusable ids; one key per publisher namespace; install shows publisher, key id, registry and repository |
| Secrets leak to the UI or logs | Secrets are write-only over the protocol; the logger redacts values of known secrets |

## Rules

- The client never receives a secret value.
- No telemetry and no automatic network requests other than ones the user turned on. Any
  new automatic request is listed in the README's privacy section in the same change.
- Every permission check failure is logged with the plugin id and the operation.
- Security issues are reported privately ([SECURITY.md](../../SECURITY.md)).
