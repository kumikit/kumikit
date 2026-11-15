# Kumikit specification

Kumikit is a small, pluggable framework for working with coding agents. A headless Rust
**core** holds all state and runs plugins; a thin **client** (an embedded system webview
with a Svelte UI) connects to one or more cores, local or remote. Agent providers, themes,
bots and most UI are plugins that the user chooses to install.

The name comes from *kumiko*, Japanese woodwork in which a lattice is assembled from many
small pieces without nails: a small frame, filled in by the pieces you choose.

This folder is the source of truth for how Kumikit is meant to work. Code that disagrees
with the spec is a bug in one of them: fix whichever is wrong in the same change.

## Sections

| File | Covers |
|---|---|
| [00-overview.md](00-overview.md) | Goals, non-goals, principles, footprint budgets, glossary |
| [01-architecture.md](01-architecture.md) | Processes, crates, packages, how the parts fit |
| [02-protocol.md](02-protocol.md) | Client–core protocol: transports, handshake, methods, events |
| [03-kernel.md](03-kernel.md) | What the core owns: domain model, storage, secrets, processes |
| [04-plugin-host.md](04-plugin-host.md) | Core plugins: WASM host, manifest, permissions, lifecycle |
| [05-providers.md](05-providers.md) | Agent providers as plugins |
| [06-client.md](06-client.md) | Desktop shell, Svelte UI kernel, connections and tabs |
| [07-ui-plugins.md](07-ui-plugins.md) | UI plugins: slots, declarative UI, isolation |
| [08-themes.md](08-themes.md) | Themes as data: tokens, validation, import |
| [09-bots.md](09-bots.md) | Bots as core plugins |
| [10-distribution.md](10-distribution.md) | Packages, registry, signing, lockfiles, preinstalled set |
| [11-security.md](11-security.md) | Threat model and the controls that answer it |
| [12-roadmap.md](12-roadmap.md) | Milestones with exit criteria, open questions |

Decisions behind the spec are recorded in [../decisions/](../decisions/).

## Status

M0 (protocol and kernel) is built; M1 (plugin host) is in progress; later milestones are not. Interfaces shown here (WIT, JSON, TOML) are the
intended shape, not frozen APIs. They freeze at version 1.0 of each interface; until then
they may change, with the change recorded in the spec and, when significant, in an ADR.
