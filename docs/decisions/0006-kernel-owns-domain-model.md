# 0006 — The kernel owns the domain model

- **Status:** Accepted
- **Date:** 2026-11-15

## Context

If threads, turns and events were defined by plugins, every UI and bot would need to
understand every provider's model, and providers could not be swapped. If the kernel knew
providers by name, adding one would mean editing the kernel.

## Decision

- Workspaces, threads, turns, events and approvals are defined and stored by the kernel.
- Providers translate their agent's output into kernel events; they own nothing else.
- Plugins can add event kinds only through the namespaced `plugin.event`.
- No kernel or UI-kernel code names a specific plugin.

## Consequences

- Any client renders any provider; providers are interchangeable per thread.
- New agent features need either a general event kind (a protocol minor version) or a
  `plugin.event` with a UI renderer plugin.
- A grep for plugin ids outside `plugins/` is a valid CI check.
