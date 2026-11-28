# 0034 — Agents start threads with a chosen model and settings

- **Status:** Accepted
- **Date:** 2026-11-28

## Context

An agent sometimes needs a second opinion from another agent. For example, "start two
threads, one with Gemini at medium effort and one with Claude Sonnet, and have each review
the last commit". The `kumikit.threads` tools ([ADR 0030](0030-thread-agent-tools-plugin.md))
can read, message, queue and steer threads that already exist, but no tool can start one.

The host already has most of what this needs:

- `kumikit:host/client@0.2.0` has `create-thread(workspace, provider, model, title)`,
  gated by `threads.create`. But that function is published and takes no model settings
  ([ADR 0033](0033-model-settings-and-provider-icons.md)), so "medium effort" can't be
  passed to it, and it takes no permission mode.
- Plugins can list providers but not their models. An agent can't map "Gemini 4 Neon at
  medium" to a provider id, a model id and `effort = medium` without seeing each model and
  its settings.
- The kernel's `thread_create_opts` already takes settings and a permission mode, and
  `thread.create` already offers both to clients. The protocol doesn't need to change.

Starting a thread starts an agent, and agents cost money and can act. A plugin must not be
able to raise the permission mode: a `standard` thread mustn't be able to start a
`full_access` one.

Published WIT versions can't change in place. A new function needs a new version.

## Decision

- Add `kumikit:threads@0.2.0`. Its `client` interface has everything from 0.1.0 plus:
  - `list-models(provider)`: the provider's models with their settings, as the JSON of
    `provider.models`. Needs `threads.create`.
  - `create-thread(request)`: `workspace`, `provider?`, `model?`, `title?`, `options`
    (model settings by id), `permission-mode?` and `origin?`. Needs `threads.create` and
    a workspace the plugin may act in. Settings are checked against the model, as in
    `thread.create`.
- Permission mode rules, enforced by the host:
  - A plugin can never create a `full_access` thread (`permission-denied`).
  - Strictness, from most to least strict: `plan`, `standard`, `accept_edits`,
    `full_access`.
  - `origin` is the thread the plugin acts for (for a tool, `tool-call.thread`). It must
    be in the same workspace. The new thread can't be looser than the origin; a looser
    mode fails with `permission-denied`. With no mode given, the new thread gets
    `standard`, or the origin's mode if that is stricter.
- The host keeps linking `kumikit:threads@0.1.0` for plugins already built against it. The
  Rust SDK's `threads` module moves to 0.2.0. Its function names don't change, so a plugin
  rebuilt with the new SDK needs a core that links 0.2.0.
- Two new tools in `kumikit.threads`:
  - `list_models` (`read`): the providers in the calling workspace's core, each with its
    models and their settings, or one provider's models.
  - `start_thread` (`write`, so it needs approval): starts a thread in the calling thread's
    workspace with a provider, model, settings, title and permission mode, all optional.
    `origin` is the calling thread. It sends an optional first message. The result names
    the new thread, so the agent can follow it with `wake_me { afterThread }` and
    `read_thread`.
  - The plugin also requests `threads.create`.

## Consequences

- Agents can fan work out to other providers and models with no kernel or provider
  special case. Users approve each start and can narrow or uninstall the plugin as before.
- A plugin can't use thread creation to get around the permission mode the user chose.
- `kumikit.threads` 0.2.0 asks for `threads.create`, which widens its grant. A bundled
  upgrade from 0.1.0 therefore waits, disabled, until the user grants the new set
  ([10-distribution](../spec/10-distribution.md)). Until then, all of its tools are gone.
- Spec changes: [03-kernel](../spec/03-kernel.md#client-interface) (the new operations and
  the mode rules) and [04-plugin-host](../spec/04-plugin-host.md#interfaces-wit) (the
  package table).
- Tests: the denied path for both new functions, the mode rules (`full_access` refused, a
  looser mode than the origin refused, the default clamped to the origin), invalid
  settings, and the plugin's input parsing.
- `kumikit:threads@0.2.0` goes into `wit/PUBLISHED` when it ships in a release.
