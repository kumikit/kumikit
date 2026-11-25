# 0032 — Matching remote core installation over SSH

- **Status:** Accepted
- **Date:** 2026-11-25

## Context

Connecting to a remote machine through SSH ([ADR 0017](0017-shell-remote-profiles-and-ssh.md))
currently requires the user to manually download, verify, install, and keep `kumikit` in sync
with the desktop app's protocol version on every remote host. When `kumikit` is missing or on an
older protocol version, `profile_add_ssh` fails or the connection stops in `mismatch`.

Remote hosts are often behind corporate firewalls or lack direct access to GitHub Releases, and
piping an unverified script into a remote shell (`curl | sh`) violates Kumikit's security model.
At the same time, an SSH sync must never kill active agent turns on the remote machine.

## Decision

1. **Client-side verified download, streamed over SSH.**
   `kumikit-shell` adds two SSH sync commands (`ssh_probe` and `ssh_sync`) plus a per-profile
   `autoSync` flag on SSH profiles (`profile_update_ssh`):
   - `ssh_probe` inspects the remote host over the existing system `ssh` (`BatchMode=yes`, no
     shell interpolation of the destination) using `uname -s`, `uname -m`, and `kumikit --version`
     (checking `PATH` and `$HOME/.local/bin/kumikit`).
   - `ssh_sync` fetches the signed release manifest for the desktop app's own version
     ([ADR 0031](0031-signed-release-manifest-and-updates.md)), downloads the matching
     `kumikit-<tag>-<target>.tar.gz` archive **locally on the client**, verifies its Ed25519
     manifest signature and SHA-256 hash (caching verified archives in `<client data>/remote-cache/`),
     and streams the archive over `ssh` `stdin` to extract and atomically install
     `$HOME/.local/bin/kumikit`.

2. **Idle check before restarting an existing remote core.**
   Before replacing a running remote core, `ssh_sync` queries `core.activity` and calls
   `core.shutdown { ifIdle: true }` over `kumikit bridge` when `ifIdle` is requested. If remote
   agents are active, it refuses with `busy` rather than interrupting work. After installing, it
   starts `kumikit serve` detached on the remote host if no core answers and waits until
   `kumikit bridge` answers `core.hello`.

3. **Prompt by default, optional auto-sync.**
   When adding an SSH core or encountering a missing or version-mismatched SSH core, the UI
   prompts before downloading and installing on the remote host. Users can opt into automatic
   sync per SSH profile (`autoSync`) or globally in Settings (`autoSyncSsh`, default `false`).

4. **TLS-paired cores stay out of scope for remote binary replacement.**
   Paired cores over TLS do not accept remote binary replacement over JSON-RPC; operators update
   them directly with `kumikit update` ([ADR 0031](0031-signed-release-manifest-and-updates.md)).

## Consequences

- Works on air-gapped or firewalled remote servers as long as the desktop machine can reach the
  release mirror or has the archive cached locally.
- Zero remote `curl | sh`; every byte written to the remote host is Ed25519- and SHA-256-verified
  before transmission.
- Non-interactive SSH shells often omit `~/.local/bin` from `PATH`; when `ssh_sync` installs to
  `~/.local/bin/kumikit` for a profile with no custom command, the shell records
  `$HOME/.local/bin/kumikit bridge` so subsequent reconnects always find it.
- [06-client](../spec/06-client.md), [10-distribution](../spec/10-distribution.md), and
  `docs/remote.md` are updated in the same change.
