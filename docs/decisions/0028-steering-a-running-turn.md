# 0028 — Steering a running turn through a host input inbox

- **Status:** Accepted
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

- **`kumikit:turn@0.1.0`** (an additive package alongside `kumikit:host@0.2.0` and
  `kumikit:provider@0.2.0`, so existing 0.2 worlds stay untouched):
  - `interface input` with `take: func() -> result<option<user-input>, error>` (non-blocking)
    and `read-stdout: func(child: borrow<child>, max: u32) -> result<read, error>`, which
    races the child's stdout against the turn's input inbox inside the host (preferring
    waiting input over stdout so streaming output cannot starve steering) and returns
    `stdout(bytes)`, `eof`, or `input(user-input)`. Available only inside `send`; outside a
    turn `take` returns none.
  - `world provider` (`include kumikit:provider/plugin@0.2.0; import input;`), listed next
    to `kumikit:provider/plugin@0.2.0` in a steering provider's `runtime.worlds`, and
    `world steering` for SDKs that add the import alongside 0.2 bindings.
  - `interface client` with `steer(thread, text) -> result<steer-result, error>` (and
    `world client-steering`), gated by `threads.steer` in `permissions.client`, so bots and
    tools can steer running turns.
- **Capability.** A model that can take input mid-turn reports `steer`
  ([05](../spec/05-providers.md#capabilities)).
- **Kernel.** `thread.steer { thread, input }` returns `{ status, turn, entry? }`:
  - `accepted`: the turn runs and its model has `steer`; the input is in the inbox. When
    the provider takes it, the kernel appends `input.steered { turn, input }`, so
    the transcript shows it where the agent saw it.
  - `queued`: the model has no `steer` (or the input has attachments); the input becomes
    the first queue entry with `queue.added { front: true, reason: "not_steered" }`
    ([ADR 0027](0027-durable-queue-and-deferred-messages.md)).
  - `conflict` when no turn runs (use `thread.send`) or the thread is archived.
  Inputs left in the inbox when the turn ends move to the head of the queue with
  `queue.added { front: true, reason: "not_steered" }`. Only text is steerable; inputs with
  attachments are queued.
- **Providers.** `provider-claude` writes a stream-json user message to stdin.
  `provider-acp` reports `steer` only for agents whose manifest entry sets `steer = true`
  and then sends a second `session/prompt`; none do by default.
- **SDK.** `kumikit-provider` wraps `steering::{take, read_stdout, SteerableLines}`;
  `kumikit-plugin` wraps `turn::steer`.

## Consequences

- Steering needs no kernel knowledge of any agent; a provider opts in with one capability.
- Providers that stay on 0.2 keep working without recompiling; their users get "queued"
  instead of a steer.
- `kumikit:turn@0.1.0` is a new published WIT package; it is added to `wit/PUBLISHED` when
  merged. This happens before the 1.0 freeze
  ([ADR 0020](0020-interface-freeze-after-m5.md)), which should review it.
- Racing stdout and input inside the host avoids duplicating the entire `kumikit:host@0.2.0`
  package just to add steering while keeping the one-call-at-a-time session rule.
- Tests: a recorded Claude Code transcript with a steer replays through the adapter; an ACP
  agent with `steer = true` takes mid-turn input via `fake-acp-agent`; the log shows
  `input.steered` before the later text; a provider without `steer` gets `queued`; inbox
  drained to the queue on turn end; the denied path for `threads.steer`.
