# 0014 — UI SDK 0.1: versioning, published lists, thread views

- **Status:** Proposed
- **Date:** 2026-11-16

## Context

[07-ui-plugins](../spec/07-ui-plugins.md) and [08-themes](../spec/08-themes.md) showed
UI manifests with `api = "^1.0"` and the token set as `tokens@1`, which would freeze
both before third parties can publish (M4). M1 made the same choice for WIT: 0.1.0
packages, with published versions protected by `wit/PUBLISHED`. Building ui-chat
raised two more gaps:

- No slot holds the main view of a thread, and the UI kernel may not name the chat
  plugin.
- The protocol allows one subscription per topic per connection. A plugin that
  subscribed to a thread itself would replace the UI kernel's subscription.

## Decision

- `@kumikit/ui-sdk` starts at **0.1.0** and freezes at 1.0, before third parties publish
  (when: [ADR 0020](0020-interface-freeze-after-m5.md), after M5).
  UI plugin and theme manifests use `api = "^0.1"`, a range over the UI SDK version.
- The slot list and the token set are versioned with the SDK. Each published version is
  a directory `web/sdk/api/<version>/` holding `slots.json` and `tokens.json`.
  `web/sdk/api/PUBLISHED` records a SHA-256 of each directory, and a test fails if one
  changes. A change means a new version directory. The TypeScript `Slot` and `TokenName`
  types are generated from these files.
- A new slot, **`thread.view`**, is the main view of a thread. The UI kernel mounts the
  first enabled contribution.
- The UI kernel owns every wire subscription. Plugins read a thread through
  **`ctx.threads.watch(address, fn)`**, which gives the UI kernel's folded view and its
  events. `ctx.connection.subscribe` fans out from the kernel's subscriptions.

## Consequences

- Slots and tokens can change before M4, but every change is a visible new version.
- Several plugins can show the same thread without extra wire traffic.
- The folded thread view is now SDK surface, so it is versioned with the SDK too.
