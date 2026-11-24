# 0028 — Steering a running turn through a host input inbox

- **Status:** Proposed
- **Date:** 2026-11-24

## Context

Users want to correct an agent while it works ("use the existing helper instead") without
stopping it. Some agents accept input mid-turn: Claude Code reads stream-json user
messages on stdin during a turn; some ACP agents accept a second `session/prompt`. Others
cannot, and the message has to wait for the next turn.

In `kumikit:provider@0.2.0`, `send` runs the whole turn and holds the session's instance
([ADR 0010](0010-wasi-p2-and-session-instances.md)). The kernel cannot call into a busy
instance a second time. In `kumikit:host@0.2.0`, `child.read-stdout` blocks until output
arrives and offers no pollable, so a provider cannot wait for "output or new input". The
only interruption today is cancellation: the blocked import returns `cancelled` once.

Options:

1. A second export `steer(session, input)` called concurrently. Impossible with one
   instance per session and blocking calls in WASI 0.2.
2. Make blocking imports return early with a new error when input is waiting. Works, but
   overloads errors with control flow and surprises 0.2 plugins.
3. An input inbox in the host, with pollables, so the provider's turn loop waits on its
   process output and new input together through `wasi:io/poll`. `client.subscribe`
   already hands plugins a pollable, so the pattern exists.

## Decision

- **`kumikit:host@0.3.0`** (a copy of 0.2.0 plus these changes):
  - `process.child` gains `stdout-ready`, `stderr-ready` and `exited` pollables.
  - A new interface `turn` with `take-input: func() -> option<user-input>` and
    `input-ready: func() -> pollable`. It is available only inside `send`; outside a turn
    `take-input` returns none.
- **`kumikit:provider@0.3.0`** imports host 0.3.0 (and carries the handoff changes of
  [ADR 0029](0029-provider-switch-and-handoff.md)). The host runs 0.2 and 0.3 providers
  side by side.
- **Capability.** A model that can take input mid-turn reports `steer`
  ([05](../spec/05-providers.md#capabilities)).
- **Kernel.** `thread.steer { thread, input }` returns `{ status }`:
  - `accepted`: the turn runs and its model has `steer`; the input is in the inbox. When
    the provider takes it, the kernel appends `input.steered { turn, input, entry? }`, so
    the transcript shows it where the agent saw it.
  - `queued`: the model has no `steer`; the input becomes the first queue entry
    ([ADR 0027](0027-durable-queue-and-deferred-messages.md)).
  - `conflict` when no turn runs (use `thread.send`).
  Inputs left in the inbox when the turn ends move to the head of the queue with
  `queue.added { reason: "not_steered" }`. Only text is steerable; inputs with
  attachments are queued.
- **Client interface.** A `threads.steer` operation, so bots and tools can steer.
- **Providers.** `provider-claude` writes a stream-json user message to stdin.
  `provider-acp` reports `steer` only for agents whose manifest entry sets `steer = true`
  and then sends a second `session/prompt`; none do by default.
- **SDK.** `sdk/rust` wraps `take-input` and a helper that polls a child's output and the
  inbox together.

## Consequences

- Steering needs no kernel knowledge of any agent; a provider opts in with one capability.
- Providers that stay on 0.2 keep working; their users get "queued" instead of a steer.
- `kumikit:host@0.3.0` and `kumikit:provider@0.3.0` are new published versions; they are
  added to `wit/PUBLISHED` when merged. This happens before the 1.0 freeze
  ([ADR 0020](0020-interface-freeze-after-m5.md)), which should review them.
- Process pollables also let other plugins wait on several children at once.
- Tests: a recorded Claude Code transcript with a steer replays through the adapter; the
  log shows `input.steered` before the later text; a provider without `steer` gets
  `queued`; inbox drained to the queue on turn end; the denied path for `threads.steer`.
