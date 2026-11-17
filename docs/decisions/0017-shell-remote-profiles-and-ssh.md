# 0017 — The shell's remote profiles, SSH through the system `ssh`, client secrets

- **Status:** Proposed
- **Date:** 2026-11-17

## Context

[ADR 0013](0013-desktop-shell.md) gave the shell a raw-line bridge whose
`bridge_connect(profile, channel)` takes a profile name, so that M3 could add remote cores
without changing it, and it let the shell start exactly one program: its bundled core.
M3 needs:

1. A way to add, list and remove remote profiles, and to see a core's certificate before
   pairing, without giving the webview tokens or a way to open arbitrary sockets.
2. Somewhere to keep device tokens (06-client: the OS keychain or a 0600 file).
3. SSH. 02-protocol described "WebSocket over an SSH tunnel" followed by a device token.
   Options:
   - a Rust SSH client (`russh`, Apache-2.0): its own crypto stack, and none of the user's
     `~/.ssh/config`, agent, `ProxyJump`, hardware keys or `known_hosts` unless
     reimplemented;
   - the system `ssh` with a port forward (`-L`) to the remote's `wss` port: needs remote
     access turned on, a free local port, and pairing on top of SSH;
   - the system `ssh` running a small `kumikit bridge` command on the remote machine, which
     copies JSON-RPC lines between SSH's stdio and that machine's local core socket.

## Decision

- **Profile commands.** Besides the bridge and the client plugin store, the shell has
  `profiles_list`, `profile_probe(address)`, `profile_pair(…)`, `profile_add_ssh(…)` and
  `profile_remove(name)`. The webview passes an address or SSH destination only when the
  user adds a profile; afterwards it uses the name. `profiles_list` returns names, labels
  and kinds, not addresses. No command returns a token.
- **Pinning.** For a TLS profile the shell checks the certificate's SHA-256 against the
  pinned value and checks the TLS 1.3 handshake signature; it ignores names and dates.
  `profile_probe` does the handshake, records the fingerprint and sends nothing, so the
  user can compare it with what `kumikit pair` printed before a code is sent.
- **Authentication in the shell.** For a TLS profile `bridge_connect` sends `auth.token`
  itself and hands the stream to the bridge only after it succeeds, so the webview never
  sees a token and its first message is still `core.hello`.
- **Client secrets.** `keyring` (MIT OR Apache-2.0) with only the macOS Keychain and Windows
  Credential Manager backends; a 0600 file in the client data directory on Linux (and
  wherever `KUMIKIT_CLIENT_SECRETS=file` asks for it). Secret Service on Linux can come
  later with its own dependency review.
- **SSH.** The shell starts the system `ssh` (found on `PATH`, or `KUMIKIT_SSH`) as
  `ssh -T -o BatchMode=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=3 --
  <destination> kumikit bridge`, with no shell in between and a validated destination.
  `kumikit bridge` connects to the remote machine's local socket, so the core sees a local
  connection of the same user. SSH is the only authentication for these profiles: no
  pairing and no device token.
- This extends ADR 0013's process exception: the shell may start its bundled core and the
  system `ssh`, nothing else.

## Consequences

- The user's SSH setup works as in a terminal. With `BatchMode` nothing can prompt, so an
  unknown host key or a passphrase-protected key without an agent fails with ssh's own
  message, which the app shows.
- `kumikit` must be on the remote `PATH`, or the profile names the remote command.
- 02-protocol's transport table and 01's diagram describe stdio over SSH instead of a
  WebSocket tunnel.
- A trusted inline UI plugin can call the profile commands; the trust prompt already says
  such a plugin has full access to the app. Sandboxed panels (M4) will not get them.
- Linux keeps tokens in a 0600 file, like the core's own secret store.
