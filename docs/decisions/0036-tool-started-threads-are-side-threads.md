# 0036 — Threads a tool starts are side threads of the calling thread

- **Status:** Accepted
- **Date:** 2026-11-28
- **Amends:** [0034](0034-agents-start-threads.md) (the note that started threads are
  ordinary sidebar threads)

## Context

[ADR 0034](0034-agents-start-threads.md) made a started thread an ordinary sidebar thread,
linked to nothing. A user who asks an agent for a second opinion wants the new thread to
belong to the thread that asked for it. It should appear in that thread's side panel, not
in the sidebar, and it should follow that thread's workspace and context.

The kernel already has this: a side thread has a `parent` and `side = true`. It is hidden
from the default thread list, and each of its turns receives the parent's unseen context
(`/btw`, see [the protocol](../spec/02-protocol.md)). The side-thread panel in `ui-chat` lists
the side threads of the active thread.

## Decision

- A thread created through `create-thread` with an `origin` is a side thread of `origin`:
  `parent = origin` and `side = true`. A thread created with no `origin` is unchanged.
- The `kumikit.threads` tool `start_thread` always passes its calling thread as `origin`,
  so every thread it starts is a side thread of the thread that called it. No new option
  is added, and the WIT interface does not change.
- The kernel's inheritance rules for side threads apply to the fields the caller leaves
  out. Each is inherited from `origin` only when left out:
  - the provider and model, when no provider is named (a named provider takes the model
    from the list, or its first model);
  - the model settings, when no provider is named;
  - the worktree path.
- The permission mode is always set explicitly from the host's bounded mode, even when it
  is `standard`. The kernel would otherwise inherit the origin's mode, which can be
  looser than the one the host checked.
- Side threads are hidden from the default sidebar list. Their read, message, steer and
  wake-up tools work by thread id, as before.

## Consequences

- A thread started by a tool no longer appears in the sidebar. It is found in the **Side
  thread** panel of the thread that started it, and by id.
- A thread started by a tool from a worktree thread runs in that worktree, as a `/btw`
  side thread does.
- Plugins that call `create-thread` with an `origin` other than the calling thread get
  side threads too. Only the bundled `kumikit.threads` passes an `origin` today.
- The WIT, the protocol and the stored thread format do not change.
