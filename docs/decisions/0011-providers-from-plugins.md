# 0011 — Providers come from plugins: qualified ids and provider backends

- **Status:** Proposed
- **Date:** 2026-11-15

## Context

In M0 the core registered one compiled-in provider
([ADR 0009](0009-temporary-scripted-provider.md)). From M1 every provider comes from an
enabled plugin, and the kernel must not name any of them
([ADR 0006](0006-kernel-owns-domain-model.md)).

Two questions follow:

1. **Ids.** Several plugins may want the same short provider id (`claude`, `default`). A
   global namespace means first-come conflicts that depend on install order.
2. **Declarative providers.** 05-providers says `kumikit.provider-acp` "reads `acp`
   entries from other plugins' manifests", so an ACP agent can be added with a manifest
   and no code. Letting one plugin read others' manifests is a special case for ACP and
   gives the reader more than it needs.

## Decision

- **Qualified provider ids.** On the wire and in storage a provider id is
  `<plugin id>/<local id>` (for example `community.provider-gemini/gemini`), where the
  plugin is the one whose manifest contributes the entry. Manifests use local ids.
  Plugin ids are unique, so provider ids cannot collide.
- **Provider backends.** A plugin may declare in its manifest that it serves a backend key:
  `[contributes] provider-backends = [{ key = "acp", schema = "acp.schema.json" }]`.
- A provider entry in any plugin's manifest may carry exactly one backend key
  (`acp = { … }`). The kernel validates that table against the backend's schema when the
  contributing plugin is enabled, and routes the provider's calls to the backend plugin,
  passing the table as JSON in `provider-ref.config`. The backend sees only the entries
  routed to it; there is no API to read other plugins' manifests.
- A delegated session runs the backend's code with the **contributing plugin's granted
  permissions**: the user approved "can start `gemini`" on that plugin. A backend needs no
  `spawn` permission of its own for delegated sessions.
- A plugin without `[runtime]` contributes only delegated entries. If the backend is not
  enabled, its providers are unavailable and not listed; they appear when it is.
- The kernel registers providers through its general `register_provider` API; the plugin
  host does the registering. Nothing in `crates/` names a plugin, backend key or provider.

## Consequences

- Declarative ACP providers need no code and no special case in the kernel or plugin
  host; other backends (a future JSON-lines adapter, say) use the same mechanism.
- Install text for a delegated provider names the backend that runs it.
- A backend plugin is trusted to act within its contributors' grants; it cannot exceed
  them because every host import checks the grants of the session it serves.
- Clients use qualified ids everywhere (`thread.create { provider }`, `Thread.provider`,
  `core.hello` contributions). M0 threads that name the scripted provider stay readable
  and fail with `plugin_failed` when sent to.
- [ADR 0009](0009-temporary-scripted-provider.md) is superseded by this record.
