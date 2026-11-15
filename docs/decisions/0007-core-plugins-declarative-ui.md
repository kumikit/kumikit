# 0007 — Core plugins contribute declarative UI only

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

A client may connect to remote cores the user does not fully control. If a core plugin
could ship JavaScript to the client, connecting to a core would run that core's code in
the user's webview, next to tabs from other cores.

## Decision

- Core plugins contribute UI only as data: settings schemas, commands, status items and
  views from a fixed vocabulary, rendered by the UI kernel.
- UI code (iframe panels, inline modules) comes only from UI plugins that the user
  installs on the client.

## Consequences

- Connecting to any core is safe for the client.
- Core plugin UI automatically matches the theme.
- Rich plugin UI needs two packages (a core plugin and a UI plugin); the UI plugin declares
  `requires.core` so it activates only where its counterpart exists.
- The declarative vocabulary must grow carefully; each addition is a UI SDK minor version.
