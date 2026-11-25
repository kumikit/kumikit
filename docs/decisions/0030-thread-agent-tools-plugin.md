# 0030 — Agent tools for threads in a first-party plugin

- **Status:** Accepted
- **Date:** 2026-11-24

## Context

Agents work better when they can act on threads themselves: wake themselves after
starting a build or a deploy, message another thread, read what another thread found,
queue or steer follow-ups, and suggest a title once the topic is clear.

The tool gateway ([ADR 0025](0025-tool-gateway.md)) serves tools contributed by plugins,
and the client interface ([03](../spec/03-kernel.md#client-interface)) lets a plugin act
like a client under granted operations. The kernel must not contain tools for one plugin
or name one ([ADR 0006](0006-kernel-owns-domain-model.md)).

## Decision

- A first-party core plugin `kumikit.threads` in its own repository,
  `kumikit/kumikit-plugin-threads`
  ([ADR 0026](0026-first-party-plugins-in-their-own-repositories.md)), built only on public
  SDKs.
- Tools (the calling thread comes from `tool-call.thread`):
  - `read` (no approval): `wake_me` (a deferred message to the calling thread, any
    trigger of [ADR 0027](0027-durable-queue-and-deferred-messages.md); command triggers
    still need their own approval), `list_threads`, `read_thread` (bounded and paged),
    `list_queue`, `suggest_title`.
  - `write` (approval): `message_thread`, `queue_message` and `steer_thread` when they
    target another thread, `cancel_deferred` for an entry the user set.
- The kernel adds general client operations, usable by any plugin with the grant:
  `threads.steer`, `threads.queue` (list, update, remove), `threads.defer` and
  `threads.title.suggest`.
- `Thread` gains `titleSource` (`auto`, `agent`, `user`). `threads.title.suggest` changes
  the title only while `titleSource` is not `user`; a rename by the user sets it to `user`.
- Limits: tool scope is the calling thread's workspace unless the plugin's `workspaces`
  grant is `all`; at most 10 pending agent-set deferred messages per thread.

## Consequences

- Agents get wake-ups and cross-thread work without a provider or kernel special case;
  users can uninstall or narrow it like any plugin.
- The new client operations need a `kumikit:host@0.3.0` `client` interface, with a test
  per operation proving the denied path.
- A new repository with the same CI, signing and registry workflow as the others.
