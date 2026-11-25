# Remote connectivity

A Kumikit desktop window can connect to several cores side by side: the local core on your
machine plus remote cores on servers, devboxes or other workstations. You can also connect
from a web browser directly to a core that serves the browser client.

Remote access is **off by default**. A core never opens a network port unless you start it
with `--listen`, and nothing about `--listen` is persisted across restarts.

## Choosing a mode

| Mode | Needs `--listen`? | Client | Auth & transport | Best for |
|---|---|---|---|---|
| **TLS + Pairing** | Yes | Desktop app | TLS 1.3 with pinned cert fingerprint + one-time pairing code → device token | Direct LAN, VPN (Tailscale/WireGuard), or exposed server |
| **SSH tunnel** | No | Desktop app | Your existing system `ssh` (`BatchMode`) running `kumikit bridge` | Remote devboxes where you already have key-based SSH access and want no extra listening port |
| **Browser client** | Yes (`--listen` + `--web`) | Any modern browser | TLS 1.3 + pairing code → `HttpOnly` cookie | Quick access from a tablet, phone or machine without the desktop app |

For wire details and threat model, see [02-protocol](spec/02-protocol.md#remote-transport),
[06-client](spec/06-client.md#multiple-cores) and [11-security](spec/11-security.md).

---

## Option 1: Pairing over TLS

### 1. Start the core with `--listen`

On the remote machine:

```sh
kumikit serve --listen 0.0.0.0:7463
```

- Port `7463` is the default if you omit it (e.g., `--listen 0.0.0.0` or `--listen 127.0.0.1`).
- There is no default bind address: pick the interface you want (`0.0.0.0` for all interfaces,
  a Tailscale/LAN IP, or `127.0.0.1` for loopback testing).
- **TLS certificate:** on first start with `--listen`, the core generates a self-signed
  ECDSA P-256 certificate in `<core data>/tls/cert.pem` and `key.pem` (`0600`). To supply
  your own certificate instead, pass `--tls-cert <cert.pem> --tls-key <key.pem>`.

### 2. Create a one-time pairing code

In another terminal on the same remote machine:

```sh
kumikit pair
```

This talks to the running core over its local socket and prints:

- a one-time 8-character pairing code (for example `ABCD-EFGH`),
- the core's certificate fingerprint (`SHA256:…`),
- the address/URL to connect to.

Pass `--json` for machine-readable output. Pairing codes expire after **5 minutes**, work
**once**, and are burnt after **5 wrong attempts**. Running `kumikit pair` again replaces
any previous unused code.

### 3. Pair from the desktop app

1. Open **Cores** in `kumikit-desktop` (click the server icon at the top-right of the left
   sidebar, or click the connection status in the bottom-right of the status bar, or run
   `Cores: add, remove, reconnect` from the command palette).
2. Click **Add a core** → **Pair over the network**.
3. Enter the core's **Address** (`host:port`, e.g., `devbox.local:7463` or `192.168.1.20:7463`)
   and a display **Label**.
4. Click **Check certificate**. Compare the `SHA256:…` fingerprint shown by the app with
   the fingerprint printed by `kumikit pair`.
5. Enter the pairing code and click **Pair**.

The app pins the certificate fingerprint, stores the returned device token in your OS
keychain (or a `0600` secrets file), and connects immediately. The remote core appears as
its own group in the left sidebar.

### Managing and revoking paired devices

On the core machine:

```sh
kumikit devices list
kumikit devices revoke <device-id>
```

Revoking a device immediately closes its active connections and invalidates its token.
Removing a core from **Cores** in the desktop app also revokes that device on the core if
it is reachable.

> **Certificate changes:** Clients pin the certificate's fingerprint. If `<core data>/tls/`
> is deleted or replaced, paired clients refuse to connect (`Certificate changed`) and do
> not retry; remove and re-pair the core to accept the new certificate.

---

## Option 2: Through SSH

SSH mode requires **no open TCP port** and **no `--listen` flag** on the remote machine.
The desktop shell spawns your system `ssh` client (with `BatchMode=yes`, no interactive
prompts) and runs `kumikit bridge` on the remote host, forwarding JSON-RPC lines over
stdin/stdout to the remote core's local socket.

### Prerequisites on the remote machine

1. Key-based, non-interactive SSH works from your local machine (`ssh <destination> true`
   succeeds without asking for a password or passphrase — use `ssh-agent` if your key has a
   passphrase).
2. `kumikit serve` is running on the remote machine.
3. `kumikit` is on the remote `PATH` (or you provide the full command path below).

### Add an SSH core in the desktop app

1. Open **Cores** in `kumikit-desktop`.
2. Click **Add a core** → **Through SSH**.
3. Enter:
   - **Label:** display name (optional; defaults to the destination).
   - **Destination:** SSH target (for example `devbox` from `~/.ssh/config`, or
     `alice@10.0.0.5`).
   - **Command (optional):** leave blank to run `kumikit bridge` using the default socket, or
     specify a custom command if the binary is not on `PATH` or uses a non-default socket,
     for example:
     ```text
     /usr/local/bin/kumikit bridge --socket /path/to/s.sock
     ```
4. Click **Add**.

The remote core appears in the sidebar and reconnects automatically if the SSH tunnel
drops.

### Installing or syncing matching `kumikit` over SSH

If `kumikit` is missing on the remote machine or is on a different version than your
desktop app:

- When adding an SSH core, Kumikit probes the remote host (`uname -s`, `uname -m`) and
  offers to download the matching `kumikit` archive **locally on your computer**, verify
  its Ed25519 signature and SHA-256 hash, stream it over `ssh` into `~/.local/bin/kumikit`,
  start `kumikit serve`, and connect — working even on firewalled hosts without direct
  GitHub access.
- On existing SSH cores in **Cores**, click **Install matching version…** whenever a core
  is on a different version, or opt into **Auto-sync matching Kumikit version** per host
  (or globally in **Settings → Updates**). An existing remote core is checked for idle
  (`core.activity`) before stopping so active remote agents are never interrupted.
- On headless or TLS-paired cores, run `kumikit update [--check] [--wait-idle] [--yes]`
  directly on that machine.

---

## Option 3: Browser client

A core can serve the Kumikit web UI over the same HTTPS port it uses for remote pairing.

### 1. Build the browser bundle

If you built from source ([building.md](building.md)), generate `target/bundle/ui-browser`:

```sh
node scripts/bundle.mjs
```

### 2. Start the core with `--listen` and `--web`

```sh
kumikit serve --listen 0.0.0.0:7463 --web ./target/bundle/ui-browser
```

### 3. Pair in the browser

1. Generate a pairing code on the core machine:
   ```sh
   kumikit pair
   ```
2. Open `https://<host>:7463` in your browser.
3. With a self-signed certificate, your browser shows a certificate warning once — verify
   the certificate's SHA-256 fingerprint matches `kumikit pair`, then proceed.
4. Enter a device name and the pairing code on the pairing screen.

The core sets a `kk_device` cookie (`HttpOnly; Secure; SameSite=Strict`) so page scripts
cannot read the token. To revoke the browser session, run **Sign out of this core** from
the command palette (`Mod+K`).

### Browser client differences

- Connects only to the single core serving the page (no multi-core profiles).
- Uses the bundled UI plugins served with `--web` (installing additional UI plugins
  requires the desktop app). Core plugins can still be installed by uploading a `.kkp` file.
- Opening a project folder prompts for a directory path on the core's machine.

### Frontend dev mode against a remote core

For UI development with Vite hot reload against a live core:

```sh
npm run dev:browser -- --core https://<host>:<port> --fingerprint <SHA256:...>
```

---

## Testing all three modes on a single machine

You can exercise TLS pairing, the browser client and SSH tunneling on one machine by
running a second `kumikit serve` instance with its own `--socket` and `--data-dir` (so it
does not collide with your local core):

### 1. Start a second core on loopback

```sh
mkdir -p /tmp/kk-remote/core && chmod 700 /tmp/kk-remote /tmp/kk-remote/core

./target/debug/kumikit serve \
  --socket /tmp/kk-remote/s.sock \
  --data-dir /tmp/kk-remote/core \
  --bundled-plugins ./target/bundle/core-plugins \
  --listen 127.0.0.1:7463 \
  --web ./target/bundle/ui-browser
```

### 2. Generate a pairing code for that core

Notice the `--socket` flag pointing at the second core:

```sh
./target/debug/kumikit pair --socket /tmp/kk-remote/s.sock
```

### 3. Try each client mode

- **Desktop app (TLS + Pairing):** in `kumikit-desktop` → **Cores** → **Pair over the
  network**, use address `127.0.0.1:7463`, verify the fingerprint, and enter the code.
- **Browser client:** run `kumikit pair --socket /tmp/kk-remote/s.sock` again for a fresh
  code, then open `https://127.0.0.1:7463` in your browser and pair.
- **Desktop app (SSH):** make sure `sshd` is running locally and key-based `ssh localhost true`
  succeeds without a password prompt. In **Cores** → **Through SSH**, set:
  - **Destination:** `localhost`
  - **Command:** `<repo-path>/target/debug/kumikit bridge --socket /tmp/kk-remote/s.sock`

List or revoke paired devices on the second core with:

```sh
./target/debug/kumikit devices --socket /tmp/kk-remote/s.sock list
./target/debug/kumikit devices --socket /tmp/kk-remote/s.sock revoke <device-id>
```

### Automated remote tests

```sh
# TLS pinning, pairing, token login, revocation, keepalive, and mock SSH
cargo test -p kumikit-shell --test remote

# Live SSH against localhost (requires key-based `ssh localhost` and `kumikit` on PATH)
KK_SSH_DEST=localhost cargo test -p kumikit-shell --test remote -- --ignored
```

## See also

- [02-protocol — Remote transport & pairing](spec/02-protocol.md#remote-transport)
- [06-client — Multiple cores & browser client](spec/06-client.md#multiple-cores)
- [11-security — Threats and controls](spec/11-security.md)
- [ADR 0016 — Remote transport and pairing](decisions/0016-remote-transport-and-pairing.md)
- [ADR 0017 — Shell remote profiles and SSH](decisions/0017-shell-remote-profiles-and-ssh.md)
