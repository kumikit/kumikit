# 0002 — Separate headless core and thin client

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

In a single-process desktop app, the UI and the backend share state and native commands,
which makes it hard to run the backend on another machine, to attach several clients, or
to keep agents running when the window closes. Experience with an earlier app
showed a headless mode growing beside the desktop path and duplicating jobs the window
used to do.

## Decision

- The core is a separate headless process (`kumikit serve`) that owns all state.
- The desktop client is a Tauri 2 shell with a system webview. It has no project state and
  no feature-specific native commands; it reaches cores only through the protocol.
- Local and remote cores differ only in transport.

## Consequences

- One code path for local, remote and headless; several clients per core come for free.
- Closing the window does not stop agents.
- Every feature needs protocol surface, which costs some design work up front.
- A local setup runs two processes; the core's footprint budget accounts for that.
